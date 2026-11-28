//! Concrete T3 runtime adapter: live RPC/CLI/HTTP spine with zero live `statev2.sqlite` reads,
//! plus strictly isolated non-live `ColdTextSource` (`history`) and `Archive` (`thread-retention`).
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use instrukt_core::config::{AgentSelection, Env};
use instrukt_core::home::Home;
use instrukt_core::ids::SessionRef;
use instrukt_core::interfaces::{CallerIdentity, CallerIdentityLookup, ConfiguredInstance};
use instrukt_core::process::{Invocation, ProcessError};
use instrukt_runtime::DispatchError::BeforeFrame;
use instrukt_runtime::cold_text::ColdTextSource;
use instrukt_runtime::{
    Archive, ArchiveOutcome, AssignmentEvidence, Capability, Conversation, DeliveryRealization,
    DispatchError, ExecutionRootCause, ExecutionRootMode, ExecutionRootStatus, FinishedThread,
    InputObservation, Message, NativeActivity, NativeCause, NativeDiagnostic, ObservedSnapshot,
    Presence, ProviderSnapshot, RUNTIME_COMMAND_TIMEOUT, Readiness, RuntimeAdapter, RuntimeError,
    RuntimeStart, SelectionOperation, SendEvidence, SessionState, Thread, diagnose,
};
use serde::Deserialize;

use crate::_projection::{self, ThreadStatus};
use crate::_t3_readiness::ReadTokenLease;

const ONE_SHOT_READ_TIMEOUT: Duration = Duration::from_secs(5);
const LONG_LIVED_READ_TIMEOUT: Duration = Duration::from_secs(10);

pub struct T3 {
    timeout: Duration,
    env: Env,
    read_lease: Option<Arc<ReadTokenLease>>,
}

impl Default for T3 {
    fn default() -> Self {
        Self::with_env(Env::from_process())
    }
}

impl T3 {
    /// One-shot T3 adapter (bounded per-call tokens, no held read lease).
    pub fn with_env(env: Env) -> Self {
        Self {
            timeout: RUNTIME_COMMAND_TIMEOUT,
            env,
            read_lease: None,
        }
    }

    /// Long-lived T3 adapter for daemon loops (`instrukt run`), reusing a shared `ReadTokenLease`.
    pub fn long_lived(env: Env) -> Self {
        Self {
            timeout: RUNTIME_COMMAND_TIMEOUT,
            env,
            read_lease: Some(Arc::new(ReadTokenLease::new())),
        }
    }

    fn default_read_timeout(&self) -> Duration {
        if self.read_lease.is_some() {
            LONG_LIVED_READ_TIMEOUT
        } else {
            ONE_SHOT_READ_TIMEOUT
        }
    }

