//! The sessions facade over the runtime mapping (`session-continuity`):
//! start, send, observe, read, link and end sessions and look up history
//! through the verbs of the global layer's runtime entries.
//!
//! Each verb is a `command`, which the facade runs, or an `agent`
//! instruction, which it hands back for the calling agent to carry out
//! with its own runtime's tools; it never runs anything for an `agent`
//! verb. An instruction names the addressed session's id literally where
//! it says `{runtime.session_id}`, and lists the call's inputs after it,
//! one per line.
//!
//! Template contract, as this facade calls it:
//!
//! - Templates are read unexpanded and filled per call: the facade puts
//!   the addressed session's runtime-native id, shell-quoted, where the
//!   template says `{runtime.session_id}`, then expands the rest of the
//!   template through configuration interpolation.
//! - Per-call inputs are appended as positional arguments, never spliced
//!   into the command text: `start` receives the session's opening
//!   message; `send` the text; `link` the other session's id (`--remove`
//!   before it to remove the link); `history.search` the text;
//!   `history.show` the session id; `alive`, `conversation`, `end` and
//!   `end_self` nothing.
//! - `start` runs with `INSTRUKT_WORKTREE` set to the requested worktree
//!   name, and with `INSTRUKT_AGENT` and `INSTRUKT_THINKING` set when an
//!   agent was selected. It prints the new session's id on its first line
//!   and, for a worktree request, the worktree's path on the next.
//! - `alive` prints one of the five states.
//! - `available` (on an agent-CLI entry) exits zero when that agent can
//!   start now.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use instrukt_core::config::{ConfigError, Configuration, Env, RuntimeEntry, VerbKind, VerbName};
use instrukt_core::ids::{IdError, RuntimeName, SessionId, SessionRef};
use serde::{Deserialize, Serialize};

use crate::agents::AgentChoice;
use crate::process::{Invocation, ProcessError};

/// How long one runtime command may run.
pub const RUNTIME_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// The placeholder the facade fills with the addressed session's id.
const SESSION_PLACEHOLDER: &str = "{runtime.session_id}";

/// The five semantic session states (`session-continuity`, "Five-state
/// status contract").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionState {
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "needs-input")]
    NeedsInput,
    #[serde(rename = "ended ok")]
    EndedOk,
    #[serde(rename = "ended failed")]
    EndedFailed,
    #[serde(rename = "gone")]
    Gone,
}

impl SessionState {
    pub const ALL: [SessionState; 5] = [
        SessionState::Running,
        SessionState::NeedsInput,
        SessionState::EndedOk,
        SessionState::EndedFailed,
        SessionState::Gone,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Running => "running",
            SessionState::NeedsInput => "needs-input",
            SessionState::EndedOk => "ended ok",
            SessionState::EndedFailed => "ended failed",
            SessionState::Gone => "gone",
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
        SessionState::ALL
            .into_iter()
            .find(|state| state.as_str() == s)
            .ok_or(())
    }
}

/// What `alive` observed. A failed observation is not `gone`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    State(SessionState),
    /// The runtime could not say; the reason is for the reader.
    Unobservable(String),
}

/// A capability of a verb that the entry's text cannot show, found
/// missing when the verb runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// `start` returning the worktree path of a worktree request.
    Worktree,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Capability::Worktree => f.write_str("worktree"),
        }
    }
}

/// What an `agent` verb hands back: the instruction, with the call's
/// inputs after it, for the calling agent to carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction(pub String);

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What one call of a verb that takes either kind came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer<T> {
    /// The `command` ran; its result.
    Ran(T),
    /// The verb is `agent`: the caller's to do.
    YoursToDo(Instruction),
}

impl<T> Answer<T> {
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Answer<U> {
        match self {
            Answer::Ran(value) => Answer::Ran(f(value)),
            Answer::YoursToDo(instruction) => Answer::YoursToDo(instruction),
        }
    }
}

/// What ending one session came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    /// The runtime fills neither `end` (nor, for the caller's own session,
    /// `end_self`): it needs no ending, and nothing was done.
    NotNeeded { runtime: String },
    /// The runtime's `end` command ended it.
    Ended(SessionRef),
    /// The runtime fills ending as an instruction for the agent.
    YoursToDo {
        session: SessionRef,
        instruction: Instruction,
    },
}

