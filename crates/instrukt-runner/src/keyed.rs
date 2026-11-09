//! Durable keyed dispatch. Callers hold the machine runner lock across operations.
//! Key ownership is kept in SQLite; no writer transaction spans runtime I/O.
use instrukt_core::fsx::LockGuard;
use instrukt_core::home::Home;
use instrukt_core::ids::SessionRef;
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use jiff::Timestamp;
use std::time::Duration;

use crate::runtime::{RuntimeAdapter, RuntimeStart, Thread};
use crate::sessions::SessionsError;
use crate::state::{self, RunnerState, StateError};

pub fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
pub fn request_id() -> Result<String, SessionsError> {
    instrukt_core::machine::random_hex(16)
        .map_err(|e| SessionsError::Refused(format!("cannot allocate request identity: {e}")))
}
pub fn standing_key(procedure: &str, project: &str) -> String {
    format!(
        "instrukt:standing:{}@{}",
        encode(procedure),
        encode(&format!("project:{project}"))
    )
}
pub fn todo_key(project: &str, slug: &str, role: &str, attempt: u64) -> String {
    format!(
        "instrukt:todo:{}/{}:{}:{attempt}",
        encode(project),
        encode(slug),
        encode(role)
    )
}
pub fn task_key(project: &str, slug: &str, occurrence: Timestamp) -> String {
    format!(
        "instrukt:task:{}/{}:{}",
        encode(project),
        encode(slug),
        encode(&occurrence.to_string())
    )
}

/// A live machine invocation. Construction recovers dead ownership once;
/// its private OS guard proves that no previous invocation is still dispatching.
pub struct RuntimeInvocation {
    home: Home,
    owner: String,
    _guard: LockGuard,
}
impl RuntimeInvocation {
    pub fn acquire(home: &Home, wait: Duration) -> Result<Self, StateError> {
        Self::from_guard(
            home,
            LockGuard::acquire(&home.state_dir().join("runtime-dispatch.lock"), wait)?,
        )
    }
    pub fn try_acquire(home: &Home) -> Result<Option<Self>, StateError> {
        LockGuard::try_acquire(&home.state_dir().join("runtime-dispatch.lock"))?
            .map(|guard| Self::from_guard(home, guard))
            .transpose()
    }
    fn from_guard(home: &Home, guard: LockGuard) -> Result<Self, StateError> {
        let owner = instrukt_core::machine::random_hex(16).map_err(|source| StateError::Read {
            path: "/dev/urandom".into(),
            source,
        })?;
        RunnerState::new(home).write::<_, StateError>(|tx| {
            tx.execute(
                "UPDATE runtime_key_locks SET owner=NULL WHERE owner IS NOT NULL",
                [],
            )
            .map_err(state::sql(&state::db_path(tx)))?;
            Ok(())
        })?;
        Ok(Self {
            home: home.clone(),
            owner,
            _guard: guard,
        })
    }
    pub fn home(&self) -> &Home {
        &self.home
    }
}

