//! Durable keyed dispatch. Exact-key OS ownership spans native operations.
//! Key ownership is kept in SQLite; no writer transaction spans runtime I/O.
use instrukt_core::fsx::LockGuard;
use instrukt_core::home::Home;
use instrukt_core::ids::SessionRef;
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use jiff::Timestamp;
use std::time::Duration;

use crate::runtime::{
    CreationPermit, Presence, RuntimeAdapter, RuntimeAssignment, RuntimeStart, Thread,
};

mod _findings;
use crate::sessions::SessionsError;
use crate::state::{self, RunnerState, StateError};
pub(crate) use _findings::{note, notify_due, status};

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

/// Admission to a dispatch. The machine gate is released on admission;
/// exact-key ownership protects the subsequent runtime calls.
pub struct RuntimeInvocation {
    home: Home,
    owner: String,
}
impl RuntimeInvocation {
    pub fn acquire(home: &Home, wait: Duration) -> Result<Self, StateError> {
        let _gate = LockGuard::acquire(&home.state_dir().join("runtime-dispatch.lock"), wait)?;
        Self::admitted(home)
    }
    pub fn try_acquire(home: &Home) -> Result<Option<Self>, StateError> {
        let Some(_gate) = LockGuard::try_acquire(&home.state_dir().join("runtime-dispatch.lock"))?
        else {
            return Ok(None);
        };
        Self::admitted(home).map(Some)
    }
    fn admitted(home: &Home) -> Result<Self, StateError> {
        let owner = instrukt_core::machine::random_hex(16).map_err(|source| StateError::Read {
            path: "/dev/urandom".into(),
            source,
        })?;
        Ok(Self {
            home: home.clone(),
            owner,
        })
    }
    pub fn home(&self) -> &Home {
        &self.home
    }
}

/// Exclusive ownership of one key, including across processes.
pub struct KeyGuard {
    state: RunnerState,
    runtime: String,
    key: String,
    owner: String,
    _lock: LockGuard,
}
impl KeyGuard {
    pub fn claim(
        invocation: &RuntimeInvocation,
        adapter: &dyn RuntimeAdapter,
        key: &str,
        project: Option<&str>,
        now: Timestamp,
    ) -> Result<Self, SessionsError> {
        Self::claim_inner(invocation, adapter, key, project, now, false)
    }

    fn claim_for_recovery(
        invocation: &RuntimeInvocation,
        adapter: &dyn RuntimeAdapter,
        key: &str,
        now: Timestamp,
    ) -> Result<Self, SessionsError> {
        Self::claim_inner(invocation, adapter, key, None, now, true)
    }

    fn claim_inner(
        invocation: &RuntimeInvocation,
        adapter: &dyn RuntimeAdapter,
        key: &str,
        project: Option<&str>,
        now: Timestamp,
        recovery: bool,
    ) -> Result<Self, SessionsError> {
        let state = RunnerState::new(invocation.home());
        let owner = invocation.owner.clone();
        let runtime = adapter.runtime().name().to_owned();
        use sha2::{Digest, Sha256};
        let name = Sha256::digest(format!("{runtime}:{key}").as_bytes());
        let path = invocation
            .home()
            .locks_dir()
            .join(format!("runtime-key-{name:x}.lock"));
        let lock = match LockGuard::try_acquire(&path).map_err(StateError::from)? {
            Some(lock) => lock,
            None if recovery => {
                #[cfg(debug_assertions)]
                if instrukt_core::fsx::fence_active()
                    && let Some(marker) = std::env::var_os("INSTRUKT_TEST_RECOVERY_WAIT")
                {
                    let _ = instrukt_core::fsx::atomic_write(
                        std::path::Path::new(&marker),
                        key.as_bytes(),
                    );
                }
                LockGuard::acquire(&path, Duration::from_secs(120)).map_err(StateError::from)?
            }
            None => {
                return Err(SessionsError::Refused(format!(
                    "runtime key `{key}` is already owned by an active dispatch"
                )));
            }
        };
        state.write::<_,StateError>(|tx| {
            tx.execute("INSERT INTO runtime_key_locks (runtime,semantic_key,project,owner,acquired_at) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(runtime,semantic_key) DO UPDATE SET owner=excluded.owner,acquired_at=excluded.acquired_at",params![runtime,key,project,owner,now.to_string()]).map_err(state::sql(&state::db_path(tx)))?;
            Ok(())
        })?;
        Ok(Self {
            state,
            runtime,
            key: key.into(),
            owner,
            _lock: lock,
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
    replacement: Option<ReplacementIntent>,
    replaced_session: Option<SessionRef>,
    opening_acknowledged: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Recovery<'a> {
    ResolveNone,
    ResolveSession,
    ReplaceDeleted,
    ResumeReplacement(&'a SessionRef),
}

impl Recovery<'_> {
    fn command(self, key: &str, session: &SessionRef) -> String {
        match self {
            Self::ResolveNone => format!("instrukt sessions resolve-start {key} --none"),
            Self::ResolveSession => {
                format!("instrukt sessions resolve-start {key} --session {session}")
            }
            Self::ReplaceDeleted => {
                format!("instrukt sessions replace-deleted {key} --stopped {session}")
            }
            Self::ResumeReplacement(stopped) => {
                format!("instrukt sessions replace-deleted {key} --stopped {stopped}")
            }
        }
    }

    fn refusal(self, key: &str, session: &SessionRef) -> SessionsError {
        match self {
            Self::ResolveNone => SessionsError::FirstStartAbsent {
                key: key.into(),
                session: session.clone(),
            },
            Self::ResolveSession => SessionsError::PendingStartPresent {
                key: key.into(),
                session: session.clone(),
            },
            Self::ReplaceDeleted => deleted(key, session),
            Self::ResumeReplacement(stopped) => SessionsError::PendingReplacement {
                key: key.into(),
                session: session.clone(),
                stopped: Box::new(stopped.clone()),
            },
        }
    }
}

impl Recorded {
    fn absent_recovery(&self) -> Recovery<'_> {
        if self.accepted || self.replacement.is_some() {
            Recovery::ReplaceDeleted
        } else {
            Recovery::ResolveNone
        }
    }

    fn recovery(&self, presence: Presence) -> Option<Recovery<'_>> {
        match presence {
            Presence::Absent => Some(self.absent_recovery()),
            Presence::Present if !self.accepted => Some(match &self.replacement {
                Some(intent) => Recovery::ResumeReplacement(&intent.stopped),
                None => Recovery::ResolveSession,
            }),
            Presence::Present | Presence::Archived => None,
        }
    }

    fn absent(&self, key: &str, session: &SessionRef) -> SessionsError {
        self.absent_recovery().refusal(key, session)
    }
}
struct ReplacementIntent {
    original: SessionRef,
    stopped: SessionRef,
}
fn record(state: &RunnerState, runtime: &str, key: &str) -> Result<Recorded, StateError> {
    Ok(state.read(|conn,path| {
        let start=conn.query_row("SELECT request,state,project,resolution FROM runtime_starts WHERE runtime=?1 AND semantic_key=?2",params![runtime,key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?))).optional().map_err(state::sql(path))?;
        let bound=conn.query_row("SELECT session,accepted,request,project,project_id,json_extract(request,'$.replaced_session'),coalesce(json_extract(request,'$.opening_acknowledged'),0) FROM runtime_bindings WHERE runtime=?1 AND semantic_key=?2",params![runtime,key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,bool>(6)?))).optional().map_err(state::sql(path))?;
        let encoded=start.as_ref().map(|r|&r.0).or_else(||bound.as_ref().and_then(|r|r.2.as_ref()));
        Ok(Recorded {
            request:encoded.map(|s|serde_json::from_str(s).map_err(|e|state::invalid(path,e))).transpose()?,
            unresolved:start.as_ref().is_some_and(|r|r.1=="unresolved"),
            retry_allowed:start.as_ref().is_some_and(|r|r.1=="retry_allowed"),
            bound:bound.as_ref().map(|r|SessionRef::parse(&r.0).map_err(|e|state::invalid(path,e))).transpose()?,
            accepted:bound.as_ref().is_some_and(|r|r.1),
            replacement:start.as_ref().and_then(|r|r.3.as_deref())
                .and_then(replacement_sessions)
                .map(|(original, stopped)| {
                    let parse = |id| SessionRef::parse(id).map_err(|e| state::invalid(path,e));
                    Ok::<_, StateError>(ReplacementIntent { original: parse(original)?, stopped: parse(stopped)? })
                }).transpose()?,
            replaced_session:bound.as_ref().and_then(|r|r.5.as_deref())
                .map(|id|SessionRef::parse(id).map_err(|e|state::invalid(path,e))).transpose()?,
            opening_acknowledged:bound.as_ref().is_some_and(|r|r.6),
            project:start.and_then(|r|r.2).or_else(||bound.as_ref().and_then(|r|r.3.clone())),
            project_id:bound.and_then(|r|r.4),
        })
    })?.unwrap_or_default())
}
pub(crate) const REPLACEMENT: &str = "operator_confirmed_replacement:";

