//! Translation of T3's normalized run failures; native diagnostic text is opaque.
use super::{CapacityOutcome, CapacityResult};
use crate::sessions::SessionsError;
use instrukt_core::ids::{ProviderInstanceId, SessionRef};
use jiff::Timestamp;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Projection {
    thread: Identity,
    runs: Vec<Run>,
    turn_items: Vec<Item>,
}

#[derive(Deserialize)]
struct Identity {
    id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RunStatus {
    Queued,
    Cancelled,
    Completed,
    Failed,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ItemKind {
    Error,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FailureClass {
    UsageLimit,
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Run {
    id: String,
    ordinal: u64,
    provider_instance_id: ProviderInstanceId,
    model_selection: Selection,
    status: RunStatus,
    started_at: Option<Timestamp>,
    completed_at: Option<Timestamp>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Selection {
    instance_id: ProviderInstanceId,
    model: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    #[serde(rename = "type")]
    kind: ItemKind,
    run_id: Option<String>,
    updated_at: Timestamp,
    failure: Option<Failure>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Failure {
    class: FailureClass,
    reset_at: Option<Timestamp>,
}

fn invalid() -> SessionsError {
    SessionsError::Refused("T3 execution capacity: invalid normalized run evidence".into())
}

pub(super) fn outcome(
    value: serde_json::Value,
    session: &SessionRef,
) -> Result<Option<CapacityOutcome>, SessionsError> {
    let projection: Projection = serde_json::from_value(value).map_err(|_| invalid())?;
    if projection.thread.id != session.id.as_str() {
        return Err(invalid());
    }
    let Some(run) = projection
        .runs
        .into_iter()
        .filter(|run| {
            run.status != RunStatus::Queued
                && !(run.status == RunStatus::Cancelled && run.started_at.is_none())
        })
        .max_by(|left, right| {
            match (left.completed_at, right.completed_at) {
                (Some(left), Some(right)) => left.cmp(&right),
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| left.ordinal.cmp(&right.ordinal))
        })
    else {
        return Ok(None);
    };
    if run.id.is_empty()
        || run.model_selection.model.trim().is_empty()
        || run.model_selection.instance_id != run.provider_instance_id
    {
        return Err(invalid());
    }
    let (at, result) = match run.status {
        RunStatus::Completed if run.completed_at.is_some() => {
            let Some(started) = run.started_at else {
                return Ok(None);
            };
            (started, CapacityResult::Succeeded)
        }
        RunStatus::Failed => {
            let Some(error) = projection
                .turn_items
                .into_iter()
                .filter(|item| {
                    item.kind == ItemKind::Error && item.run_id.as_deref() == Some(&run.id)
                })
                .max_by_key(|item| item.updated_at)
            else {
                return Ok(None);
            };
            let Some(failure) = error
                .failure
                .filter(|failure| failure.class == FailureClass::UsageLimit)
            else {
                return Ok(None);
            };
            (
                error.updated_at,
                CapacityResult::QuotaExhausted {
                    resets_at: failure.reset_at,
                },
            )
        }
        _ => return Ok(None),
    };
    Ok(Some(CapacityOutcome {
        run: run.id,
        instance: run.provider_instance_id,
        model: run.model_selection.model,
        at,
        result,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn projection(status: &str, class: &str) -> serde_json::Value {
        json!({
            "thread": {"id":"capacity-thread", "providerInstanceId":"codex"},
            "runs": [{
                "id":"quota-run", "ordinal":1, "providerInstanceId":"antigravity",
                "modelSelection":{"instanceId":"antigravity","model":"gemini-4-neon"},
                "status":status, "startedAt":"2026-11-28T14:00:00Z",
                "completedAt":"2026-11-28T14:01:00Z"
            }],
            "turnItems":[{
                "type":"error", "runId":"quota-run", "updatedAt":"2026-11-28T14:01:00Z",
                "failure":{"class":class,"message":"quota exhausted; resets in 999 hours", "resetAt":"2026-12-03T07:17:12Z"}
            }]
        })
    }

    #[test]
    fn quota_uses_the_failed_runs_selection_even_after_the_thread_switches() {
        let session = SessionRef::parse("t3:capacity-thread").unwrap();
        let observed = outcome(projection("failed", "usage_limit"), &session)
            .unwrap()
            .unwrap();
        assert_eq!(observed.instance.as_str(), "antigravity");
        assert_eq!(observed.model, "gemini-4-neon");
        assert_eq!(
            observed.result,
            CapacityResult::QuotaExhausted {
                resets_at: Some("2026-12-03T07:17:12Z".parse().unwrap()),
            }
        );
    }

    #[test]
    fn quota_diagnostics_in_other_failures_supply_no_capacity_evidence() {
        let session = SessionRef::parse("t3:capacity-thread").unwrap();
        assert!(
            outcome(projection("failed", "provider_error"), &session)
                .unwrap()
                .is_none()
        );
        assert!(
            outcome(projection("running", "usage_limit"), &session)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn success_is_attributed_to_when_the_successful_run_started() {
        let session = SessionRef::parse("t3:capacity-thread").unwrap();
        let mut value = projection("failed", "usage_limit");
        let mut succeeded = value["runs"][0].clone();
        succeeded["id"] = json!("success-run");
        succeeded["ordinal"] = json!(2);
        succeeded["status"] = json!("completed");
        succeeded["startedAt"] = json!("2026-11-28T14:02:00Z");
        succeeded["completedAt"] = json!("2026-11-28T14:03:00Z");
        value["runs"].as_array_mut().unwrap().push(succeeded);
        let observed = outcome(value, &session).unwrap().unwrap();
        assert_eq!(observed.result, CapacityResult::Succeeded);
        assert_eq!(
            observed.at,
            "2026-11-28T14:02:00Z".parse::<Timestamp>().unwrap()
        );
    }

    #[test]
    fn missing_reset_remains_unknown_and_does_not_parse_the_message() {
        let session = SessionRef::parse("t3:capacity-thread").unwrap();
        let mut value = projection("failed", "usage_limit");
        value["turnItems"][0]["failure"]
            .as_object_mut()
            .unwrap()
            .remove("resetAt");
        assert_eq!(
            outcome(value, &session).unwrap().unwrap().result,
            CapacityResult::QuotaExhausted { resets_at: None }
        );
    }

    #[test]
    fn queued_requests_do_not_hide_executed_quota_failures() {
        let session = SessionRef::parse("t3:capacity-thread").unwrap();
        let mut value = projection("failed", "usage_limit");
        let mut request = value["runs"][0].clone();
        request["id"] = json!("later-request");
        request["ordinal"] = json!(2);
        request["status"] = json!("queued");
        request["startedAt"] = json!(null);
        request["completedAt"] = json!(null);
        value["runs"].as_array_mut().unwrap().push(request);
        let observed = outcome(value, &session).unwrap().unwrap();
        assert_eq!(observed.run, "quota-run");
        assert!(matches!(
            observed.result,
            CapacityResult::QuotaExhausted { .. }
        ));
    }

    #[test]
    fn never_started_cancelled_requests_do_not_hide_executed_quota_failures() {
        let session = SessionRef::parse("t3:capacity-thread").unwrap();
        let mut value = projection("failed", "usage_limit");
        let mut request = value["runs"][0].clone();
        request["id"] = json!("later-request");
        request["ordinal"] = json!(2);
        request["status"] = json!("cancelled");
        request["startedAt"] = json!(null);
        request["completedAt"] = json!("2026-11-28T14:02:00Z");
        value["runs"].as_array_mut().unwrap().push(request);
        let observed = outcome(value, &session).unwrap().unwrap();
        assert_eq!(observed.run, "quota-run");
        assert!(matches!(
            observed.result,
            CapacityResult::QuotaExhausted { .. }
        ));
    }

    #[test]
    fn execution_completion_order_takes_precedence_over_request_ordinal() {
        let session = SessionRef::parse("t3:capacity-thread").unwrap();
        let mut value = projection("failed", "usage_limit");
        let mut completed = value["runs"][0].clone();
        completed["id"] = json!("earlier-execution");
        completed["ordinal"] = json!(2);
        completed["status"] = json!("completed");
        completed["startedAt"] = json!("2026-11-28T13:58:00Z");
        completed["completedAt"] = json!("2026-11-28T13:59:00Z");
        value["runs"].as_array_mut().unwrap().push(completed);
        assert_eq!(outcome(value, &session).unwrap().unwrap().run, "quota-run");
    }
}