pub(crate) struct KeyGuard {
    state: RunnerState,
    runtime: String,
    key: String,
    owner: String,
}
impl KeyGuard {
    pub(crate) fn claim(
        invocation: &RuntimeInvocation,
        adapter: &dyn RuntimeAdapter,
        key: &str,
        project: Option<&str>,
        now: Timestamp,
    ) -> Result<Self, SessionsError> {
        let state = RunnerState::new(invocation.home());
        let owner = invocation.owner.clone();
        let runtime = adapter.runtime().name().to_owned();
        let acquired=state.write::<_,StateError>(|tx| {
            tx.execute("INSERT INTO runtime_key_locks (runtime,semantic_key,project,owner,acquired_at) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(runtime,semantic_key) DO UPDATE SET owner=excluded.owner,acquired_at=excluded.acquired_at WHERE runtime_key_locks.owner IS NULL",params![runtime,key,project,owner,now.to_string()]).map_err(state::sql(&state::db_path(tx)))
        })?;
        if acquired != 1 {
            return Err(SessionsError::Refused(format!(
                "runtime key `{key}` is already owned by an active dispatch"
            )));
        }
        Ok(Self {
            state,
            runtime,
            key: key.into(),
            owner,
        })
    }
}
impl Drop for KeyGuard {
    fn drop(&mut self) {
        let result = self.state.write::<_,StateError>(|tx| {
            tx.execute("UPDATE runtime_key_locks SET owner=NULL WHERE runtime=?1 AND semantic_key=?2 AND owner=?3", params![self.runtime,self.key,self.owner]).map_err(state::sql(&state::db_path(tx)))?;
            Ok(())
        });
        if let Err(error) = result {
            tracing::error!(event="runtime_key_release_failed", key=%self.key, error=%error);
        }
    }
}
#[derive(Default)]
struct Recorded {
    request: Option<RuntimeStart>,
    unresolved: bool,
    retry_allowed: bool,
    bound: Option<SessionRef>,
    accepted: bool,
    project: Option<String>,
    project_id: Option<String>,
}
fn record(state: &RunnerState, runtime: &str, key: &str) -> Result<Recorded, StateError> {
    Ok(state.read(|conn,path| {
        let start=conn.query_row("SELECT request,state,project FROM runtime_starts WHERE runtime=?1 AND semantic_key=?2",params![runtime,key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?))).optional().map_err(state::sql(path))?;
        let bound=conn.query_row("SELECT session,accepted,request,project,project_id FROM runtime_bindings WHERE runtime=?1 AND semantic_key=?2",params![runtime,key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?))).optional().map_err(state::sql(path))?;
        let encoded=start.as_ref().map(|r|&r.0).or_else(||bound.as_ref().and_then(|r|r.2.as_ref()));
        Ok(Recorded {
            request:encoded.map(|s|serde_json::from_str(s).map_err(|e|state::invalid(path,e))).transpose()?,
            unresolved:start.as_ref().is_some_and(|r|r.1=="unresolved"),
            retry_allowed:start.as_ref().is_some_and(|r|r.1=="retry_allowed"),
            bound:bound.as_ref().map(|r|SessionRef::parse(&r.0).map_err(|e|state::invalid(path,e))).transpose()?,
            accepted:bound.as_ref().is_some_and(|r|r.1),
            project:start.and_then(|r|r.2).or_else(||bound.as_ref().and_then(|r|r.3.clone())),
            project_id:bound.and_then(|r|r.4),
        })
    })?.unwrap_or_default())
}
fn kind(key: &str) -> &str {
    key.strip_prefix("instrukt:")
        .and_then(|s| s.split(':').next())
        .unwrap_or("run")
}
#[derive(Clone, Copy)]
enum RequestEvidence {
    Retained,
    Unknown,
}

fn remember(
    state: &RunnerState,
    adapter: &dyn RuntimeAdapter,
    input: &RuntimeStart,
    thread: &Thread,
    project: Option<&str>,
    accepted: bool,
    evidence: RequestEvidence,
) -> Result<(), StateError> {
    state.write(|tx| {
        let path = state::db_path(tx);
        let request=match evidence {RequestEvidence::Retained=>Some(serde_json::to_string(input).map_err(|e|state::invalid(&path,e))?),RequestEvidence::Unknown=>None};
        tx.execute("INSERT INTO runtime_bindings (runtime,semantic_key,kind,project,subject,session,project_id,checkout,worktree,branch,base_head,accepted,request,display_title) VALUES (?1,?2,?3,?4,?2,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(runtime,semantic_key) DO UPDATE SET session=excluded.session,project_id=COALESCE(excluded.project_id,runtime_bindings.project_id),worktree=excluded.worktree,accepted=excluded.accepted,display_title=excluded.display_title",params![adapter.runtime().name(),input.key,kind(&input.key),project,thread.session.to_string(),thread.project_id,input.checkout.to_string_lossy(),thread.worktree.as_ref().map(|p|p.to_string_lossy()),thread.branch,input.worktree.as_ref().map(|w|&w.base),accepted,request,thread.title]).map_err(state::sql(&path))?;
        if accepted { tx.execute("DELETE FROM runtime_starts WHERE runtime=?1 AND semantic_key=?2",params![adapter.runtime().name(),input.key]).map_err(state::sql(&path))?; }
        Ok(())
    })
}