/// A sessions facade operation failed.
#[derive(Debug, thiserror::Error)]
pub enum SessionsError {
    #[error("no runtime is configured; add an entry to `runtimes` in the global layer")]
    NoRuntime,
    #[error("`{0}` names no runtime of the runtime mapping")]
    UnknownRuntime(String),
    #[error("runtime `{runtime}` leaves `{verb}` empty, so this call is not available")]
    MissingVerb { runtime: String, verb: VerbName },
    #[error(
        "this call addresses the runtime the caller runs in, and no configured runtime's session variable is set"
    )]
    NoActiveRuntime,
    #[error("runtime `{runtime}` lacks the {capability} capability")]
    MissingCapability {
        runtime: String,
        capability: Capability,
    },
    #[error("runtime `{runtime}`: {source}")]
    Template {
        runtime: String,
        source: ConfigError,
    },
    #[error("runtime `{runtime}` {operation} failed: {detail}")]
    Failed {
        runtime: String,
        operation: &'static str,
        detail: String,
    },
    #[error("runtime `{runtime}` start printed no session id")]
    NoSessionId { runtime: String },
    #[error("runtime `{runtime}`: {source}")]
    Identity { runtime: String, source: IdError },
    #[error("{a} and {b} belong to different runtimes; a link joins sessions of one runtime")]
    CrossRuntime { a: SessionRef, b: SessionRef },
    #[error(
        "no agent of the dispatch order could start (tried: {}); the order is exhausted",
        tried.join(", ")
    )]
    OrderExhausted { tried: Vec<String> },
    #[error(transparent)]
    Process(#[from] ProcessError),
}

impl SessionsError {
    /// Whether a failed start certainly created no session: the runtime
    /// was never reached, or it answered that the start failed. A start
    /// that timed out, printed no usable id, or lost its worktree path may
    /// have created one; its result is uncertain.
    pub fn start_certainly_failed(&self) -> bool {
        matches!(
            self,
            SessionsError::NoRuntime
                | SessionsError::UnknownRuntime(_)
                | SessionsError::MissingVerb { .. }
                | SessionsError::Template { .. }
                | SessionsError::Failed { .. }
                | SessionsError::OrderExhausted { .. }
                | SessionsError::Process(ProcessError::Spawn { .. })
        )
    }
}

/// What to start.
#[derive(Debug, Clone, Default)]
pub struct StartRequest<'a> {
    /// The session's opening message.
    pub message: &'a str,
    /// A worktree the runtime creates or reattaches.
    pub worktree: Option<&'a str>,
    /// Where the runtime command runs.
    pub cwd: Option<&'a Path>,
}

/// A started session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub session: SessionRef,
    /// The worktree path the runtime returned for a worktree request.
    pub worktree: Option<PathBuf>,
    /// The agent that started, when an agent was selected.
    pub agent: Option<AgentChoice>,
}

/// The facade over one configuration's runtime mapping.
pub struct Sessions<'c> {
    config: &'c Configuration,
    env: &'c Env,
    timeout: Duration,
}

impl<'c> Sessions<'c> {
    pub fn new(config: &'c Configuration, env: &'c Env) -> Self {
        Self {
            config,
            env,
            timeout: RUNTIME_COMMAND_TIMEOUT,
        }
    }