fn replacement_sessions(reason: &str) -> Option<(&str, &str)> {
    let sessions = reason
        .strip_prefix(REPLACEMENT)
        .or_else(|| reason.strip_prefix("operator_confirmed_todo_replacement:"))?;
    // A session reference cannot contain whitespace. Older markers carry one ID.
    Some(sessions.split_once(' ').unwrap_or((sessions, sessions)))
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
        remember_in(
            tx,
            adapter.runtime(),
            input,
            thread,
            project,
            accepted,
            evidence,
        )
    })
}

fn remember_in(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    runtime: instrukt_core::config::Runtime,
    input: &RuntimeStart,
    thread: &Thread,
    project: Option<&str>,
    accepted: bool,
    evidence: RequestEvidence,
) -> Result<(), StateError> {
    let path = state::db_path(tx);
    let request = match evidence {
        RequestEvidence::Retained => {
            Some(serde_json::to_string(input).map_err(|e| state::invalid(&path, e))?)
        }
        RequestEvidence::Unknown => None,
    };
    tx.execute("INSERT INTO runtime_bindings (runtime,semantic_key,kind,project,subject,session,project_id,checkout,worktree,branch,base_head,accepted,request,display_title) VALUES (?1,?2,?3,?4,?2,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(runtime,semantic_key) DO UPDATE SET session=excluded.session,project_id=COALESCE(excluded.project_id,runtime_bindings.project_id),worktree=excluded.worktree,branch=COALESCE(runtime_bindings.branch,excluded.branch),accepted=excluded.accepted,display_title=excluded.display_title",params![runtime.name(),input.key,kind(&input.key),project,thread.session.to_string(),thread.project_id,input.checkout.to_string_lossy(),thread.worktree.as_ref().map(|p|p.to_string_lossy()),thread.branch,input.worktree.as_ref().map(|w|&w.base),accepted,request,thread.title]).map_err(state::sql(&path))?;
    if accepted {
        let resolution: Option<String> = tx
            .query_row(
                "SELECT resolution FROM runtime_starts WHERE runtime=?1 AND semantic_key=?2",
                params![runtime.name(), input.key],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(state::sql(&path))?
            .flatten();
        if let Some((_, stopped)) = resolution.as_deref().and_then(replacement_sessions) {
            tx.execute(
                "UPDATE runtime_bindings SET request=json_set(request,'$.replaced_session',?3)
                 WHERE runtime=?1 AND semantic_key=?2",
                params![runtime.name(), input.key, stopped],
            )
            .map_err(state::sql(&path))?;
        }

        tx.execute(
            "DELETE FROM runtime_starts WHERE runtime=?1 AND semantic_key=?2",
            params![runtime.name(), input.key],
        )
        .map_err(state::sql(&path))?;
    }
    Ok(())
}

pub(crate) fn remember_manual(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    runtime: instrukt_core::config::Runtime,
    input: &RuntimeStart,
    thread: &Thread,
    project: Option<&str>,
) -> Result<(), StateError> {
    remember_in(
        tx,
        runtime,
        input,
        thread,
        project,
        true,
        RequestEvidence::Retained,
    )
}

struct TodoReplacement<'a> {
    work: &'a dyn crate::interfaces::SessionBindings,
    original: &'a SessionRef,
    stopped: &'a SessionRef,
}