/// Discover before every start. Empty discovery cannot clear an ambiguous marker.
pub(crate) fn start(
    invocation: &RuntimeInvocation,
    adapter: &dyn RuntimeAdapter,
    input: &RuntimeStart,
    project: Option<&str>,
    _wake: bool,
    now: Timestamp,
    creation: crate::runtime::CreationPermit,
) -> Result<Thread, SessionsError> {
    let guard = KeyGuard::claim(invocation, adapter, &input.key, project, now)?;
    let mut recorded = record(&guard.state, &guard.runtime, &input.key)?;
    let mut resolved = input.clone();
    if let Some(retained) = &recorded.request {
        // The retained model choice stays authoritative through ambiguous or
        // accepted starts, including a caller holding an earlier request copy.
        resolved.title = retained.title.clone();
        resolved.selection = retained.selection.clone();
        resolved.selection_evidence = retained.selection_evidence.clone();
        if &resolved != retained {
            return Err(SessionsError::ChangedRequest(input.key.clone()));
        }
    }
    if recorded.retry_allowed && recorded.bound.is_none() && !resolved.preferences.is_empty() {
        let config =
            instrukt_core::config::Configuration::load_at(invocation.home(), &resolved.checkout)?;
        resolved.selection =
            crate::agents::selections(&config, invocation.home(), &resolved.preferences, now)?
                .into_iter()
                .next();
        let request = serde_json::to_string(&resolved)
            .map_err(|error| SessionsError::Refused(error.to_string()))?;
        guard.state.write::<_,StateError>(|tx| {tx.execute("UPDATE runtime_starts SET request=?3 WHERE runtime=?1 AND semantic_key=?2 AND state='retry_allowed'",params![guard.runtime,resolved.key,request]).map_err(state::sql(&state::db_path(tx)))?;Ok(())})?;
        recorded.request = Some(resolved.clone());
    }
    let input = &resolved;
    let fresh = recorded.request.is_none()
        && recorded.bound.is_none()
        && creation == crate::runtime::CreationPermit::Initial;
    if recorded.request.is_none() && recorded.bound.is_none() {
        guard.state.write::<_,StateError>(|tx| {
            let path=state::db_path(tx); let request=serde_json::to_string(input).map_err(|e|state::invalid(&path,e))?;
            tx.execute("INSERT INTO runtime_starts (runtime,semantic_key,project,kind,subject,checkout,brief,request,begun_at,reported,state) VALUES (?1,?2,?3,?4,?2,?5,?6,?7,?8,0,'unresolved')",params![guard.runtime,input.key,project,kind(&input.key),input.checkout.to_string_lossy(),input.brief,request,now.to_string()]).map_err(state::sql(&path))?; Ok(())
        })?;
        recorded = record(&guard.state, &guard.runtime, &input.key)?;
    }
    let listed = match adapter.list(&input.checkout) {
        Ok(listed) => listed,
        Err(error) => {
            if fresh {
                guard.state.write::<_,StateError>(|tx| {tx.execute("UPDATE runtime_starts SET state='retry_allowed',resolved_at=?3,resolved_by='instrukt',resolution='discovery_rejected_before_creation' WHERE runtime=?1 AND semantic_key=?2",params![guard.runtime,input.key,now.to_string()]).map_err(state::sql(&state::db_path(tx)))?;Ok(())})?;
            }
            return Err(SessionsError::Refused(format!(
                "discovery for `{}` failed before a new start: {error}; retry the retained start key",
                input.key
            )));
        }
    };

    let mut matches = listed
        .into_iter()
        .filter(|t| t.semantic_key.as_deref() == Some(input.key.as_str()));
    let found = matches.next();
    if matches.next().is_some() {
        return Err(SessionsError::KeyConflict(input.key.clone()));
    }
    if let Some(found) = found {
        if recorded.bound.as_ref().is_some_and(|s| s != &found.session) {
            return Err(SessionsError::KeyConflict(input.key.clone()));
        }
        let thread = adapter.read_state(&found.session)?;
        if crate::runtime::project_conflict(&thread.project_id, &found.project_id)
            || crate::runtime::project_conflict(&recorded.project_id, &thread.project_id)
            || crate::runtime::project_conflict(&recorded.project_id, &found.project_id)
            || thread.semantic_key.as_deref() != Some(input.key.as_str())
        {
            return Err(SessionsError::KeyConflict(input.key.clone()));
        }
        adapter.verify_start(input, &thread)?;
        remember(
            &guard.state,
            adapter,
            input,
            &thread,
            project,
            recorded.accepted,
            RequestEvidence::Retained,
        )?;
        if !recorded.accepted {
            let supplied = adapter
                .observe_input(&thread.session, &input.brief, now)?
                .state
                != crate::runtime::InputState::Unknown;
            if !supplied {
                adapter.send(&thread.session, &input.brief)?;
            }
            remember(
                &guard.state,
                adapter,
                input,
                &thread,
                project,
                true,
                RequestEvidence::Retained,
            )?;
            crate::inputs::record_initial(
                &guard.state,
                &thread.session,
                &format!("assignment:{}", input.key),
                &input.brief,
                now,
            )?;
        }
        return Ok(thread);
    }
    if let Some(session) = recorded.bound {
        return Err(SessionsError::MissingBound {
            key: input.key.clone(),
            session,
        });
    }
    if recorded.unresolved && !fresh {
        return Err(SessionsError::Unresolved {
            key: input.key.clone(),
        });
    }
    let prepared = match crate::agents::prepare_start(invocation.home(), adapter, input, now) {
        Ok(prepared) => prepared,
        Err(error) => {
            guard.state.write::<_,StateError>(|tx| {tx.execute("UPDATE runtime_starts SET state='retry_allowed',resolved_at=?3,resolved_by='instrukt',resolution='readiness_refused_before_creation' WHERE runtime=?1 AND semantic_key=?2",params![guard.runtime,input.key,now.to_string()]).map_err(state::sql(&state::db_path(tx)))?;Ok(())})?;
            return Err(error);
        }
    };
    let input = &prepared;
    guard.state.write::<_,StateError>(|tx| {
        let path=state::db_path(tx);
        let request=serde_json::to_string(input).map_err(|e|state::invalid(&path,e))?;
        tx.execute("INSERT INTO runtime_starts (runtime,semantic_key,project,kind,subject,checkout,brief,request,begun_at,reported,state) VALUES (?1,?2,?3,?4,?2,?5,?6,?7,?8,0,'unresolved') ON CONFLICT(runtime,semantic_key) DO UPDATE SET state='unresolved',request=excluded.request,reported=0,resolved_at=NULL,resolved_by=NULL,resolution=NULL",params![guard.runtime,input.key,project,kind(&input.key),input.checkout.to_string_lossy(),input.brief,request,now.to_string()]).map_err(state::sql(&path))?;
        Ok(())
    })?;
    let thread = adapter
        .create_thread(input)
        .map_err(|e| SessionsError::Uncertain {
            key: input.key.clone(),
            detail: e.to_string(),
        })?;
    remember(
        &guard.state,
        adapter,
        input,
        &thread,
        project,
        true,
        RequestEvidence::Retained,
    )?;
    crate::inputs::record_initial(
        &guard.state,
        &thread.session,
        &format!("assignment:{}", input.key),
        &input.brief,
        now,
    )?;
    Ok(thread)
}

