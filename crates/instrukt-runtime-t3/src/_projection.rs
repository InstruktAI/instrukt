//! Live decoding of pinned T3 `v0.1.8` `orchestration.getThreadProjection` responses.
//! Zero live operational paths open T3's `statev2.sqlite`.
use std::path::PathBuf;

use instrukt_core::ids::{InputId, RunId, SessionRef};
use instrukt_runtime::{
    ActiveTurn, AssignmentEvidence, InputObservation, InputState, NativeActivity, ObservedTurn,
    OpeningInput, Presence, RuntimeError, SendEvidence, SessionState, Thread, TurnState,
};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::Value;

/// A thread's status as T3 spells it in `t3 thread show`, `t3 thread list`, and projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThreadStatus {
    Running,
    Preparing,
    Starting,
    Resuming,
    Queued,
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
    pub(crate) fn parse(native: &str) -> Option<Self> {
        use serde::de::IntoDeserializer as _;
        let spelling: serde::de::value::StrDeserializer<'_, serde::de::value::Error> =
            native.into_deserializer();
        Self::deserialize(spelling).ok()
    }

    pub(crate) fn state(self) -> SessionState {
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
struct ProjectionEnvelope {
    thread: ProjectionThread,
    #[serde(default)]
    runs: Vec<ProjectionRun>,
    #[serde(default)]
    messages: Vec<ProjectionMessage>,
    #[serde(default)]
    runtime_requests: Vec<ProjectionRuntimeRequest>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectionModelSelection {
    instance_id: Option<String>,
    model: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectionThread {
    id: Option<String>,
    thread_id: Option<String>,
    project_id: Option<String>,
    title: Option<String>,
    status: Option<String>,
    branch: Option<String>,
    worktree_path: Option<PathBuf>,
    archived_at: Option<String>,
    deleted_at: Option<String>,
    archived: Option<bool>,
    pending_request_count: Option<u64>,
    model_selection: Option<ProjectionModelSelection>,
    provider_instance_id: Option<String>,
    model: Option<String>,
    settled_override: Option<String>,
    settled: Option<bool>,
    settled_at: Option<String>,
    active_run_id: Option<String>,
    latest_run_id: Option<String>,
    updated_at: Option<String>,
    #[serde(flatten)]
    details: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectionRun {
    id: String,
    #[serde(default)]
    ordinal: Option<u64>,
    #[serde(default)]
    user_message_id: Option<String>,
    status: String,
    #[serde(default)]
    requested_at: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    completed_at: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectionMessage {
    id: String,
    role: String,
    text: String,
    #[serde(default)]
    run_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectionRuntimeRequest {
    status: Option<String>,
}

fn obs(detail: impl Into<String>) -> RuntimeError {
    RuntimeError::Observation(detail.into())
}

fn parse_timestamp(value: &str, field: &str) -> Result<Timestamp, RuntimeError> {
    value
        .parse::<Timestamp>()
        .map_err(|e| obs(format!("invalid projection {field} timestamp: {e}")))
}

fn parse_opt_timestamp(
    value: Option<&str>,
    field: &str,
) -> Result<Option<Timestamp>, RuntimeError> {
    value
        .filter(|s| !s.is_empty())
        .map(|s| parse_timestamp(s, field))
        .transpose()
}

fn is_active_run_status(status: &str) -> bool {
    matches!(status, "starting" | "running" | "waiting")
}

fn is_unended_run_status(status: &str) -> bool {
    !matches!(
        status,
        "completed" | "failed" | "interrupted" | "cancelled" | "rolled_back"
    )
}

fn parse_input_id(raw: Option<&str>) -> Result<Option<InputId>, RuntimeError> {
    raw.filter(|s| !s.is_empty())
        .map(|id| InputId::parse(id).map_err(|e| obs(format!("invalid projection input id: {e}"))))
        .transpose()
}

fn decode_observed_turn(run: &ProjectionRun) -> Result<ObservedTurn, RuntimeError> {
    let run_id =
        RunId::parse(&run.id).map_err(|e| obs(format!("invalid projection run id: {e}")))?;
    let input_id = parse_input_id(run.user_message_id.as_deref())?;
    let started_at = parse_opt_timestamp(run.started_at.as_deref(), "startedAt")?;
    let completed_at = parse_opt_timestamp(run.completed_at.as_deref(), "completedAt")?;
    let requested_at = parse_opt_timestamp(run.requested_at.as_deref(), "requestedAt")?;

    let state = match run.status.as_str() {
        "preparing" => TurnState::Preparing,
        "queued" => TurnState::Queued,
        "starting" | "running" | "waiting" => match started_at {
            Some(ts) => TurnState::Active { started_at: ts },
            None => TurnState::Preparing,
        },
        "completed" => {
            let done =
                completed_at.ok_or_else(|| obs("completed run missing completedAt timestamp"))?;
            let start = started_at
                .or(requested_at)
                .ok_or_else(|| obs("completed run missing startedAt timestamp"))?;
            TurnState::Completed {
                started_at: start,
                completed_at: done,
            }
        }
        "failed" | "interrupted" | "cancelled" | "rolled_back" => {
            let done = completed_at
                .or(requested_at)
                .ok_or_else(|| obs("failed run missing completedAt timestamp"))?;
            TurnState::Failed {
                started_at,
                completed_at: done,
            }
        }
        other => {
            return Err(obs(format!("unrecognized T3 run status `{other}`")));
        }
    };

    Ok(ObservedTurn {
        run_id,
        input_id,
        state,
    })
}

fn sorted_runs(runs: &[ProjectionRun]) -> Vec<&ProjectionRun> {
    let mut indexed: Vec<(usize, &ProjectionRun)> = runs.iter().enumerate().collect();
    indexed.sort_by_key(|(idx, run)| (run.ordinal.unwrap_or(*idx as u64), *idx));
    indexed.into_iter().map(|(_, run)| run).collect()
}

fn opening_input_for(messages: &[ProjectionMessage], input_id: Option<&InputId>) -> OpeningInput {
    let Some(id) = input_id else {
        return OpeningInput::Unavailable;
    };
    let text = messages
        .iter()
        .find(|m| m.role == "user" && m.id == id.as_str())
        .map(|m| m.text.as_str());
    OpeningInput::from_text(text)
}

fn decode_envelope(raw: Value) -> Result<ProjectionEnvelope, RuntimeError> {
    serde_json::from_value(raw).map_err(|e| obs(format!("malformed thread projection: {e}")))
}

impl ProjectionThread {
    fn thread_id(&self) -> &str {
        self.id
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| self.thread_id.as_deref().filter(|s| !s.is_empty()))
            .unwrap_or_default()
    }
}

/// Validates that a returned `orchestration.getThreadProjection` names `expected_thread_id`.
pub(crate) fn verify_thread_exists(
    raw: Value,
    expected_thread_id: &str,
) -> Result<(), RuntimeError> {
    let env = decode_envelope(raw)?;
    if env.thread.thread_id() != expected_thread_id {
        return Err(obs("thread projection does not match requested identity"));
    }
    Ok(())
}

/// Decodes a live `orchestration.getThreadProjection` payload into a neutral [`Thread`].
pub(crate) fn decode_thread(raw: Value, expected: &SessionRef) -> Result<Thread, RuntimeError> {
    let env = decode_envelope(raw)?;
    decode_envelope_thread(&env, expected)
}

fn decode_envelope_thread(
    env: &ProjectionEnvelope,
    expected: &SessionRef,
) -> Result<Thread, RuntimeError> {
    let thread_id = env.thread.thread_id();
    if thread_id.is_empty() {
        return Err(obs("incomplete thread identity in projection"));
    }
    if thread_id != expected.id.as_str() {
        return Err(obs(
            "thread projection response does not match the requested identity",
        ));
    }
    let session = SessionRef::parse(&format!("t3:{thread_id}"))
        .map_err(|e| obs(format!("invalid projected thread id: {e}")))?;

    let presence = if env
        .thread
        .deleted_at
        .as_deref()
        .is_some_and(|s| !s.is_empty())
    {
        Presence::Absent
    } else if env
        .thread
        .archived_at
        .as_deref()
        .is_some_and(|s| !s.is_empty())
        || env.thread.archived == Some(true)
    {
        Presence::Archived
    } else {
        Presence::Present
    };

    let ordered_runs = sorted_runs(&env.runs);
    let mut recent_turns = Vec::with_capacity(ordered_runs.len());
    for run in &ordered_runs {
        recent_turns.push(decode_observed_turn(run)?);
    }

    let pending_requests = if !env.runtime_requests.is_empty() {
        env.runtime_requests
            .iter()
            .filter(|req| req.status.as_deref() == Some("pending"))
            .count() as u64
    } else {
        env.thread.pending_request_count.unwrap_or(0)
    };

    let active_run = ordered_runs
        .iter()
        .rev()
        .find(|run| is_active_run_status(&run.status))
        .copied();

    let active_turn = if presence == Presence::Present {
        if let Some(run) = active_run {
            let started_at = parse_opt_timestamp(run.started_at.as_deref(), "startedAt")?;
            if let Some(started_at) = started_at {
                let run_id = RunId::parse(&run.id)
                    .map_err(|e| obs(format!("invalid active run id: {e}")))?;
                let input_id = parse_input_id(run.user_message_id.as_deref())?;
                let opening_input = opening_input_for(&env.messages, input_id.as_ref());
                Some(ActiveTurn {
                    run_id,
                    input_id,
                    started_at,
                    opening_input,
                })
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let native_status_str = match presence {
        Presence::Absent => "deleted".to_owned(),
        Presence::Archived => "archived".to_owned(),
        Presence::Present => {
            if let Some(status) = env.thread.status.as_deref() {
                status.to_owned()
            } else if let Some(run) = active_run {
                run.status.clone()
            } else if let Some(run) = ordered_runs
                .iter()
                .rev()
                .find(|run| matches!(run.status.as_str(), "preparing" | "queued"))
            {
                run.status.clone()
            } else if let Some(latest) = ordered_runs.last() {
                latest.status.clone()
            } else {
                "idle".to_owned()
            }
        }
    };

    let (state, interrupted) = match presence {
        Presence::Absent | Presence::Archived => (SessionState::Gone, false),
        Presence::Present => {
            let parsed = ThreadStatus::parse(&native_status_str).ok_or_else(|| {
                obs(format!(
                    "unrecognized T3 thread status `{native_status_str}`"
                ))
            })?;
            let state = if pending_requests > 0 {
                SessionState::NeedsInput
            } else {
                parsed.state()
            };
            (state, parsed == ThreadStatus::Interrupted)
        }
    };

    let settled = env.thread.settled_override.as_deref() == Some("settled")
        || env.thread.settled == Some(true);
    let provider = env
        .thread
        .model_selection
        .as_ref()
        .and_then(|s| s.instance_id.clone())
        .or_else(|| env.thread.provider_instance_id.clone());
    let model = env
        .thread
        .model_selection
        .as_ref()
        .and_then(|s| s.model.clone())
        .or_else(|| env.thread.model.clone());

    let activity = [
        env.thread.updated_at.as_deref(),
        env.thread.settled_at.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(|text| parse_timestamp(text, "activity"))
    .collect::<Result<Vec<_>, _>>()?
    .into_iter()
    .max()
    .map(|at| NativeActivity {
        at,
        source: "t3 updatedAt/settledAt".into(),
        meaning: "native latest activity or settling".into(),
    });

    let run_id = active_turn
        .as_ref()
        .map(|turn| turn.run_id.to_string())
        .or_else(|| ordered_runs.last().map(|run| run.id.clone()))
        .or_else(|| env.thread.active_run_id.clone())
        .or_else(|| env.thread.latest_run_id.clone());

    Ok(Thread {
        session,
        project_id: env.thread.project_id.clone(),
        title: env.thread.title.clone(),
        semantic_key: None,
        state,
        presence,
        branch: env.thread.branch.clone(),
        worktree: env
            .thread
            .worktree_path
            .clone()
            .filter(|p| !p.as_os_str().is_empty()),
        provider,
        model,
        interrupted,
        settled,
        settled_at: env.thread.settled_at.clone(),
        native_status: native_status_str,
        run_id,
        active_turn,
        recent_turns,
        activity,
        native_details: Value::Object(env.thread.details.clone()),
    })
}

/// Evaluates `observe_send` from a live `orchestration.getThreadProjection`.
/// Restricts text matching strictly to `command_id.is_none()`.
pub(crate) fn decode_send_evidence(
    raw: Value,
    session: &SessionRef,
    text: &str,
    command_id: Option<&str>,
) -> Result<SendEvidence, RuntimeError> {
    let env = decode_envelope(raw)?;
    let thread = decode_envelope_thread(&env, session)?;
    if thread.presence != Presence::Present {
        return Ok(SendEvidence::Presence(thread.presence));
    }
    let accepted = match command_id {
        Some(id) => env.messages.iter().any(|m| m.role == "user" && m.id == id),
        None => env
            .messages
            .iter()
            .any(|m| m.role == "user" && m.text == text),
    };
    if accepted {
        return Ok(SendEvidence::Accepted);
    }
    let no_unended_run = !env
        .runs
        .iter()
        .any(|run| is_unended_run_status(&run.status));
    let no_pending = !env
        .runtime_requests
        .iter()
        .any(|req| req.status.as_deref() == Some("pending"))
        && env.thread.pending_request_count.unwrap_or(0) == 0;
    if no_unended_run
        && no_pending
        && matches!(
            thread.state,
            SessionState::InputReady | SessionState::EndedFailed
        )
    {
        Ok(SendEvidence::IdleWithoutInput)
    } else {
        Ok(SendEvidence::Unresolved)
    }
}

fn message_input_state(
    msg: &ProjectionMessage,
    runs: &[ProjectionRun],
) -> Result<InputState, RuntimeError> {
    let mut state = InputState::Accepted;
    if let Some(run_id) = msg.run_id.as_deref().filter(|s| !s.is_empty())
        && let Some(run) = runs.iter().find(|r| r.id == run_id)
    {
        if !matches!(
            run.status.as_str(),
            "preparing"
                | "queued"
                | "starting"
                | "running"
                | "waiting"
                | "completed"
                | "interrupted"
                | "failed"
                | "cancelled"
                | "rolled_back"
        ) {
            return Err(obs(format!(
                "unsupported input run status `{}`",
                run.status
            )));
        }
        // T3 appends steering messages before its steer effect runs. The run's
        // earlier start proves entry only for its opening userMessageId, and
        // cancelled/failed queued runs without a parseable startedAt stay Accepted.
        if run.user_message_id.as_deref() == Some(msg.id.as_str())
            && !matches!(run.status.as_str(), "queued" | "preparing")
            && let Some(started) = run.started_at.as_deref().filter(|s| !s.is_empty())
        {
            parse_timestamp(started, "startedAt")?;
            state = InputState::Consumed;
        }
    }
    Ok(state)
}

/// Evaluates `observe_input` from a live `orchestration.getThreadProjection`.
pub(crate) fn decode_input_observation(
    raw: Value,
    session: &SessionRef,
    text: &str,
    now: Timestamp,
) -> Result<InputObservation, RuntimeError> {
    let env = decode_envelope(raw)?;
    let thread = decode_envelope_thread(&env, session)?;
    if thread.presence != Presence::Present {
        return Err(RuntimeError::ThreadNotFound {
            thread_id: session.id.to_string(),
        });
    }
    let found = env
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user" && m.text == text);
    let (state, reference) = match found {
        Some(msg) => (
            message_input_state(msg, &env.runs)?,
            msg.run_id.clone().filter(|s| !s.is_empty()),
        ),
        None => (InputState::Unknown, None),
    };
    Ok(InputObservation {
        state,
        reference,
        source: "T3 thread projection".into(),
        observed_at: now,
    })
}

fn unwrap_control_frame(text: &str) -> &str {
    for tag in ["Instrukt Notification", "Instrukt Assignment"] {
        let open = format!("[{tag}]\n");
        let close = format!("\n[/{tag}]");
        if let Some(rest) = text.strip_prefix(&open)
            && let Some((body, tail)) = rest.split_once(&close)
            && (tail.is_empty() || tail.starts_with('\n'))
        {
            return body;
        }
    }
    text
}

/// Evaluates `assignment_evidence` from a live `orchestration.getThreadProjection`.
pub(crate) fn decode_assignment_evidence(
    raw: Value,
    session: &SessionRef,
    now: Timestamp,
) -> Result<AssignmentEvidence, RuntimeError> {
    let env = decode_envelope(raw)?;
    let _thread = decode_envelope_thread(&env, session)?;
    let opening = env
        .messages
        .iter()
        .filter(|m| m.role == "user")
        .find(|m| {
            m.text
                .lines()
                .any(|line| line.starts_with("Assignment facts: "))
        })
        .ok_or_else(|| RuntimeError::Failed {
            runtime: "t3".into(),
            operation: "assignment recovery".into(),
            detail: "bounded recent conversation has no initialization evidence".into(),
        })?
        .text
        .clone();

    let mut waiting = None;
    for msg in env.messages.iter().rev().filter(|m| m.role == "user") {
        let state = message_input_state(msg, &env.runs)?;
        let body = unwrap_control_frame(&msg.text);
        if state != InputState::Consumed
            && (body.starts_with("instrukt delivery log\n")
                || body.starts_with("instrukt system-origin procedure assignment\n"))
        {
            waiting = Some(msg.text.clone());
            break;
        }
    }

    Ok(AssignmentEvidence {
        opening,
        waiting,
        source: "T3 thread projection: bounded attributed initialization input".into(),
        observed_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session() -> SessionRef {
        SessionRef::parse("t3:thread-1").unwrap()
    }

    fn base_projection() -> Value {
        json!({
            "thread": {
                "id": "thread-1",
                "projectId": "proj-1",
                "title": "Acme conversation",
                "branch": "main",
                "worktreePath": null,
                "modelSelection": {"instanceId": "codex", "model": "gpt-5"},
                "archivedAt": null,
                "deletedAt": null,
                "settledOverride": null,
                "settledAt": null,
                "updatedAt": "2026-11-28T10:00:00Z"
            },
            "runs": [
                {
                    "id": "run-1",
                    "ordinal": 1,
                    "userMessageId": "cmd-1",
                    "status": "completed",
                    "requestedAt": "2026-11-28T09:50:00Z",
                    "startedAt": "2026-11-28T09:50:01Z",
                    "completedAt": "2026-11-28T09:51:00Z"
                },
                {
                    "id": "run-2",
                    "ordinal": 2,
                    "userMessageId": "cmd-2",
                    "status": "running",
                    "requestedAt": "2026-11-28T10:00:00Z",
                    "startedAt": "2026-11-28T10:00:02Z",
                    "completedAt": null
                }
            ],
            "messages": [
                {"id": "cmd-1", "role": "user", "text": "first turn", "runId": "run-1"},
                {"id": "cmd-2", "role": "user", "text": "second turn", "runId": "run-2"}
            ],
            "runtimeRequests": []
        })
    }

    #[test]
    fn decodes_active_turn_and_recent_turns_from_projection() {
        let thread = decode_thread(base_projection(), &session()).unwrap();
        assert_eq!(thread.presence, Presence::Present);
        assert_eq!(thread.state, SessionState::Running);
        assert_eq!(thread.provider.as_deref(), Some("codex"));
        assert_eq!(thread.model.as_deref(), Some("gpt-5"));
        let active = thread.active_turn.expect("active turn");
        assert_eq!(active.run_id.as_str(), "run-2");
        assert_eq!(active.input_id.as_ref().map(InputId::as_str), Some("cmd-2"));
        assert_eq!(
            active.opening_input,
            OpeningInput::Text("second turn".into())
        );
        assert_eq!(thread.recent_turns.len(), 2);
        assert!(matches!(
            thread.recent_turns[0].state,
            TurnState::Completed { .. }
        ));
        assert!(matches!(
            thread.recent_turns[1].state,
            TurnState::Active { .. }
        ));
    }

    #[test]
    fn queued_or_preparing_run_has_no_active_turn_and_does_not_fabricate_started_at() {
        let mut proj = base_projection();
        proj["runs"] = json!([
            {
                "id": "run-q",
                "ordinal": 1,
                "userMessageId": "cmd-q",
                "status": "queued",
                "requestedAt": "2026-11-28T10:00:00Z",
                "startedAt": null,
                "completedAt": null
            }
        ]);
        let thread = decode_thread(proj, &session()).unwrap();
        assert_eq!(thread.state, SessionState::Running);
        assert_eq!(thread.active_turn, None);
        assert_eq!(thread.recent_turns.len(), 1);
        assert_eq!(thread.recent_turns[0].state, TurnState::Queued);
    }

    #[test]
    fn blank_opening_input_normalizes_to_unavailable() {
        let mut proj = base_projection();
        proj["messages"][1]["text"] = json!("   \n\t  ");
        let thread = decode_thread(proj, &session()).unwrap();
        assert_eq!(
            thread.active_turn.unwrap().opening_input,
            OpeningInput::Unavailable
        );
    }

    #[test]
    fn cancelled_queued_run_with_null_started_at_stays_accepted_in_observe_input() {
        let mut proj = base_projection();
        proj["runs"] = json!([
            {
                "id": "run-c",
                "ordinal": 1,
                "userMessageId": "cmd-c",
                "status": "cancelled",
                "requestedAt": "2026-11-28T10:00:00Z",
                "startedAt": null,
                "completedAt": "2026-11-28T10:00:05Z"
            }
        ]);
        proj["messages"] = json!([
            {"id": "cmd-c", "role": "user", "text": "hello", "runId": "run-c"}
        ]);
        let now: Timestamp = "2026-11-28T10:01:00Z".parse().unwrap();
        let obs = decode_input_observation(proj, &session(), "hello", now).unwrap();
        assert_eq!(obs.state, InputState::Accepted);
    }

    #[test]
    fn observe_send_restricts_text_match_strictly_to_command_id_none() {
        let mut proj = base_projection();
        proj["runs"] = json!([
            {
                "id": "run-1",
                "ordinal": 1,
                "userMessageId": "other-id",
                "status": "completed",
                "requestedAt": "2026-11-28T09:50:00Z",
                "startedAt": "2026-11-28T09:50:01Z",
                "completedAt": "2026-11-28T09:51:00Z"
            }
        ]);
        proj["messages"] = json!([
            {"id": "other-id", "role": "user", "text": "same text", "runId": "run-1"}
        ]);
        // With command_id = Some("expected-cmd"), matching text on a different message id is NOT Accepted.
        assert_eq!(
            decode_send_evidence(proj.clone(), &session(), "same text", Some("expected-cmd"))
                .unwrap(),
            SendEvidence::IdleWithoutInput
        );
        // With command_id = None, matching text IS Accepted.
        assert_eq!(
            decode_send_evidence(proj, &session(), "same text", None).unwrap(),
            SendEvidence::Accepted
        );
    }

    #[test]
    fn unknown_status_is_an_observation_error_never_gone() {
        let mut proj = base_projection();
        proj["thread"]["status"] = json!("brand_new_status");
        assert!(matches!(
            decode_thread(proj, &session()),
            Err(RuntimeError::Observation(_))
        ));
    }
}
