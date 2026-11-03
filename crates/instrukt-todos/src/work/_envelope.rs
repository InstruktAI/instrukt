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
const RECORD_MARKER: &str = "<!-- instrukt:work-record:v1:";
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

/// The issue of `record`.
pub(crate) fn issue(record: &TodoRecord) -> IssueFields {
    let mut lines = vec![format!("{ISSUE_MARKER}{}{COMMENT_CLOSE}", record.id)];
    lines.push(format!(
        "**Kind:** {} · **Status:** {}",
        record.kind,
        status_name(record.status)
    ));
    if let Some(hold) = &record.hold {
        lines.push(format!("**On hold:** {}", hold.reason));
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

/// The record comment of `record` (contents stripped).
pub(crate) fn record_comment(record: &TodoRecord) -> Result<String, serde_json::Error> {
    let payload = serde_json::to_string(&record.without_contents())?;
    let mut text = format!(
        "[instrukt record · {} · rev {}]\n",
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

/// The record a comment carries: `None` when it is no record comment,
/// `Some(Err)` when its record line does not parse.
pub(crate) fn parse_record_comment(body: &str) -> Option<Result<TodoRecord, serde_json::Error>> {
    let line = body
        .lines()
        .find(|line| line.starts_with(RECORD_MARKER) && line.ends_with(COMMENT_CLOSE))?;
    let payload = &line[RECORD_MARKER.len()..line.len() - COMMENT_CLOSE.len()];
    Some(serde_json::from_str(payload))
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
            schema: 1,
            id: TodoId::parse(&"ab".repeat(16)).unwrap(),
            project: ProjectSlug::parse("demo").unwrap(),
            slug: TodoSlug::parse("depth").unwrap(),
            kind: TodoKind::Bug,
            status: TodoStatus::Active,
            created: "2026-11-03T10:00:00Z".parse().unwrap(),
            created_by: Actor::Person {
                name: "<dev> & co".into(),
            },
            human_involved: None,
            revision: 3,
            artifacts: Vec::new(),
            stamps: Vec::new(),
            claim: None,
            bindings: Vec::new(),
            hold: None,
            hold_history: Vec::new(),
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
        let body = record_comment(&record).unwrap();
        let parsed = parse_record_comment(&body).unwrap().unwrap();
        assert_eq!(parsed, record);
        assert!(parse_record_comment("a plain comment").is_none());
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