    /// The runtime the runner itself uses (`runner.runtime`, else the first
    /// entry).
    pub fn runner_runtime(&self) -> Result<&'c RuntimeEntry, SessionsError> {
        match self.config.runner_runtime() {
            Ok(Some(entry)) => Ok(entry),
            Ok(None) => Err(SessionsError::NoRuntime),
            Err(source) => Err(SessionsError::Template {
                runtime: "runner.runtime".into(),
                source,
            }),
        }
    }

    /// Starts a session, walking `order` for the first agent the runtime
    /// reports available; an empty order starts through the runner's
    /// runtime with no agent selection. Without an `available` check, a
    /// start that certainly failed moves on to the next agent; an uncertain
    /// one ends the walk with its error.
    pub fn start(
        &self,
        order: &[AgentChoice],
        request: &StartRequest<'_>,
    ) -> Result<Started, SessionsError> {
        if order.is_empty() {
            let entry = self.runner_runtime()?;
            return self.start_on(entry, None, request);
        }
        let mut tried = Vec::new();
        for choice in order {
            tried.push(choice.agent.clone());
            let Some(entry) = self.agent_entry(&choice.agent) else {
                tracing::warn!(event = "dispatch_agent_unmapped", agent = %choice.agent);
                continue;
            };
            if let Some(check) = entry.command(VerbName::Available) {
                let available = self
                    .invocation(entry, check, None, &[])?
                    .run(self.timeout)
                    .map(|done| done.success)
                    .unwrap_or_else(|err| {
                        tracing::warn!(event = "dispatch_availability_unobservable", agent = %choice.agent, error = %err);
                        false
                    });
                if !available {
                    tracing::info!(event = "dispatch_agent_unavailable", agent = %choice.agent);
                    continue;
                }
                return self.start_on(entry, Some(choice), request);
            }
            match self.start_on(entry, Some(choice), request) {
                Ok(started) => return Ok(started),
                Err(err) if err.start_certainly_failed() => {
                    tracing::warn!(event = "dispatch_start_failed", agent = %choice.agent, error = %err);
                }
                // A start that may have created a session ends the walk:
                // trying the next agent could start a second one.
                Err(err) => return Err(err),
            }
        }
        Err(SessionsError::OrderExhausted { tried })
    }

    fn start_on(
        &self,
        entry: &RuntimeEntry,
        choice: Option<&AgentChoice>,
        request: &StartRequest<'_>,
    ) -> Result<Started, SessionsError> {
        let template = self.template(entry, VerbName::Start)?;
        let mut invocation = self.invocation(entry, template, None, &[request.message])?;
        if let Some(name) = request.worktree {
            invocation = invocation.env("INSTRUKT_WORKTREE", name);
        }
        if let Some(choice) = choice {
            invocation = invocation.env("INSTRUKT_AGENT", &choice.agent);
            if let Some(thinking) = &choice.thinking {
                invocation = invocation.env("INSTRUKT_THINKING", thinking.as_str());
            }
        }
        if let Some(cwd) = request.cwd {
            invocation = invocation.cwd(cwd);
        }
        let done = invocation.run(self.timeout)?;
        if !done.success {
            return Err(failed(entry, "start", &done.stderr));
        }
        let mut lines = done.stdout.lines().map(str::trim).filter(|l| !l.is_empty());
        let id = lines.next().ok_or_else(|| SessionsError::NoSessionId {
            runtime: entry.name.clone(),
        })?;
        let session = self.session_ref(entry, id)?;
        let worktree = match request.worktree {
            None => None,
            Some(_) => Some(PathBuf::from(lines.next().ok_or_else(|| {
                SessionsError::MissingCapability {
                    runtime: entry.name.clone(),
                    capability: Capability::Worktree,
                }
            })?)),
        };
        tracing::info!(event = "session_started", session = %session, agent = choice.map(|c| c.agent.as_str()));
        Ok(Started {
            session,
            worktree,
            agent: choice.cloned(),
        })
    }

    /// Sends `text` to `session`.
    pub fn send(&self, session: &SessionRef, text: &str) -> Result<(), SessionsError> {
        let entry = self.entry(session)?;
        let template = self.template(entry, VerbName::Send)?;
        let done = self
            .invocation(entry, template, Some(&session.id), &[text])?
            .run(self.timeout)?;
        if !done.success {
            return Err(failed(entry, "send", &done.stderr));
        }
        tracing::info!(event = "session_sent", session = %session);
        Ok(())
    }

    /// Observes `session`'s state.
    pub fn alive(&self, session: &SessionRef) -> Result<Observation, SessionsError> {
        let entry = self.entry(session)?;
        let template = self.template(entry, VerbName::Alive)?;
        let observed = self
            .invocation(entry, template, Some(&session.id), &[])?
            .run(self.timeout);
        Ok(match observed {
            Err(err) => Observation::Unobservable(err.to_string()),
            Ok(done) if !done.success => Observation::Unobservable(format!(
                "`alive` exited unsuccessfully: {}",
                done.stderr.trim()
            )),
            Ok(done) => match done.stdout.trim().parse::<SessionState>() {
                Ok(state) => Observation::State(state),
                Err(()) => Observation::Unobservable(format!(
                    "`alive` printed `{}`, which is none of the five states",
                    done.stdout.trim()
                )),
            },
        })
    }

    /// The model and version `session` runs, from its runtime's optional
    /// `model` template called with the session id, which prints `<model>
    /// <version>` (`stamps`, "Evidence and attribution"). `None` when the
    /// runtime has no template, or the template fails or prints anything
    /// else: the model is then unknown, never guessed.
    pub fn model(&self, session: &SessionRef) -> Option<(String, String)> {
        let entry = self.entry(session).ok()?;
        let template = entry.command(VerbName::Model)?;
        let done = match self
            .invocation(entry, template, Some(&session.id), &[])
            .and_then(|invocation| invocation.run(self.timeout).map_err(SessionsError::from))
        {
            Ok(done) => done,
            Err(err) => {
                tracing::warn!(event = "session_model_unknown", session = %session, error = %err, "the runtime's `model` template could not run");
                return None;
            }
        };
        if !done.success {
            tracing::warn!(event = "session_model_unknown", session = %session, stderr = %done.stderr.trim(), "the runtime's `model` template failed");
            return None;
        }
        let mut words = done.stdout.split_whitespace();
        match (words.next(), words.next(), words.next()) {
            (Some(model), Some(version), None) => Some((model.to_owned(), version.to_owned())),
            _ => {
                tracing::warn!(event = "session_model_unknown", session = %session, "the runtime's `model` template did not print `<model> <version>`");
                None
            }
        }
    }

    /// Reads `session`'s conversation as the runtime prints it.
    pub fn conversation(&self, session: &SessionRef) -> Result<String, SessionsError> {
        let entry = self.entry(session)?;
        let template = self.template(entry, VerbName::Conversation)?;
        let done = self
            .invocation(entry, template, Some(&session.id), &[])?
            .run(self.timeout)?;
        if !done.success {
            return Err(failed(entry, "conversation", &done.stderr));
        }
        Ok(done.stdout)
    }

    /// Links `a` and `b`, or removes their link; refused naming `link`
    /// when the runtime leaves it empty.
    pub fn link(
        &self,
        a: &SessionRef,
        b: &SessionRef,
        remove: bool,
    ) -> Result<Answer<()>, SessionsError> {
        if a.runtime != b.runtime {
            return Err(SessionsError::CrossRuntime {
                a: a.clone(),
                b: b.clone(),
            });
        }
        let entry = self.entry(a)?;
        let operands: Vec<&str> = if remove {
            vec!["--remove", b.id.as_str()]
        } else {
            vec![b.id.as_str()]
        };
        let answer = self.call(entry, VerbName::Link, Some(&a.id), &operands)?;
        if let Answer::Ran(_) = answer {
            tracing::info!(event = "sessions_linked", a = %a, b = %b, remove);
        }
        Ok(answer.map(|_| ()))
    }

    /// Ends `session` through its runtime's `end`.
    pub fn end(&self, session: &SessionRef) -> Result<Ending, SessionsError> {
        let entry = self.entry(session)?;
        if !entry.fills(VerbName::End) {
            return Ok(Ending::NotNeeded {
                runtime: entry.name.clone(),
            });
        }
        self.ending(entry, VerbName::End, session)
    }

    /// The caller's own session: the active runtime's `session_id_env`.
    pub fn caller(&self) -> Result<SessionRef, SessionsError> {
        let entry = self
            .config
            .active_runtime(self.env)
            .map_err(|_| SessionsError::NoActiveRuntime)?;
        let id = self.env.get(&entry.session_id_env).unwrap_or_default();
        self.session_ref(entry, id)
    }

    /// Ends the caller's own session through `end_self`, or through `end`
    /// with the caller's id when `end_self` is empty.
    pub fn end_self(&self) -> Result<Ending, SessionsError> {
        let caller = self.caller()?;
        let entry = self.entry(&caller)?;
        if entry.fills(VerbName::EndSelf) {
            return self.ending(entry, VerbName::EndSelf, &caller);
        }
        self.end(&caller)
    }

    fn ending(
        &self,
        entry: &RuntimeEntry,
        verb: VerbName,
        session: &SessionRef,
    ) -> Result<Ending, SessionsError> {
        Ok(match self.call(entry, verb, Some(&session.id), &[])? {
            Answer::Ran(_) => {
                tracing::info!(event = "session_ended", session = %session);
                Ending::Ended(session.clone())
            }
            Answer::YoursToDo(instruction) => Ending::YoursToDo {
                session: session.clone(),
                instruction,
            },
        })
    }

    /// Finds earlier conversations by `text` in the runtime the caller runs
    /// in (`history.search`); a command's output names their session ids.
    pub fn history_search(&self, text: &str) -> Result<Answer<String>, SessionsError> {
        let entry = self.active_entry()?;
        self.call(entry, VerbName::HistorySearch, None, &[text])
    }

    /// Shows one conversation of the runtime the caller runs in
    /// (`history.show`). `id` is the runtime-native id, or the
    /// `{runtime}:{id}` a record carries for that runtime.
    pub fn history_show(&self, id: &str) -> Result<Answer<String>, SessionsError> {
        let entry = self.active_entry()?;
        let native = SessionRef::parse(id)
            .ok()
            .filter(|session| session.runtime.as_str() == entry.name)
            .map_or_else(|| id.to_owned(), |session| session.id.to_string());
        self.call(entry, VerbName::HistoryShow, None, &[&native])
    }

    fn active_entry(&self) -> Result<&'c RuntimeEntry, SessionsError> {
        self.config
            .active_runtime(self.env)
            .map_err(|_| SessionsError::NoActiveRuntime)
    }

    /// Calls `verb` of `entry` for `session` with `operands`: runs a
    /// `command` and answers its output, or answers an `agent` verb's
    /// instruction. Refused naming the verb when the entry leaves it
    /// empty.
    fn call(
        &self,
        entry: &RuntimeEntry,
        verb: VerbName,
        session: Option<&SessionId>,
        operands: &[&str],
    ) -> Result<Answer<String>, SessionsError> {
        let filled = entry.verb(verb).ok_or_else(|| SessionsError::MissingVerb {
            runtime: entry.name.clone(),
            verb,
        })?;
        match filled.kind {
            VerbKind::Agent => Ok(Answer::YoursToDo(self.instruction(
                entry,
                &filled.text,
                session,
                operands,
            )?)),
            VerbKind::Command => {
                let done = self
                    .invocation(entry, &filled.text, session, operands)?
                    .run(self.timeout)?;
                if !done.success {
                    return Err(failed(entry, verb.key(), &done.stderr));
                }
                Ok(Answer::Ran(done.stdout))
            }
        }
    }

    /// An `agent` verb's instruction: `{runtime.session_id}` replaced by
    /// the addressed session's id as text, the rest expanded, and each
    /// input on a line of its own after it.
    fn instruction(
        &self,
        entry: &RuntimeEntry,
        text: &str,
        session: Option<&SessionId>,
        operands: &[&str],
    ) -> Result<Instruction, SessionsError> {
        let mut out = String::new();
        for (index, piece) in text.split(SESSION_PLACEHOLDER).enumerate() {
            if index > 0 {
                out.push_str(session.map(SessionId::as_str).unwrap_or_default());
            }
            out.push_str(&self.config.expand(piece, self.env).map_err(|source| {
                SessionsError::Template {
                    runtime: entry.name.clone(),
                    source,
                }
            })?);
        }
        for operand in operands {
            out.push('\n');
            out.push_str(operand);
        }
        Ok(Instruction(out))
    }

    /// The runtime entry `session` belongs to.
    pub fn entry(&self, session: &SessionRef) -> Result<&'c RuntimeEntry, SessionsError> {
        self.config
            .runtimes()
            .iter()
            .find(|entry| entry.name == session.runtime.as_str())
            .ok_or_else(|| SessionsError::UnknownRuntime(session.runtime.to_string()))
    }

    /// The first agent-CLI entry starting `agent`.
    fn agent_entry(&self, agent: &str) -> Option<&'c RuntimeEntry> {
        self.config
            .runtimes()
            .iter()
            .find(|entry| entry.agent.as_deref() == Some(agent))
    }

    fn template<'e>(
        &self,
        entry: &'e RuntimeEntry,
        verb: VerbName,
    ) -> Result<&'e str, SessionsError> {
        entry
            .command(verb)
            .ok_or_else(|| SessionsError::MissingVerb {
                runtime: entry.name.clone(),
                verb,
            })
    }

    /// The command for `template` of `entry`, its session placeholder
    /// filled with `session`, with `operands` appended.
    pub(crate) fn invocation(
        &self,
        entry: &RuntimeEntry,
        template: &str,
        session: Option<&SessionId>,
        operands: &[&str],
    ) -> Result<Invocation<'static>, SessionsError> {
        let expand = |piece: &str| {
            self.config
                .expand(piece, self.env)
                .map_err(|source| SessionsError::Template {
                    runtime: entry.name.clone(),
                    source,
                })
        };
        let Some(session) = session else {
            return Ok(Invocation::shell(&expand(template)?, operands));
        };
        let command = fill(template, expand)?;
        Ok(Invocation::shell(&command, operands).env(SESSION_ID_VAR, session.as_str()))
    }

    fn session_ref(&self, entry: &RuntimeEntry, id: &str) -> Result<SessionRef, SessionsError> {
        let identity = |source| SessionsError::Identity {
            runtime: entry.name.clone(),
            source,
        };
        Ok(SessionRef {
            runtime: RuntimeName::parse(&entry.name).map_err(identity)?,
            id: SessionId::parse(id).map_err(identity)?,
        })
    }
}

