//! How a record is shown and carried on GitHub (`github-record`, "Issues
//! and visible work" and "Writer and convergence"): the issue a person
//! reads, and the record envelope — a comment whose readable first line
//! names the revision and whose hidden line carries the committed record
//! with its artifact-version references and stamp ledger.

use instrukt_runner::interfaces::TodoKind;
use serde_json::{Value, json};

use super::_model::{TodoId, TodoRecord};
use crate::evidence::{Actor, TodoStatus, artifact_names};

/// The marker of the hidden record line.
const RECORD_MARKER: &str = "<!-- instrukt:work-record:v2:";
/// The marker naming a todo's identity in its issue body.
const ISSUE_MARKER: &str = "<!-- instrukt:todo:";
const COMMENT_CLOSE: &str = " -->";

/// The label every reflected todo carries.
pub(crate) const TODO_LABEL: &str = "instrukt";

/// The issue as the record shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IssueFields {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub closed: bool,
    pub state_reason: &'static str,
}

impl IssueFields {
    pub fn create_json(&self) -> Value {
        json!({"title": self.title, "body": self.body, "labels": self.labels})
    }

    pub fn update_json(&self) -> Value {
        json!({
            "title": self.title,
            "body": self.body,
            "labels": self.labels,
            "state": if self.closed { "closed" } else { "open" },
            "state_reason": self.state_reason,
        })
    }
}

/// The record's status as the issue and board show it: active, iceboxed,
/// held, delivered or withdrawn (a structural holder is active work),
/// never a workflow phase.
pub(crate) fn record_status(record: &TodoRecord) -> &'static str {
    match (record.status, &record.hold) {
        (TodoStatus::Delivered, _) => "delivered",
        (TodoStatus::Removed, _) => "withdrawn",
        (TodoStatus::Iceboxed, _) => "iceboxed",
        (_, Some(_)) => "held",
        (TodoStatus::Active | TodoStatus::StructuralHolder, None) => "active",
    }
}

/// The issue of `record`.
pub(crate) fn issue(record: &TodoRecord) -> IssueFields {
    let mut lines = vec![format!("{ISSUE_MARKER}{}{COMMENT_CLOSE}", record.id)];
    // The record's status, never a workflow phase.
    let status = record_status(record);
    lines.push(format!("**Kind:** {} · **Status:** {status}", record.kind));
    if let Some(hold) = &record.hold {
        lines.push(format!("**On hold:** {}", hold.reason));
    }
    for (gate, digest) in open_gates(record) {
        let shown = super::_inbound::shown(&digest);
        lines.push(format!(
            "**Awaiting approval:** {gate} `{shown}` — an approver approves it with `/instrukt approve {gate} {shown}`"
        ));
    }
    if let Some(group) = &record.group {
        lines.push(format!("**Group:** {}", group.slug));
    }
    if !record.after.is_empty() {
        let after: Vec<&str> = record.after.iter().map(|r| r.slug.as_str()).collect();
        lines.push(format!("**Prerequisites:** {}", after.join(", ")));
    }
    if let Some(delivery) = &record.delivery {
        lines.push(format!("**Delivered in:** {}", delivery.commit));
    }
    let intent = record
        .artifact(artifact_names::INPUT)
        .and_then(|a| a.versions.last())
        .and_then(|v| v.content.as_deref());
    if let Some(intent) = intent {
        lines.push(String::new());
        lines.push("## Intent".to_owned());
        lines.push(String::new());
        lines.push(intent.trim_end().to_owned());
    }
    let mut labels = vec![
        TODO_LABEL.to_owned(),
        format!("kind:{}", record.kind),
        format!("status:{}", status_name(record.status)),
    ];
    if record.hold.is_some() {
        labels.push("on-hold".to_owned());
    }
    let (closed, state_reason) = match record.status {
        TodoStatus::Delivered => (true, "completed"),
        TodoStatus::Removed => (true, "not_planned"),
        _ => (false, "reopened"),
    };
    IssueFields {
        title: record.slug.to_string(),
        body: lines.join("\n") + "\n",
        labels,
        closed,
        state_reason,
    }
}

/// The identity an issue body names, if it is a reflected todo's.
pub(crate) fn issue_identity(body: &str) -> Option<TodoId> {
    body.lines().find_map(|line| {
        line.trim()
            .strip_prefix(ISSUE_MARKER)?
            .strip_suffix(COMMENT_CLOSE)
            .and_then(TodoId::parse)
    })
}

/// One revision as GitHub carries it: the committed record, and the
/// revision it was built on. Revisions are appended, never edited; the
/// committed record is the chain of trusted envelopes each built on the
/// one before it (see `_reflect::committed`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Envelope {
    /// The revision this one replaces; 0 for the first.
    pub base: u64,
    pub record: TodoRecord,
}