    fn operation_home(&self) -> Result<Home, RuntimeError> {
        let user_home = self
            .env
            .get("HOME")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| failure("launch", "machine home unavailable"))?;
        Ok(Home::at_user_home(user_home.into()))
    }

    fn call(&self, args: &[&str], input: Option<&str>) -> Result<String, RuntimeError> {
        let mut invocation = Invocation::program("t3", args);
        if let Some(input) = input {
            invocation = invocation.input(input.as_bytes());
        }
        let result = invocation.run(self.timeout)?;
        if !result.success {
            return Err(failure(
                args.get(1).copied().unwrap_or("thread"),
                result.stderr,
            ));
        }
        Ok(result.stdout)
    }

    fn fetch_projection(
        &self,
        session: &SessionRef,
        timeout: Duration,
    ) -> Result<serde_json::Value, RuntimeError> {
        if session.runtime.as_str() != "t3" {
            return Err(RuntimeError::UnknownRuntime(session.runtime.to_string()));
        }
        crate::_t3_readiness::get_thread_projection(
            &self.operation_home()?,
            &self.env,
            self.read_lease.as_deref(),
            session.id.as_str(),
            timeout,
        )
    }

    fn show(&self, session: &SessionRef, limit: usize) -> Result<Shown, RuntimeError> {
        if session.runtime.as_str() != "t3" {
            return Err(RuntimeError::UnknownRuntime(session.runtime.to_string()));
        }
        let result = self.call(
            &[
                "thread",
                "show",
                "--json",
                "--messages",
                &limit.to_string(),
                "--",
                session.id.as_str(),
            ],
            None,
        );
        let out = match result {
            Err(RuntimeError::Failed { detail, .. })
                if detail.trim() == format!("Thread {} is no longer available.", session.id) =>
            {
                return Err(RuntimeError::ThreadNotFound {
                    thread_id: session.id.to_string(),
                });
            }
            result => result?,
        };
        let shown: Shown =
            serde_json::from_str(&out).map_err(|e| RuntimeError::InvalidResponse {
                runtime: "t3".into(),
                operation: "show".into(),
                category: match e.classify() {
                    serde_json::error::Category::Io => "io",
                    serde_json::error::Category::Syntax => "syntax",
                    serde_json::error::Category::Data => "data",
                    serde_json::error::Category::Eof => "eof",
                },
                line: e.line(),
                column: e.column(),
            })?;
        if shown.thread.thread_id != session.id.as_str() {
            return Err(failure("show", "response identifies a different thread"));
        }
        if shown.thread.pending_request_count.is_none() || shown.thread.archived.is_none() {
            return Err(failure("show", "missing observation metadata"));
        }
        Ok(shown)
    }

    fn create_thread_inner(&self, request: &RuntimeStart) -> Result<SessionRef, RuntimeError> {
        let launch = request
            .native_launch
            .as_ref()
            .ok_or_else(|| failure("launch", "missing captured native identity"))?;
        let modes = request
            .launch_modes
            .as_ref()
            .ok_or_else(|| failure("launch", "missing captured modes"))?;
        let selection = request
            .selection
            .as_ref()
            .ok_or_else(|| failure("launch", "missing provider/model selection"))?;
        let workspace = match (&request.worktree, &launch.existing_worktree) {
            (Some(worktree), Some(path)) => {
                serde_json::json!({"type":"existing_worktree", "worktreePath":path, "branch":worktree.branch})
            }
            (Some(worktree), None) => {
                serde_json::json!({"type":"worktree", "baseRef":worktree.from, "branch":worktree.branch})
            }
            (None, Some(path)) => {
                let branch = instrukt_core::git::run(path, ["branch", "--show-current"])
                    .map_err(|e| failure("launch", e.to_string()))?;
                let branch = branch.trim();
                if branch.is_empty() {
                    return Err(failure(
                        "launch",
                        format!(
                            "existing worktree {} has no checked-out branch",
                            path.display()
                        ),
                    ));
                }
                serde_json::json!({"type":"existing_worktree","worktreePath":path,"branch":branch})
            }
            (None, None) => serde_json::json!({"type":"root"}),
        };
        let value = crate::_t3_readiness::launch(
            &self.operation_home()?,
            &self.env,
            serde_json::json!({
                "commandId": launch.command_id,
                "threadId": launch.thread_id,
                "projectId": launch.project_id,
                "title": request.title,
                "generateTitle": false,
                "creationSource": "mcp",
                "modelSelection": crate::_t3_options::model_selection(selection),
                "runtimeMode": modes.runtime,
                "interactionMode": modes.interaction,
                "workspaceStrategy": workspace,
                "initialMessage": {"messageId": launch.command_id, "text": request.brief, "attachments": []}
            }),
        )?;
        if value["threadId"].as_str() != Some(&launch.thread_id)
            || value
                .pointer("/projection/thread/projectId")
                .and_then(serde_json::Value::as_str)
                != Some(&launch.project_id)
        {
            return Err(failure(
                "launch",
                "native launch identity/project differs from captured request",
            ));
        }
        SessionRef::parse(&format!("t3:{}", launch.thread_id))
            .map_err(|e| failure("launch", e.to_string()))
    }
}

impl CallerIdentityLookup for T3 {
    fn lookup_caller(&self, env: &Env) -> Option<CallerIdentity> {
        let id = env.get("T3CODE_THREAD_ID")?;
        Some(CallerIdentity {
            session: SessionRef::parse(&format!("t3:{id}")).ok()?,
            agent: None,
        })
    }

    fn caller_variables(&self) -> &'static [&'static str] {
        &["T3CODE_THREAD_ID"]
    }
}

