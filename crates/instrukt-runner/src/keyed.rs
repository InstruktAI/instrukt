//! Durable keyed dispatch. Exact-key OS ownership spans native operations.
//! Key ownership is kept in SQLite; no writer transaction spans runtime I/O.
use instrukt_core::fsx::LockGuard;
use instrukt_core::home::Home;
use instrukt_core::ids::SessionRef;
use instrukt_core::process::{Limit, Look};
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use jiff::Timestamp;
use std::path::Path;
use std::time::Duration;

use crate::runtime::{Presence, RuntimeAdapter, RuntimeAssignment, RuntimeStart, Thread};

mod _findings;
mod _wake;
use crate::sessions::SessionsError;
use crate::state::{self, RunnerState, StateError};
pub(crate) use _findings::note;
pub use _wake::{
    NotWoken, WakeOutcome, WakeTarget, WakeTargetLookup, check_wake, wake, wake_target,
};

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
/// The requested reviewer's start key for `head` of `project`
/// (`session-continuity`, Start keys and locks).
pub fn review_key(project: &str, head: &str) -> String {
    format!("instrukt:review:{}:{}", encode(project), encode(head))
}

/// Whether `key` is a requested-reviewer start key.
pub(crate) fn is_review_key(key: &str) -> bool {
    kind(key) == "review"
}