/// The record comment of `record`, built on revision `base` (contents
/// stripped: artifact content is published as native artifacts).
pub(crate) fn record_comment(record: &TodoRecord, base: u64) -> Result<String, serde_json::Error> {
    let payload = serde_json::to_string(&Envelope {
        base,
        record: record.without_contents(),
    })?;
    let mut text = format!(
        "[instrukt record · {} · rev {} on rev {base}]\n",
        record.slug, record.revision
    );
    for stamp in &record.stamps {
        let fact = serde_json::to_value(&stamp.fact)?;
        text.push_str(&format!(
            "- {} {} by {}\n",
            stamp.at.strftime("%Y-%m-%dT%H:%M:%SZ"),
            fact["fact"].as_str().unwrap_or_default(),
            match &stamp.actor {
                Actor::Session(session) => format!("session {session}"),
                Actor::GitHub { login } => format!("@{login}"),
                Actor::Person { name } => name.clone(),
            }
        ));
    }
    text.push_str(&format!(
        "{RECORD_MARKER}{}{COMMENT_CLOSE}\n",
        hide(&payload)
    ));
    Ok(text)
}

/// The envelope a comment carries: `None` when it is no record comment,
/// `Some(Err)` when its record line does not parse. Content an envelope
/// supplies is dropped: evidence content comes only from verified
/// artifact versions.
pub(crate) fn parse_record_comment(
    body: &str,
) -> Option<Result<Envelope, super::_github::GitHubError>> {
    let line = body.lines().find(|line| {
        line.starts_with("<!-- instrukt:work-record:v") && line.ends_with(COMMENT_CLOSE)
    })?;
    let wire = &line["<!-- instrukt:work-record:v".len()..line.len() - COMMENT_CLOSE.len()];
    let (envelope_version, payload) = wire.split_once(':').unwrap_or((wire, ""));
    Some((|| {
        let invalid = |message: String| super::_github::GitHubError::RecordSchema { message };
        if envelope_version != "2" {
            return Err(invalid(format!(
                "unsupported GitHub record envelope version {envelope_version} (supported: 2); upgrade instrukt before acting on this todo"
            )));
        }
        let mut value: Value = serde_json::from_str(payload).map_err(|e| invalid(e.to_string()))?;
        let observed = value["record"]["schema"].as_u64().ok_or_else(|| {
            invalid(
                "record has no schema version; upgrade instrukt or restore a supported record"
                    .into(),
            )
        })?;
        let supported = u64::from(super::_model::SCHEMA);
        if !(1..=supported).contains(&observed) {
            return Err(invalid(format!(
                "todo `{}` has record schema {observed}; this binary supports 1..={supported}; upgrade instrukt before acting on this todo",
                value["record"]["slug"].as_str().unwrap_or("unknown")
            )));
        }
        // v1 lacks record-order numbers. Serde defaults and number_unsequenced
        // preserve retained evidence while assigning its existing order.
        if observed < 3
            && let Some(bindings) = value["record"]["bindings"].as_array_mut()
        {
            for binding in bindings {
                let role = binding
                    .get("role")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if let Some(placement) = binding
                    .get_mut("native_placement")
                    .filter(|v| v.is_object())
                    && placement.get("title").is_none_or(Value::is_null)
                {
                    let title = role.as_deref()
                        .zip(placement.get("branch").and_then(Value::as_str))
                        .zip(placement.get("key").and_then(Value::as_str))
                        .and_then(|((role, branch), key)| instrukt_runner::runtime::legacy_driver_title(role, branch, key))
                        .ok_or_else(|| invalid("legacy native placement lacks retained title or original role/phase evidence; restore that evidence before retrying".into()))?;
                    placement["title"] = title.into();
                }
            }
        }
        value["record"]["schema"] = supported.into();
        let mut envelope: Envelope =
            serde_json::from_value(value).map_err(|e| invalid(e.to_string()))?;
        envelope.record = envelope.record.without_contents();
        envelope.record.number_unsequenced();
        Ok(envelope)
    })())
}

/// Keeps JSON inside an HTML comment: `<`, `>` and `&` as JSON escapes.
fn hide(json: &str) -> String {
    json.replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
}

/// The artifact type an artifact uses: plans use the provider's plan
/// capability, everything else its document capability.
pub(crate) fn artifact_type(name: &str) -> &'static str {
    match name {
        artifact_names::IMPLEMENTATION_PLAN | artifact_names::TASK_PLAN => "plan",
        _ => "generic",
    }
}