/// Repair a lost cache only after the caller recovered the assignment's original facts.
pub(crate) fn recover_binding(
    adapter: &dyn RuntimeAdapter,
    assignment: &crate::runtime::RuntimeAssignment<'_>,
    thread: &Thread,
) -> Result<(), SessionsError> {
    let guard = KeyGuard::claim(
        assignment.invocation,
        adapter,
        &assignment.request.key,
        assignment.project,
        assignment.now,
    )?;
    adapter.verify_start(assignment.request, thread)?;
    remember(
        &guard.state,
        adapter,
        assignment.request,
        thread,
        assignment.project,
        true,
        RequestEvidence::Retained,
    )?;
    Ok(())
}

/// Resolve only a recorded ambiguous request; a named session must appear in
/// a successful keyed listing and satisfy the original worktree request.
pub fn resolve(
    invocation: &RuntimeInvocation,
    adapter: &dyn RuntimeAdapter,
    key: &str,
    session: Option<&SessionRef>,
    now: Timestamp,
) -> Result<(), SessionsError> {
    let guard = KeyGuard::claim(invocation, adapter, key, None, now)?;
    let recorded = record(&guard.state, &guard.runtime, key)?;
    let input = recorded
        .request
        .ok_or_else(|| SessionsError::Refused(format!("`{key}` has no recorded start")))?;
    if !recorded.unresolved {
        return Err(SessionsError::Refused(format!(
            "`{key}` has no unresolved start"
        )));
    }
    if let Some(session) = session {
        if recorded
            .bound
            .as_ref()
            .is_some_and(|bound| bound != session)
        {
            return Err(SessionsError::KeyConflict(key.into()));
        }
        let matches: Vec<_> = adapter
            .list(&input.checkout)?
            .into_iter()
            .filter(|t| t.semantic_key.as_deref() == Some(key))
            .collect();
        if matches.len() != 1 || matches[0].session != *session {
            return Err(SessionsError::KeyConflict(key.into()));
        }
        let thread = adapter.read_state(session)?;
        if crate::runtime::project_conflict(&thread.project_id, &matches[0].project_id) {
            return Err(SessionsError::KeyConflict(key.into()));
        }
        adapter.verify_start(&input, &thread)?;
        remember(
            &guard.state,
            adapter,
            &input,
            &thread,
            recorded.project.as_deref(),
            false,
            RequestEvidence::Retained,
        )?;
        // Retain the marker until the original input is accepted.
    } else {
        if recorded.bound.is_some() {
            return Err(SessionsError::Refused(
                "a delivery target is already bound; retry its pending input".into(),
            ));
        }
        guard.state.write::<_,StateError>(|tx| { tx.execute("UPDATE runtime_starts SET state='retry_allowed',resolved_at=?3,resolved_by='operator',resolution='none' WHERE runtime=?1 AND semantic_key=?2",params![guard.runtime,key,now.to_string()]).map_err(state::sql(&state::db_path(tx)))?; Ok(()) })?;
    }
    Ok(())
}