/// Whether `key` is a requested-reviewer start key belonging to `project`.
pub fn is_review_key_for_project(key: &str, project: &instrukt_core::ids::ProjectSlug) -> bool {
    key.starts_with(&format!("instrukt:review:{}:", encode(project.as_str())))
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
        use sha2::{Digest, Sha256};
        let runtime = adapter.runtime_name().to_owned();
        let name = Sha256::digest(format!("{runtime}:{key}").as_bytes());
        let path = invocation
            .home()
            .state_dir()
            .join(format!("runtime-key-{name:x}.lock"));
        let lock = LockGuard::try_acquire(&path)
            .map_err(StateError::from)?
            .ok_or_else(|| {
                SessionsError::Refused(format!(
                    "runtime key `{key}` is already owned by an active dispatch"
                ))
            })?;
        let state = RunnerState::new(invocation.home());
        state.write::<_,StateError>(|tx| {
            tx.execute("INSERT INTO runtime_key_locks(runtime,semantic_key,project,owner,acquired_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(runtime,semantic_key) DO UPDATE SET owner=excluded.owner,acquired_at=excluded.acquired_at",params![runtime,key,project,invocation.owner,now.to_string()]).map_err(state::sql(&state::db_path(tx)))?; Ok(())
        })?;
        Ok(Self {
            state,
            runtime,
            key: key.into(),
            owner: invocation.owner.clone(),
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
pub(crate) fn recovery_command<'a>(arguments: impl IntoIterator<Item = &'a str>) -> String {
    arguments
        .into_iter()
        .map(|arg| {
            if !arg.is_empty()
                && arg
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&byte))
            {
                arg.to_owned()
            } else {
                instrukt_core::process::shell_quote(arg)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Default)]
struct Recorded {
    request: Option<RuntimeStart>,
    bound: Option<SessionRef>,
    accepted: bool,
    generation: u64,
    project: Option<String>,
    project_id: Option<String>,
    replaced_session: Option<SessionRef>,
    legacy_modes: Option<crate::runtime::ResolvedLaunchModes>,
}
fn record(state: &RunnerState, runtime: &str, key: &str) -> Result<Recorded, StateError> {
    Ok(state.read(|conn,path| {
        conn.query_row("SELECT session,accepted,request,project,project_id,generation,replaced_session,legacy_modes FROM runtime_bindings WHERE runtime=?1 AND semantic_key=?2",params![runtime,key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,u64>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,Option<String>>(7)?))).optional().map_err(state::sql(path))?
        .map(|(session,accepted,request,project,project_id,generation,replaced,legacy_modes)| {
            let parsed:Option<RuntimeStart>=request.as_deref().map(serde_json::from_str).transpose().map_err(|e|state::invalid(path,e))?;
            let bound=SessionRef::parse(&session).map_err(|e|state::invalid(path,e))?;
            if parsed.as_ref().is_some_and(|request|request.key!=key || request.native_launch.as_ref().is_some_and(|native|native.thread_id!=bound.id.as_str())) {
                return Err(state::invalid(path,format!("binding `{key}` contradicts its captured object/native identity")));
            }
            Ok(Recorded {
            bound:Some(SessionRef::parse(&session).map_err(|e|state::invalid(path,e))?), accepted,
            request:request.map(|v|serde_json::from_str(&v).map_err(|e|state::invalid(path,e))).transpose()?, project,project_id,generation,
            legacy_modes:legacy_modes.map(|v|serde_json::from_str(&v).map_err(|e|state::invalid(path,e))).transpose()?,
            replaced_session:replaced.map(|v|SessionRef::parse(&v).map_err(|e|state::invalid(path,e))).transpose()?,
        })}).transpose()
    })?.flatten().unwrap_or_default())
}
const OWNER_GENERATION: &str = "SELECT max(n) FROM (SELECT coalesce(max(generation),0)+1 AS n FROM runtime_generations WHERE runtime=?1 AND semantic_key=?2 UNION ALL SELECT generation AS n FROM procedure_responsibilities WHERE semantic_key=?2)";

fn kind(key: &str) -> &str {
    key.strip_prefix("instrukt:")
        .and_then(|s| s.split(':').next())
        .unwrap_or("run")
}

fn native_identity(
    home: &Home,
    adapter: &dyn RuntimeAdapter,
    request: &RuntimeStart,
    generation: u64,
    existing_worktree: Option<&Path>,
) -> Result<crate::runtime::NativeLaunch, SessionsError> {
    use sha2::{Digest, Sha256};
    // JSON preserves component boundaries, including exact home and checkout scope.
    let scope = serde_json::to_vec(&(
        adapter.runtime_name(),
        home.root(),
        &request.checkout,
        &request.key,
        generation,
        adapter.launch_scope(home),
    ))
    .map_err(|error| {
        SessionsError::Refused(format!("cannot encode native execution scope: {error}"))
    })?;
    let command_id = format!("instrukt:launch:{:x}", Sha256::digest(&scope));
    Ok(crate::runtime::NativeLaunch {
        thread_id: command_id.clone(),
        command_id,
        project_id: String::new(),
        existing_worktree: existing_worktree.map(Path::to_path_buf),
    })
}

fn persist(
    guard: &KeyGuard,
    request: &RuntimeStart,
    project: Option<&str>,
    generation: u64,
    replaced: Option<&SessionRef>,
) -> Result<(), SessionsError> {
    guard.state.write::<_, SessionsError>(|tx| {
        persist_in(tx, guard, request, project, generation, replaced)
    })
}
fn persist_in(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    guard: &KeyGuard,
    request: &RuntimeStart,
    project: Option<&str>,
    generation: u64,
    replaced: Option<&SessionRef>,
) -> Result<(), SessionsError> {
    let native = request
        .native_launch
        .as_ref()
        .ok_or_else(|| SessionsError::Refused("missing native launch identity".into()))?;
    let encoded =
        serde_json::to_string(request).map_err(|e| SessionsError::Refused(e.to_string()))?;
    tx.execute("INSERT INTO runtime_bindings(runtime,semantic_key,kind,project,subject,session,display_title,project_id,checkout,worktree,branch,base_head,accepted,request,generation,start_key,replaced_session) VALUES(?1,?2,?3,?4,?2,?5,?6,?7,?8,?9,?10,?11,0,?12,?13,?14,?15) ON CONFLICT(runtime,semantic_key) DO UPDATE SET session=excluded.session,request=excluded.request,project_id=excluded.project_id,start_key=excluded.start_key",params![guard.runtime,request.key,kind(&request.key),project,format!("{}:{}",guard.runtime,native.thread_id),request.title,(!native.project_id.is_empty()).then_some(&native.project_id),request.checkout.to_string_lossy(),native.existing_worktree.as_ref().map(|p|p.to_string_lossy()),request.worktree.as_ref().map(|w|&w.branch),request.worktree.as_ref().map(|w|&w.base),encoded,generation,native.command_id,replaced.map(ToString::to_string)]).map_err(state::sql(&state::db_path(tx)))?;
    Ok(())
}

pub(crate) fn remember_manual(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    runtime: &str,
    input: &RuntimeStart,
    thread: &Thread,
    project: Option<&str>,
) -> Result<(), StateError> {
    let path = state::db_path(tx);
    let generation = tx
        .query_row(OWNER_GENERATION, params![runtime, input.key], |row| {
            row.get::<_, u64>(0)
        })
        .map_err(state::sql(&path))?;
    let request = serde_json::to_string(input).map_err(|e| state::invalid(&path, e))?;
    tx.execute("INSERT INTO runtime_bindings(runtime,semantic_key,kind,project,subject,session,project_id,checkout,worktree,branch,base_head,accepted,request,display_title,generation) VALUES(?1,?2,?3,?4,?2,?5,?6,?7,?8,?9,?10,1,?11,?12,?13) ON CONFLICT(runtime,semantic_key) DO UPDATE SET project_id=coalesce(excluded.project_id,runtime_bindings.project_id),accepted=1,worktree=excluded.worktree",params![runtime,input.key,kind(&input.key),project,thread.session.to_string(),thread.project_id,input.checkout.to_string_lossy(),thread.worktree.as_ref().map(|p|p.to_string_lossy()),thread.branch,input.worktree.as_ref().map(|w|&w.base),request,thread.title,generation]).map_err(state::sql(&path))?;
    Ok(())
}

pub(crate) fn retire_in(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    runtime: &str,
    key: &str,
) -> Result<(), StateError> {
    let path = state::db_path(tx);
    tx.execute("INSERT INTO runtime_generations(runtime,semantic_key,generation,session,request,project_id,checkout,worktree,branch,base_head,accepted,display_title,start_key,replaced_session,legacy_modes) SELECT runtime,semantic_key,generation,session,request,project_id,checkout,worktree,branch,base_head,accepted,display_title,start_key,replaced_session,legacy_modes FROM runtime_bindings WHERE runtime=?1 AND semantic_key=?2",params![runtime,key]).map_err(state::sql(&path))?;
    tx.execute(
        "DELETE FROM runtime_bindings WHERE runtime=?1 AND semantic_key=?2",
        params![runtime, key],
    )
    .map_err(state::sql(&path))?;
    Ok(())
}

pub fn start(
    adapter: &dyn RuntimeAdapter,
    assignment: &RuntimeAssignment<'_>,
) -> Result<Thread, SessionsError> {
    let guard = KeyGuard::claim(
        assignment.invocation,
        adapter,
        &assignment.request.key,
        assignment.project,
        assignment.now,
    )?;
    let result = start_owned(adapter, assignment, &guard);
    note(
        &guard.state,
        adapter.runtime_name(),
        &assignment.request.key,
        assignment.now,
        result.as_ref().map(|_| ()),
    );
    result
}
pub(crate) fn start_owned(
    adapter: &dyn RuntimeAdapter,
    assignment: &RuntimeAssignment<'_>,
    guard: &KeyGuard,
) -> Result<Thread, SessionsError> {
    start_with_work(adapter, assignment, guard, assignment.bindings)
}
fn start_with_work(
    adapter: &dyn RuntimeAdapter,
    assignment: &RuntimeAssignment<'_>,
    guard: &KeyGuard,
    work: Option<&dyn crate::interfaces::SessionBindings>,
) -> Result<Thread, SessionsError> {
    let recorded = record(&guard.state, &guard.runtime, &assignment.request.key)?;
    if recorded.accepted {
        let session = recorded
            .bound
            .as_ref()
            .ok_or_else(|| SessionsError::Refused("accepted binding missing native ID".into()))?;
        let thread = adapter.read_state(session)?;
        if thread.presence != Presence::Present {
            return Err(SessionsError::Gone(session.clone()));
        }
        let input = recorded.request.as_ref().unwrap_or(assignment.request);
        if recorded.project_id.is_none()
            || (input.launch_modes.is_none() && recorded.legacy_modes.is_none())
        {
            capture_legacy(adapter, guard, input, &recorded, &thread)?;
        }
        verify_recorded_project(&guard.state, &assignment.request.key, &thread)?;
        adapter.verify_placement(input, &thread)?;
        return Ok(thread);
    }
    if let (Some(session), Some(input)) = (&recorded.bound, &recorded.request)
        && input.native_launch.is_none()
        && !session.id.as_str().starts_with("instrukt:unstarted:")
    {
        let thread = adapter.read_state(session)?;
        if thread.presence == Presence::Present {
            capture_legacy(adapter, guard, input, &recorded, &thread)?;
            adapter.verify_placement(input, &thread)?;
            if adapter
                .observe_input(session, &input.brief, assignment.now)?
                .state
                == crate::runtime::InputState::Unknown
            {
                adapter.send(session, &input.brief)?;
            }
            verify_recorded_project(&guard.state, &input.key, &thread)?;
            accept_thread(guard, adapter, assignment, input, &thread, &recorded, work)?;
            return Ok(thread);
        } else {
            return succeed_claimed(
                assignment.invocation,
                adapter,
                assignment.now,
                work,
                guard,
                Some((assignment.request, assignment.existing_worktree)),
                false,
            );
        }
    }
    if let (Some(session), Some(input)) = (&recorded.bound, &recorded.request)
        && input.native_launch.is_some()
    {
        let thread = adapter.read_state(session)?;
        // A created thread that is gone, or idle in a runtime project other
        // than its checkout's own, is succeeded: launching the retained
        // request again would only replay its project. A misplaced thread
        // that is not idle keeps the start pending until it is.
        let succeed = if thread.presence == Presence::Present {
            match adapter.verify_thread_project(&input.checkout, &thread, Some(&input.key)) {
                Ok(()) => false,
                Err(crate::runtime::RuntimeError::ThreadMisplaced { idle: true, .. }) => {
                    adapter.accepted_start(input)?
                }
                Err(error) => return Err(error.into()),
            }
        } else {
            adapter.accepted_start(input)?
        };
        if succeed {
            return succeed_claimed(
                assignment.invocation,
                adapter,
                assignment.now,
                work,
                guard,
                None,
                false,
            );
        }
    }
    let mut input = match &recorded.request {
        Some(request) => request.clone(),
        None => crate::agents::prepare_start(
            assignment.invocation.home(),
            adapter,
            assignment.request,
            assignment.now,
        )?,
    };
    if input.native_launch.is_none() {
        input = crate::agents::prepare_start(
            assignment.invocation.home(),
            adapter,
            &input,
            assignment.now,
        )?;
    }
    let generation = if recorded.generation == 0 {
        guard
            .state
            .read(|conn, path| {
                conn.query_row(OWNER_GENERATION, params![guard.runtime, input.key], |r| {
                    r.get::<_, u64>(0)
                })
                .map_err(state::sql(path))
            })?
            .unwrap_or(1)
    } else {
        recorded.generation
    };
    if input.native_launch.is_none() {
        input.native_launch = Some(native_identity(
            assignment.invocation.home(),
            adapter,
            &input,
            generation,
            assignment.existing_worktree,
        )?);
    }
    input = adapter.resolve_launch(&input, None)?;
    persist(
        guard,
        &input,
        assignment.project,
        generation,
        recorded.replaced_session.as_ref(),
    )?;
    let session = adapter.create_thread(&input)?;
    let thread = observe_launch(adapter, guard, &input, &session)?;
    accept_thread(guard, adapter, assignment, &input, &thread, &recorded, work)?;
    Ok(thread)
}
/// How long a launched thread is observed while its runtime still prepares
/// its placement.
const PLACEMENT_WINDOW: Duration = Duration::from_secs(10);
/// The pause between two observations of a placement still being prepared.
const PLACEMENT_PAUSE: Duration = Duration::from_millis(250);
/// Reads and verifies the thread a launch created. While the runtime answers
/// that its placement is still being prepared, it observes the thread again
/// after a pause, until verification answers otherwise or the placement
/// window has elapsed; the last answer is the start's. A pause never asks
/// for more than the window has left, and no observation starts once the
/// window has elapsed.
fn observe_launch(
    adapter: &dyn RuntimeAdapter,
    guard: &KeyGuard,
    input: &RuntimeStart,
    session: &SessionRef,
) -> Result<Thread, SessionsError> {
    let window = Limit::start(PLACEMENT_WINDOW);
    loop {
        let thread = adapter.read_state(session)?;
        verify_recorded_project(&guard.state, &input.key, &thread)?;
        match adapter.verify_start(input, &thread) {
            Ok(()) => return Ok(thread),
            Err(pending @ crate::runtime::RuntimeError::PlacementPreparing(_)) => {
                std::thread::sleep(PLACEMENT_PAUSE.min(window.remaining()));
                if window.judge(None::<()>) == Look::Passed {
                    return Err(pending.into());
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
}
fn accept_thread(
    guard: &KeyGuard,
    adapter: &dyn RuntimeAdapter,
    assignment: &RuntimeAssignment<'_>,
    input: &RuntimeStart,
    thread: &Thread,
    recorded: &Recorded,
    work: Option<&dyn crate::interfaces::SessionBindings>,
) -> Result<(), SessionsError> {
    let mut accept = |tx: &instrukt_core::state_db::rusqlite::Transaction<'_>| {
        remember_manual(
            tx,
            adapter.runtime_name(),
            input,
            thread,
            assignment.project,
        )?;
        crate::inputs::record_initial_in(
            tx,
            &thread.session,
            &format!("assignment:{}", input.key),
            &input.brief,
            assignment.now,
        )
    };
    if let Some(original) = recorded
        .replaced_session
        .as_ref()
        .filter(|_| matches!(kind(&input.key), "todo" | "task"))
    {
        let work = work.ok_or_else(|| {
            SessionsError::Refused("pending work successor requires its binding owner".into())
        })?;
        let project = replacement_project(assignment.project, input)?;
        let binding = crate::interfaces::SessionBinding {
            title: thread.title.clone(),
            key: input.key.clone(),
            session: thread.session.clone(),
            worktree: thread.worktree.clone(),
        };
        work.replace_dispatch(&project, &binding, original, &mut accept)
            .map_err(|e| SessionsError::Refused(e.to_string()))?;
    } else {
        guard.state.write(accept)?;
    }
    Ok(())
}
fn capture_legacy(
    adapter: &dyn RuntimeAdapter,
    guard: &KeyGuard,
    input: &RuntimeStart,
    recorded: &Recorded,
    thread: &Thread,
) -> Result<(), SessionsError> {
    // The recorded project is no proof of placement: the thread must run in
    // its checkout's own project before anything is persisted, and the
    // project captured is the one resolved for the checkout now.
    adapter.verify_thread_project(&input.checkout, thread, Some(&input.key))?;
    let session = &thread.session;
    let mut realized = input.clone();
    realized.native_launch = Some(crate::runtime::NativeLaunch {
        command_id: String::new(),
        thread_id: session.id.to_string(),
        project_id: String::new(),
        existing_worktree: None,
    });
    realized.launch_modes = realized
        .launch_modes
        .or_else(|| recorded.legacy_modes.clone());
    realized = adapter.resolve_launch(&realized, Some(session))?;
    let project = realized
        .native_launch
        .as_ref()
        .map(|launch| launch.project_id.clone());
    realized.native_launch = input.native_launch.clone();
    let request =
        serde_json::to_string(&realized).map_err(|e| SessionsError::Refused(e.to_string()))?;
    let modes = serde_json::to_string(&realized.launch_modes)
        .map_err(|e| SessionsError::Refused(e.to_string()))?;
    guard.state.write::<_,StateError>(|tx|{tx.execute("UPDATE runtime_bindings SET request=CASE WHEN request IS NULL THEN NULL ELSE ?3 END,project_id=?4,legacy_modes=CASE WHEN request IS NULL THEN ?5 ELSE NULL END WHERE runtime=?1 AND semantic_key=?2",params![guard.runtime,guard.key,request,project,modes]).map_err(state::sql(&state::db_path(tx)))?;Ok(())})?;
    Ok(())
}

/// Keys whose prior generations held this session. Consumers use these facts
/// without interpreting generation storage or retirement predicates.
pub fn retired_binding_keys(home: &Home, session: &SessionRef) -> Result<Vec<String>, StateError> {
    RunnerState::new(home)
        .read(|conn, path| {
            let mut query = conn
                .prepare(
                    "SELECT DISTINCT semantic_key FROM runtime_generations
                 WHERE runtime=?1 AND session=?2 AND generation>0",
                )
                .map_err(state::sql(path))?;
            query
                .query_map(
                    params![session.runtime.as_str(), session.to_string()],
                    |row| row.get(0),
                )
                .map_err(state::sql(path))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(state::sql(path))
        })
        .map(Option::unwrap_or_default)
}

/// Adopt an ID already retained by its owning work/standing record, never by title discovery.
pub(crate) fn recover_binding(
    guard: &KeyGuard,
    adapter: &dyn RuntimeAdapter,
    assignment: &RuntimeAssignment<'_>,
    thread: &Thread,
    known_request: bool,
) -> Result<(), SessionsError> {
    let recorded = record(&guard.state, &guard.runtime, &assignment.request.key)?;
    if recorded
        .bound
        .as_ref()
        .is_some_and(|id| id != &thread.session)
    {
        return Err(conflict(&assignment.request.key, [&thread.session]));
    }
    if retired_binding_keys(guard.state.home(), &thread.session)?.contains(&assignment.request.key)
    {
        return Err(SessionsError::Refused("the owner record names a retired native generation; restore its current binding before dispatch".into()));
    }
    adapter.verify_placement(assignment.request, thread)?;
    let mut input = assignment.request.clone();
    // The project recorded is the one resolved for the checkout now, never
    // an earlier recorded one.
    input.native_launch = Some(crate::runtime::NativeLaunch {
        command_id: String::new(),
        thread_id: thread.session.id.to_string(),
        project_id: String::new(),
        existing_worktree: None,
    });
    input = adapter.resolve_launch(&input, Some(&thread.session))?;
    let project = input
        .native_launch
        .as_ref()
        .map(|launch| launch.project_id.clone());
    input.native_launch = None;
    let modes = serde_json::to_string(&input.launch_modes)
        .map_err(|e| SessionsError::Refused(e.to_string()))?;
    guard.state.write::<_,StateError>(|tx|{
        remember_manual(tx,adapter.runtime_name(),&input,thread,assignment.project)?;
        tx.execute("UPDATE runtime_bindings SET project_id=?3,request=CASE WHEN ?4 THEN request ELSE NULL END,legacy_modes=CASE WHEN ?4 THEN NULL ELSE ?5 END WHERE runtime=?1 AND semantic_key=?2",params![guard.runtime,assignment.request.key,project,known_request,modes]).map_err(state::sql(&state::db_path(tx)))?;Ok(())
    })?;
    Ok(())
}

/// Called only for outstanding owner-admitted work after confirmed native terminal presence,
/// or once an idle thread is confirmed to run in another runtime project than its checkout's own.
pub fn succeed(
    adapter: &dyn RuntimeAdapter,
    assignment: &crate::runtime::FollowupAssignment<'_>,
) -> Result<Thread, SessionsError> {
    let invocation = assignment.invocation;
    let key = &assignment.request.key;
    let now = assignment.now;
    let guard = KeyGuard::claim(invocation, adapter, key, assignment.project, now)?;
    if record(&guard.state, &guard.runtime, key)?.bound.is_none() {
        if retired_binding_keys(guard.state.home(), assignment.session)?
            .iter()
            .any(|retired| retired == key)
        {
            return Err(SessionsError::Refused("the owner record names a retired native generation; restore its current binding before dispatch".into()));
        }
        let mut thread = adapter.read_state(assignment.session)?;
        let source = assignment.original_request.unwrap_or(assignment.request);
        // An idle thread in another project retires like a terminal one; a
        // present thread in its own project is adopted.
        if thread.presence == Presence::Present
            && !idle_misplaced(adapter, &source.checkout, &thread, Some(key))?
        {
            recover_binding(
                &guard,
                adapter,
                &RuntimeAssignment {
                    bindings: assignment.bindings,
                    invocation,
                    project: assignment.project,
                    request: source,
                    existing_worktree: None,
                    now,
                },
                &thread,
                assignment.original_request.is_some(),
            )?;
        } else {
            thread.worktree = thread
                .worktree
                .or_else(|| assignment.worktree_path.map(Into::into));
            guard.state.write::<_,StateError>(|tx|{
                remember_manual(tx,adapter.runtime_name(),source,&thread,assignment.project)?;
                if assignment.original_request.is_none() {tx.execute("UPDATE runtime_bindings SET request=NULL WHERE runtime=?1 AND semantic_key=?2",params![guard.runtime,key]).map_err(state::sql(&state::db_path(tx)))?;}
                Ok(())
            })?;
        }
    }
    let result = succeed_claimed(
        invocation,
        adapter,
        now,
        assignment.bindings,
        &guard,
        Some((assignment.request, assignment.worktree_path)),
        assignment.replace_brief,
    );
    note(
        &guard.state,
        adapter.runtime_name(),
        key,
        now,
        result.as_ref().map(|_| ()),
    );
    result
}

/// Retires a present, idle runtime binding into `runtime_generations` and
/// launches one successor generation with `assignment.request`'s newly rendered
/// opening brief (never copying `conversation_assignment_prefix` from the
/// prior generation). Refuses if the session is absent/archived or has an
/// active turn (`active_turn.is_some()` or `Running`/`NeedsInput`).
pub fn rollover_present_idle(
    adapter: &dyn RuntimeAdapter,
    assignment: &crate::runtime::FollowupAssignment<'_>,
) -> Result<Thread, SessionsError> {
    let invocation = assignment.invocation;
    let key = &assignment.request.key;
    let now = assignment.now;
    let guard = KeyGuard::claim(invocation, adapter, key, assignment.project, now)?;
    let result = rollover_present_idle_claimed(invocation, adapter, assignment, &guard);
    note(
        &guard.state,
        adapter.runtime_name(),
        key,
        now,
        result.as_ref().map(|_| ()),
    );
    result
}

fn rollover_present_idle_claimed(
    invocation: &RuntimeInvocation,
    adapter: &dyn RuntimeAdapter,
    assignment: &crate::runtime::FollowupAssignment<'_>,
    guard: &KeyGuard,
) -> Result<Thread, SessionsError> {
    let key = guard.key.as_str();
    let recorded = record(&guard.state, &guard.runtime, key)?;
    let session = recorded
        .bound
        .as_ref()
        .ok_or_else(|| SessionsError::Refused(format!("`{key}` has no bound native ID")))?;
    if session != assignment.session {
        return Err(stopped_mismatch(key, assignment.session));
    }
    let checkout = guard.state.read(|conn, path| {
        conn.query_row(
            "SELECT checkout FROM runtime_bindings WHERE runtime=?1 AND semantic_key=?2",
            params![guard.runtime, key],
            |row| row.get::<_, String>(0),
        )
        .map_err(state::sql(path))
    })?;
    if assignment.request.key != key || checkout.as_deref() != assignment.request.checkout.to_str()
    {
        return Err(SessionsError::ChangedRequest(key.into()));
    }
    let thread = adapter.read_state(session)?;
    if thread.presence != Presence::Present {
        return Err(SessionsError::Gone(session.clone()));
    }
    let is_idle = thread.active_turn.is_none()
        && matches!(
            thread.state,
            crate::sessions::SessionState::InputReady | crate::sessions::SessionState::EndedFailed
        );
    if !is_idle {
        return Err(SessionsError::Refused(format!(
            "thread {session} is not idle and cannot roll over yet"
        )));
    }
    let mut request = assignment.request.clone();
    request.launch_modes = None;
    let generation = recorded
        .generation
        .checked_add(1)
        .ok_or_else(|| SessionsError::Refused("generation exhausted".into()))?;
    let native = native_identity(
        invocation.home(),
        adapter,
        &request,
        generation,
        assignment.worktree_path,
    )?;
    request.native_launch = Some(native);
    request = crate::agents::prepare_start(invocation.home(), adapter, &request, assignment.now)?;
    request = adapter.resolve_launch(&request, None)?;
    guard.state.write::<_, SessionsError>(|tx| {
        retire_in(tx, &guard.runtime, key)?;
        persist_in(
            tx,
            guard,
            &request,
            recorded.project.as_deref(),
            generation,
            Some(session),
        )?;
        tx.execute(
            "UPDATE runtime_inputs SET state='failed',waiting=0,failure='explicit-frontstage rollover; retired generation' WHERE runtime=?1 AND session=?2 AND kind!=?3 AND state IN ('pending','selected','accepted')",
            params![
                session.runtime.as_str(),
                session.id.as_str(),
                crate::inputs::InputKind::Verdict.name()
            ],
        )
        .map_err(state::sql(&state::db_path(tx)))?;
        crate::inputs::close_verdict_inputs_on_succession(tx, session, key)?;
        Ok(())
    })?;
    start_with_work(
        adapter,
        &RuntimeAssignment {
            bindings: None,
            invocation,
            project: recorded.project.as_deref(),
            request: &request,
            existing_worktree: None,
            now: assignment.now,
        },
        guard,
        assignment.bindings,
    )
}

/// Succeeds the generation of `key` under the `guard` its caller already
/// holds, from `replacement` and its optional `existing_worktree`, after
/// confirmed native terminal presence.
pub(crate) fn succeed_owned(
    guard: &KeyGuard,
    invocation: &RuntimeInvocation,
    adapter: &dyn RuntimeAdapter,
    replacement: &RuntimeStart,
    existing_worktree: Option<&Path>,
    now: Timestamp,
) -> Result<Thread, SessionsError> {
    let key = &replacement.key;
    let result = succeed_claimed(
        invocation,
        adapter,
        now,
        None,
        guard,
        Some((replacement, existing_worktree)),
        false,
    );
    note(
        &guard.state,
        adapter.runtime_name(),
        key,
        now,
        result.as_ref().map(|_| ()),
    );
    result
}
fn succeed_claimed(
    invocation: &RuntimeInvocation,
    adapter: &dyn RuntimeAdapter,
    now: Timestamp,
    work: Option<&dyn crate::interfaces::SessionBindings>,
    guard: &KeyGuard,
    replacement: Option<(&RuntimeStart, Option<&Path>)>,
    replace_brief: bool,
) -> Result<Thread, SessionsError> {
    let key = guard.key.as_str();
    let replacement_request = replacement.map(|(request, _)| request);
    let replacement_existing_worktree = replacement.and_then(|(_, worktree)| worktree);
    let recorded = record(&guard.state, &guard.runtime, key)?;
    let session = recorded
        .bound
        .as_ref()
        .ok_or_else(|| SessionsError::Refused(format!("`{key}` has no bound native ID")))?;
    if let Some(request) = replacement_request {
        let checkout = guard.state.read(|conn, path| {
            conn.query_row(
                "SELECT checkout FROM runtime_bindings WHERE runtime=?1 AND semantic_key=?2",
                params![guard.runtime, key],
                |row| row.get::<_, String>(0),
            )
            .map_err(state::sql(path))
        })?;
        if request.key != key || checkout.as_deref() != request.checkout.to_str() {
            return Err(SessionsError::ChangedRequest(key.into()));
        }
    }
    let thread = adapter.read_state(session)?;
    let created = if session.id.as_str().starts_with("instrukt:unstarted:") {
        false
    } else if recorded.accepted || thread.presence == Presence::Archived {
        true
    } else if let Some(request) = recorded
        .request
        .as_ref()
        .filter(|request| request.native_launch.is_some())
    {
        adapter.accepted_start(request)?
    } else {
        true
    };
    // Besides terminal presence, an idle thread confirmed in another runtime
    // project than its checkout's own is succeeded.
    let checkout = recorded
        .request
        .as_ref()
        .or(replacement_request)
        .map(|request| request.checkout.clone());
    let misplaced = thread.presence == Presence::Present
        && created
        && match &checkout {
            Some(checkout) => idle_misplaced(adapter, checkout, &thread, Some(key))?,
            None => false,
        };
    if (thread.presence == Presence::Present && !misplaced) || !created {
        return start_with_work(
            adapter,
            &RuntimeAssignment {
                bindings: None,
                invocation,
                project: recorded.project.as_deref(),
                request: recorded
                    .request
                    .as_ref()
                    .ok_or_else(|| SessionsError::Refused("missing request".into()))?,
                existing_worktree: None,
                now,
            },
            guard,
            work,
        );
    }
    if kind(key) == "standing" {
        return crate::standing::replace_deleted(guard, invocation, adapter, key, session, now);
    }
    let mut request = recorded
        .request
        .clone()
        .or_else(|| replacement_request.cloned())
        .ok_or_else(|| {
            SessionsError::Refused(
                "missing retained request; its work owner must supply the successor assignment"
                    .into(),
            )
        })?;
    let todo = matches!(kind(key), "todo" | "task");
    let original = if recorded.accepted {
        session
    } else {
        recorded.replaced_session.as_ref().unwrap_or(session)
    };
    let project = if todo {
        Some(replacement_project(recorded.project.as_deref(), &request)?)
    } else {
        None
    };
    if let Some(project) = &project {
        work.ok_or_else(|| SessionsError::Refused("work binding owner required".into()))?
            .check_dispatch_replacement(project, key, original)
            .map_err(|e| SessionsError::Refused(e.to_string()))?;
    }
    if kind(key) == "conversation" {
        if replace_brief {
            if let Some(replacement) = replacement_request {
                request.brief = replacement.brief.clone();
            }
        } else {
            request.brief = conversation_assignment_prefix(&request.brief)
                .ok_or_else(|| SessionsError::Refused("missing conversation assignment".into()))?;
        }
    }
    let is_review_replacement = replacement_request.is_some() && is_review_key(key);
    if let Some(replacement) = replacement_request {
        request.title = replacement.title.clone();
        // A requested reviewer's successor opens with the replacement ask's
        // rendering and admitted worktree placement rather than inheriting its
        // predecessor's.
        if is_review_key(key) {
            request.brief = replacement.brief.clone();
            request.worktree = None;
        }
    }
    request.launch_modes = None;
    let generation = recorded
        .generation
        .checked_add(1)
        .ok_or_else(|| SessionsError::Refused("generation exhausted".into()))?;
    // A successor never inherits the retired generation's project: it
    // resolves its checkout's own project afresh.
    let mut native = native_identity(invocation.home(), adapter, &request, generation, None)?;
    native.existing_worktree = if is_review_replacement {
        replacement_existing_worktree.map(Path::to_path_buf)
    } else {
        let retained_path = guard
            .state
            .read(|conn, path| {
                conn.query_row(
                    "SELECT worktree FROM runtime_bindings WHERE runtime=?1 AND semantic_key=?2",
                    params![guard.runtime, key],
                    |r| r.get::<_, Option<String>>(0),
                )
                .map_err(state::sql(path))
            })?
            .flatten()
            .map(Into::into);
        // A worktree is kept only when it is a registered worktree of the
        // checkout on its branch; a misplaced thread's worktree belongs to another
        // repository and is left untouched.
        let candidate: Option<std::path::PathBuf> = thread.worktree.clone().or(retained_path);
        let mut dropped = None;
        let existing = match candidate {
            Some(path) => {
                let branch = match &request.worktree {
                    Some(worktree) => Some(worktree.branch.clone()),
                    None => instrukt_core::git::run(&path, ["branch", "--show-current"])
                        .ok()
                        .map(|branch| branch.trim().to_owned()),
                };
                let registered = match &branch {
                    Some(branch) => {
                        instrukt_core::git::registered_worktree(&request.checkout, &path, branch)
                            .map_err(|e| SessionsError::Refused(e.to_string()))?
                    }
                    None => false,
                };
                if registered {
                    Some(path)
                } else {
                    dropped = Some(path);
                    None
                }
            }
            None => None,
        };
        if let Some(path) = &dropped
            && kind(key) == "todo"
            && request.worktree.is_none()
        {
            return Err(SessionsError::Refused(format!(
                "`{key}` has no captured worktree request and its worktree {} is not a registered worktree of {}",
                path.display(),
                request.checkout.display()
            )));
        }
        existing
    };
    if !is_review_key(key)
        && request.worktree.is_none()
        && let Some(path) = native.existing_worktree.as_ref()
    {
        let branch = instrukt_core::git::run(path, ["branch", "--show-current"])
            .map_err(|e| SessionsError::Refused(e.to_string()))?;
        let base = instrukt_core::git::run(path, ["rev-parse", "HEAD"])
            .map_err(|e| SessionsError::Refused(e.to_string()))?;
        // Work validates its original phase base; this legacy generation captures the current head.
        request.worktree = Some(crate::runtime::WorktreeRequest {
            from: branch.trim().into(),
            branch: branch.trim().into(),
            base: base.trim().into(),
        });
    }
    request.native_launch = Some(native);
    request = crate::agents::prepare_start(invocation.home(), adapter, &request, now)?;
    request = adapter.resolve_launch(&request, None)?;
    guard.state.write::<_,SessionsError>(|tx| {
        retire_in(tx,&guard.runtime,key)?;
        persist_in(tx,guard,&request,recorded.project.as_deref(),generation,Some(original))?;
        let failure = if misplaced {
            "misplaced target; outstanding owner work retained for successor"
        } else {
            "native terminal target; outstanding owner work retained for successor"
        };
        tx.execute("UPDATE runtime_inputs SET state='failed',waiting=0,failure=?3 WHERE runtime=?1 AND session=?2 AND kind!=?4 AND state IN ('pending','selected','accepted')",params![session.runtime.as_str(),session.id.as_str(),failure,crate::inputs::InputKind::Verdict.name()]).map_err(state::sql(&state::db_path(tx)))?;
        crate::inputs::close_verdict_inputs_on_succession(tx, session, key)?;
        Ok(())
    })?;
    start_with_work(
        adapter,
        &RuntimeAssignment {
            bindings: None,
            invocation,
            project: recorded.project.as_deref(),
            request: &request,
            existing_worktree: None,
            now,
        },
        guard,
        work,
    )
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

pub(crate) fn verify_recorded_project(
    state: &RunnerState,
    key: &str,
    thread: &Thread,
) -> Result<(), SessionsError> {
    let project = retained_project(state, key, &thread.session)?;
    let Some(retained) = project else {
        return Err(unverified_project(key, &thread.session));
    };
    if let Some(observed) = thread.project_id.as_ref()
        && &retained != observed
    {
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

/// What owns a thread the input pump delivers to, and where it runs.
enum DeliveryOwner {
    /// A runtime binding of any kind, with its recorded project and checkout.
    Binding {
        key: String,
        recorded_project: bool,
        checkout: Option<std::path::PathBuf>,
    },
    /// A standing responsibility without a runtime binding.
    Standing {
        key: String,
        checkout: std::path::PathBuf,
    },
    /// An unreleased todo session without a runtime binding.
    Todo {
        key: String,
        checkout: Option<std::path::PathBuf>,
    },
}

struct SessionBindingRow {
    key: String,
    generation: u64,
    project_id: Option<String>,
    request: Option<String>,
    checkout: String,
}

impl SessionBindingRow {
    fn checkout_path(&self) -> Option<std::path::PathBuf> {
        let requested = self
            .request
            .as_deref()
            .and_then(|text| serde_json::from_str::<RuntimeStart>(text).ok())
            .map(|request| request.checkout);
        requested.or_else(|| (!self.checkout.is_empty()).then(|| self.checkout.clone().into()))
    }
}

fn bindings_for_session(
    state: &RunnerState,
    runtime: &str,
    session: &SessionRef,
) -> Result<Vec<SessionBindingRow>, StateError> {
    let session_text = session.to_string();
    Ok(state
        .read(|conn, path| {
            let mut stmt = conn
                .prepare("SELECT semantic_key,generation,project_id,request,checkout FROM runtime_bindings WHERE runtime=?1 AND session=?2 ORDER BY rowid")
                .map_err(state::sql(path))?;
            stmt.query_map(params![runtime, session_text], |r| {
                Ok(SessionBindingRow {
                    key: r.get(0)?,
                    generation: r.get(1)?,
                    project_id: r.get(2)?,
                    request: r.get(3)?,
                    checkout: r.get(4)?,
                })
            })
            .map_err(state::sql(path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state::sql(path))
        })?
        .unwrap_or_default())
}

fn delivery_owner(
    state: &RunnerState,
    runtime: &str,
    session: &SessionRef,
    work: Option<&dyn crate::interfaces::SessionBindings>,
) -> Result<Option<DeliveryOwner>, SessionsError> {
    let session_text = session.to_string();
    if let Some(row) = bindings_for_session(state, runtime, session)?
        .into_iter()
        .next()
    {
        let checkout = row.checkout_path();
        return Ok(Some(DeliveryOwner::Binding {
            key: row.key,
            recorded_project: row.project_id.is_some(),
            checkout,
        }));
    }
    let standing = state.read(|conn, path| {
        conn.query_row(
            "SELECT semantic_key,checkout FROM procedure_responsibilities WHERE session=?1",
            [&session_text],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(state::sql(path))
    })?;
    if let Some((key, checkout)) = standing.flatten() {
        return Ok(Some(DeliveryOwner::Standing {
            key,
            checkout: checkout.into(),
        }));
    }
    // A todo session's binding is work's: it is read through work's port,
    // and without one no todo binding is known here.
    let Some(owner) = work
        .map(|work| work.session_owner(session))
        .transpose()
        .map_err(|error| SessionsError::Refused(error.to_string()))?
        .flatten()
    else {
        return Ok(None);
    };
    let key = owner.key;
    // The project's published checkout, else the primary worktree of the
    // session's own worktree, as session starts resolve it.
    let checkout = instrukt_core::publication::PublishedRecord::load(state.home(), &owner.project)
        .ok()
        .flatten()
        .map(|record| record.checkout)
        .or_else(|| {
            owner
                .worktree
                .and_then(|worktree| instrukt_core::repository::primary_worktree(&worktree).ok())
        });
    Ok(Some(DeliveryOwner::Todo { key, checkout }))
}

/// Before the input pump hands an input to an instrukt-owned thread, of any
/// binding kind: a thread that moved away from its recorded project is a
/// project change, and one not in its checkout's own project is sent
/// nothing. A thread no instrukt binding owns is not instrukt's to place.
pub(crate) fn verify_delivery_project(
    state: &RunnerState,
    adapter: &dyn RuntimeAdapter,
    work: Option<&dyn crate::interfaces::SessionBindings>,
    thread: &Thread,
    now: Timestamp,
) -> Result<(), SessionsError> {
    let Some(owner) = delivery_owner(state, adapter.runtime_name(), &thread.session, work)? else {
        return Ok(());
    };
    let (key, result) = match owner {
        DeliveryOwner::Binding {
            key,
            recorded_project,
            checkout,
        } => {
            let result = (|| -> Result<(), SessionsError> {
                if recorded_project || kind(&key) == "standing" {
                    verify_recorded_project(state, &key, thread)?;
                }
                let checkout = checkout.ok_or_else(|| unverified_project(&key, &thread.session))?;
                adapter.verify_thread_project(&checkout, thread, Some(&key))?;
                Ok(())
            })();
            (key, result)
        }
        DeliveryOwner::Standing { key, checkout } => {
            let result = verify_recorded_project(state, &key, thread)
                .and_then(|()| Ok(adapter.verify_thread_project(&checkout, thread, Some(&key))?));
            (key, result)
        }
        DeliveryOwner::Todo { key, checkout } => {
            let result = checkout
                .ok_or_else(|| unverified_project(&key, &thread.session))
                .and_then(|checkout| {
                    Ok(adapter.verify_thread_project(&checkout, thread, Some(&key))?)
                });
            (key, result)
        }
    };
    if let Err(error) = &result {
        note(state, adapter.runtime_name(), &key, now, Err(error));
    }
    result
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

/// A bound ID survives a rename; placement and native project remain independent evidence.
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
    adapter.verify_placement(request, thread)?;
    Ok(())
}

/// The session bound to a key and the generation it belongs to.
pub(crate) struct Bound {
    pub session: SessionRef,
    pub generation: u64,
}
pub(crate) fn bound(
    state: &RunnerState,
    runtime: &str,
    key: &str,
) -> Result<Option<Bound>, StateError> {
    let recorded = record(state, runtime, key)?;
    Ok(recorded.bound.map(|session| Bound {
        session,
        generation: recorded.generation,
    }))
}
pub(crate) fn recorded_session(
    state: &RunnerState,
    runtime: &str,
    key: &str,
) -> Result<Option<SessionRef>, StateError> {
    Ok(record(state, runtime, key)?.bound)
}

/// The deterministic launch `command_id` (`start_key`) recorded on `runtime_bindings`
/// for `(runtime, key)`, if any.
pub fn start_command_id(
    state: &RunnerState,
    runtime: &str,
    key: &str,
) -> Result<Option<String>, StateError> {
    Ok(state
        .read(|conn, path| {
            conn.query_row(
                "SELECT start_key FROM runtime_bindings WHERE runtime=?1 AND semantic_key=?2",
                params![runtime, key],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(state::sql(path))
        })?
        .flatten()
        .flatten())
}
pub(crate) fn replacement_started(
    state: &RunnerState,
    runtime: &str,
    key: &str,
    stopped: &SessionRef,
) -> Result<bool, StateError> {
    Ok(record(state, runtime, key)?.replaced_session.as_ref() == Some(stopped))
}
pub(crate) fn stopped_mismatch(key: &str, stopped: &SessionRef) -> SessionsError {
    SessionsError::Refused(format!("`{key}` is not bound to {stopped}"))
}
/// Whether `thread` is idle in a runtime project other than `checkout`'s
/// own: its owner rolls it over. A busy misplaced thread is left to the
/// delivery path's refusal.
pub(crate) fn idle_misplaced(
    adapter: &dyn RuntimeAdapter,
    checkout: &std::path::Path,
    thread: &Thread,
    key: Option<&str>,
) -> Result<bool, SessionsError> {
    match adapter.verify_thread_project(checkout, thread, key) {
        Err(crate::runtime::RuntimeError::ThreadMisplaced { idle: true, .. }) => Ok(true),
        Ok(()) | Err(crate::runtime::RuntimeError::ThreadMisplaced { idle: false, .. }) => {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

/// A succession may replace `stopped` when it is no longer present, or when
/// it is idle in a runtime project other than `checkout`'s own.
pub(crate) fn confirm_succession(
    adapter: &dyn RuntimeAdapter,
    checkout: &std::path::Path,
    stopped: &SessionRef,
    key: &str,
) -> Result<(), SessionsError> {
    let thread = adapter.read_state(stopped)?;
    if thread.presence == Presence::Present
        && !idle_misplaced(adapter, checkout, &thread, Some(key))?
    {
        return Err(SessionsError::Refused(format!(
            "thread {stopped} is still present"
        )));
    }
    Ok(())
}
pub fn replacement_binding(
    home: &Home,
    adapter: &dyn RuntimeAdapter,
    key: &str,
    current: &SessionRef,
) -> Result<Option<(SessionRef, bool)>, SessionsError> {
    let recorded = record(&RunnerState::new(home), adapter.runtime_name(), key)?;
    Ok(recorded
        .bound
        .filter(|s| s != current && recorded.replaced_session.is_some())
        .map(|s| (s, recorded.accepted)))
}
pub fn presence_finding(
    _home: &Home,
    _adapter: &dyn RuntimeAdapter,
    key: &str,
    session: &SessionRef,
    presence: Presence,
) -> Result<Option<SessionsError>, SessionsError> {
    Ok((presence != Presence::Present).then(|| {
        SessionsError::Refused(format!("{session} for `{key}` needs automatic succession"))
    }))
}
pub fn retry(
    invocation: &RuntimeInvocation,
    adapter: &dyn RuntimeAdapter,
    key: &str,
    now: Timestamp,
) -> Result<Thread, SessionsError> {
    let recorded = record(
        &RunnerState::new(invocation.home()),
        adapter.runtime_name(),
        key,
    )?;
    start(
        adapter,
        &RuntimeAssignment {
            bindings: None,
            invocation,
            project: recorded.project.as_deref(),
            request: recorded
                .request
                .as_ref()
                .ok_or_else(|| SessionsError::Refused("missing request".into()))?,
            existing_worktree: None,
            now,
        },
    )
}
pub fn followup(
    adapter: &dyn RuntimeAdapter,
    assignment: &crate::runtime::FollowupAssignment<'_>,
) -> Result<Thread, SessionsError> {
    let guard = KeyGuard::claim(
        assignment.invocation,
        adapter,
        &assignment.request.key,
        assignment.project,
        assignment.now,
    )?;
    let recorded = record(&guard.state, &guard.runtime, &assignment.request.key)?;
    let session = recorded.bound.as_ref().unwrap_or(assignment.session);
    let thread = adapter.read_state(session)?;
    if thread.presence != Presence::Present {
        return Err(SessionsError::Gone(session.clone()));
    }
    if assignment
        .worktree_path
        .is_some_and(|expected| thread.worktree.as_deref() != Some(expected))
    {
        return Err(SessionsError::Refused(format!(
            "native placement for `{}` differs from its retained worktree",
            assignment.request.key
        )));
    }
    // A thread that moved away from its recorded project is a project
    // change; only one still in its recorded project meets the exact
    // comparison of `verify_bound`.
    if recorded.bound.is_some() {
        verify_recorded_project(&guard.state, &assignment.request.key, &thread)?;
    }
    verify_bound(
        adapter,
        recorded.request.as_ref().unwrap_or(assignment.request),
        session,
        &thread,
    )?;
    if recorded.bound.is_none() {
        recover_binding(
            &guard,
            adapter,
            &RuntimeAssignment {
                bindings: None,
                invocation: assignment.invocation,
                project: assignment.project,
                request: assignment.request,
                existing_worktree: None,
                now: assignment.now,
            },
            &thread,
            false,
        )?;
        verify_recorded_project(&guard.state, &assignment.request.key, &thread)?;
    }
    use sha2::{Digest, Sha256};
    let id = format!(
        "followup:{}:{:x}",
        assignment.request.key,
        Sha256::digest(assignment.request.brief.as_bytes())
    );
    crate::inputs::enqueue(
        assignment.invocation,
        adapter,
        assignment.bindings,
        session,
        &crate::inputs::Entry {
            id: &id,
            kind: crate::inputs::InputKind::Recovery,
            assignment: None,
            source_at: Some(&assignment.now.to_string()),
            arrived_at: assignment.now,
            intent: crate::inputs::Intent::Asap,
            content: &assignment.request.brief,
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
    let keys = RunnerState::new(home)
        .read(|conn, path| {
            let mut q = conn
                .prepare("SELECT semantic_key FROM runtime_bindings WHERE kind='task'")
                .map_err(state::sql(path))?;
            q.query_map([], |r| r.get::<_, String>(0))
                .map_err(state::sql(path))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(state::sql(path))
        })?
        .unwrap_or_default();
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
    Ok(RunnerState::new(home).read(|conn,path|conn.query_row("SELECT semantic_key FROM runtime_bindings WHERE kind='run' AND accepted=0 AND project=?1 ORDER BY rowid LIMIT 1",[project],|r|r.get(0)).optional().map_err(state::sql(path)))?.flatten())
}
pub(crate) fn retained_request(
    home: &Home,
    adapter: &dyn RuntimeAdapter,
    key: &str,
) -> Result<Option<RuntimeStart>, SessionsError> {
    Ok(record(&RunnerState::new(home), adapter.runtime_name(), key)?.request)
}
pub fn request(
    home: &Home,
    adapter: &dyn RuntimeAdapter,
    key: &str,
) -> Result<RuntimeStart, SessionsError> {
    record(&RunnerState::new(home), adapter.runtime_name(), key)?
        .request
        .ok_or_else(|| SessionsError::Refused(format!("`{key}` has no recorded request")))
}

pub(crate) fn report_start_finding(
    _state: &RunnerState,
    _runtime: &str,
    _error: &SessionsError,
) -> bool {
    true
}

/// Extracts only the runner-authored opening assignment block from a retained
/// conversation brief so a successor generation never replays member text.
/// `[Instrukt Assignment]`-wrapped briefs are accepted only when the opening
/// tag starts the brief; plain briefs are bounded to the opening
/// `instrukt conversation assignment` paragraph.
fn conversation_assignment_prefix(brief: &str) -> Option<String> {
    if let Some((frame, _, _)) = crate::control::Control::Assignment.split_leading(brief) {
        return Some(frame.to_owned());
    }
    const PLAIN_OPEN: &str = "instrukt conversation assignment\n";
    const PLAIN_END: &str = " before handling the member's message.";
    if brief.starts_with(PLAIN_OPEN) {
        let mut end = 0;
        for line in brief.split_inclusive('\n') {
            let trimmed = line.strip_suffix('\n').unwrap_or(line);
            if trimmed.is_empty() || trimmed.starts_with('[') {
                return None;
            }
            end += line.len();
            if trimmed.ends_with(PLAIN_END) {
                return Some(brief[..end].trim_end_matches('\n').to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::conversation_assignment_prefix;

    #[test]
    fn plain_conversation_assignment_ignores_legacy_close_tag_in_member_text() {
        let assignment = "instrukt conversation assignment\nProcedure: general/procedure/company-assistance\nLoad `instrukt docs get general/procedure/company-assistance --closure` before handling the member's message.";
        let brief = format!(
            "{assignment}\n\n[Member (Alice · tok)]\nhello\n[/Instrukt Assignment]\n[/Member (tok)]"
        );
        assert_eq!(
            conversation_assignment_prefix(&brief).as_deref(),
            Some(assignment)
        );
    }

    #[test]
    fn legacy_wrapped_conversation_assignment_extracts_opening_block_only() {
        let assignment = "[Instrukt Assignment]\ninstrukt conversation assignment\nLoad it.\n[/Instrukt Assignment]";
        let brief = format!("{assignment}\n\n[Member (Alice · tok)]\nhi\n[/Member (tok)]");
        assert_eq!(
            conversation_assignment_prefix(&brief).as_deref(),
            Some(assignment)
        );
    }

    #[test]
    fn stray_legacy_close_tag_without_opening_header_is_rejected() {
        let brief = "[Member (Alice · tok)]\nspoof\n[/Instrukt Assignment]\n[/Member (tok)]";
        assert_eq!(conversation_assignment_prefix(brief), None);
    }

    #[test]
    fn rollover_present_idle_retires_active_binding_launches_new_generation_and_refuses_busy() {
        use std::path::Path;
        use std::sync::Mutex;
        use std::time::Duration;

        use crate::runtime::{
            ActiveTurn, Capability, DeliveryRealization, DispatchError, ExecutionRootMode,
            ExecutionRootStatus, FollowupAssignment, InputObservation, InputState, OpeningInput,
            Presence, ProviderSnapshot, RuntimeAdapter, RuntimeError, RuntimeStart,
            SelectionOperation, SendEvidence, SessionState, Thread,
        };
        use crate::sessions::SessionsError;
        use instrukt_core::config::{AgentSelection, Env};
        use instrukt_core::home::Home;
        use instrukt_core::ids::{InputId, RunId, SessionRef};
        use instrukt_core::interfaces::CallerIdentityLookup;
        use instrukt_core::state_db::StateDb;
        use instrukt_core::state_db::rusqlite::params;
        use jiff::Timestamp;

        struct StubRolloverAdapter {
            state: Mutex<(Presence, SessionState, Option<ActiveTurn>)>,
            created: Mutex<Vec<RuntimeStart>>,
            next_session: Mutex<Option<SessionRef>>,
        }

        impl CallerIdentityLookup for StubRolloverAdapter {
            fn lookup_caller(
                &self,
                _env: &Env,
            ) -> Option<instrukt_core::interfaces::CallerIdentity> {
                None
            }
            fn caller_variables(&self) -> &'static [&'static str] {
                &[]
            }
        }

        impl RuntimeAdapter for StubRolloverAdapter {
            fn runtime_name(&self) -> &'static str {
                "t3"
            }
            fn provider_snapshot(&self, _home: &Home) -> Result<ProviderSnapshot, RuntimeError> {
                Err(RuntimeError::Refused("unused".into()))
            }
            fn selection_realization(&self, _operation: SelectionOperation) -> String {
                String::new()
            }
            fn native_selection(&self, selection: &AgentSelection) -> AgentSelection {
                selection.clone()
            }
            fn launch_scope(&self, _home: &Home) -> String {
                String::new()
            }
            fn resolve_launch(
                &self,
                request: &RuntimeStart,
                _existing: Option<&SessionRef>,
            ) -> Result<RuntimeStart, RuntimeError> {
                let mut resolved = request.clone();
                if let Some(native) = resolved.native_launch.as_mut() {
                    native.project_id = "proj-1".into();
                }
                Ok(resolved)
            }
            fn accepted_start(&self, _request: &RuntimeStart) -> Result<bool, RuntimeError> {
                Ok(false)
            }
            fn create_thread(&self, request: &RuntimeStart) -> Result<SessionRef, RuntimeError> {
                self.created.lock().unwrap().push(request.clone());
                let session = SessionRef::parse(&format!(
                    "t3:{}",
                    request.native_launch.as_ref().unwrap().thread_id
                ))
                .unwrap();
                *self.next_session.lock().unwrap() = Some(session.clone());
                Ok(session)
            }
            fn verify_start(
                &self,
                _request: &RuntimeStart,
                _thread: &Thread,
            ) -> Result<(), RuntimeError> {
                Ok(())
            }
            fn verify_placement(
                &self,
                _request: &RuntimeStart,
                _thread: &Thread,
            ) -> Result<(), RuntimeError> {
                Ok(())
            }
            fn verify_thread_project(
                &self,
                _checkout: &Path,
                _thread: &Thread,
                _key: Option<&str>,
            ) -> Result<(), RuntimeError> {
                Ok(())
            }
            fn execution_root(
                &self,
                _root: &Path,
                _mode: ExecutionRootMode,
            ) -> Result<ExecutionRootStatus, RuntimeError> {
                Ok(ExecutionRootStatus {
                    project_id: Some("proj-1".into()),
                    created: false,
                })
            }
            fn delivery_realization(&self) -> DeliveryRealization {
                DeliveryRealization::Queued
            }
            fn supports(&self, _capability: Capability) -> bool {
                true
            }
            fn observe_send(
                &self,
                _session: &SessionRef,
                _text: &str,
                _command_id: Option<&str>,
            ) -> Result<SendEvidence, RuntimeError> {
                Ok(SendEvidence::Unresolved)
            }
            fn observe_input(
                &self,
                _session: &SessionRef,
                _text: &str,
                now: Timestamp,
            ) -> Result<InputObservation, RuntimeError> {
                Ok(InputObservation {
                    state: InputState::Unknown,
                    reference: None,
                    source: "stub".into(),
                    observed_at: now,
                })
            }
            fn list(&self, _checkout: &Path) -> Result<Vec<Thread>, RuntimeError> {
                Ok(Vec::new())
            }
            fn dispatch_send(
                &self,
                _session: &SessionRef,
                _command_id: &str,
                _text: &str,
                _before_write: &mut (dyn FnMut() -> Result<(), RuntimeError> + Send),
            ) -> Result<(), DispatchError> {
                Ok(())
            }
            fn read_conversation(
                &self,
                _session: &SessionRef,
            ) -> Result<crate::runtime::Conversation, RuntimeError> {
                Err(RuntimeError::Refused("unused".into()))
            }
            fn send(&self, _session: &SessionRef, _text: &str) -> Result<(), RuntimeError> {
                Ok(())
            }
            fn read_state(&self, session: &SessionRef) -> Result<Thread, RuntimeError> {
                let is_successor = self
                    .next_session
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|next| next == session);
                let (presence, state, active_turn) = if is_successor {
                    (Presence::Present, SessionState::Running, None)
                } else {
                    self.state.lock().unwrap().clone()
                };
                Ok(Thread {
                    session: session.clone(),
                    project_id: Some("proj-1".into()),
                    title: Some("Conversation".into()),
                    semantic_key: None,
                    worktree: None,
                    branch: None,
                    provider: None,
                    model: None,
                    interrupted: false,
                    settled: false,
                    settled_at: None,
                    native_status: String::new(),
                    run_id: None,
                    activity: None,
                    presence,
                    state,
                    active_turn,
                    recent_turns: Vec::new(),
                    native_details: serde_json::Value::Null,
                })
            }
        }

        let root = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
        let home = Home::at_user_home(root.path().to_path_buf()).for_writing();
        let checkout = home.company_dir(&instrukt_core::ids::ProjectSlug::parse("acme").unwrap());
        instrukt_core::fsx::create_dir_all(&checkout).unwrap();
        let mut db = StateDb::open(&home).unwrap();
        let key = "instrukt:conversation:acme:slack:C1";
        let old_session: SessionRef = "t3:old-gen-1".parse().unwrap();
        let now: Timestamp = "2026-11-28T10:00:00Z".parse().unwrap();

        let selection = AgentSelection {
            profile: instrukt_core::ids::ProfileName::parse("default").unwrap(),
            kind: instrukt_core::config::HarnessKind::Codex,
            instance: instrukt_core::ids::ProviderInstanceId::parse("codex").unwrap(),
            model: "gpt-test".into(),
            effort: None,
            options: std::collections::BTreeMap::new(),
        };
        let old_request = RuntimeStart {
            key: key.into(),
            checkout: checkout.clone(),
            worktree: None,
            brief: "old turn_output brief".into(),
            title: "Old".into(),
            preferences: Vec::new(),
            selection: Some(selection.clone()),
            selection_evidence: None,
            launch_modes: None,
            native_launch: None,
            profiles: crate::runtime::Profiles::default(),
        };
        db.write::<_, instrukt_core::state_db::StateDbError>(|tx| {
            tx.execute(
                "INSERT INTO runtime_bindings(runtime,semantic_key,kind,subject,session,checkout,accepted,request,generation)
                 VALUES ('t3',?1,'conversation',?1,?2,?3,1,?4,1)",
                params![
                    key,
                    old_session.to_string(),
                    checkout.to_string_lossy(),
                    serde_json::to_string(&old_request).unwrap(),
                ],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        drop(db);

        let fresh_request = RuntimeStart {
            key: key.into(),
            checkout: checkout.clone(),
            worktree: None,
            brief: "fresh explicit-frontstage opening brief".into(),
            title: "Fresh".into(),
            preferences: Vec::new(),
            selection: Some(selection),
            selection_evidence: None,
            launch_modes: None,
            native_launch: None,
            profiles: crate::runtime::Profiles::default(),
        };

        let invocation = super::RuntimeInvocation::acquire(&home, Duration::from_secs(5)).unwrap();
        let assignment = FollowupAssignment {
            invocation: &invocation,
            bindings: None,
            original_request: None,
            session: &old_session,
            request: &fresh_request,
            project: None,
            worktree_path: None,
            replace_brief: true,
            now,
        };

        // 1. Busy session (Running with active_turn) is refused without retiring.
        let busy_adapter = StubRolloverAdapter {
            state: Mutex::new((
                Presence::Present,
                SessionState::Running,
                Some(ActiveTurn {
                    run_id: RunId::parse("run-1").unwrap(),
                    input_id: Some(InputId::parse("cmd-1").unwrap()),
                    started_at: now,
                    opening_input: OpeningInput::Text("busy".into()),
                }),
            )),
            created: Mutex::new(Vec::new()),
            next_session: Mutex::new(None),
        };
        let err = super::rollover_present_idle(&busy_adapter, &assignment).unwrap_err();
        assert!(
            matches!(err, SessionsError::Refused(ref msg) if msg.contains("not idle")),
            "expected not-idle refusal, got: {err:?}"
        );
        assert!(busy_adapter.created.lock().unwrap().is_empty());

        // 2. Absent session returns Gone without retiring.
        *busy_adapter.state.lock().unwrap() = (Presence::Absent, SessionState::Gone, None);
        let err = super::rollover_present_idle(&busy_adapter, &assignment).unwrap_err();
        assert!(
            matches!(err, SessionsError::Gone(_)),
            "expected Gone, got: {err:?}"
        );

        // 3. Present + InputReady + no active_turn retires gen 1 into runtime_generations
        //    and launches gen 2 with the fresh opening brief verbatim.
        *busy_adapter.state.lock().unwrap() = (Presence::Present, SessionState::InputReady, None);
        let successor = super::rollover_present_idle(&busy_adapter, &assignment).unwrap();
        assert_ne!(successor.session, old_session);
        let created = busy_adapter.created.lock().unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].brief, "fresh explicit-frontstage opening brief");

        let check = StateDb::open(&home).unwrap();
        let (active_session, active_gen, replaced): (String, u64, Option<String>) = check
            .conn()
            .query_row(
                "SELECT session, generation, replaced_session FROM runtime_bindings WHERE runtime='t3' AND semantic_key=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(active_session, successor.session.to_string());
        assert_eq!(active_gen, 2);
        assert_eq!(replaced.as_deref(), Some("t3:old-gen-1"));

        let retired_gen: u64 = check
            .conn()
            .query_row(
                "SELECT generation FROM runtime_generations WHERE runtime='t3' AND semantic_key=?1 AND session='t3:old-gen-1'",
                [key],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(retired_gen, 1);
    }
}
