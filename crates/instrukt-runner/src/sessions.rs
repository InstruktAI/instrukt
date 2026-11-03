//! The sessions facade over the runtime mapping (`session-continuity`):
//! start, send, observe, read, link and end sessions through the command
//! templates of the global layer's runtime entries.
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
//!   before it to remove the link); `alive`, `conversation` and `end`
//!   nothing.
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

use instrukt_core::config::{ConfigError, Configuration, Env, RuntimeEntry};
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

/// A capability a runtime entry declares by carrying its template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Link,
    End,
    Notify,
    Worktree,
}

impl Capability {
    fn name(self) -> &'static str {
        match self {
            Capability::Link => "link",
            Capability::End => "end",
            Capability::Notify => "notify",
            Capability::Worktree => "worktree",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A sessions facade operation failed.
#[derive(Debug, thiserror::Error)]
pub enum SessionsError {
    #[error("no runtime is configured; add an entry to `runtimes` in the global layer")]
    NoRuntime,
    #[error("`{0}` names no runtime of the runtime mapping")]
    UnknownRuntime(String),
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
    /// runtime with no agent selection.
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
            if let Some(check) = entry.field("available") {
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
                Err(err) => {
                    tracing::warn!(event = "dispatch_start_failed", agent = %choice.agent, error = %err);
                }
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
        let template = self.template(entry, "start")?;
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
        let template = self.template(entry, "send")?;
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
        let template = self.template(entry, "alive")?;
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

    /// Reads `session`'s conversation as the runtime prints it.
    pub fn conversation(&self, session: &SessionRef) -> Result<String, SessionsError> {
        let entry = self.entry(session)?;
        let template = self.template(entry, "conversation")?;
        let done = self
            .invocation(entry, template, Some(&session.id), &[])?
            .run(self.timeout)?;
        if !done.success {
            return Err(failed(entry, "conversation", &done.stderr));
        }
        Ok(done.stdout)
    }

    /// Links `a` and `b`, or removes their link.
    pub fn link(&self, a: &SessionRef, b: &SessionRef, remove: bool) -> Result<(), SessionsError> {
        if a.runtime != b.runtime {
            return Err(SessionsError::CrossRuntime {
                a: a.clone(),
                b: b.clone(),
            });
        }
        let entry = self.entry(a)?;
        let template = self.capability(entry, "link", Capability::Link)?;
        let operands: Vec<&str> = if remove {
            vec!["--remove", b.id.as_str()]
        } else {
            vec![b.id.as_str()]
        };
        let done = self
            .invocation(entry, template, Some(&a.id), &operands)?
            .run(self.timeout)?;
        if !done.success {
            return Err(failed(entry, "link", &done.stderr));
        }
        tracing::info!(event = "sessions_linked", a = %a, b = %b, remove);
        Ok(())
    }

    /// Ends `session`.
    pub fn end(&self, session: &SessionRef) -> Result<(), SessionsError> {
        let entry = self.entry(session)?;
        let template = self.capability(entry, "end", Capability::End)?;
        let done = self
            .invocation(entry, template, Some(&session.id), &[])?
            .run(self.timeout)?;
        if !done.success {
            return Err(failed(entry, "end", &done.stderr));
        }
        tracing::info!(event = "session_ended", session = %session);
        Ok(())
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
        field: &'static str,
    ) -> Result<&'e str, SessionsError> {
        entry.field(field).ok_or_else(|| SessionsError::Failed {
            runtime: entry.name.clone(),
            operation: field,
            detail: format!("the entry has no `{field}` template"),
        })
    }

    fn capability<'e>(
        &self,
        entry: &'e RuntimeEntry,
        field: &str,
        capability: Capability,
    ) -> Result<&'e str, SessionsError> {
        entry
            .field(field)
            .ok_or_else(|| SessionsError::MissingCapability {
                runtime: entry.name.clone(),
                capability,
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

    /// Runs `template`, filled for `id`, printing the session id it
    /// received between bars.
    fn received(template: &str, id: &SessionId) -> String {
        let command = fill::<()>(template, |piece| Ok(piece.to_owned())).unwrap();
        let done = Invocation::shell(&command, &[])
            .env(SESSION_ID_VAR, id.as_str())
            .run(Duration::from_secs(10))
            .unwrap();
        assert!(done.success, "{}", done.stderr);
        done.stdout
    }

    #[test]
    fn a_session_id_reaches_the_command_whole_in_every_quoting_context_and_runs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let planted = dir.path().join("planted");
        let hostile = format!(
            "x$(touch${{IFS}}{p})`touch${{IFS}}{p}`'\"${{HOME}}*;",
            p = planted.display()
        );
        let id = SessionId::parse(&hostile).unwrap();
        for (template, prefix) in [
            ("printf '|%s|' {runtime.session_id}", ""),
            ("printf '|%s|' \"{runtime.session_id}\"", ""),
            ("printf '|%s|' '{runtime.session_id}'", ""),
            ("printf '|%s|' \"a\"'{runtime.session_id}'", "a"),
        ] {
            assert_eq!(
                received(template, &id),
                format!("|{prefix}{hostile}|"),
                "{template}"
            );
        }
        assert!(!planted.exists(), "the id's command substitution ran");
    }
}