pub fn retry(
    invocation: &RuntimeInvocation,
    adapter: &dyn RuntimeAdapter,
    key: &str,
    now: Timestamp,
) -> Result<Thread, SessionsError> {
    let recorded = record(
        &RunnerState::new(invocation.home()),
        adapter.runtime().name(),
        key,
    )?;
    let input = recorded
        .request
        .ok_or_else(|| SessionsError::Refused(format!("`{key}` has no recorded request")))?;
    start(
        invocation,
        adapter,
        &input,
        recorded.project.as_deref(),
        false,
        now,
        crate::runtime::CreationPermit::Reconcile,
    )
}

pub fn followup(
    adapter: &dyn RuntimeAdapter,
    assignment: &crate::runtime::FollowupAssignment<'_>,
) -> Result<Thread, SessionsError> {
    let input = assignment.request;
    let guard = KeyGuard::claim(
        assignment.invocation,
        adapter,
        &input.key,
        assignment.project,
        assignment.now,
    )?;
    let recorded = record(&guard.state, &guard.runtime, &input.key)?;
    if recorded
        .bound
        .as_ref()
        .is_some_and(|bound| bound != assignment.session)
    {
        return Err(SessionsError::KeyConflict(input.key.clone()));
    }
    let found: Vec<_> = adapter
        .list(&input.checkout)?
        .into_iter()
        .filter(|t| t.semantic_key.as_deref() == Some(input.key.as_str()))
        .collect();
    if found.is_empty() {
        return Err(SessionsError::MissingBound {
            key: input.key.clone(),
            session: assignment.session.clone(),
        });
    }
    if found.len() != 1 || &found[0].session != assignment.session {
        return Err(SessionsError::KeyConflict(input.key.clone()));
    }
    let thread = adapter.read_state(assignment.session)?;
    if crate::runtime::project_conflict(&thread.project_id, &found[0].project_id)
        || crate::runtime::project_conflict(&recorded.project_id, &thread.project_id)
        || crate::runtime::project_conflict(&recorded.project_id, &found[0].project_id)
        || assignment
            .worktree_path
            .is_some_and(|path| thread.worktree.as_deref() != Some(path))
    {
        return Err(SessionsError::KeyConflict(input.key.clone()));
    }
    let mut verification = input.clone();
    if let Some(original) = &recorded.request {
        verification.title = original.title.clone();
    }
    adapter.verify_start(&verification, &thread)?;
    if recorded.bound.is_none() {
        remember(
            &guard.state,
            adapter,
            input,
            &thread,
            assignment.project,
            true,
            RequestEvidence::Unknown,
        )?;
    }
    use sha2::{Digest, Sha256};
    let id = format!(
        "followup:{}:{:x}",
        input.key,
        Sha256::digest(input.brief.as_bytes())
    );
    crate::inputs::enqueue(
        assignment.invocation,
        adapter,
        assignment.session,
        &crate::inputs::Entry {
            id: &id,
            kind: crate::inputs::InputKind::Recovery,
            assignment: None,
            source_at: Some(&assignment.now.to_string()),
            arrived_at: assignment.now,
            intent: crate::inputs::Intent::Asap,
            content: &input.brief,
        },
    )?;
    Ok(thread)
}

