//! Durable keyed dispatch. Exact-key OS ownership spans native operations.
//! Key ownership is kept in SQLite; no writer transaction spans runtime I/O.
use instrukt_core::fsx::LockGuard;
use instrukt_core::home::Home;
use instrukt_core::ids::SessionRef;
use instrukt_core::process::{Limit, Look};
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use jiff::Timestamp;
use std::time::Duration;

use crate::runtime::{Presence, RuntimeAdapter, RuntimeAssignment, RuntimeStart, Thread};

mod _findings;
use crate::sessions::SessionsError;
use crate::state::{self, RunnerState, StateError};
pub(crate) use _findings::note;

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
        use sha2::{Digest, Sha256};
        let runtime = adapter.runtime().name().to_owned();
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
) -> Result<crate::runtime::NativeLaunch, SessionsError> {
    use sha2::{Digest, Sha256};
    // JSON preserves component boundaries, including exact home and checkout scope.
    let scope = serde_json::to_vec(&(
        adapter.runtime().name(),
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
        existing_worktree: None,
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
    runtime: instrukt_core::config::Runtime,
    input: &RuntimeStart,
    thread: &Thread,
    project: Option<&str>,
) -> Result<(), StateError> {
    let path = state::db_path(tx);
    let generation = tx
        .query_row(
            OWNER_GENERATION,
            params![runtime.name(), input.key],
            |row| row.get::<_, u64>(0),
        )
        .map_err(state::sql(&path))?;
    let request = serde_json::to_string(input).map_err(|e| state::invalid(&path, e))?;
    tx.execute("INSERT INTO runtime_bindings(runtime,semantic_key,kind,project,subject,session,project_id,checkout,worktree,branch,base_head,accepted,request,display_title,generation) VALUES(?1,?2,?3,?4,?2,?5,?6,?7,?8,?9,?10,1,?11,?12,?13) ON CONFLICT(runtime,semantic_key) DO UPDATE SET project_id=coalesce(excluded.project_id,runtime_bindings.project_id),accepted=1,worktree=excluded.worktree",params![runtime.name(),input.key,kind(&input.key),project,thread.session.to_string(),thread.project_id,input.checkout.to_string_lossy(),thread.worktree.as_ref().map(|p|p.to_string_lossy()),thread.branch,input.worktree.as_ref().map(|w|&w.base),request,thread.title,generation]).map_err(state::sql(&path))?;
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

pub(crate) fn start(
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
        adapter.runtime(),
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
            capture_legacy(adapter, guard, input, &recorded, session)?;
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
            capture_legacy(adapter, guard, input, &recorded, session)?;
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
                &input.key,
                assignment.now,
                work,
                guard,
                Some(assignment.request),
            );
        }
    }
    if let (Some(session), Some(input)) = (&recorded.bound, &recorded.request)
        && input.native_launch.is_some()
    {
        let thread = adapter.read_state(session)?;
        if thread.presence != Presence::Present && adapter.accepted_start(input)? {
            return succeed_claimed(
                assignment.invocation,
                adapter,
                &input.key,
                assignment.now,
                work,
                guard,
                None,
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
/// window has elapsed; the last answer is the start's.
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
            Err(SessionsError::PlacementPreparing(_))
                if window.judge(None::<()>) == Look::Running =>
            {
                std::thread::sleep(PLACEMENT_PAUSE);
            }
            Err(error) => return Err(error),
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
        remember_manual(tx, adapter.runtime(), input, thread, assignment.project)?;
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
        if let Err(error) = work.reflect_dispatch_replacement(&project, &input.key) {
            tracing::warn!(event="dispatch_reflection_pending",%error);
        }
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
    session: &SessionRef,
) -> Result<(), SessionsError> {
    let mut realized = input.clone();
    realized.native_launch = Some(crate::runtime::NativeLaunch {
        command_id: String::new(),
        thread_id: session.id.to_string(),
        project_id: recorded.project_id.clone().unwrap_or_default(),
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

fn retired_binding(
    state: &RunnerState,
    runtime: &str,
    key: &str,
    session: &SessionRef,
) -> Result<bool, StateError> {
    state.read(|conn,path|conn.query_row("SELECT EXISTS(SELECT 1 FROM runtime_generations WHERE runtime=?1 AND semantic_key=?2 AND session=?3 AND generation>0)",params![runtime,key,session.to_string()],|row|row.get::<_,bool>(0)).map_err(state::sql(path)))?.ok_or_else(||state::invalid(&state.home().state_db_file(),"runtime state disappeared during key ownership"))
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
    if retired_binding(
        &guard.state,
        &guard.runtime,
        &assignment.request.key,
        &thread.session,
    )? {
        return Err(SessionsError::Refused("the owner record names a retired native generation; restore its current binding before dispatch".into()));
    }
    adapter.verify_placement(assignment.request, thread)?;
    let mut input = assignment.request.clone();
    input.native_launch = Some(crate::runtime::NativeLaunch {
        command_id: String::new(),
        thread_id: thread.session.id.to_string(),
        project_id: recorded.project_id.clone().unwrap_or_default(),
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
        remember_manual(tx,adapter.runtime(),&input,thread,assignment.project)?;
        tx.execute("UPDATE runtime_bindings SET project_id=?3,request=CASE WHEN ?4 THEN request ELSE NULL END,legacy_modes=CASE WHEN ?4 THEN NULL ELSE ?5 END WHERE runtime=?1 AND semantic_key=?2",params![guard.runtime,assignment.request.key,project,known_request,modes]).map_err(state::sql(&state::db_path(tx)))?;Ok(())
    })?;
    Ok(())
}

/// Called only for outstanding owner-admitted work after confirmed native terminal presence.
pub fn succeed(
    adapter: &dyn RuntimeAdapter,
    assignment: &crate::runtime::FollowupAssignment<'_>,
) -> Result<Thread, SessionsError> {
    let invocation = assignment.invocation;
    let key = &assignment.request.key;
    let now = assignment.now;
    let guard = KeyGuard::claim(invocation, adapter, key, assignment.project, now)?;
    if record(&guard.state, &guard.runtime, key)?.bound.is_none() {
        if retired_binding(&guard.state, &guard.runtime, key, assignment.session)? {
            return Err(SessionsError::Refused("the owner record names a retired native generation; restore its current binding before dispatch".into()));
        }
        let mut thread = adapter.read_state(assignment.session)?;
        let source = assignment.original_request.unwrap_or(assignment.request);
        if thread.presence == Presence::Present {
            recover_binding(
                &guard,
                adapter,
                &RuntimeAssignment {
                    bindings: assignment.bindings,
                    invocation,
                    project: assignment.project,
                    request: source,
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
                remember_manual(tx,adapter.runtime(),source,&thread,assignment.project)?;
                if assignment.original_request.is_none() {tx.execute("UPDATE runtime_bindings SET request=NULL WHERE runtime=?1 AND semantic_key=?2",params![guard.runtime,key]).map_err(state::sql(&state::db_path(tx)))?;}
                Ok(())
            })?;
        }
    }
    let result = succeed_claimed(
        invocation,
        adapter,
        key,
        now,
        assignment.bindings,
        &guard,
        Some(assignment.request),
    );
    note(
        &guard.state,
        adapter.runtime(),
        key,
        now,
        result.as_ref().map(|_| ()),
    );
    result
}
fn succeed_claimed(
    invocation: &RuntimeInvocation,
    adapter: &dyn RuntimeAdapter,
    key: &str,
    now: Timestamp,
    work: Option<&dyn crate::interfaces::SessionBindings>,
    guard: &KeyGuard,
    replacement_request: Option<&RuntimeStart>,
) -> Result<Thread, SessionsError> {
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
    if thread.presence == Presence::Present || !created {
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
        request.brief = request
            .brief
            .split_once("\n[/Instrukt Assignment]")
            .map(|(assignment, _)| format!("{assignment}\n[/Instrukt Assignment]"))
            .ok_or_else(|| {
                SessionsError::Refused("missing conversation assignment envelope".into())
            })?;
    }
    if let Some(replacement) = replacement_request {
        request.title = replacement.title.clone();
    }
    request.launch_modes = None;
    let generation = recorded
        .generation
        .checked_add(1)
        .ok_or_else(|| SessionsError::Refused("generation exhausted".into()))?;
    let mut native = native_identity(invocation.home(), adapter, &request, generation)?;
    native.project_id = recorded.project_id.clone().unwrap_or_default();
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
    native.existing_worktree = thread.worktree.clone().or(retained_path);
    if request.worktree.is_none()
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
        tx.execute("UPDATE runtime_inputs SET state='failed',waiting=0,failure='native terminal target; outstanding owner work retained for successor' WHERE runtime=?1 AND session=?2 AND state IN ('pending','selected','accepted')",params![session.runtime.as_str(),session.id.as_str()]).map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })?;
    start_with_work(
        adapter,
        &RuntimeAssignment {
            bindings: None,
            invocation,
            project: recorded.project.as_deref(),
            request: &request,
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
    adapter.verify_placement(request, thread)
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
    Ok(record(state, runtime, key)?.replaced_session.as_ref() == Some(stopped))
}
pub(crate) fn stopped_mismatch(key: &str, stopped: &SessionRef) -> SessionsError {
    SessionsError::Refused(format!("`{key}` is not bound to {stopped}"))
}
pub(crate) fn confirm_deleted(
    adapter: &dyn RuntimeAdapter,
    stopped: &SessionRef,
) -> Result<(), SessionsError> {
    if adapter.read_state(stopped)?.presence == Presence::Present {
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
    let recorded = record(&RunnerState::new(home), adapter.runtime().name(), key)?;
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
        adapter.runtime().name(),
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
                now: assignment.now,
            },
            &thread,
            false,
        )?;
    }
    verify_recorded_project(&guard.state, &assignment.request.key, &thread)?;
    use sha2::{Digest, Sha256};
    let id = format!(
        "followup:{}:{:x}",
        assignment.request.key,
        Sha256::digest(assignment.request.brief.as_bytes())
    );
    crate::inputs::enqueue(
        assignment.invocation,
        adapter,
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

pub(crate) fn report_start_finding(
    _state: &RunnerState,
    _runtime: instrukt_core::config::Runtime,
    _error: &SessionsError,
) -> bool {
    true
}
