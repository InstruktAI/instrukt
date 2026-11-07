//! Unattended runtime operations with durable, keyed start recovery.
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use instrukt_core::config::{Configuration, Env, Runtime};
use instrukt_core::home::Home;
use instrukt_core::ids::SessionRef;
use serde::{Deserialize, Serialize};

use crate::agents::AgentChoice;
use crate::keyed;
use crate::process::ProcessError;
use crate::runtime::{Capability, Conversation, RuntimeAdapter, RuntimeStart, WorktreeRequest};

pub const RUNTIME_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionState {
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "needs-input")]
    NeedsInput,
    #[serde(rename = "input-ready")]
    InputReady,
    #[serde(rename = "ended failed")]
    EndedFailed,
    #[serde(rename = "gone")]
    Gone,
    #[serde(rename = "unknown")]
    Unknown,
}
impl SessionState {
    pub const ALL: [Self; 6] = [
        Self::Running,
        Self::NeedsInput,
        Self::InputReady,
        Self::EndedFailed,
        Self::Gone,
        Self::Unknown,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::NeedsInput => "needs-input",
            Self::InputReady => "input-ready",
            Self::EndedFailed => "ended failed",
            Self::Gone => "gone",
            Self::Unknown => "unknown",
        }
    }
}
impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl FromStr for SessionState {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL.into_iter().find(|x| x.as_str() == s).ok_or(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    State(SessionState),
    Unobservable(String),
}

#[derive(Debug, thiserror::Error)]
pub enum SessionsError {
    #[error("no runtime is chosen; set global `runner.runtime: t3`")]
    NoRuntime,
    #[error("unknown compiled runtime adapter `{0}`")]
    UnknownRuntime(String),
    #[error("thread {0} is no longer available")]
    Gone(SessionRef),
    #[error("no selected runtime caller context is available")]
    NoActiveRuntime,
    #[error("runtime `{runtime}` lacks the {capability} capability")]
    MissingCapability {
        runtime: String,
        capability: Capability,
    },
    #[error("runtime `{runtime}` {operation} failed: {detail}")]
    Failed {
        runtime: String,
        operation: String,
        detail: String,
    },
    #[error(
        "start `{key}` remains unresolved; use `instrukt sessions resolve-start {key} --session <runtime:id> | --none`"
    )]
    Unresolved { key: String },
    #[error(
        "start `{key}` has an unconfirmed result: {detail}; use `instrukt sessions resolve-start {key} --session <runtime:id> | --none`"
    )]
    Uncertain { key: String, detail: String },
    #[error("runtime key conflict for `{0}`; no thread was selected or started")]
    KeyConflict(String),
    #[error("recorded start inputs differ for `{0}`; retry with the original inputs")]
    ChangedRequest(String),
    #[error("bound thread {session} is absent or renamed for `{key}`; no replacement was started")]
    MissingBound { key: String, session: SessionRef },
    #[error("{0}")]
    Refused(String),
    #[error("no agent of the dispatch order could start (tried: {})", tried.join(", "))]
    OrderExhausted { tried: Vec<String> },
    #[error(transparent)]
    Config(#[from] instrukt_core::config::ConfigError),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error(transparent)]
    Process(#[from] ProcessError),
}
impl SessionsError {
    pub fn start_certainly_failed(&self) -> bool {
        matches!(
            self,
            Self::NoRuntime
                | Self::UnknownRuntime(_)
                | Self::MissingCapability { .. }
                | Self::Config(_)
                | Self::ChangedRequest(_)
                | Self::Refused(_)
                | Self::OrderExhausted { .. }
        )
    }
}

#[derive(Clone)]
pub struct StartRequest<'a> {
    pub now: jiff::Timestamp,
    pub creation: crate::runtime::CreationPermit,
    pub invocation: &'a keyed::RuntimeInvocation,
    pub message: &'a str,
    pub worktree: Option<&'a str>,
    pub cwd: Option<&'a Path>,
    pub key: Option<&'a str>,
    pub base: Option<&'a str>,
    pub from: Option<&'a str>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub session: SessionRef,
    pub worktree: Option<PathBuf>,
    pub agent: Option<AgentChoice>,
    pub key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListedSession {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub session: SessionRef,
    pub title: Option<String>,
    pub state: String,
    pub settled: bool,
    pub semantic_key: Option<String>,
    pub last_activity: Option<jiff::Timestamp>,
    pub activity_source: Option<String>,
}

pub struct Sessions<'c> {
    home: &'c Home,
    config: &'c Configuration,
    env: &'c Env,
}
impl<'c> Sessions<'c> {
    pub fn new(home: &'c Home, config: &'c Configuration, env: &'c Env) -> Self {
        Self { home, config, env }
    }
    pub fn runner_runtime(&self) -> Result<Runtime, SessionsError> {
        self.config
            .runner_runtime()?
            .ok_or(SessionsError::NoRuntime)
    }
    pub fn adapter(&self) -> Result<Box<dyn RuntimeAdapter>, SessionsError> {
        Ok(crate::runtime::adapter(self.runner_runtime()?))
    }
    pub fn start(
        &self,
        order: &[AgentChoice],
        request: &StartRequest<'_>,
    ) -> Result<Started, SessionsError> {
        let adapter = self.adapter()?;
        let (mut input, project) = self.input(request)?;
        if request.creation == crate::runtime::CreationPermit::Reconcile
            && let Some(retained) =
                keyed::retained_request(self.home, adapter.as_ref(), &input.key)?
        {
            if retained.checkout != input.checkout {
                return Err(SessionsError::ChangedRequest(input.key.clone()));
            }
            input = retained;
        }
        if input.selection.is_none() {
            input.preferences = order.to_vec();
            input.selection =
                crate::agents::selections(self.config, self.home, order, request.now)?
                    .into_iter()
                    .next();
            input.selection_evidence=Some("T3 CLI receives provider/model; effort and options cannot be expressed by thread start and remain requested native facts.".into());
        }
        let thread = adapter.start(&crate::runtime::RuntimeAssignment {
            creation: request.creation,
            invocation: request.invocation,
            project: Some(project.as_str()),
            request: &input,
            now: request.now,
        })?;
        Ok(Started {
            session: thread.session,
            worktree: thread.worktree,
            agent: None,
            key: input.key,
        })
    }
    fn input(
        &self,
        request: &StartRequest<'_>,
    ) -> Result<(RuntimeStart, instrukt_core::ids::ProjectSlug), SessionsError> {
        let project = self.config.project()?.ok_or_else(|| {
            SessionsError::Refused("session starts need a registered project".into())
        })?;
        let checkout = self.config.project_root().or(request.cwd).ok_or_else(|| {
            SessionsError::Refused("session starts need a project checkout".into())
        })?;
        // The published checkout selects the runtime project even when invoked in a linked worktree.
        let checkout = instrukt_core::publication::PublishedRecord::load(self.home, &project)
            .map_err(|e| SessionsError::Refused(e.to_string()))?
            .map(|p| p.checkout)
            .map(Ok)
            .unwrap_or_else(|| instrukt_core::repository::primary_worktree(checkout))
            .map_err(|e| SessionsError::Refused(e.to_string()))?;
        if request.key.is_none()
            && let Some(key) = keyed::pending_run(self.home, project.as_str())?
        {
            return Err(SessionsError::Refused(format!(
                "recorded request `{key}` is still pending; use `instrukt sessions resolve-start {key}`"
            )));
        }
        let key = match request.key {
            Some(key) => key.to_owned(),
            None => format!(
                "instrukt:run:{}:{}",
                keyed::encode(project.as_str()),
                keyed::request_id()?
            ),
        };
        let worktree = request
            .worktree
            .map(|branch| {
                let valid =
                    instrukt_core::git::run(&checkout, ["check-ref-format", "--branch", branch])
                        .map_err(|e| SessionsError::Refused(e.to_string()))?;
                if valid != branch {
                    return Err(SessionsError::Refused(
                        "a worktree needs a literal new branch name".into(),
                    ));
                }
                if request.key.is_none()
                    && instrukt_core::git::succeeds(
                        &checkout,
                        [
                            "show-ref",
                            "--verify",
                            "--quiet",
                            &format!("refs/heads/{branch}"),
                        ],
                    )
                    .map_err(|e| SessionsError::Refused(e.to_string()))?
                {
                    return Err(SessionsError::Refused(format!(
                        "worktree branch `{branch}` already exists; no runtime request was started"
                    )));
                }
                let from = match request.from {
                    Some(base) => base.to_owned(),
                    None => instrukt_core::github::default_branch(&checkout)
                        .map_err(|e| SessionsError::Refused(e.to_string()))?,
                };
                let base = instrukt_core::git::run(
                    &checkout,
                    [
                        "rev-parse",
                        "--verify",
                        &format!("{}^{{commit}}", request.base.unwrap_or(&from)),
                    ],
                )
                .map_err(|e| SessionsError::Refused(e.to_string()))?;
                Ok::<_, SessionsError>(WorktreeRequest {
                    from,
                    base,
                    branch: branch.to_owned(),
                })
            })
            .transpose()?;
        let input = RuntimeStart {
            preferences: Vec::new(),
            selection: None,
            selection_evidence: None,
            checkout,
            key,
            brief: request.message.into(),
            worktree,
        };
        Ok((input, project))
    }
    pub fn followup(
        &self,
        session: &SessionRef,
        path: Option<&Path>,
        request: &StartRequest<'_>,
    ) -> Result<Started, SessionsError> {
        if request.key.is_none() {
            return Err(SessionsError::Refused(
                "a follow-up needs its recorded start key".into(),
            ));
        }
        let (input, project) = self.input(request)?;
        let thread = keyed::followup(
            self.adapter()?.as_ref(),
            &crate::runtime::FollowupAssignment {
                invocation: request.invocation,
                request: &input,
                session,
                worktree_path: path,
                project: Some(project.as_str()),
                now: request.now,
            },
        )?;
        Ok(Started {
            session: thread.session,
            worktree: thread.worktree,
            agent: None,
            key: input.key,
        })
    }
    pub fn send(
        &self,
        invocation: &keyed::RuntimeInvocation,
        session: &SessionRef,
        entry: &crate::inputs::Entry<'_>,
    ) -> Result<crate::inputs::InputOutcome, SessionsError> {
        crate::inputs::enqueue(invocation, self.adapter()?.as_ref(), session, entry)
    }
    /// Operational listing keeps unknown activity visible and filters only supported native activity.
    pub fn list(
        &self,
        cwd: &Path,
        since: Option<Duration>,
        now: jiff::Timestamp,
    ) -> Result<Vec<ListedSession>, SessionsError> {
        let project = self.config.project()?.ok_or_else(|| {
            SessionsError::Refused("session listing needs a registered project".into())
        })?;
        let checkout = instrukt_core::publication::PublishedRecord::load(self.home, &project)
            .map_err(|e| SessionsError::Refused(e.to_string()))?
            .map(|r| r.checkout)
            .map(Ok)
            .unwrap_or_else(|| {
                instrukt_core::repository::primary_worktree(
                    self.config.project_root().unwrap_or(cwd),
                )
            })
            .map_err(|e| SessionsError::Refused(e.to_string()))?;
        let cutoff = since
            .map(|since| {
                let age = jiff::SignedDuration::try_from(since)
                    .map_err(|e| SessionsError::Refused(e.to_string()))?;
                now.checked_sub(age)
                    .map_err(|e| SessionsError::Refused(e.to_string()))
            })
            .transpose()?;
        let mut listed = Vec::new();
        for thread in self.adapter()?.list(&checkout)? {
            if thread.state == SessionState::Gone {
                continue;
            }
            let last_activity = thread.activity.as_ref().map(|a| a.at);
            if thread.settled
                && cutoff
                    .zip(last_activity)
                    .is_some_and(|(cutoff, activity)| activity < cutoff)
            {
                continue;
            }
            let state = if thread.settled && thread.state == SessionState::InputReady {
                "settled".into()
            } else {
                thread.state.as_str().into()
            };
            listed.push(ListedSession {
                provider: thread.provider,
                model: thread.model,
                session: thread.session,
                title: thread.title,
                semantic_key: thread.semantic_key,
                activity_source: thread
                    .activity
                    .map(|a| format!("{}: {}", a.source, a.meaning)),
                state,
                settled: thread.settled,
                last_activity,
            });
        }
        listed.sort_by(|a, b| {
            b.last_activity
                .cmp(&a.last_activity)
                .then(a.session.to_string().cmp(&b.session.to_string()))
        });
        Ok(listed)
    }
    pub fn alive(&self, session: &SessionRef) -> Result<Observation, SessionsError> {
        Ok(match self.adapter()?.read_state(session) {
            Ok(thread) => Observation::State(thread.state),
            Err(SessionsError::Gone(_)) => Observation::State(SessionState::Gone),
            Err(e) => Observation::Unobservable(e.to_string()),
        })
    }
    pub fn model(&self, session: &SessionRef) -> Option<(String, String)> {
        match self.adapter().and_then(|a| a.read_state(session)) {
            Ok(thread) => thread.provider.zip(thread.model),
            Err(error) => {
                tracing::warn!(event = "session_model_unknown", session = %session, error = %error);
                None
            }
        }
    }
    pub fn conversation(&self, session: &SessionRef) -> Result<Conversation, SessionsError> {
        self.adapter()?.read_conversation(session)
    }
    pub fn link(&self, a: &SessionRef, b: &SessionRef, remove: bool) -> Result<(), SessionsError> {
        self.adapter()?.link(a, b, remove)
    }
    pub fn caller(&self) -> Result<SessionRef, SessionsError> {
        self.adapter()?
            .caller_identity(self.env)
            .map(|caller| caller.session)
            .ok_or(SessionsError::NoActiveRuntime)
    }
}
