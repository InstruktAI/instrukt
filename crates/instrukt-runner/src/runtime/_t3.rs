//! T3 host CLI, verified against list and show responses on 2026-11-05.
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
    pub(crate) fn create_thread(&self, request: &RuntimeStart) -> Result<Thread, SessionsError> {
        let checkout = request.checkout.to_string_lossy();
        let mut args = vec!["thread", "start", "--title", request.title.as_str()];
        if let Some(worktree) = &request.worktree {
            args.extend([
                "--worktree",
                worktree.from.as_str(),
                "--branch",
                worktree.branch.as_str(),
            ]);
        }
        if let Some(selection) = &request.selection {
            args.extend([
                "--provider",
                selection.instance.as_str(),
                "--model",
                &selection.model,
            ]);
        }
        args.extend(["--", &checkout, "-"]);
        let out = self.call(&args, Some(&request.brief))?;
        let id = out.trim();
        if id.is_empty() || id.chars().any(char::is_whitespace) {
            return Err(failure("start", "no unambiguous native thread id returned"));
        }
        let session =
            SessionRef::parse(&format!("t3:{id}")).map_err(|e| failure("start", e.to_string()))?;
        let thread = self.read_state(&session)?;
        verify_request(request, &thread)?;
        Ok(thread)
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
    fn selection_realization(&self, operation: super::SelectionOperation) -> String {
        match operation {
            super::SelectionOperation::ThreadStart=>"T3 thread start accepts provider instance/model; requested effort/options are not expressible through this CLI operation.".into(),
            super::SelectionOperation::NativeDelegation=>"T3 native delegation receives provider instance/model and translated harness options where the native operation supports them.".into(),
        }
    }
    fn native_selection(
        &self,
        selection: &instrukt_core::config::AgentSelection,
    ) -> instrukt_core::config::AgentSelection {
        let mut native = selection.clone();
        if let Some(effort) = selection.effort {
            let key = match selection.kind {
                instrukt_core::config::HarnessKind::Codex
                | instrukt_core::config::HarnessKind::Grok
                | instrukt_core::config::HarnessKind::Kiro
                | instrukt_core::config::HarnessKind::Muse => "reasoningEffort",
                instrukt_core::config::HarnessKind::Antigravity => "thinking_level",
                instrukt_core::config::HarnessKind::Pi => "thinking",
                instrukt_core::config::HarnessKind::OpenCode => "variant",
                instrukt_core::config::HarnessKind::Claude
                | instrukt_core::config::HarnessKind::Cursor
                | instrukt_core::config::HarnessKind::Fx
                | instrukt_core::config::HarnessKind::AcpRegistry => "effort",
            };
            // Effort remains a closed semantic variant until this native boundary.
            native.options.insert(
                key.into(),
                serde_json::Value::String(
                    match effort {
                        instrukt_core::config::Effort::None => "none",
                        instrukt_core::config::Effort::Minimal => "minimal",
                        instrukt_core::config::Effort::Low => "low",
                        instrukt_core::config::Effort::Medium => "medium",
                        instrukt_core::config::Effort::High => "high",
                        instrukt_core::config::Effort::Xhigh => "xhigh",
                        instrukt_core::config::Effort::Max => "max",
                        instrukt_core::config::Effort::Ultra => "ultra",
                        instrukt_core::config::Effort::Ultrathink => "ultrathink",
                        instrukt_core::config::Effort::Off => "off",
                        instrukt_core::config::Effort::Auto => "auto",
                    }
                    .into(),
                ),
            );
        }
        native
    }
    fn create_thread(&self, request: &RuntimeStart) -> Result<Thread, SessionsError> {
        T3::create_thread(self, request)
    }
    fn verify_start(&self, request: &RuntimeStart, thread: &Thread) -> Result<(), SessionsError> {
        verify_request(request, thread)
    }
    fn start_or_wake(
        &self,
        assignment: &super::RuntimeAssignment<'_>,
    ) -> Result<Thread, SessionsError> {
        crate::keyed::start(
            assignment.invocation,
            self,
            assignment.request,
            assignment.project,
            true,
            assignment.now,
            assignment.creation,
        )
    }
    fn delivery_realization(&self) -> super::DeliveryRealization {
        super::DeliveryRealization::Queued
    }
    fn runtime(&self) -> Runtime {
        Runtime::T3
    }
    fn supports(&self, capability: Capability) -> bool {
        matches!(
            capability,
            Capability::ConversationSource
                | Capability::CreateWorktree
                | Capability::AgentSelection
                | Capability::InputConsumption
                | Capability::AssignmentEvidence
        )
    }
    fn observe_input(
        &self,
        session: &SessionRef,
        text: &str,
        now: jiff::Timestamp,
    ) -> Result<super::InputObservation, SessionsError> {
        let conversation = self.read_conversation(session)?;
        let message = conversation
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "user" && m.text == text);
        Ok(super::InputObservation {
            state: match message {
                Some(message) if message.run_id.is_some() => super::InputState::Consumed,
                Some(_) => super::InputState::Accepted,
                None => super::InputState::Unknown,
            },
            reference: message.and_then(|m| m.run_id.clone()),
            source: "t3 thread show: exact user input with native run assignment".into(),
            observed_at: now,
        })
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
        let waiting = conversation
            .messages
            .iter()
            .rev()
            .find(|m| {
                m.role == "user"
                    && m.run_id.is_none()
                    && (m.text.starts_with("instrukt delivery log\n")
                        || m.text
                            .starts_with("instrukt system-origin procedure assignment\n"))
            })
            .map(|m| m.text.clone());
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
        crate::keyed::start(
            assignment.invocation,
            self,
            assignment.request,
            assignment.project,
            false,
            assignment.now,
            assignment.creation,
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
            Ok(shown) => shown.thread.decode(),
            Err(SessionsError::Gone(_)) => Ok(Thread {
                session: session.clone(),
                project_id: None,
                title: None,
                semantic_key: None,
                state: SessionState::Gone,
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
    if thread.semantic_key.as_deref() != Some(request.key.as_str())
        || ![request.title.as_str(), request.key.as_str()]
            .into_iter()
            .any(|title| thread.title.as_deref() == Some(title))
    {
        return Err(failure(
            "start recovery",
            "thread title differs from the recorded key",
        ));
    }
    if let Some(worktree) = &request.worktree {
        if thread.branch.as_deref() != Some(&worktree.branch)
            || thread.worktree.as_ref().is_none_or(|p| !p.is_absolute())
        {
            return Err(failure(
                "start recovery",
                "requested role branch or absolute worktree path is missing",
            ));
        }
        let listing =
            instrukt_core::git::run(&request.checkout, ["worktree", "list", "--porcelain"])
                .map_err(|e| failure("start recovery", e.to_string()))?;
        let expected_path = thread
            .worktree
            .as_ref()
            .ok_or_else(|| failure("start recovery", "missing worktree path"))?;
        let registered = listing.split("\n\n").any(|entry| {
            entry
                .lines()
                .any(|l| l == format!("worktree {}", expected_path.display()))
                && entry
                    .lines()
                    .any(|l| l == format!("branch refs/heads/{}", worktree.branch))
        });
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
    fn decode(self) -> Result<Thread, SessionsError> {
        if self.thread_id.is_empty() {
            return Err(failure("decode", "incomplete thread identity"));
        }
        let status = match self.status.as_str() {
            "running" | "starting" | "resuming" | "queued" => SessionState::Running,
            "waiting" | "waiting_for_input" | "waiting_for_approval" => SessionState::NeedsInput,
            "idle" | "completed" | "stopped" => SessionState::InputReady,
            "failed" | "interrupted" | "cancelled" | "rolled_back" => SessionState::EndedFailed,
            _ => SessionState::Unknown,
        };
        let state = if self.archived == Some(true) {
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
            semantic_key: semantic_key(&self.title).map(str::to_owned),
            title: Some(self.title),
            state,
            branch: self.branch,
            worktree: self.worktree_path.filter(|p| !p.as_os_str().is_empty()),
            provider: self.provider_instance_id,
            model: self.model,
            interrupted: self.status == "interrupted",
            settled: self.settled,
            settled_at: self.settled_at,
            native_status: self.status,
            run_id: self.active_run_id.or(self.latest_run_id),
            activity,
            native_details: serde_json::Value::Object(self.details),
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
fn failure(operation: &str, detail: impl Into<String>) -> SessionsError {
    SessionsError::Failed {
        runtime: "t3".into(),
        operation: operation.into(),
        detail: detail.into(),
    }
}

// The label is opaque. Legacy whole-key titles and labelled suffixes share identity.
fn semantic_key(title: &str) -> Option<&str> {
    let key = title.rsplit(" · ").next()?;
    [
        "instrukt:todo:",
        "instrukt:standing:",
        "instrukt:task:",
        "instrukt:run:",
    ]
    .iter()
    .any(|prefix| {
        key.strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty())
    })
    .then_some(key)
    .filter(|key| !key.chars().any(char::is_whitespace))
}
