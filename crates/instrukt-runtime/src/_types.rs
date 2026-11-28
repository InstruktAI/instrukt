//! Runtime-neutral data types for threads, turns, launches, readiness, and capabilities.
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use instrukt_core::config::{AgentPreference, AgentSelection, Agents, HarnessKind};
use instrukt_core::ids::{InputId, ProviderInstanceId, RunId, SessionRef};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

pub const RUNTIME_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// Closed five-state session lifecycle observed through a runtime adapter.
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
}

impl SessionState {
    pub const ALL: [Self; 5] = [
        Self::Running,
        Self::NeedsInput,
        Self::InputReady,
        Self::EndedFailed,
        Self::Gone,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::NeedsInput => "needs-input",
            Self::InputReady => "input-ready",
            Self::EndedFailed => "ended failed",
            Self::Gone => "gone",
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

/// What the runtime established about this thread's identity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    #[default]
    Present,
    Archived,
    Absent,
}

/// Opening user input of an active turn; blank or missing native text normalizes to `Unavailable`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "text", rename_all = "snake_case")]
pub enum OpeningInput {
    Text(String),
    Unavailable,
}

impl OpeningInput {
    pub fn from_text(text: Option<&str>) -> Self {
        match text {
            Some(raw) if !raw.trim().is_empty() => Self::Text(raw.to_owned()),
            _ => Self::Unavailable,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text.as_str()),
            Self::Unavailable => None,
        }
    }
}

/// Structured lifecycle state of a single runtime turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TurnState {
    Preparing,
    Queued,
    Active {
        started_at: Timestamp,
    },
    Completed {
        started_at: Timestamp,
        completed_at: Timestamp,
    },
    Failed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        started_at: Option<Timestamp>,
        completed_at: Timestamp,
    },
}

/// The currently active (started, non-terminal) turn on a thread, when present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveTurn {
    pub run_id: RunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_id: Option<InputId>,
    pub started_at: Timestamp,
    pub opening_input: OpeningInput,
}

/// One turn reported in a thread's bounded recent-turns window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedTurn {
    pub run_id: RunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_id: Option<InputId>,
    pub state: TurnState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRequest {
    pub from: String,
    pub base: String,
    pub branch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LaunchRuntimeMode {
    ApprovalRequired,
    AutoAcceptEdits,
    Auto,
    FullAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LaunchInteractionMode {
    Plan,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedLaunchModes {
    pub runtime: LaunchRuntimeMode,
    pub interaction: LaunchInteractionMode,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeLaunch {
    pub command_id: String,
    pub thread_id: String,
    pub project_id: String,
    pub existing_worktree: Option<PathBuf>,
}

/// Persisted launch inputs. Deliberately omits `deny_unknown_fields` so legacy
/// records carrying `$.observed_session` or `$.replaced_session` remain readable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_modes: Option<ResolvedLaunchModes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_launch: Option<NativeLaunch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferences: Vec<AgentPreference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<AgentSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_evidence: Option<String>,
    pub checkout: PathBuf,
    pub key: String,
    pub title: String,
    pub brief: String,
    pub worktree: Option<WorktreeRequest>,
    /// The agent profiles of the admitted generation the start was prepared in;
    /// a request without them reads `agents.yaml` when it is started.
    #[serde(skip)]
    pub profiles: Profiles,
}

/// The agent profiles a start carries in memory. They are never stored, so
/// they take no part in comparing two requests.
#[derive(Debug, Clone, Default)]
pub struct Profiles(pub Option<Arc<Agents>>);

impl PartialEq for Profiles {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Profiles {}

/// Readable labels are presentation, independent of durable object identity.
pub fn thread_title(label: &str, _key: &str) -> String {
    label.to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeActivity {
    pub at: Timestamp,
    pub source: String,
    pub meaning: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub session: SessionRef,
    pub project_id: Option<String>,
    pub title: Option<String>,
    pub semantic_key: Option<String>,
    pub state: SessionState,
    #[serde(default)]
    pub presence: Presence,
    pub branch: Option<String>,
    pub worktree: Option<PathBuf>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub interrupted: bool,
    pub settled: bool,
    pub settled_at: Option<String>,
    pub native_status: String,
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_turn: Option<ActiveTurn>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_turns: Vec<ObservedTurn>,
    pub activity: Option<NativeActivity>,
    pub native_details: serde_json::Value,
}

/// A thread the runtime lists for a project and reports finished, as thread retention reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedThread {
    pub session: SessionRef,
    pub finished_at: Option<Timestamp>,
    pub latest_activity: Option<Timestamp>,
    pub quiet: bool,
    pub pinned: bool,
}

/// What an archive request found and did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveOutcome {
    Archived,
    AlreadyArchived,
    Changed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub text: String,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub session: SessionRef,
    pub messages: Vec<Message>,
    pub limit: usize,
    pub bounded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    ColdTextSource,
    CreateWorktree,
    AttachWorktree,
    Link,
    Notify,
    AgentSelection,
    InputConsumption,
    AssignmentEvidence,
    ConfiguredRuntimes,
    Archive,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ColdTextSource => "cold conversation text mirror",
            Self::CreateWorktree => "worktree creation",
            Self::AttachWorktree => "worktree attachment",
            Self::Link => "link",
            Self::Notify => "notification",
            Self::AgentSelection => "agent/model selection",
            Self::InputConsumption => "input consumption observation",
            Self::AssignmentEvidence => "assignment evidence recovery",
            Self::ConfiguredRuntimes => "configured agent runtimes",
            Self::Archive => "thread archiving",
        })
    }
}