impl Archive for T3 {
    fn finished_threads(&self, checkout: &Path) -> Result<Vec<FinishedThread>, RuntimeError> {
        let out = self.call(
            &[
                "thread",
                "list",
                "--project",
                &checkout.to_string_lossy(),
                "--json",
            ],
            None,
        )?;
        let listed: Vec<serde_json::Value> =
            serde_json::from_str(&out).map_err(|e| failure("list", e.to_string()))?;
        let finished: Vec<ListedFinished> =
            listed.iter().filter_map(ListedFinished::read).collect();
        let ids: Vec<&str> = finished.iter().map(|thread| thread.id.as_str()).collect();
        let rows =
            crate::_t3_conversations::retention_rows(&self.operation_home()?, &self.env, &ids)
                .map_err(|error| failure("finished threads", error.to_string()))?;
        finished
            .into_iter()
            .map(|thread| {
                let row = rows.get(&thread.id);
                thread.observed(row)
            })
            .collect()
    }

    fn archive(
        &self,
        observed: &FinishedThread,
        now: jiff::Timestamp,
    ) -> Result<ArchiveOutcome, RuntimeError> {
        let session = &observed.session;
        if session.runtime.as_str() != "t3" {
            return Err(RuntimeError::UnknownRuntime(session.runtime.to_string()));
        }
        let Some(latest_activity) = observed.latest_activity else {
            return Ok(ArchiveOutcome::Changed);
        };
        let home = self.operation_home()?;
        let thread = session.id.as_str();
        let holds =
            crate::_t3_conversations::archive_holds(&home, &self.env, thread, latest_activity, now)
                .map_err(|error| failure("archive", error.to_string()))?;
        if !holds {
            return Ok(ArchiveOutcome::Changed);
        }
        let command_id = format!(
            "instrukt:retention:{}",
            instrukt_core::machine::random_hex(16)
                .map_err(|error| failure("archive", error.to_string()))?
        );
        let dispatched = crate::_t3_readiness::dispatch(
            &home,
            &self.env,
            "instrukt-retention",
            "archive",
            serde_json::json!({
                "type": "thread.archive",
                "commandId": command_id,
                "threadId": thread,
            }),
            &mut || Ok(()),
        );
        match dispatched {
            Ok(()) => Ok(ArchiveOutcome::Archived),
            Err(error) => match crate::_t3_conversations::archived(&home, &self.env, thread) {
                Ok(true) => Ok(ArchiveOutcome::AlreadyArchived),
                _ => Err(error.into_inner()),
            },
        }
    }
}