/// An occurrence chosen before an interrupted dispatch wins over a new due time.
pub fn pending_task(
    home: &Home,
    project: &str,
    slug: &str,
    last: Option<Timestamp>,
) -> Result<Option<Timestamp>, StateError> {
    let state = RunnerState::new(home);
    let id = state
        .read(|conn, _| crate::state::todo_id(conn, project, slug))?
        .flatten();
    let Some(id) = id else {
        return Ok(None);
    };
    let prefix = format!("instrukt:task:{}/{}:", encode(project), encode(&id));
    let keys=RunnerState::new(home).read(|conn,path| {
        let mut q=conn.prepare("SELECT semantic_key FROM runtime_starts WHERE kind='task' UNION SELECT semantic_key FROM runtime_bindings WHERE kind='task'").map_err(state::sql(path))?;
        q.query_map([],|r|r.get::<_,String>(0)).map_err(state::sql(path))?.collect::<Result<Vec<_>,_>>().map_err(state::sql(path))
    })?.unwrap_or_default();
    let mut pending = Vec::new();
    for key in keys {
        if let Some(value) = key.strip_prefix(&prefix) {
            let value = value.replace("%3A", ":").replace("%2B", "+");
            let occurrence = value
                .parse::<Timestamp>()
                .map_err(|e| state::invalid(&home.state_db_file(), e))?;
            if last.is_none_or(|last| occurrence > last) {
                pending.push(occurrence);
            }
        }
    }
    Ok(pending.into_iter().min())
}

pub fn pending_run(home: &Home, project: &str) -> Result<Option<String>, StateError> {
    Ok(RunnerState::new(home).read(|conn,path|conn.query_row("SELECT semantic_key FROM runtime_starts WHERE kind='run' AND project=?1 ORDER BY begun_at LIMIT 1",[project],|r|r.get(0)).optional().map_err(state::sql(path)))?.flatten())
}
pub(crate) fn retained_request(
    home: &Home,
    adapter: &dyn RuntimeAdapter,
    key: &str,
) -> Result<Option<RuntimeStart>, SessionsError> {
    Ok(record(&RunnerState::new(home), adapter.runtime().name(), key)?.request)
}
pub fn request(
    home: &Home,
    adapter: &dyn RuntimeAdapter,
    key: &str,
) -> Result<RuntimeStart, SessionsError> {
    record(&RunnerState::new(home), adapter.runtime().name(), key)?
        .request
        .ok_or_else(|| SessionsError::Refused(format!("`{key}` has no recorded request")))
}

/// Background retries retain their cause without repeating one unresolved-start finding.
pub(crate) fn report_start_finding(
    state: &RunnerState,
    runtime: Option<instrukt_core::config::Runtime>,
    error: &SessionsError,
) -> bool {
    let Some(runtime) = runtime else {
        return true;
    };
    let key = match error {
        SessionsError::Unresolved { key } | SessionsError::Uncertain { key, .. } => key,
        _ => return true,
    };
    let result = state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        let reported = tx
            .query_row(
                "SELECT reported FROM runtime_starts WHERE semantic_key=?1 AND runtime=?2",
                params![key, runtime.name()],
                |r| r.get::<_, bool>(0),
            )
            .optional()
            .map_err(state::sql(&path))?;
        if reported == Some(true) {
            return Ok(false);
        }
        tx.execute(
            "UPDATE runtime_starts SET reported=1 WHERE semantic_key=?1 AND runtime=?2",
            params![key, runtime.name()],
        )
        .map_err(state::sql(&path))?;
        Ok(true)
    });
    match result {
        Ok(report) => report,
        Err(failure) => {
            tracing::error!(event="start_finding_retention_failed",key,error=%failure);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn components_encode_utf8_with_uppercase_hex() {
        assert_eq!(
            encode("sdlc/procedure/é@demo"),
            "sdlc%2Fprocedure%2F%C3%A9%40demo"
        );
    }
    #[test]
    fn occurrence_keys_normalize_offsets_and_fractional_seconds() {
        assert_eq!(
            task_key(
                "demo",
                "daily",
                "2026-11-05T10:00:00.1000+01:00".parse().unwrap()
            ),
            "instrukt:task:demo/daily:2026-11-05T09%3A00%3A00.1Z"
        );
    }
}