/// The approval gates whose current evidence no approval binds yet: a
/// prd's, and a task's plan.
fn open_gates(record: &TodoRecord) -> Vec<(&'static str, crate::evidence::ContentDigest)> {
    use crate::evidence::{ApprovalGate, StampFact};
    let approved = |gate: ApprovalGate, digest: &crate::evidence::ContentDigest| {
        record.stamps.iter().any(|s| {
            matches!(&s.fact, StampFact::Approval { gate: g, target, .. } if *g == gate && target == digest)
        })
    };
    let mut open = Vec::new();
    if record.status != TodoStatus::Active {
        return open;
    }
    if let Some(prd) = record.current_digest(artifact_names::PRD)
        && !approved(ApprovalGate::Prd, prd)
    {
        open.push(("prd", prd.clone()));
    }
    if record.kind == TodoKind::Task
        && let Some(plan) = record.current_digest(artifact_names::TASK_PLAN)
        && !approved(ApprovalGate::Task, plan)
    {
        open.push(("task", plan.clone()));
    }
    open
}

pub(crate) fn status_name(status: TodoStatus) -> &'static str {
    match status {
        TodoStatus::Active => "active",
        TodoStatus::Iceboxed => "iceboxed",
        TodoStatus::StructuralHolder => "holder",
        TodoStatus::Delivered => "delivered",
        TodoStatus::Removed => "removed",
    }
}

/// Whether `kind` delivers repository work (and so has a pull request).
pub(crate) fn is_repository_work(kind: TodoKind) -> bool {
    !matches!(kind, TodoKind::Task)
}

#[cfg(test)]
mod tests {
    use super::*;
    use instrukt_core::ids::{ProjectSlug, TodoSlug};
    use instrukt_runner::interfaces::CatchUp;

    fn record() -> TodoRecord {
        TodoRecord {
            schema: super::super::_model::SCHEMA,
            id: TodoId::parse(&"ab".repeat(16)).unwrap(),
            project: ProjectSlug::parse("demo").unwrap(),
            slug: TodoSlug::parse("depth").unwrap(),
            prior_slugs: Vec::new(),
            kind: TodoKind::Bug,
            status: TodoStatus::Active,
            created: "2026-11-03T10:00:00Z".parse().unwrap(),
            created_by: Actor::Person {
                name: "<dev> & co".into(),
            },
            human_involved: None,
            revision: 3,
            sequence: 0,
            artifacts: Vec::new(),
            stamps: Vec::new(),
            claim: None,
            bindings: Vec::new(),
            hold: None,
            hold_history: Vec::new(),
            label_actions: Vec::new(),
            after: Vec::new(),
            group: None,
            schedule: None,
            catch_up: CatchUp::Once,
            agent: None,
            branch: None,
            delivery: None,
            removal: None,
        }
    }

    #[test]
    fn a_record_comment_round_trips_even_with_markup_in_its_text() {
        let record = record();
        let body = record_comment(&record, 2).unwrap();
        let parsed = parse_record_comment(&body).unwrap().unwrap();
        assert_eq!(parsed, Envelope { base: 2, record });
        assert!(parse_record_comment("a plain comment").is_none());
    }

    #[test]
    fn content_an_envelope_supplies_is_never_read_as_evidence() {
        let mut record = record();
        let forged = crate::work::_model::Version {
            digest: crate::work::_model::digest_of("real"),
            writer: Actor::Person { name: "x".into() },
            created: "2026-11-03T10:00:00Z".parse().unwrap(),
            derived_from: Vec::new(),
            sequence: 0,
            grounding: Default::default(),
            remote: Some(1),
            content: Some("forged".into()),
        };
        record.artifacts.push(crate::work::_model::Artifact {
            name: crate::evidence::ArtifactName::new("prd"),
            address: Some(1),
            versions: vec![forged],
        });
        let payload = serde_json::to_string(&Envelope { base: 0, record }).unwrap();
        let body = format!("{RECORD_MARKER}{payload}{COMMENT_CLOSE}");
        let parsed = parse_record_comment(&body).unwrap().unwrap();
        assert_eq!(parsed.record.artifacts[0].versions[0].content, None);
    }

    #[test]
    fn an_issue_body_names_the_identity_it_reflects() {
        let record = record();
        assert_eq!(issue_identity(&issue(&record).body), Some(record.id));
        assert_eq!(issue_identity("no marker"), None);
    }

    #[test]
    fn withdrawn_and_delivered_work_closes_its_issue_with_its_reason() {
        let mut record = record();
        record.status = TodoStatus::Removed;
        let removed = issue(&record);
        assert!(removed.closed);
        assert_eq!(removed.state_reason, "not_planned");
        record.status = TodoStatus::Delivered;
        assert_eq!(issue(&record).state_reason, "completed");
        record.status = TodoStatus::Active;
        assert!(!issue(&record).closed);
    }
}