fn failed(entry: &RuntimeEntry, operation: &'static str, stderr: &str) -> SessionsError {
    SessionsError::Failed {
        runtime: entry.name.clone(),
        operation,
        detail: if stderr.trim().is_empty() {
            "it exited unsuccessfully".to_owned()
        } else {
            stderr.trim().to_owned()
        },
    }
}

/// The environment variable that carries the addressed session's id to
/// the command; the command text only ever refers to it.
pub(crate) const SESSION_ID_VAR: &str = "INSTRUKT_SESSION_ID";

/// `template` with every piece but `{runtime.session_id}` passed through
/// `expand`, and each placeholder replaced by a reference to
/// [`SESSION_ID_VAR`] that reads as the whole id in the shell quoting
/// context where it stands: `"$VAR"` bare, `${VAR}` inside double quotes,
/// and `'"$VAR"'` inside single quotes. The id's characters never become
/// command text, so no content of an id can run.
pub(crate) fn fill<E>(
    template: &str,
    mut expand: impl FnMut(&str) -> Result<String, E>,
) -> Result<String, E> {
    let mut command = String::new();
    for (index, piece) in template.split(SESSION_PLACEHOLDER).enumerate() {
        if index > 0 {
            let reference = match quoting_at_end(&command) {
                Quoting::Bare => format!("\"${SESSION_ID_VAR}\""),
                Quoting::Double => format!("${{{SESSION_ID_VAR}}}"),
                Quoting::Single => format!("'\"${SESSION_ID_VAR}\"'"),
            };
            command.push_str(&reference);
        }
        command.push_str(&expand(piece)?);
    }
    Ok(command)
}