/// Whether `RuntimeAdapter::execution_root` only reads or also creates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionRootMode {
    Check,
    Ensure,
}

/// The runtime project of one execution root, as the runtime holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRootStatus {
    pub project_id: Option<String>,
    pub created: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryRealization {
    Steering,
    Queued,
    EntryOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputState {
    Unknown,
    Accepted,
    Consumed,
}

/// The runtime's reconciliation of one exact send attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendEvidence {
    Accepted,
    Rejected,
    IdleWithoutInput,
    Unresolved,
    Presence(Presence),
}

/// Runtime-owned input evidence, independent of transcript/run-id representation.
#[derive(Debug, Clone)]
pub struct InputObservation {
    pub state: InputState,
    pub reference: Option<String>,
    pub source: String,
    pub observed_at: Timestamp,
}

#[derive(Debug, Clone)]
pub struct AssignmentEvidence {
    pub opening: String,
    pub waiting: Option<String>,
    pub source: String,
    pub observed_at: Timestamp,
}

/// A provider snapshot with the time it was observed.
pub struct ObservedSnapshot {
    pub snapshot: ProviderSnapshot,
    pub at: Timestamp,
}

/// Adapter-translated provider facts; expiry and selection belong to runner policy.
pub struct ProviderSnapshot {
    pub providers: Vec<ProviderFacts>,
    pub native_picks: Vec<String>,
}

pub struct ProviderFacts {
    pub instance: ProviderInstanceId,
    pub kind: Option<HarnessKind>,
    pub unavailable_reason: Option<String>,
    pub models: Vec<ReportedModel>,
    pub usage: Vec<UsageWindow>,
}

pub struct ReportedModel {
    pub name: String,
    pub aliases: Vec<String>,
}

pub struct UsageWindow {
    pub id: String,
    pub used_percent: f64,
    pub resets_at: Option<Timestamp>,
    pub model_family: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionOperation {
    ThreadStart,
    NativeDelegation,
}

#[cfg(test)]
mod tests {
    use super::*;
    use instrukt_core::config::Tier;
    use instrukt_core::ids::ProfileName;

    #[test]
    fn opening_input_rejects_none_and_whitespace_only_text() {
        assert_eq!(OpeningInput::from_text(None), OpeningInput::Unavailable);
        assert_eq!(OpeningInput::from_text(Some("")), OpeningInput::Unavailable);
        assert_eq!(
            OpeningInput::from_text(Some("   \n\t  ")),
            OpeningInput::Unavailable
        );
        assert_eq!(
            OpeningInput::from_text(Some("  hello\n")),
            OpeningInput::Text("  hello\n".into())
        );
    }

    #[test]
    fn session_state_closed_five_state_vocabulary_round_trips() {
        let expected = [
            (SessionState::Running, "running"),
            (SessionState::NeedsInput, "needs-input"),
            (SessionState::InputReady, "input-ready"),
            (SessionState::EndedFailed, "ended failed"),
            (SessionState::Gone, "gone"),
        ];
        assert_eq!(SessionState::ALL.len(), expected.len());
        for (state, wire) in expected {
            assert_eq!(state.as_str(), wire);
            assert_eq!(state.to_string(), wire);
            assert_eq!(wire.parse::<SessionState>(), Ok(state));
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, format!("\"{wire}\""));
            assert_eq!(serde_json::from_str::<SessionState>(&json).unwrap(), state);
        }
        assert!("unknown".parse::<SessionState>().is_err());
    }

    #[test]
    fn turn_state_payloads_round_trip() {
        let t1: Timestamp = "2026-11-28T10:00:00Z".parse().unwrap();
        let t2: Timestamp = "2026-11-28T10:01:00Z".parse().unwrap();
        let states = [
            TurnState::Preparing,
            TurnState::Queued,
            TurnState::Active { started_at: t1 },
            TurnState::Completed {
                started_at: t1,
                completed_at: t2,
            },
            TurnState::Failed {
                started_at: None,
                completed_at: t2,
            },
            TurnState::Failed {
                started_at: Some(t1),
                completed_at: t2,
            },
        ];
        for state in states {
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(serde_json::from_str::<TurnState>(&json).unwrap(), state);
        }
    }

    #[test]
    fn runtime_start_deserializes_with_legacy_extra_keys_and_agent_preference_wire_shape() {
        let legacy = serde_json::json!({
            "checkout": "/repo",
            "key": "instrukt:conversation:acme:slack:C1",
            "title": "Acme",
            "brief": "Hello",
            "worktree": null,
            "preferences": [{"agent": "claude", "tier": "strong"}],
            "observed_session": "t3:legacy-1",
            "replaced_session": "t3:legacy-0"
        });
        let decoded: RuntimeStart = serde_json::from_value(legacy).unwrap();
        assert_eq!(
            decoded.preferences,
            vec![AgentPreference {
                agent: ProfileName::parse("claude").unwrap(),
                tier: Some(Tier::Strong),
            }]
        );
        let round_trip = serde_json::to_value(&decoded).unwrap();
        assert_eq!(
            round_trip["preferences"],
            serde_json::json!([{"agent": "claude", "tier": "strong"}])
        );
    }
}
