//! How a record is shown on GitHub (`github-record`, "Issues and visible
//! work"): the issue a person reads, its output labels and the artifact
//! type each artifact uses. The record itself is never carried there.

use instrukt_runner::interfaces::TodoKind;
use serde_json::{Value, json};

use super::_model::{TodoId, TodoRecord};
use crate::evidence::{TodoStatus, artifact_names};

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

/// A todo's record status as the issue and board show it: active,
/// iceboxed, held, delivered or withdrawn (a structural holder is active
/// work), never a workflow phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShownStatus {
    Active,
    Iceboxed,
    Held,
    Delivered,
    Withdrawn,
}

impl ShownStatus {
    /// The status shown for a todo in `status` with a hold set or not: a
    /// delivered, withdrawn or iceboxed todo shows that status whatever
    /// hold it retains; otherwise a hold shows as held.
    pub(crate) fn of(status: TodoStatus, held: bool) -> Self {
        match (status, held) {
            (TodoStatus::Delivered, _) => Self::Delivered,
            (TodoStatus::Removed, _) => Self::Withdrawn,
            (TodoStatus::Iceboxed, _) => Self::Iceboxed,
            (_, true) => Self::Held,
            (TodoStatus::Active | TodoStatus::StructuralHolder, false) => Self::Active,
        }
    }

    /// The word the issue body and the board's status field carry.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Iceboxed => "iceboxed",
            Self::Held => "held",
            Self::Delivered => "delivered",
            Self::Withdrawn => "withdrawn",
        }
    }
}

/// The record's status as the issue and board show it.
pub(crate) fn record_status(record: &TodoRecord) -> ShownStatus {
    ShownStatus::of(record.status, record.hold.is_some())
}

/// The issue of `record`.
pub(crate) fn issue(record: &TodoRecord) -> IssueFields {
    let mut lines = vec![format!("{ISSUE_MARKER}{}{COMMENT_CLOSE}", record.id)];
    // The record's status, never a workflow phase.
    let status = record_status(record).as_str();
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
        labels.push("hold".to_owned());
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
    use crate::evidence::Actor;
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