impl RuntimeAdapter for T3 {
    fn runtime_name(&self) -> &'static str {
        crate::T3_DECLARATION.name
    }

    fn diagnostic(&self, operation: &str, error: &RuntimeError) -> NativeDiagnostic {
        let native = match error {
            RuntimeError::Failed { detail, .. } => crate::_t3_diagnostics::cause(detail),
            _ => None,
        };
        diagnose("t3", operation, error, native)
    }

    fn send_certainly_rejected(&self, session: &SessionRef, error: &RuntimeError) -> bool {
        match error {
            RuntimeError::Process(ProcessError::Spawn { .. })
            | RuntimeError::ExecutionRootUnavailable { .. }
            | RuntimeError::ThreadMisplaced { .. }
            | RuntimeError::CommandPreviouslyRejected { .. } => true,
            RuntimeError::Failed {
                operation, detail, ..
            } if operation == "send" => {
                let line = detail.lines().next().unwrap_or_default().trim();
                line == format!("Thread {} is no longer available.", session.id)
                    || matches!(
                        crate::_t3_diagnostics::cause(line),
                        Some(NativeCause::ServerNotRunning)
                    )
                    || line
                        .strip_prefix("Could not reach the T3 Code server at ")
                        .and_then(|rest| rest.split_once(". "))
                        .is_some_and(|(_, reason)| reason.starts_with("connect ECONNREFUSED "))
            }
            _ => false,
        }
    }

    fn cold_text_source(
        &self,
        home: &Home,
        env: &Env,
    ) -> Result<Box<dyn ColdTextSource>, RuntimeError> {
        Ok(Box::new(crate::_t3_conversations::T3Conversations::new(
            home, env,
        )))
    }

    fn provider_snapshot(&self, home: &Home) -> Result<ProviderSnapshot, RuntimeError> {
        crate::_t3_readiness::snapshot(home, &self.env)
    }

    fn readiness(&self) -> Option<Box<dyn Readiness>> {
        Some(Box::new(crate::_t3_readiness::Readiness::new(
            self.env.clone(),
        )))
    }

    fn last_snapshot(&self) -> Option<Arc<ObservedSnapshot>> {
        crate::_t3_readiness::last()
    }

    fn configured_agent_runtimes(
        &self,
        home: &Home,
        env: &Env,
    ) -> Result<Vec<ConfiguredInstance>, RuntimeError> {
        crate::_t3_readiness::configured(home, env)
    }

    fn launch_scope(&self, home: &Home) -> String {
        crate::_t3_home::T3Home::discover(home, &self.env)
            .database()
            .to_string_lossy()
            .into_owned()
    }

    fn accepted_start(&self, request: &RuntimeStart) -> Result<bool, RuntimeError> {
        let launch = request
            .native_launch
            .as_ref()
            .ok_or_else(|| failure("launch", "missing native identity"))?;
        match crate::_t3_readiness::get_thread_projection(
            &self.operation_home()?,
            &self.env,
            self.read_lease.as_deref(),
            &launch.thread_id,
            self.default_read_timeout(),
        ) {
            Ok(projection) => {
                _projection::verify_thread_exists(projection, &launch.thread_id)?;
                Ok(true)
            }
            Err(RuntimeError::ThreadNotFound { .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn resolve_launch(
        &self,
        request: &RuntimeStart,
        existing: Option<&SessionRef>,
    ) -> Result<RuntimeStart, RuntimeError> {
        let home = self.operation_home()?;
        match crate::_t3_readiness::resolve_launch(&home, &self.env, request, existing) {
            Err(RuntimeError::ExecutionRootUnavailable {
                cause: ExecutionRootCause::Missing,
                ..
            }) if existing.is_none() => {
                let project = self
                    .execution_root(&request.checkout, ExecutionRootMode::Ensure)?
                    .project_id
                    .ok_or_else(|| RuntimeError::ExecutionRootUnavailable {
                        checkout: request.checkout.clone(),
                        cause: ExecutionRootCause::AdditionRefused,
                    })?;
                let mut registered = request.clone();
                if let Some(launch) = registered.native_launch.as_mut() {
                    launch.project_id = project;
                }
                crate::_t3_readiness::resolve_launch(&home, &self.env, &registered, existing)
            }
            result => result,
        }
    }

    fn selection_realization(&self, operation: SelectionOperation) -> String {
        match operation {
            SelectionOperation::ThreadStart => "T3 native launch receives the selected provider/model with its translated native effort/options and captured native modes.".into(),
            SelectionOperation::NativeDelegation => "T3 native delegation receives provider instance/model and translated harness options where the native operation supports them.".into(),
        }
    }

    fn native_selection(&self, selection: &AgentSelection) -> AgentSelection {
        crate::_t3_options::native(selection)
    }

    fn create_thread(&self, request: &RuntimeStart) -> Result<SessionRef, RuntimeError> {
        self.create_thread_inner(request)
    }

    fn verify_start(&self, request: &RuntimeStart, thread: &Thread) -> Result<(), RuntimeError> {
        verify_request(request, thread)?;
        self.verify_thread_project(&request.checkout, thread, Some(&request.key))
    }

    fn verify_placement(
        &self,
        request: &RuntimeStart,
        thread: &Thread,
    ) -> Result<(), RuntimeError> {
        self.verify_thread_project(&request.checkout, thread, Some(&request.key))?;
        verify_placement(request, thread)
    }

    fn verify_thread_project(
        &self,
        checkout: &Path,
        thread: &Thread,
        key: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let Some(observed) = thread.project_id.as_deref() else {
            return Err(RuntimeError::ProjectUnverified {
                key: key.unwrap_or_default().into(),
                session: thread.session.clone(),
            });
        };
        let exact =
            crate::_t3_readiness::exact_project_id(&self.operation_home()?, &self.env, checkout)?;
        if exact.as_deref() == Some(observed) {
            return Ok(());
        }
        Err(RuntimeError::ThreadMisplaced {
            key: key.map(str::to_owned),
            session: thread.session.clone(),
            checkout: checkout.to_path_buf(),
            idle: matches!(
                thread.state,
                SessionState::InputReady | SessionState::EndedFailed
            ),
        })
    }

    fn execution_root(
        &self,
        root: &Path,
        mode: ExecutionRootMode,
    ) -> Result<ExecutionRootStatus, RuntimeError> {
        let home = self.operation_home()?;
        if let Some(id) = crate::_t3_readiness::exact_project_id(&home, &self.env, root)? {
            return Ok(ExecutionRootStatus {
                project_id: Some(id),
                created: false,
            });
        }
        if mode == ExecutionRootMode::Check {
            return Ok(ExecutionRootStatus {
                project_id: None,
                created: false,
            });
        }
        let instrukt_home =
            Home::from_env().map_err(|error| failure("project add", error.to_string()))?;
        let title = project_title(root, &instrukt_home);
        let root_text = root.to_string_lossy();
        let mut args = vec!["project", "add"];
        if let Some(title) = title.as_deref() {
            args.extend(["--title", title]);
        }
        args.extend(["--", root_text.as_ref()]);
        let added = self.call(&args, None).is_ok();
        match crate::_t3_readiness::exact_project_id(&home, &self.env, root)? {
            Some(id) => Ok(ExecutionRootStatus {
                project_id: Some(id),
                created: added,
            }),
            None => Err(RuntimeError::ExecutionRootUnavailable {
                checkout: root.to_path_buf(),
                cause: ExecutionRootCause::AdditionRefused,
            }),
        }
    }

    fn delivery_realization(&self) -> DeliveryRealization {
        DeliveryRealization::Queued
    }

    fn supports(&self, capability: Capability) -> bool {
        match capability {
            Capability::Archive => crate::T3_DECLARATION.archives_threads,
            Capability::ColdTextSource => crate::T3_DECLARATION.cold_text_source,
            Capability::AttachWorktree => crate::T3_DECLARATION.worktree_attach,
            Capability::CreateWorktree
            | Capability::AgentSelection
            | Capability::InputConsumption
            | Capability::AssignmentEvidence
            | Capability::ConfiguredRuntimes => true,
            Capability::Link | Capability::Notify => false,
        }
    }

    fn observe_send(
        &self,
        session: &SessionRef,
        text: &str,
        command_id: Option<&str>,
    ) -> Result<SendEvidence, RuntimeError> {
        match self.fetch_projection(session, self.default_read_timeout()) {
            Ok(projection) => {
                _projection::decode_send_evidence(projection, session, text, command_id)
            }
            Err(RuntimeError::ThreadNotFound { .. }) => {
                Ok(SendEvidence::Presence(Presence::Absent))
            }
            Err(error) => Err(error),
        }
    }

    fn observe_input(
        &self,
        session: &SessionRef,
        text: &str,
        now: jiff::Timestamp,
    ) -> Result<InputObservation, RuntimeError> {
        let projection = self.fetch_projection(session, self.default_read_timeout())?;
        _projection::decode_input_observation(projection, session, text, now)
    }

    fn assignment_evidence(
        &self,
        session: &SessionRef,
        now: jiff::Timestamp,
    ) -> Result<AssignmentEvidence, RuntimeError> {
        let projection = self.fetch_projection(session, self.default_read_timeout())?;
        _projection::decode_assignment_evidence(projection, session, now)
    }

    fn list(&self, checkout: &Path) -> Result<Vec<Thread>, RuntimeError> {
        let out = self.call(
            &[
                "thread",
                "list",
                "--project",
                &checkout.to_string_lossy(),
                "--json",
            ],
            None,
        )?;
        let threads: Vec<NativeThread> =
            serde_json::from_str(&out).map_err(|e| failure("list", e.to_string()))?;
        threads.into_iter().map(NativeThread::decode).collect()
    }

    fn dispatch_send(
        &self,
        session: &SessionRef,
        command_id: &str,
        text: &str,
        before_write: &mut (dyn FnMut() -> Result<(), RuntimeError> + Send),
    ) -> Result<(), DispatchError> {
        if session.runtime.as_str() != "t3" {
            return Err(BeforeFrame(RuntimeError::UnknownRuntime(
                session.runtime.to_string(),
            )));
        }
        let home = self.operation_home().map_err(BeforeFrame)?;
        crate::_t3_readiness::dispatch(
            &home,
            &self.env,
            "instrukt-conversation-send",
            "send",
            serde_json::json!({
                "type": "message.dispatch",
                "commandId": command_id,
                "threadId": session.id,
                "messageId": command_id,
                "text": text,
                "createdBy": "agent",
                "creationSource": "mcp",
                "attachments": [],
                "dispatchMode": {"type": "queue_after_active"}
            }),
            before_write,
        )
    }

    fn send(&self, session: &SessionRef, text: &str) -> Result<(), RuntimeError> {
        if session.runtime.as_str() != "t3" {
            return Err(RuntimeError::UnknownRuntime(session.runtime.to_string()));
        }
        self.call(
            &["thread", "send", "--", session.id.as_str(), "-"],
            Some(text),
        )?;
        Ok(())
    }

    fn read_state(&self, session: &SessionRef) -> Result<Thread, RuntimeError> {
        self.read_state_within(session, self.default_read_timeout())
    }

    fn read_state_within(
        &self,
        session: &SessionRef,
        timeout: Duration,
    ) -> Result<Thread, RuntimeError> {
        match self.fetch_projection(session, timeout) {
            Ok(projection) => _projection::decode_thread(projection, session),
            Err(RuntimeError::ThreadNotFound { .. }) => Ok(Thread {
                session: session.clone(),
                project_id: None,
                title: None,
                semantic_key: None,
                state: SessionState::Gone,
                presence: Presence::Absent,
                branch: None,
                worktree: None,
                provider: None,
                model: None,
                interrupted: false,
                settled: false,
                settled_at: None,
                native_status: "gone".into(),
                run_id: None,
                active_turn: None,
                recent_turns: Vec::new(),
                activity: None,
                native_details: serde_json::Value::Null,
            }),
            Err(error) => Err(error),
        }
    }

    fn read_conversation(&self, session: &SessionRef) -> Result<Conversation, RuntimeError> {
        let shown = self.show(session, 100)?;
        shown.thread.decode()?;
        Ok(Conversation {
            session: session.clone(),
            messages: shown
                .messages
                .into_iter()
                .map(|m| Message {
                    role: m.role,
                    text: m.text,
                    run_id: m.run_id,
                })
                .collect(),
            limit: 100,
            bounded: true,
        })
    }

    fn archive_capability(&self) -> Result<&dyn Archive, RuntimeError> {
        Ok(self)
    }

    fn shutdown(&self) {
        if let Some(lease) = &self.read_lease {
            lease.shutdown();
        }
    }
}

pub(crate) fn verify_request(request: &RuntimeStart, thread: &Thread) -> Result<(), RuntimeError> {
    if thread.presence != Presence::Present {
        return Err(failure("start recovery", "thread is not present for work"));
    }
    verify_placement(request, thread)
}

fn placement_preparing(request: &RuntimeStart, thread: &Thread) -> bool {
    let new_worktree = request
        .native_launch
        .as_ref()
        .is_none_or(|launch| launch.existing_worktree.is_none());
    new_worktree
        && request
            .worktree
            .as_ref()
            .is_some_and(|worktree| thread.branch.as_deref() == Some(&worktree.branch))
        && thread.presence == Presence::Present
        && ThreadStatus::parse(&thread.native_status) == Some(ThreadStatus::Preparing)
        && thread.worktree.is_none()
}

fn verify_placement(request: &RuntimeStart, thread: &Thread) -> Result<(), RuntimeError> {
    if let Some(path) = request
        .native_launch
        .as_ref()
        .and_then(|launch| launch.existing_worktree.as_ref())
    {
        if !path.is_absolute() || thread.worktree.as_ref() != Some(path) {
            return Err(failure(
                "placement",
                "native successor differs from the captured existing worktree",
            ));
        }
        if request.worktree.is_none() {
            let listing =
                instrukt_core::git::run(&request.checkout, ["worktree", "list", "--porcelain"])
                    .map_err(|e| failure("placement", e.to_string()))?;
            let branch = instrukt_core::git::run(path, ["branch", "--show-current"])
                .map_err(|e| failure("placement", e.to_string()))?;
            if !listing
                .lines()
                .any(|line| line == format!("worktree {}", path.display()))
                || thread.branch.as_deref() != Some(branch.trim())
            {
                return Err(failure(
                    "placement",
                    "native successor lacks its registered phase worktree/branch",
                ));
            }
        }
    }
    if let Some(worktree) = &request.worktree {
        if placement_preparing(request, thread) {
            return Err(RuntimeError::PlacementPreparing(thread.session.clone()));
        }
        if thread.branch.as_deref() != Some(&worktree.branch)
            || thread.worktree.as_ref().is_none_or(|p| !p.is_absolute())
        {
            return Err(failure(
                "start recovery",
                "requested role branch or absolute worktree path is missing",
            ));
        }
        let expected_path = thread
            .worktree
            .as_ref()
            .ok_or_else(|| failure("start recovery", "missing worktree path"))?;
        let registered = instrukt_core::git::registered_worktree(
            &request.checkout,
            expected_path,
            &worktree.branch,
        )
        .map_err(|e| failure("start recovery", e.to_string()))?;
        if !registered {
            return Err(failure(
                "start recovery",
                "returned path is not the requested branch's registered worktree",
            ));
        }
        let head = instrukt_core::git::run(&request.checkout, ["rev-parse", &worktree.branch])
            .map_err(|e| failure("start recovery", e.to_string()))?;
        instrukt_core::git::run(
            &request.checkout,
            ["merge-base", "--is-ancestor", &worktree.base, head.trim()],
        )
        .map_err(|e| {
            failure(
                "start recovery",
                format!("role branch is not rooted at captured base: {e}"),
            )
        })?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeThread {
    thread_id: String,
    project_id: Option<String>,
    title: String,
    status: String,
    branch: Option<String>,
    worktree_path: Option<std::path::PathBuf>,
    pending_request_count: Option<u64>,
    archived: Option<bool>,
    provider_instance_id: Option<String>,
    model: Option<String>,
    #[serde(default)]
    settled: bool,
    settled_at: Option<String>,
    latest_run_id: Option<String>,
    active_run_id: Option<String>,
    updated_at: Option<String>,
    #[serde(flatten)]
    details: serde_json::Map<String, serde_json::Value>,
}

impl NativeThread {
    fn decode(self) -> Result<Thread, RuntimeError> {
        if self.thread_id.is_empty() {
            return Err(failure("decode", "incomplete thread identity"));
        }
        let native_status = ThreadStatus::parse(&self.status).ok_or_else(|| {
            RuntimeError::Observation(format!("unrecognized T3 thread status `{}`", self.status))
        })?;
        let presence = if self.archived == Some(true) {
            Presence::Archived
        } else {
            Presence::Present
        };
        let state = if presence == Presence::Archived {
            SessionState::Gone
        } else if self.pending_request_count.unwrap_or(0) > 0 {
            SessionState::NeedsInput
        } else {
            native_status.state()
        };
        let activity = [self.updated_at.as_deref(), self.settled_at.as_deref()]
            .into_iter()
            .flatten()
            .map(|text| {
                text.parse::<jiff::Timestamp>()
                    .map_err(|e| failure("activity observation", e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .max()
            .map(|at| NativeActivity {
                at,
                source: "t3 updatedAt/settledAt".into(),
                meaning: "native latest activity or settling".into(),
            });
        Ok(Thread {
            session: SessionRef::parse(&format!("t3:{}", self.thread_id))
                .map_err(|e| failure("decode", e.to_string()))?,
            project_id: self.project_id,
            semantic_key: None,
            title: Some(self.title),
            state,
            presence,
            branch: self.branch,
            worktree: self.worktree_path.filter(|p| !p.as_os_str().is_empty()),
            provider: self.provider_instance_id,
            model: self.model,
            interrupted: native_status == ThreadStatus::Interrupted,
            settled: self.settled,
            settled_at: self.settled_at,
            native_status: self.status,
            run_id: self.active_run_id.or(self.latest_run_id),
            active_turn: None,
            recent_turns: Vec::new(),
            activity,
            native_details: serde_json::Value::Object(self.details),
        })
    }
}

struct ListedFinished {
    id: String,
    settled_at: jiff::Timestamp,
    updated_at: Option<jiff::Timestamp>,
    quiet: bool,
}

impl ListedFinished {
    fn read(item: &serde_json::Value) -> Option<Self> {
        let id = item["threadId"].as_str().filter(|id| !id.is_empty())?;
        if item["settled"].as_bool() != Some(true) {
            return None;
        }
        let instant = |value: &serde_json::Value| {
            value
                .as_str()
                .and_then(|text| text.parse::<jiff::Timestamp>().ok())
        };
        let settled_at = instant(&item["settledAt"])?;
        let quiet = item["status"]
            .as_str()
            .and_then(ThreadStatus::parse)
            .is_some_and(|status| {
                matches!(
                    status.state(),
                    SessionState::InputReady | SessionState::EndedFailed
                )
            })
            && item["snoozed"].as_bool() == Some(false)
            && item
                .get("pendingBackgroundTasks")
                .is_none_or(|tasks| tasks.as_array().is_some_and(Vec::is_empty));
        Some(Self {
            id: id.to_owned(),
            settled_at,
            updated_at: instant(&item["updatedAt"]),
            quiet,
        })
    }

    fn observed(
        self,
        row: Option<&crate::_t3_conversations::RetentionRow>,
    ) -> Result<FinishedThread, RuntimeError> {
        let opened_at = row.map_or(Err(()), |row| row.opened_at);
        let latest_activity = match (self.updated_at, opened_at) {
            (Some(updated_at), Ok(opened_at)) => Some(
                updated_at
                    .max(self.settled_at)
                    .max(opened_at.unwrap_or(self.settled_at)),
            ),
            _ => None,
        };
        Ok(FinishedThread {
            session: SessionRef::parse(&format!("t3:{}", self.id))
                .map_err(|e| failure("decode", e.to_string()))?,
            finished_at: Some(self.settled_at),
            latest_activity,
            quiet: self.quiet,
            pinned: row.is_none_or(|row| row.pinned),
        })
    }
}

#[derive(Deserialize)]
struct Shown {
    thread: NativeThread,
    messages: Vec<NativeMessage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeMessage {
    role: String,
    text: String,
    run_id: Option<String>,
}

fn project_title(root: &Path, home: &Home) -> Option<String> {
    if root.parent() != Some(home.companies_dir().as_path()) {
        return None;
    }
    let name = instrukt_core::ids::ProjectSlug::parse(root.file_name()?.to_str()?).ok()?;
    instrukt_core::config::company_repository(&name)
        .ok()
        .map(|repository| repository.to_string())
}

fn failure(operation: &str, detail: impl Into<String>) -> RuntimeError {
    RuntimeError::Failed {
        runtime: "t3".into(),
        operation: operation.into(),
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use instrukt_runtime::{NativeLaunch, Profiles, WorktreeRequest};

    fn new_worktree_request() -> RuntimeStart {
        RuntimeStart {
            launch_modes: None,
            native_launch: Some(NativeLaunch {
                command_id: "instrukt:launch:1".into(),
                thread_id: "instrukt:launch:1".into(),
                project_id: "project".into(),
                existing_worktree: None,
            }),
            preferences: Vec::new(),
            selection: None,
            selection_evidence: None,
            checkout: std::path::PathBuf::from("/checkout"),
            key: "instrukt:todo:demo/parser:todo-orchestrator:1".into(),
            title: "parser".into(),
            brief: "Assignment.".into(),
            worktree: Some(WorktreeRequest {
                from: "origin/main".into(),
                base: "base".into(),
                branch: "parser/work".into(),
            }),
            profiles: Profiles::default(),
        }
    }

    fn shown_without_worktree(status: &str) -> Thread {
        let native: NativeThread = serde_json::from_value(serde_json::json!({
            "threadId": "instrukt:launch:1", "projectId": "project", "title": "parser",
            "status": status, "branch": "parser/work", "worktreePath": null,
            "pendingRequestCount": 0, "archived": false,
            "latestRunId": "run:1", "activeRunId": if status == "failed" { None } else { Some("run:1") },
        }))
        .unwrap();
        native.decode().unwrap()
    }

    #[test]
    fn a_new_worktree_t3_is_still_preparing_is_pending_placement() {
        let thread = shown_without_worktree("preparing");
        let outcome = verify_request(&new_worktree_request(), &thread);
        assert!(
            matches!(outcome, Err(RuntimeError::PlacementPreparing(ref session)) if *session == thread.session)
        );
    }

    #[test]
    fn a_company_home_is_titled_with_its_repository_name() {
        let home = Home::at_user_home("/machine".into());
        let company = instrukt_core::ids::ProjectSlug::parse("acme").unwrap();
        assert_eq!(
            project_title(&home.company_dir(&company), &home),
            Some(
                instrukt_core::config::company_repository(&company)
                    .unwrap()
                    .to_string()
            )
        );
    }

    #[test]
    fn a_checkout_inside_a_company_home_keeps_the_runtime_title() {
        let home = Home::at_user_home("/machine".into());
        let company = instrukt_core::ids::ProjectSlug::parse("acme").unwrap();
        let checkout = home.company_dir(&company).join("projects").join("site");
        assert_eq!(project_title(&checkout, &home), None);
    }

    #[test]
    fn a_failed_preparation_on_the_requested_branch_is_a_failed_placement() {
        let outcome = verify_request(&new_worktree_request(), &shown_without_worktree("failed"));
        assert!(matches!(outcome, Err(RuntimeError::Failed { .. })));
    }
}
