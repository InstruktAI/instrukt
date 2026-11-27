//! T3 native keyed launch and native-ID CLI observations.
use std::path::Path;
use std::time::Duration;

use instrukt_core::config::Runtime;
use instrukt_core::ids::SessionRef;
use serde::Deserialize;

use super::{Capability, Conversation, Message, RuntimeAdapter, RuntimeStart, Thread};
use crate::process::Invocation;
use crate::sessions::{RUNTIME_COMMAND_TIMEOUT, SessionState, SessionsError};

pub struct T3 {
    timeout: Duration,
    env: instrukt_core::config::Env,
}
impl Default for T3 {
    fn default() -> Self {
        Self {
            timeout: RUNTIME_COMMAND_TIMEOUT,
            env: instrukt_core::config::Env::from_process(),
        }
    }
}
impl T3 {
    fn input_evidence(
        &self,
        session: &SessionRef,
        text: Option<&str>,
    ) -> Result<Vec<super::_t3_conversations::NativeInput>, SessionsError> {
        let user_home = self
            .env
            .get("HOME")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| failure("input observation", "machine home is unavailable"))?;
        let home = instrukt_core::home::Home::at_user_home(user_home.into());
        super::_t3_conversations::inputs(&home, &self.env, session.id.as_str(), text)
            .map_err(|error| failure("input observation", error.to_string()))
    }

    pub(crate) fn create_thread(
        &self,
        request: &RuntimeStart,
    ) -> Result<SessionRef, SessionsError> {
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
                serde_json::json!({"type":"existing_worktree","worktreePath":path})
            }
            (None, None) => serde_json::json!({"type":"root"}),
        };
        let value = super::_t3_readiness::launch(
            &self.operation_home()?,
            &self.env,
            serde_json::json!({
                "commandId":launch.command_id, "threadId":launch.thread_id, "projectId":launch.project_id,
                "title":request.title, "generateTitle":false, "creationSource":"mcp",
                "modelSelection":super::_t3_options::model_selection(selection),
                "runtimeMode":modes.runtime, "interactionMode":modes.interaction, "workspaceStrategy":workspace,
                "initialMessage":{"messageId":launch.command_id,"text":request.brief,"attachments":[]}
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

    fn operation_home(&self) -> Result<instrukt_core::home::Home, SessionsError> {
        let user_home = self
            .env
            .get("HOME")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| failure("launch", "machine home unavailable"))?;
        Ok(instrukt_core::home::Home::at_user_home(user_home.into()))
    }

    fn call(&self, args: &[&str], input: Option<&str>) -> Result<String, SessionsError> {
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
    fn show(&self, session: &SessionRef, limit: usize) -> Result<Shown, SessionsError> {
        if session.runtime.as_str() != "t3" {
            return Err(SessionsError::UnknownRuntime(session.runtime.to_string()));
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
            Err(SessionsError::Failed { detail, .. })
                if detail.trim() == format!("Thread {} is no longer available.", session.id) =>
            {
                return Err(SessionsError::Gone(session.clone()));
            }
            result => result?,
        };
        let shown: Shown =
            serde_json::from_str(&out).map_err(|e| SessionsError::InvalidResponse {
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
}
impl instrukt_core::interfaces::CallerIdentityLookup for T3 {
    fn lookup_caller(
        &self,
        env: &instrukt_core::config::Env,
    ) -> Option<instrukt_core::interfaces::CallerIdentity> {
        let id = env.get("T3CODE_THREAD_ID")?;
        Some(instrukt_core::interfaces::CallerIdentity {
            session: SessionRef::parse(&format!("t3:{id}")).ok()?,
            agent: None,
        })
    }
    fn caller_variables(&self) -> &'static [&'static str] {
        &["T3CODE_THREAD_ID"]
    }
}

impl RuntimeAdapter for T3 {
    fn diagnostic(&self, operation: &str, error: &SessionsError) -> super::NativeDiagnostic {
        let native = match error {
            SessionsError::Failed { detail, .. } => super::_t3_diagnostics::cause(detail),
            _ => None,
        };
        super::_diagnostics::diagnose("t3", operation, error, native)
    }
    fn send_certainly_rejected(&self, session: &SessionRef, error: &SessionsError) -> bool {
        match error {
            SessionsError::Process(instrukt_core::process::ProcessError::Spawn { .. })
            | SessionsError::ExecutionRootUnavailable { .. }
            | SessionsError::ThreadMisplaced { .. } => true,
            SessionsError::Failed {
                operation, detail, ..
            } if operation == "send" => {
                let line = detail.lines().next().unwrap_or_default().trim();
                line == format!("Thread {} is no longer available.", session.id)
                    || matches!(
                        super::_t3_diagnostics::cause(line),
                        Some(super::NativeCause::ServerNotRunning)
                    )
                    || line
                        .strip_prefix("Could not reach the T3 Code server at ")
                        .and_then(|rest| rest.split_once(". "))
                        .is_some_and(|(_, reason)| reason.starts_with("connect ECONNREFUSED "))
            }
            _ => false,
        }
    }
    fn conversation_source(
        &self,
        home: &instrukt_core::home::Home,
        env: &instrukt_core::config::Env,
    ) -> Result<Box<dyn super::conversations::ConversationSource>, SessionsError> {
        Ok(Box::new(super::_t3_conversations::T3Conversations::new(
            home, env,
        )))
    }
    fn provider_snapshot(
        &self,
        home: &instrukt_core::home::Home,
    ) -> Result<super::ProviderSnapshot, SessionsError> {
        super::_t3_readiness::snapshot(home, &self.env)
    }
    fn readiness(&self) -> Option<Box<dyn super::Readiness>> {
        Some(Box::new(super::_t3_readiness::Readiness::new(
            self.env.clone(),
        )))
    }
    fn last_snapshot(&self) -> Option<std::sync::Arc<super::ObservedSnapshot>> {
        super::_t3_readiness::last()
    }
    fn configured_agent_runtimes(
        &self,
        home: &instrukt_core::home::Home,
        env: &instrukt_core::config::Env,
    ) -> Result<Vec<instrukt_core::interfaces::ConfiguredInstance>, SessionsError> {
        super::_t3_readiness::configured(home, env)
    }
    fn launch_scope(&self, home: &instrukt_core::home::Home) -> String {
        super::_t3_home::T3Home::discover(home, &self.env)
            .database()
            .to_string_lossy()
            .into_owned()
    }
    fn accepted_start(&self, request: &RuntimeStart) -> Result<bool, SessionsError> {
        let launch = request
            .native_launch
            .as_ref()
            .ok_or_else(|| failure("launch", "missing native identity"))?;
        super::_t3_conversations::accepted_start(
            &self.operation_home()?,
            &self.env,
            &launch.thread_id,
            &launch.command_id,
        )
        .map_err(|e| failure("launch", e.to_string()))
    }
    fn resolve_launch(
        &self,
        request: &RuntimeStart,
        existing: Option<&SessionRef>,
    ) -> Result<RuntimeStart, SessionsError> {
        let home = self.operation_home()?;
        match super::_t3_readiness::resolve_launch(&home, &self.env, request, existing) {
            // A launch registers its exact checkout when T3 holds no project
            // there, then resolves again; adopting a present thread never
            // registers.
            Err(SessionsError::ExecutionRootUnavailable {
                cause: crate::sessions::ExecutionRootCause::Missing,
                ..
            }) if existing.is_none() => {
                let project = self
                    .execution_root(&request.checkout, super::ExecutionRootMode::Ensure)?
                    .project_id
                    .ok_or_else(|| SessionsError::ExecutionRootUnavailable {
                        checkout: request.checkout.clone(),
                        cause: crate::sessions::ExecutionRootCause::AdditionRefused,
                    })?;
                let mut registered = request.clone();
                if let Some(launch) = registered.native_launch.as_mut() {
                    launch.project_id = project;
                }
                super::_t3_readiness::resolve_launch(&home, &self.env, &registered, existing)
            }
            result => result,
        }
    }
    fn selection_realization(&self, operation: super::SelectionOperation) -> String {
        match operation {
            super::SelectionOperation::ThreadStart=>"T3 native launch receives the selected provider/model with its translated native effort/options and captured native modes.".into(),
            super::SelectionOperation::NativeDelegation=>"T3 native delegation receives provider instance/model and translated harness options where the native operation supports them.".into(),
        }
    }
    fn native_selection(
        &self,
        selection: &instrukt_core::config::AgentSelection,
    ) -> instrukt_core::config::AgentSelection {
        super::_t3_options::native(selection)
    }
    fn create_thread(&self, request: &RuntimeStart) -> Result<SessionRef, SessionsError> {
        T3::create_thread(self, request)
    }
    fn verify_start(&self, request: &RuntimeStart, thread: &Thread) -> Result<(), SessionsError> {
        // Presence and placement first: a thread still being placed keeps
        // its pending outcome, and the snapshot is read only once placement
        // verifies.
        verify_request(request, thread)?;
        self.verify_thread_project(&request.checkout, thread, Some(&request.key))
    }
    fn verify_placement(
        &self,
        request: &RuntimeStart,
        thread: &Thread,
    ) -> Result<(), SessionsError> {
        self.verify_thread_project(&request.checkout, thread, Some(&request.key))?;
        verify_placement(request, thread)
    }
    fn verify_thread_project(
        &self,
        checkout: &Path,
        thread: &Thread,
        key: Option<&str>,
    ) -> Result<(), SessionsError> {
        let Some(observed) = thread.project_id.as_deref() else {
            return Err(SessionsError::ProjectUnverified {
                key: key.unwrap_or_default().into(),
                session: thread.session.clone(),
            });
        };
        let exact =
            super::_t3_readiness::exact_project_id(&self.operation_home()?, &self.env, checkout)?;
        if exact.as_deref() == Some(observed) {
            return Ok(());
        }
        Err(SessionsError::ThreadMisplaced {
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
        mode: super::ExecutionRootMode,
    ) -> Result<super::ExecutionRootStatus, SessionsError> {
        let home = self.operation_home()?;
        if let Some(id) = super::_t3_readiness::exact_project_id(&home, &self.env, root)? {
            return Ok(super::ExecutionRootStatus {
                project_id: Some(id),
                created: false,
            });
        }
        if mode == super::ExecutionRootMode::Check {
            return Ok(super::ExecutionRootStatus {
                project_id: None,
                created: false,
            });
        }
        // A company home is created titled with its repository's name;
        // every other root keeps T3's own title, its folder name. A refusal
        // of a root that already has an active project (a concurrent
        // creator) is a reuse the snapshot read below confirms; T3's own
        // words are never kept.
        let instrukt_home = instrukt_core::home::Home::from_env()
            .map_err(|error| failure("project add", error.to_string()))?;
        let title = project_title(root, &instrukt_home);
        let root_text = root.to_string_lossy();
        let mut args = vec!["project", "add"];
        if let Some(title) = title.as_deref() {
            args.extend(["--title", title]);
        }
        args.extend(["--", root_text.as_ref()]);
        let added = self.call(&args, None).is_ok();
        match super::_t3_readiness::exact_project_id(&home, &self.env, root)? {
            Some(id) => Ok(super::ExecutionRootStatus {
                project_id: Some(id),
                created: added,
            }),
            None => Err(SessionsError::ExecutionRootUnavailable {
                checkout: root.to_path_buf(),
                cause: crate::sessions::ExecutionRootCause::AdditionRefused,
            }),
        }
    }
    fn start_or_wake(
        &self,
        assignment: &super::RuntimeAssignment<'_>,
    ) -> Result<Thread, SessionsError> {
        crate::keyed::start(self, assignment)
    }
    fn delivery_realization(&self) -> super::DeliveryRealization {
        super::DeliveryRealization::Queued
    }
    fn runtime(&self) -> Runtime {
        Runtime::T3
    }
    fn supports(&self, capability: Capability) -> bool {
        match capability {
            // The runtime's one archive fact, which configuration admits the
            // retention window against.
            Capability::Archive => self.runtime().archives_threads(),
            Capability::ConversationSource
            | Capability::CreateWorktree
            | Capability::AttachWorktree
            | Capability::AgentSelection
            | Capability::InputConsumption
            | Capability::AssignmentEvidence
            | Capability::ConfiguredRuntimes => true,
            Capability::Link | Capability::Notify => false,
        }
    }
    fn finished_threads(
        &self,
        checkout: &Path,
    ) -> Result<Vec<super::FinishedThread>, SessionsError> {
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
            super::_t3_conversations::retention_rows(&self.operation_home()?, &self.env, &ids)
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
        observed: &super::FinishedThread,
        now: jiff::Timestamp,
    ) -> Result<super::ArchiveOutcome, SessionsError> {
        use super::ArchiveOutcome;
        let session = &observed.session;
        if session.runtime.as_str() != "t3" {
            return Err(SessionsError::UnknownRuntime(session.runtime.to_string()));
        }
        let Some(latest_activity) = observed.latest_activity else {
            return Ok(ArchiveOutcome::Changed);
        };
        let home = self.operation_home()?;
        let thread = session.id.as_str();
        // A re-read that cannot be made archives nothing and is a failure of
        // its own; only an observed change is `Changed`.
        let holds =
            super::_t3_conversations::archive_holds(&home, &self.env, thread, latest_activity, now)
                .map_err(|error| failure("archive", error.to_string()))?;
        if !holds {
            return Ok(ArchiveOutcome::Changed);
        }
        let command_id = format!(
            "instrukt:retention:{}",
            instrukt_core::machine::random_hex(16)
                .map_err(|error| failure("archive", error.to_string()))?
        );
        let dispatched = super::_t3_readiness::dispatch(
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
            // T3 refuses to archive an archived thread; its row shows which.
            Err(error) => match super::_t3_conversations::archived(&home, &self.env, thread) {
                Ok(true) => Ok(ArchiveOutcome::AlreadyArchived),
                _ => Err(error.into_inner()),
            },
        }
    }
    fn observe_send(
        &self,
        session: &SessionRef,
        text: &str,
        command_id: Option<&str>,
    ) -> Result<super::SendEvidence, SessionsError> {
        use super::SendEvidence;
        let shown = match self.show(session, 0) {
            Ok(shown) => shown,
            Err(SessionsError::Gone(_)) => {
                return Ok(SendEvidence::Presence(super::Presence::Absent));
            }
            Err(error) => return Err(error),
        };
        let idle =
            shown.thread.active_run_id.is_none() && shown.thread.pending_request_count == Some(0);
        let thread = shown.thread.decode()?;
        if thread.presence != super::Presence::Present {
            return Ok(SendEvidence::Presence(thread.presence));
        }
        let user_home = self
            .env
            .get("HOME")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| failure("input observation", "machine home is unavailable"))?;
        let home = instrukt_core::home::Home::at_user_home(user_home.into());
        let (accepted, empty, rejected) = super::_t3_conversations::send_evidence(
            &home,
            &self.env,
            session.id.as_str(),
            text,
            command_id,
        )
        .map_err(|error| failure("input observation", error.to_string()))?;
        Ok(if accepted {
            SendEvidence::Accepted
        } else if rejected {
            SendEvidence::Rejected
        } else if idle
            && empty
            && matches!(
                thread.state,
                SessionState::InputReady | SessionState::EndedFailed
            )
        {
            SendEvidence::IdleWithoutInput
        } else {
            SendEvidence::Unresolved
        })
    }

    fn observe_input(
        &self,
        session: &SessionRef,
        text: &str,
        now: jiff::Timestamp,
    ) -> Result<super::InputObservation, SessionsError> {
        self.show(session, 0)?;
        let inputs = self.input_evidence(session, Some(text))?;
        let message = inputs.into_iter().find(|input| input.text == text);
        Ok(super::InputObservation {
            state: message
                .as_ref()
                .map_or(super::InputState::Unknown, |m| m.state),
            reference: message.and_then(|m| m.reference),
            source: "T3 persisted user input and native run entry".into(),
            observed_at: now,
        })
    }
    fn active_turn_input(&self, session: &SessionRef) -> Result<Option<String>, SessionsError> {
        let user_home = self
            .env
            .get("HOME")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| failure("turn observation", "machine home is unavailable"))?;
        let home = instrukt_core::home::Home::at_user_home(user_home.into());
        super::_t3_conversations::active_opening(&home, &self.env, session.id.as_str())
            .map_err(|error| failure("turn observation", error.to_string()))
    }
    fn assignment_evidence(
        &self,
        session: &SessionRef,
        now: jiff::Timestamp,
    ) -> Result<super::AssignmentEvidence, SessionsError> {
        let conversation = self.read_conversation(session)?;
        let opening = conversation
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .find(|m| {
                m.text
                    .lines()
                    .any(|line| line.starts_with("Assignment facts: "))
            })
            .ok_or_else(|| {
                failure(
                    "assignment recovery",
                    "bounded recent conversation has no initialization evidence",
                )
            })?
            .text
            .clone();
        let waiting = self
            .input_evidence(session, None)?
            .into_iter()
            .find(|m| {
                m.state != super::InputState::Consumed
                    && (m.text.starts_with("instrukt delivery log\n")
                        || m.text
                            .starts_with("instrukt system-origin procedure assignment\n"))
            })
            .map(|m| m.text);
        Ok(super::AssignmentEvidence {
            opening,
            waiting,
            source: "t3 thread show: bounded attributed initialization input".into(),
            observed_at: now,
        })
    }
    fn list(&self, checkout: &Path) -> Result<Vec<Thread>, SessionsError> {
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
    fn start(&self, assignment: &super::RuntimeAssignment<'_>) -> Result<Thread, SessionsError> {
        crate::keyed::start(self, assignment)
    }
    fn dispatch_send(
        &self,
        session: &SessionRef,
        command_id: &str,
        text: &str,
        before_write: &mut dyn FnMut() -> Result<(), SessionsError>,
    ) -> Result<(), super::DispatchError> {
        use super::DispatchError::BeforeFrame;
        if session.runtime.as_str() != "t3" {
            return Err(BeforeFrame(SessionsError::UnknownRuntime(
                session.runtime.to_string(),
            )));
        }
        let user_home = self
            .env
            .get("HOME")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| BeforeFrame(failure("send", "machine home is unavailable")))?;
        let home = instrukt_core::home::Home::at_user_home(user_home.into());
        super::_t3_readiness::dispatch(
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
    fn send(&self, session: &SessionRef, text: &str) -> Result<(), SessionsError> {
        if session.runtime.as_str() != "t3" {
            return Err(SessionsError::UnknownRuntime(session.runtime.to_string()));
        }
        self.call(
            &["thread", "send", "--", session.id.as_str(), "-"],
            Some(text),
        )?;
        Ok(())
    }
    fn read_state(&self, session: &SessionRef) -> Result<Thread, SessionsError> {
        match self.show(session, 0) {
            Ok(shown) => {
                let thread = shown.thread.decode()?;
                if &thread.session != session {
                    return Err(failure(
                        "show",
                        "thread response does not match the requested identity",
                    ));
                }
                Ok(thread)
            }
            Err(SessionsError::Gone(_)) => Ok(Thread {
                session: session.clone(),
                project_id: None,
                title: None,
                semantic_key: None,
                state: SessionState::Gone,
                presence: super::Presence::Absent,
                branch: None,
                worktree: None,
                provider: None,
                model: None,
                interrupted: false,
                settled: false,
                settled_at: None,
                native_status: "gone".into(),
                run_id: None,
                activity: None,
                native_details: serde_json::Value::Null,
            }),
            Err(error) => Err(error),
        }
    }
    fn read_conversation(&self, session: &SessionRef) -> Result<Conversation, SessionsError> {
        let shown = self.show(session, 100)?;
        // Validate the identity and status as well as the message shape.
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
}

pub(crate) fn verify_request(request: &RuntimeStart, thread: &Thread) -> Result<(), SessionsError> {
    if thread.presence != super::Presence::Present {
        return Err(failure("start recovery", "thread is not present for work"));
    }
    verify_placement(request, thread)
}

/// Whether T3 is still preparing the new worktree `request` asks for: the
/// present thread is on the requested branch, T3 reports it as preparing, and
/// it has no worktree path yet. T3 records the path after it answers the
/// launch (`project/instrukt/spec/feature/runner/t3-contract`).
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
        && thread.presence == super::Presence::Present
        && ThreadStatus::parse(&thread.native_status) == Some(ThreadStatus::Preparing)
        && thread.worktree.is_none()
}

fn verify_placement(request: &RuntimeStart, thread: &Thread) -> Result<(), SessionsError> {
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
            return Err(SessionsError::PlacementPreparing(thread.session.clone()));
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
        let registered =
            crate::keyed::registered_worktree(&request.checkout, expected_path, &worktree.branch)
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

/// A thread's status as T3 spells it in `t3 thread show` and `t3 thread list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ThreadStatus {
    Running,
    Preparing,
    Starting,
    Resuming,
    Queued,
    /// Not a wait for input; its translation is owned by
    /// `project/instrukt/spec/feature/runner/session-continuity`, T3
    /// observation translation.
    Waiting,
    WaitingForInput,
    WaitingForApproval,
    Idle,
    Completed,
    Stopped,
    Failed,
    Interrupted,
    Cancelled,
    RolledBack,
}
impl ThreadStatus {
    /// The member T3's spelling `native` names; a spelling outside the set
    /// names none and is observed as an unknown state.
    fn parse(native: &str) -> Option<Self> {
        use serde::de::IntoDeserializer as _;
        let spelling: serde::de::value::StrDeserializer<'_, serde::de::value::Error> =
            native.into_deserializer();
        Self::deserialize(spelling).ok()
    }
    fn state(self) -> SessionState {
        match self {
            Self::Running
            | Self::Preparing
            | Self::Starting
            | Self::Resuming
            | Self::Queued
            | Self::Waiting => SessionState::Running,
            Self::WaitingForInput | Self::WaitingForApproval => SessionState::NeedsInput,
            Self::Idle | Self::Completed | Self::Stopped => SessionState::InputReady,
            Self::Failed | Self::Interrupted | Self::Cancelled | Self::RolledBack => {
                SessionState::EndedFailed
            }
        }
    }
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
    fn decode(self) -> Result<Thread, SessionsError> {
        if self.thread_id.is_empty() {
            return Err(failure("decode", "incomplete thread identity"));
        }
        let native_status = ThreadStatus::parse(&self.status);
        let status = native_status.map_or(SessionState::Unknown, ThreadStatus::state);
        let presence = if self.archived == Some(true) {
            super::Presence::Archived
        } else {
            super::Presence::Present
        };
        let state = if presence == super::Presence::Archived {
            SessionState::Gone
        } else if self.pending_request_count.unwrap_or(0) > 0 {
            SessionState::NeedsInput
        } else {
            status
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
            .map(|at| super::NativeActivity {
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
            interrupted: native_status == Some(ThreadStatus::Interrupted),
            settled: self.settled,
            settled_at: self.settled_at,
            native_status: self.status,
            run_id: self.active_run_id.or(self.latest_run_id),
            activity,
            native_details: serde_json::Value::Object(self.details),
        })
    }
}
/// A thread `t3 thread list --json` reports settled with a readable
/// settling time, read field by field so that one malformed field leaves
/// only that thread not quiet or its activity unknown.
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
            // Releases that predate the field omit it, and report none.
            && item.get("pendingBackgroundTasks").is_none_or(|tasks| {
                tasks.as_array().is_some_and(Vec::is_empty)
            });
        Some(Self {
            id: id.to_owned(),
            settled_at,
            updated_at: instant(&item["updatedAt"]),
            quiet,
        })
    }
    /// The observation, with the projection row `row` of the thread: a
    /// thread without one is pinned, with unknown latest activity.
    fn observed(
        self,
        row: Option<&super::_t3_conversations::RetentionRow>,
    ) -> Result<super::FinishedThread, SessionsError> {
        let opened_at = row.map_or(Err(()), |row| row.opened_at);
        let latest_activity = match (self.updated_at, opened_at) {
            (Some(updated_at), Ok(opened_at)) => Some(
                updated_at
                    .max(self.settled_at)
                    .max(opened_at.unwrap_or(self.settled_at)),
            ),
            _ => None,
        };
        Ok(super::FinishedThread {
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
/// The title T3 creates `root`'s project with, when it is not T3's own: a
/// company home, a folder directly in `home`'s companies folder named as a
/// company is (`company-homes`), is titled with the company's repository
/// name, `company-<name>`, so a company project is told apart from others.
fn project_title(root: &Path, home: &instrukt_core::home::Home) -> Option<String> {
    if root.parent() != Some(home.companies_dir().as_path()) {
        return None;
    }
    let name = instrukt_core::ids::ProjectSlug::parse(root.file_name()?.to_str()?).ok()?;
    instrukt_core::config::company_repository(&name)
        .ok()
        .map(|repository| repository.to_string())
}

fn failure(operation: &str, detail: impl Into<String>) -> SessionsError {
    SessionsError::Failed {
        runtime: "t3".into(),
        operation: operation.into(),
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A todo pickup asking T3 for a new worktree of `parser/work`.
    fn new_worktree_request() -> RuntimeStart {
        RuntimeStart {
            launch_modes: None,
            native_launch: Some(crate::runtime::NativeLaunch {
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
            worktree: Some(crate::runtime::WorktreeRequest {
                from: "origin/main".into(),
                base: "base".into(),
                branch: "parser/work".into(),
            }),
            profiles: crate::runtime::Profiles::default(),
        }
    }

    /// The thread `t3 thread show --json` reports with T3 status `status`, on
    /// the requested branch and with no worktree path.
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
            matches!(outcome, Err(SessionsError::PlacementPreparing(ref session)) if *session == thread.session)
        );
    }

    #[test]
    fn a_company_home_is_titled_with_its_repository_name() {
        let home = instrukt_core::home::Home::at_user_home("/machine".into());
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
        let home = instrukt_core::home::Home::at_user_home("/machine".into());
        let company = instrukt_core::ids::ProjectSlug::parse("acme").unwrap();
        let checkout = home.company_dir(&company).join("projects").join("site");
        assert_eq!(project_title(&checkout, &home), None);
    }

    #[test]
    fn a_failed_preparation_on_the_requested_branch_is_a_failed_placement() {
        let outcome = verify_request(&new_worktree_request(), &shown_without_worktree("failed"));
        assert!(matches!(outcome, Err(SessionsError::Failed { .. })));
    }
}