fn replacement_project(
    project: Option<&str>,
    request: &RuntimeStart,
) -> Result<crate::interfaces::ProjectRef, SessionsError> {
    let slug = project
        .ok_or_else(|| SessionsError::Refused("todo replacement has no retained project".into()))?;
    let slug = instrukt_core::ids::ProjectSlug::parse(slug)
        .map_err(|error| SessionsError::Refused(error.to_string()))?;
    Ok(crate::interfaces::ProjectRef {
        slug,
        root: request.checkout.clone(),
    })
}

fn accept_started(
    guard: &KeyGuard,
    adapter: &dyn RuntimeAdapter,
    assignment: &RuntimeAssignment<'_>,
    thread: &Thread,
    replacement: Option<&TodoReplacement<'_>>,
) -> Result<(), SessionsError> {
    let mut accept = |tx: &instrukt_core::state_db::rusqlite::Transaction<'_>| {
        remember_in(
            tx,
            adapter.runtime(),
            assignment.request,
            thread,
            assignment.project,
            true,
            RequestEvidence::Retained,
        )?;
        crate::inputs::record_initial_in(
            tx,
            &thread.session,
            &format!("assignment:{}", assignment.request.key),
            &assignment.request.brief,
            assignment.now,
        )
    };
    let Some(replacement) = replacement else {
        return Ok(guard.state.write(accept)?);
    };
    let project = replacement_project(assignment.project, assignment.request)?;
    let binding = crate::interfaces::SessionBinding {
        title: thread.title.clone(),
        key: assignment.request.key.clone(),
        session: thread.session.clone(),
        worktree: thread.worktree.clone(),
    };
    replacement.work.replace_dispatch(&project, &binding, replacement.original, &mut accept)
        .map_err(|error| SessionsError::Refused(format!(
            "replacement for `{}` was not confirmed in the todo record: {error}. Retry `instrukt sessions replace-deleted {} --stopped {}`",
            binding.key, binding.key, replacement.stopped,
        )))
}

/// Discover before every start. Empty discovery cannot clear an ambiguous marker.
pub(crate) fn start(
    adapter: &dyn RuntimeAdapter,
    assignment: &RuntimeAssignment<'_>,
) -> Result<Thread, SessionsError> {
    let RuntimeAssignment {
        invocation,
        project,
        request,
        now,
        ..
    } = *assignment;
    let guard = KeyGuard::claim(invocation, adapter, &request.key, project, now)?;
    let result = start_claimed(adapter, assignment, &guard, None);
    if kind(&request.key) != "standing" {
        note(
            &guard.state,
            adapter.runtime(),
            &request.key,
            now,
            result.as_ref().map(|_| ()),
        );
    }
    result
}

pub(crate) fn start_owned(
    adapter: &dyn RuntimeAdapter,
    assignment: &RuntimeAssignment<'_>,
    guard: &KeyGuard,
) -> Result<Thread, SessionsError> {
    start_claimed(adapter, assignment, guard, None)
}