/// The shell quoting context at the end of `text`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quoting {
    Bare,
    Single,
    Double,
}

fn quoting_at_end(text: &str) -> Quoting {
    let mut state = Quoting::Bare;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (state, c) {
            (Quoting::Bare, '\\') | (Quoting::Double, '\\') => {
                chars.next();
            }
            (Quoting::Bare, '\'') => state = Quoting::Single,
            (Quoting::Bare, '"') => state = Quoting::Double,
            (Quoting::Single, '\'') | (Quoting::Double, '"') => state = Quoting::Bare,
            _ => {}
        }
    }
    state
}

/// `value` as one single-quoted shell word.
pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_five_states_round_trip_and_nothing_else_parses() {
        for state in SessionState::ALL {
            assert_eq!(state.as_str().parse::<SessionState>(), Ok(state));
        }
        for other in ["", "ended", "RUNNING", "alive"] {
            assert_eq!(other.parse::<SessionState>(), Err(()));
        }
    }

    #[test]
    fn the_quoting_context_follows_shell_quotes_and_escapes() {
        for (text, expected) in [
            ("rt send ", Quoting::Bare),
            ("rt send \"", Quoting::Double),
            ("rt send '", Quoting::Single),
            ("rt send \"a\"'", Quoting::Single),
            ("rt send '\"'", Quoting::Bare),
            ("rt send \\'", Quoting::Bare),
            ("rt send \"\\\"", Quoting::Double),
            ("rt send '\\'", Quoting::Bare),
        ] {
            assert_eq!(quoting_at_end(text), expected, "{text}");
        }
    }

    #[test]
    fn only_a_start_that_never_reached_the_runtime_or_was_refused_certainly_failed() {
        let failed = SessionsError::Failed {
            runtime: "rt".into(),
            operation: "start",
            detail: String::new(),
        };
        assert!(failed.start_certainly_failed());
        let timed_out = SessionsError::Process(ProcessError::Timeout {
            program: "sh".into(),
            timeout: Duration::from_secs(1),
        });
        assert!(!timed_out.start_certainly_failed());
        assert!(
            !SessionsError::NoSessionId {
                runtime: "rt".into()
            }
            .start_certainly_failed()
        );
        assert!(
            !SessionsError::MissingCapability {
                runtime: "rt".into(),
                capability: Capability::Worktree,
            }
            .start_certainly_failed()
        );
    }
}