fn start_claimed(
    adapter: &dyn RuntimeAdapter,
    assignment: &RuntimeAssignment<'_>,
    guard: &KeyGuard,
    replacement: Option<&TodoReplacement<'_>>,
) -> Result<Thread, SessionsError> {
    let RuntimeAssignment {
        invocation,
        project,
        request: input,
        now,
        creation,
    } = *assignment;
    let mut recorded = record(&guard.state, &guard.runtime, &input.key)?;
    if matches!(kind(&input.key), "todo" | "task")
        && let Some(intent) = &recorded.replacement
        && replacement.is_none_or(|replacement| replacement.original != &intent.original)
    {
        if let Some(pending) = &recorded.bound
            && !recorded.accepted
        {
            let presence = adapter.read_state(pending)?.presence;
            if presence == Presence::Archived {
                return Err(archived(&input.key, pending));
            }
            if let Some(door) = recorded.recovery(presence) {
                return Err(door.refusal(&input.key, pending));
            }
        }
        return Err(deleted(&input.key, &intent.stopped));
    }
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
    if (!recorded.accepted || recorded.opening_acknowledged)
        && let Some(session) = &recorded.bound
    {
        let thread = adapter.read_state(session)?;
        match thread.presence {
            Presence::Archived => return Err(archived(&input.key, session)),
            Presence::Absent => return Err(recorded.absent(&input.key, session)),
            Presence::Present => {}
        }
        if recorded.opening_acknowledged && !recorded.accepted {
            verify_pending_start(adapter, &resolved, &thread, &recorded)?;
        } else {
            if crate::runtime::project_conflict(&recorded.project_id, &thread.project_id) {
                return Err(conflict(&input.key, [session]));
            }
            adapter.verify_placement(&resolved, &thread)?;
        }
        if recorded.accepted {
            return Ok(thread);
        }
        if !recorded.opening_acknowledged
            && adapter.observe_input(session, &resolved.brief, now)?.state
                == crate::runtime::InputState::Unknown
        {
            adapter.send(session, &resolved.brief)?;
        }
        accept_started(
            guard,
            adapter,
            &RuntimeAssignment {
                request: &resolved,
                ..*assignment
            },
            &thread,
            replacement,
        )?;
        return Ok(thread);
    }
    if recorded.replacement.is_some() && !recorded.retry_allowed && recorded.bound.is_none() {
        return Err(SessionsError::Unresolved {
            key: input.key.clone(),
        });
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
        && creation == CreationPermit::Initial;
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

    let matches: Vec<_> = listed
        .iter()
        .filter(|thread| thread.semantic_key.as_deref() == Some(input.key.as_str()))
        .filter(|thread| {
            thread.presence == Presence::Present || recorded.bound.as_ref() == Some(&thread.session)
        })
        .collect();
    if matches.len() > 1 || (recorded.replacement.is_some() && !matches.is_empty()) {
        return Err(conflict(
            &input.key,
            matches.iter().map(|thread| &thread.session),
        ));
    }
    let found = matches.first().copied();
    if let Some(bound) = &recorded.bound
        && let Some(found) = found
        && bound != &found.session
    {
        return Err(conflict(&input.key, [bound, &found.session]));
    }
    if found.is_none()
        && recorded.bound.is_none()
        && let Some(thread) = listed.iter().find(|thread| {
            thread.presence == Presence::Archived
                && thread.semantic_key.as_deref() == Some(input.key.as_str())
        })
    {
        return Err(archived(&input.key, &thread.session));
    }
    let observed = recorded
        .bound
        .as_ref()
        .or_else(|| found.map(|thread| &thread.session));
    if let Some(session) = observed {
        let thread = adapter.read_state(session)?;
        match thread.presence {
            Presence::Archived => return Err(archived(&input.key, session)),
            Presence::Absent if recorded.bound.is_some() => {
                return Err(recorded.absent(&input.key, session));
            }
            Presence::Absent => return Err(SessionsError::Gone(session.clone())),
            Presence::Present => {
                if crate::runtime::project_conflict(&recorded.project_id, &thread.project_id)
                    || found.is_some_and(|found| {
                        crate::runtime::project_conflict(&thread.project_id, &found.project_id)
                            || crate::runtime::project_conflict(
                                &recorded.project_id,
                                &found.project_id,
                            )
                    })
                {
                    return Err(conflict(&input.key, [&thread.session]));
                }
                if recorded.bound.is_some() {
                    verify_bound(adapter, input, session, &thread)?;
                } else {
                    adapter.verify_start(input, &thread)?;
                }
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
                    accept_started(
                        guard,
                        adapter,
                        &RuntimeAssignment {
                            request: input,
                            ..*assignment
                        },
                        &thread,
                        replacement,
                    )?;
                }
                return Ok(thread);
            }
        }
    }
    if recorded.unresolved && !fresh {
        return Err(SessionsError::Unresolved {
            key: input.key.clone(),
        });
    }
    let prepared = match crate::agents::prepare_start(invocation.home(), adapter, input, now) {
        Ok(prepared) => prepared,
        Err(error) => {
            guard.state.write::<_,StateError>(|tx| {tx.execute("UPDATE runtime_starts SET state='retry_allowed',resolved_at=?3,resolved_by='instrukt',resolution=CASE WHEN resolution GLOB 'operator_confirmed_*replacement:*' THEN resolution ELSE 'readiness_refused_before_creation' END WHERE runtime=?1 AND semantic_key=?2",params![guard.runtime,input.key,now.to_string()]).map_err(state::sql(&state::db_path(tx)))?;Ok(())})?;
            return Err(error);
        }
    };
    let input = &prepared;
    guard.state.write::<_,StateError>(|tx| {
        let path=state::db_path(tx);
        let request=serde_json::to_string(input).map_err(|e|state::invalid(&path,e))?;
        tx.execute("INSERT INTO runtime_starts (runtime,semantic_key,project,kind,subject,checkout,brief,request,begun_at,reported,state) VALUES (?1,?2,?3,?4,?2,?5,?6,?7,?8,0,'unresolved') ON CONFLICT(runtime,semantic_key) DO UPDATE SET state='unresolved',request=excluded.request,begun_at=excluded.begun_at,reported=0,resolved_at=NULL,resolved_by=NULL,resolution=CASE WHEN runtime_starts.resolution GLOB 'operator_confirmed_*replacement:*' THEN runtime_starts.resolution END",params![guard.runtime,input.key,project,kind(&input.key),input.checkout.to_string_lossy(),input.brief,request,now.to_string()]).map_err(state::sql(&path))?;
        Ok(())
    })?;
    let session = adapter
        .create_thread(input)
        .map_err(|e| SessionsError::Uncertain {
            key: input.key.clone(),
            detail: e.to_string(),
        })?;
    guard.state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        let request = serde_json::to_string(input).map_err(|error| state::invalid(&path, error))?;
        tx.execute(
            "INSERT INTO runtime_bindings
                (runtime,semantic_key,kind,project,subject,session,checkout,base_head,accepted,request)
             VALUES (?1,?2,?3,?4,?2,?5,?6,?7,0,json_set(?8,'$.opening_acknowledged',1))",
            params![guard.runtime,input.key,kind(&input.key),project,session.to_string(),
                input.checkout.to_string_lossy(),input.worktree.as_ref().map(|w| &w.base),request],
        ).map_err(state::sql(&path))?;
        Ok(())
    })?;
    let thread = adapter.read_state(&session)?;
    match thread.presence {
        Presence::Archived => return Err(archived(&input.key, &session)),
        Presence::Absent => {
            return Err(
                record(&guard.state, &guard.runtime, &input.key)?.absent(&input.key, &session)
            );
        }
        Presence::Present => {}
    }
    verify_pending_start(adapter, input, &thread, &recorded)?;
    remember(
        &guard.state,
        adapter,
        input,
        &thread,
        project,
        false,
        RequestEvidence::Retained,
    )?;
    accept_started(
        guard,
        adapter,
        &RuntimeAssignment {
            request: input,
            ..*assignment
        },
        &thread,
        replacement,
    )?;
    Ok(thread)
}

pub(crate) fn conflict<'a>(
    key: &str,
    threads: impl IntoIterator<Item = &'a SessionRef>,
) -> SessionsError {
    let mut threads: Vec<_> = threads.into_iter().cloned().collect();
    threads.sort_by_key(ToString::to_string);
    threads.dedup();
    SessionsError::ConflictingThreads {
        key: key.into(),
        threads,
    }
}

pub(crate) fn archived(key: &str, session: &SessionRef) -> SessionsError {
    SessionsError::ArchivedBound {
        key: key.into(),
        session: session.clone(),
    }
}

pub(crate) fn deleted(key: &str, session: &SessionRef) -> SessionsError {
    SessionsError::DeletedBound {
        key: key.into(),
        session: session.clone(),
    }
}

pub(crate) fn verify_recorded_project(
    state: &RunnerState,
    key: &str,
    thread: &Thread,
) -> Result<(), SessionsError> {
    let project = retained_project(state, key, &thread.session)?;
    let (Some(retained), Some(observed)) = (project, thread.project_id.as_ref()) else {
        return Err(unverified_project(key, &thread.session));
    };
    if &retained != observed {
        return Err(SessionsError::ProjectChanged {
            key: key.into(),
            session: thread.session.clone(),
            retained,
            observed: observed.clone(),
        });
    }
    Ok(())
}

pub(crate) fn unverified_project(key: &str, session: &SessionRef) -> SessionsError {
    SessionsError::ProjectUnverified {
        key: key.into(),
        session: session.clone(),
    }
}

pub(crate) fn retained_project(
    state: &RunnerState,
    key: &str,
    session: &SessionRef,
) -> Result<Option<String>, StateError> {
    Ok(state.read(|conn, path| {
        conn.query_row(
            "SELECT project_id FROM runtime_bindings WHERE runtime=?1 AND semantic_key=?2 AND session=?3",
            params![session.runtime.as_str(), key, session.to_string()],
            |row| row.get(0),
        ).optional().map_err(state::sql(path))
    })?.flatten().flatten())
}

/// A bound ID survives a rename. Validate other assignment facts using the
/// retained title, after ruling out any different semantic key.
pub(crate) fn verify_bound(
    adapter: &dyn RuntimeAdapter,
    request: &RuntimeStart,
    session: &SessionRef,
    thread: &Thread,
) -> Result<(), SessionsError> {
    if &thread.session != session
        || thread
            .semantic_key
            .as_ref()
            .is_some_and(|key| key != &request.key)
    {
        return Err(conflict(&request.key, [session, &thread.session]));
    }
    if thread.semantic_key.is_some() {
        return adapter.verify_start(request, thread);
    }
    adapter.verify_placement(request, thread)
}

fn verify_pending_start(
    adapter: &dyn RuntimeAdapter,
    request: &RuntimeStart,
    thread: &Thread,
    recorded: &Recorded,
) -> Result<(), SessionsError> {
    let refusal = |detail: String| SessionsError::PendingPlacement {
        key: request.key.clone(),
        session: thread.session.clone(),
        detail,
        retry: recorded
            .recovery(Presence::Present)
            .unwrap_or(Recovery::ResolveSession)
            .command(&request.key, &thread.session)
            .into_boxed_str(),
    };
    if thread.semantic_key.as_deref() != Some(&request.key) {
        return Err(refusal(format!(
            "native semantic key is {}",
            thread.semantic_key.as_deref().unwrap_or("missing")
        )));
    }
    let Some(project) = thread
        .project_id
        .as_deref()
        .filter(|project| !project.is_empty())
    else {
        return Err(refusal("native project ID is missing".into()));
    };
    if let Some(retained) = recorded.project_id.as_deref() {
        if project != retained {
            return Err(refusal(format!(
                "native project changed from {retained} to {project}"
            )));
        }
    } else {
        // The scoped listing resolves the checkout. Its project IDs are opaque.
        let listed = adapter
            .list(&request.checkout)
            .map_err(|error| refusal(error.to_string()))?;
        if !listed.iter().any(|entry| {
            entry.session == thread.session
                && entry.presence == Presence::Present
                && entry.project_id.as_deref() == Some(project)
        }) {
            return Err(refusal(format!(
                "native project placement cannot be verified in {}",
                request.checkout.display()
            )));
        }
    }
    adapter
        .verify_placement(request, thread)
        .map_err(|error| refusal(error.to_string()))
}

fn retire_gone(
    guard: &KeyGuard,
    session: &SessionRef,
    original: &SessionRef,
    input: &RuntimeStart,
    project: Option<&str>,
    now: Timestamp,
) -> Result<(), SessionsError> {
    let request =
        serde_json::to_string(input).map_err(|error| SessionsError::Refused(error.to_string()))?;
    let resolution = if session == original {
        format!("{REPLACEMENT}{session}")
    } else {
        format!("{REPLACEMENT}{original} {session}")
    };
    guard.state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        tx.execute("DELETE FROM runtime_bindings WHERE runtime=?1 AND semantic_key=?2 AND session=?3",
            params![guard.runtime, guard.key, session.to_string()]).map_err(state::sql(&path))?;
        // Store permission before releasing the old binding. An interrupted
        // replacement can then continue without treating absence as a lost reply.
        tx.execute("INSERT INTO runtime_starts(runtime,semantic_key,project,kind,subject,checkout,brief,request,begun_at,reported,state,resolution,resolved_by,resolved_at)
            VALUES(?1,?2,?3,?4,?2,?5,?6,?7,?8,0,'retry_allowed',?9,'operator',?8)
            ON CONFLICT(runtime,semantic_key) DO UPDATE SET request=excluded.request,state='retry_allowed',resolution=excluded.resolution,resolved_by=excluded.resolved_by,resolved_at=excluded.resolved_at",
            params![guard.runtime, guard.key, project, kind(&guard.key), input.checkout.to_string_lossy(), input.brief, request, now.to_string(), resolution]).map_err(state::sql(&path))?;
        tx.execute("UPDATE runtime_inputs SET state='failed', waiting=0, failure='target absent before observed consumption' WHERE runtime=?1 AND session=?2 AND state IN ('pending','selected','accepted')",
            params![session.runtime.as_str(), session.id.as_str()]).map_err(state::sql(&path))?;
        Ok(())
    })?;
    Ok(())
}

/// Repair a lost cache only after the caller recovered the assignment's original facts.
pub(crate) fn recover_binding(
    guard: &KeyGuard,
    adapter: &dyn RuntimeAdapter,
    assignment: &crate::runtime::RuntimeAssignment<'_>,
    thread: &Thread,
) -> Result<(), SessionsError> {
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
/// the retained checkout and satisfy the original worktree request.
pub fn resolve(
    invocation: &RuntimeInvocation,
    adapter: &dyn RuntimeAdapter,
    key: &str,
    session: Option<&SessionRef>,
    now: Timestamp,
) -> Result<(), SessionsError> {
    let guard = KeyGuard::claim_for_recovery(invocation, adapter, key, now)?;
    let recorded = record(&guard.state, &guard.runtime, key)?;
    let input = recorded
        .request
        .as_ref()
        .ok_or_else(|| SessionsError::Refused(format!("`{key}` has no recorded start")))?;
    let observed = if let Some(bound) = &recorded.bound
        && session.is_none_or(|requested| requested == bound)
    {
        let thread = adapter.read_state(bound)?;
        if thread.presence == Presence::Archived {
            return Err(archived(key, bound));
        }
        if let Some(recovery) = recorded.recovery(thread.presence) {
            let requested = if session.is_none() {
                Recovery::ResolveNone
            } else {
                Recovery::ResolveSession
            };
            if recovery != requested {
                let error = recovery.refusal(key, bound);
                if thread.presence == Presence::Absent
                    || matches!(recovery, Recovery::ResumeReplacement(_))
                {
                    note(&guard.state, adapter.runtime(), key, now, Err(&error));
                }
                return Err(error);
            }
        }
        Some(thread)
    } else {
        None
    };
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
        let thread = match observed {
            Some(thread) => thread,
            None => adapter.read_state(session)?,
        };
        if recorded.opening_acknowledged {
            verify_pending_start(adapter, input, &thread, &recorded)?;
        }
        let listed = adapter.list(&input.checkout)?;
        let Some(listed) = listed
            .iter()
            .find(|thread| thread.session == *session && thread.presence == Presence::Present)
        else {
            return Err(SessionsError::KeyConflict(key.into()));
        };
        if thread.presence != Presence::Present
            || crate::runtime::project_conflict(&thread.project_id, &listed.project_id)
            || crate::runtime::project_conflict(&recorded.project_id, &thread.project_id)
            || crate::runtime::project_conflict(&recorded.project_id, &listed.project_id)
        {
            return Err(SessionsError::KeyConflict(key.into()));
        }
        if recorded.project_id.is_some()
            && thread.project_id.is_none()
            && listed.project_id.is_none()
        {
            return Err(SessionsError::Refused(format!(
                "native project for {session} cannot be verified against the retained project. Restore native project visibility and retry `instrukt sessions resolve-start {key} --session {session}`"
            )));
        }
        if !recorded.opening_acknowledged {
            adapter.verify_placement(input, &thread)?;
        }
        #[cfg(debug_assertions)]
        hold_for_fixture("INSTRUKT_TEST_RESOLVE_BINDING_HOLD", key);
        // Different keys can resolve concurrently. The writer transaction
        // orders the ownership check with the binding change.
        guard.state.write::<_, SessionsError>(|tx| {
            let path = state::db_path(tx);
            let owned_elsewhere = tx.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM runtime_bindings WHERE session=?1 AND semantic_key!=?2
                    UNION ALL SELECT 1 FROM session_bindings WHERE session=?1 AND semantic_key!=?2 AND released_at IS NULL
                    UNION ALL SELECT 1 FROM procedure_responsibilities WHERE session=?1 AND semantic_key!=?2
                    UNION ALL SELECT 1 FROM connected_conversations WHERE session=?1 AND semantic_key!=?2
                 )",
                params![session.to_string(), key],
                |row| row.get::<_, bool>(0),
            ).map_err(state::sql(&path))?;
            if owned_elsewhere {
                return Err(SessionsError::Refused(format!(
                    "thread {session} already belongs to another runtime key; choose a different session with `instrukt sessions resolve-start {key} --session <runtime:id>`"
                )));
            }
            remember_in(
                tx, adapter.runtime(), input, &thread, recorded.project.as_deref(), false,
                RequestEvidence::Retained,
            )?;
            Ok(())
        })?;
        // Retain the marker until the original input is accepted.
    } else {
        let absent = observed.filter(|thread| thread.presence == Presence::Absent);
        if recorded.bound.is_some() && absent.is_none() {
            return Err(SessionsError::KeyConflict(key.into()));
        }
        guard.state.write::<_,StateError>(|tx| {
            let path = state::db_path(tx);
            if let Some(absent) = &absent {
                tx.execute("DELETE FROM runtime_bindings WHERE runtime=?1 AND semantic_key=?2 AND session=?3 AND accepted=0",
                    params![guard.runtime,key,absent.session.to_string()]).map_err(state::sql(&path))?;
            }
            tx.execute("UPDATE runtime_starts SET state='retry_allowed',resolved_at=?3,resolved_by='operator',resolution=CASE WHEN resolution GLOB 'operator_confirmed_*replacement:*' THEN resolution ELSE 'none' END WHERE runtime=?1 AND semantic_key=?2",
                params![guard.runtime,key,now.to_string()]).map_err(state::sql(&path))?;
            Ok(())
        })?;
    }
    note(&guard.state, adapter.runtime(), key, now, Ok(()));
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn hold_for_fixture(setting: &str, key: &str) {
    if !instrukt_core::fsx::fence_active() {
        return;
    }
    let Some(path) = std::env::var_os(setting).map(std::path::PathBuf::from) else {
        return;
    };
    if std::fs::read_to_string(&path).ok().as_deref() != Some(key)
        || instrukt_core::fsx::write(path.with_extension("observed"), "").is_err()
    {
        return;
    }
    let until = std::time::Instant::now() + Duration::from_secs(30);
    while path.exists() && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(crate) fn recorded_session(
    state: &RunnerState,
    runtime: &str,
    key: &str,
) -> Result<Option<SessionRef>, StateError> {
    Ok(record(state, runtime, key)?.bound)
}

pub(crate) fn replacement_started(
    state: &RunnerState,
    runtime: &str,
    key: &str,
    stopped: &SessionRef,
) -> Result<bool, StateError> {
    let recorded = record(state, runtime, key)?;
    Ok(recorded
        .replacement
        .as_ref()
        .is_some_and(|intent| &intent.stopped == stopped)
        || recorded.replaced_session.as_ref() == Some(stopped))
}

pub struct ReplacementResult {
    pub session: SessionRef,
    pub warning: Option<String>,
}

fn reflect_replacement(
    work: Option<&dyn crate::interfaces::SessionBindings>,
    recorded: &Recorded,
) -> Option<String> {
    let work = work?;
    let request = recorded.request.as_ref()?;
    replacement_project(recorded.project.as_deref(), request)
        .and_then(|project| {
            work.reflect_dispatch_replacement(&project, &request.key)
                .map_err(|error| SessionsError::Refused(error.to_string()))
        })
        .err()
        .map(|error| format!("todo reflection is pending: {error}"))
}

/// Replace a deleted binding on the operator's confirmation that
/// the provider session named by the command has stopped.
pub fn replace_deleted(
    invocation: &RuntimeInvocation,
    adapter: &dyn RuntimeAdapter,
    key: &str,
    stopped: &SessionRef,
    now: Timestamp,
    work: Option<&dyn crate::interfaces::SessionBindings>,
) -> Result<ReplacementResult, SessionsError> {
    let state = RunnerState::new(invocation.home());
    let runtime = adapter.runtime().name();
    let guard = KeyGuard::claim_for_recovery(invocation, adapter, key, now)?;
    let recorded = record(&state, runtime, key)?;
    let standing = kind(key) == "standing";
    let todo = matches!(kind(key), "todo" | "task");
    let work = if todo { work } else { None };
    if recorded.replaced_session.as_ref() == Some(stopped)
        && let Some(session) = recorded.bound.as_ref().filter(|_| recorded.accepted)
        && (!standing || crate::standing::replacement_is_bound(&state, key, session)?)
    {
        return Ok(ReplacementResult {
            session: session.clone(),
            warning: reflect_replacement(work, &recorded),
        });
    }
    if recorded.bound.as_ref() == Some(stopped)
        && recorded.absent_recovery() == Recovery::ResolveNone
    {
        let thread = adapter.read_state(stopped)?;
        let error = recorded
            .recovery(thread.presence)
            .map_or_else(|| archived(key, stopped), |door| door.refusal(key, stopped));
        note(&state, adapter.runtime(), key, now, Err(&error));
        return Err(error);
    }
    let _inputs =
        KeyGuard::claim_for_recovery(invocation, adapter, &format!("input:{stopped}"), now)?;
    if standing {
        let thread =
            crate::standing::replace_deleted(&guard, invocation, adapter, key, stopped, now)?;
        return Ok(ReplacementResult {
            session: thread.session,
            warning: None,
        });
    }
    let resuming = recorded
        .replacement
        .as_ref()
        .is_some_and(|intent| &intent.stopped == stopped);
    let pending_successor = recorded.replacement.is_some() && !recorded.accepted;
    if !resuming && recorded.bound.as_ref() != Some(stopped) {
        return Err(stopped_mismatch(key, stopped));
    }
    if !resuming && !pending_successor && (recorded.unresolved || recorded.retry_allowed) {
        return Err(SessionsError::Unresolved { key: key.into() });
    }
    confirm_deleted(adapter, stopped)?;
    let request = recorded
        .request
        .as_ref()
        .ok_or_else(|| SessionsError::Refused(format!("`{key}` has no retained start request")))?;
    // A conversation successor receives outstanding inputs from its relay.
    // Reusing the original opening would deliver an already-served message.
    let conversation_request;
    let request = if kind(key) == "conversation" {
        let (assignment, _) = request
            .brief
            .strip_prefix("[Instrukt Assignment]\n")
            .and_then(|brief| brief.split_once("\n[/Instrukt Assignment]"))
            .ok_or_else(|| {
                SessionsError::Refused(format!("`{key}` has no retained conversation assignment"))
            })?;
        conversation_request = RuntimeStart {
            brief: crate::envelopes::Control::Assignment.frame(assignment),
            ..request.clone()
        };
        &conversation_request
    } else {
        request
    };
    let original = recorded
        .replacement
        .as_ref()
        .map_or(stopped, |intent| &intent.original);
    let replacement = if todo {
        let work = work.ok_or_else(|| {
            SessionsError::Refused(
                "todo replacement needs its work record; no replacement started".into(),
            )
        })?;
        Some(TodoReplacement {
            work,
            original,
            stopped,
        })
    } else {
        None
    };
    if let Some(replacement) = &replacement {
        let project = replacement_project(recorded.project.as_deref(), request)?;
        replacement
            .work
            .check_dispatch_replacement(&project, key, replacement.original)
            .map_err(|error| {
                SessionsError::Refused(format!("todo replacement is ineligible: {error}"))
            })?;
    }
    if !resuming {
        check_replacement_key(adapter, request, stopped, None)?;
        if let Some(worktree) = &request.worktree
            && instrukt_core::git::succeeds(
                &request.checkout,
                [
                    "show-ref",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{}", worktree.branch),
                ],
            )
            .map_err(|error| SessionsError::Refused(error.to_string()))?
        {
            return Err(SessionsError::Refused(format!(
                "phase branch `{}` still exists; T3 cannot start a replacement in its existing worktree",
                worktree.branch
            )));
        }
        retire_gone(
            &guard,
            stopped,
            original,
            request,
            recorded.project.as_deref(),
            now,
        )?;
    }
    let result = start_claimed(
        adapter,
        &RuntimeAssignment {
            invocation,
            project: recorded.project.as_deref(),
            request,
            now,
            creation: CreationPermit::Reconcile,
        },
        &guard,
        replacement.as_ref(),
    );
    note(
        &guard.state,
        adapter.runtime(),
        key,
        now,
        result.as_ref().map(|_| ()),
    );
    let thread = result?;
    Ok(ReplacementResult {
        session: thread.session,
        warning: reflect_replacement(work, &recorded),
    })
}

/// A replacement binding, with its acceptance still gated by the keyed protocol.
pub fn replacement_binding(
    home: &Home,
    adapter: &dyn RuntimeAdapter,
    key: &str,
    current: &SessionRef,
) -> Result<Option<(SessionRef, bool)>, SessionsError> {
    let recorded = record(&RunnerState::new(home), adapter.runtime().name(), key)?;
    Ok(recorded
        .bound
        .filter(|session| {
            (recorded.replacement.is_some() || recorded.replaced_session.is_some())
                && session != current
        })
        .map(|session| (session, recorded.accepted)))
}

/// Report the same recovery door as the keyed commands for the observed binding.
pub fn presence_finding(
    home: &Home,
    adapter: &dyn RuntimeAdapter,
    key: &str,
    session: &SessionRef,
    presence: Presence,
) -> Result<Option<SessionsError>, SessionsError> {
    let recorded = record(&RunnerState::new(home), adapter.runtime().name(), key)?;
    if presence == Presence::Archived {
        return Ok(Some(archived(key, session)));
    }
    Ok(recorded
        .recovery(presence)
        .map(|door| door.refusal(key, session)))
}

pub(crate) fn stopped_mismatch(key: &str, stopped: &SessionRef) -> SessionsError {
    SessionsError::Refused(format!(
        "`{key}` is not bound to {stopped}; run `instrukt events status` for the current key and stopped-session ID"
    ))
}

pub(crate) fn confirm_deleted(
    adapter: &dyn RuntimeAdapter,
    stopped: &SessionRef,
) -> Result<(), SessionsError> {
    if adapter.read_state(stopped)?.presence != Presence::Absent {
        return Err(SessionsError::Refused(format!(
            "thread {stopped} must be deleted before replacing it"
        )));
    }
    Ok(())
}

pub(crate) fn check_replacement_key(
    adapter: &dyn RuntimeAdapter,
    request: &RuntimeStart,
    stopped: &SessionRef,
    standing: Option<&str>,
) -> Result<(), SessionsError> {
    let matches: Vec<_> = adapter
        .list(&request.checkout)?
        .into_iter()
        .filter(|thread| {
            &thread.session != stopped
                && thread.semantic_key.as_deref().is_some_and(|key| {
                    key == request.key
                        || standing.is_some_and(|family| {
                            key == family
                                || key
                                    .strip_prefix(family)
                                    .is_some_and(|suffix| suffix.starts_with(":generation:"))
                        })
                })
        })
        .collect();
    if !matches.is_empty() {
        return Err(conflict(
            &request.key,
            matches.iter().map(|thread| &thread.session),
        ));
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
        adapter,
        &RuntimeAssignment {
            invocation,
            request: &input,
            project: recorded.project.as_deref(),
            now,
            creation: CreationPermit::Reconcile,
        },
    )
}

pub fn followup(
    adapter: &dyn RuntimeAdapter,
    assignment: &crate::runtime::FollowupAssignment<'_>,
) -> Result<Thread, SessionsError> {
    let result = followup_inner(adapter, assignment);
    note(
        &RunnerState::new(assignment.invocation.home()),
        adapter.runtime(),
        &assignment.request.key,
        assignment.now,
        result.as_ref().map(|_| ()),
    );
    result
}

fn followup_inner(
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
    if let Some(bound) = &recorded.bound
        && bound != assignment.session
    {
        // The caller can still hold the former ID if it stopped before it
        // recorded the replacement. Accept only the already journaled result.
        if adapter.read_state(assignment.session)?.presence != Presence::Absent {
            return Err(conflict(&input.key, [assignment.session, bound]));
        }
        let request = recorded
            .request
            .as_ref()
            .ok_or_else(|| conflict(&input.key, [assignment.session, bound]))?;
        if request.checkout != input.checkout
            || request.brief != input.brief
            || request.worktree != input.worktree
        {
            return Err(conflict(&input.key, [assignment.session, bound]));
        }
        return start_claimed(
            adapter,
            &RuntimeAssignment {
                invocation: assignment.invocation,
                project: assignment.project,
                request,
                now: assignment.now,
                creation: CreationPermit::Reconcile,
            },
            &guard,
            None,
        );
    }
    let found: Vec<_> = adapter
        .list(&input.checkout)?
        .into_iter()
        .filter(|t| {
            t.semantic_key.as_deref() == Some(input.key.as_str())
                && (t.presence == Presence::Present || &t.session == assignment.session)
        })
        .collect();
    if found.len() > 1
        || found
            .first()
            .is_some_and(|thread| &thread.session != assignment.session)
    {
        return Err(conflict(
            &input.key,
            found
                .iter()
                .map(|thread| &thread.session)
                .chain([assignment.session]),
        ));
    }
    let thread = adapter.read_state(assignment.session)?;
    match thread.presence {
        Presence::Archived => return Err(archived(&input.key, assignment.session)),
        Presence::Absent => {
            let Some(request) = recorded.request.as_ref().filter(|_| {
                recorded.bound.is_none() && (recorded.unresolved || recorded.retry_allowed)
            }) else {
                return Err(recorded.absent(&input.key, assignment.session));
            };
            return start_claimed(
                adapter,
                &RuntimeAssignment {
                    invocation: assignment.invocation,
                    project: assignment.project,
                    request,
                    now: assignment.now,
                    creation: CreationPermit::Reconcile,
                },
                &guard,
                None,
            );
        }
        Presence::Present => {}
    }
    if found.first().is_some_and(|found| {
        crate::runtime::project_conflict(&thread.project_id, &found.project_id)
            || crate::runtime::project_conflict(&recorded.project_id, &found.project_id)
    }) || crate::runtime::project_conflict(&recorded.project_id, &thread.project_id)
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
    verify_bound(adapter, &verification, assignment.session, &thread)?;
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
    runtime: instrukt_core::config::Runtime,
    error: &SessionsError,
) -> bool {
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
