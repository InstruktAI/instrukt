//! Automatic review enablement (`layered-configuration`, "Automatic review
//! enablement"): the findings at `review.automatic` that need no GitHub.

use std::path::Path;

use serde_yaml_ng::Value;

use super::_effective::effective_value;
use super::_error::{ConfigError, FindingKind};
use super::_schema::{DEFAULT_GITHUB_API_URL, KeyFinding};
use super::_validate::events_on;
use crate::home::Home;
use crate::ids::ProjectSlug;
use crate::index::{GlobalIndex, IndexEntry, IndexScope};
use crate::subscription::{SubscriptionHome, subscriber_patterns};

/// The key that opts a project in.
pub const REVIEW_AUTOMATIC_KEY: &str = "review.automatic";

/// The event type that asks for a review (`pull-request-review`, "Review
/// requests").
pub const REVIEW_NEEDED_EVENT: &str = "runner.review_needed";

/// The procedure that answers a review request.
pub const REVIEW_PROCEDURE: &str = "sdlc/procedure/lifecycle/work/review-commits";

/// The role whose App publishes the verdict.
pub const REVIEW_ROLE: &str = "commit-reviewer";

/// Whether the layer document `value` switches automatic review on.
pub(crate) fn review_on(value: &Value) -> bool {
    value
        .get("review")
        .and_then(|review| review.get("automatic"))
        .and_then(Value::as_bool)
        == Some(true)
}

/// Whether a patch from `before` to `after` switches automatic review on.
pub(crate) fn switches_on(before: &Value, after: &Value) -> bool {
    !review_on(before) && review_on(after)
}

/// The GitHub repository of the project layer `value` rooted at `root`,
/// judged with the machine's `global` layer; none when its code is not on
/// GitHub.
pub(crate) fn repository(
    root: &Path,
    value: &Value,
    global: Option<&Value>,
) -> Result<Option<String>, ConfigError> {
    let configured = value
        .get("github")
        .and_then(|g| g.get("repository"))
        .and_then(Value::as_str);
    match crate::github::resolve_code_host(configured, api_url(global), root)? {
        crate::github::CodeHost::GitHub { repository } => Ok(Some(repository)),
        crate::github::CodeHost::Elsewhere { .. } => Ok(None),
    }
}

/// The GitHub API address, from the machine's own layer, never a
/// repository's.
fn api_url(global: Option<&Value>) -> &str {
    global
        .and_then(|g| g.get("github"))
        .and_then(|g| g.get("api_url"))
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_GITHUB_API_URL)
}

/// The findings at `review.automatic` of the project layer `value` rooted at
/// `root`, judged with the machine's `global` layer: none while the setting
/// is off.
pub(crate) fn findings(
    home: &Home,
    root: &Path,
    value: &Value,
    global: Option<&Value>,
) -> Result<Vec<KeyFinding>, ConfigError> {
    if !review_on(value) {
        return Ok(Vec::new());
    }
    let finding = |message: String| {
        KeyFinding::new(
            REVIEW_AUTOMATIC_KEY,
            FindingKind::AutomaticReviewNotReady,
            message,
        )
    };
    let mut found = Vec::new();
    if !events_on(&effective_value(global, Some(value))?) {
        found.push(finding(
            "automatic review asks for each review with an event, and the project's effective `modules.events` is off; switch event responses on first".to_owned(),
        ));
    }
    let configured = value
        .get("github")
        .and_then(|g| g.get("repository"))
        .and_then(Value::as_str);
    if let crate::github::CodeHost::Elsewhere { reason } =
        crate::github::resolve_code_host(configured, api_url(global), root)?
    {
        found.push(finding(format!(
            "automatic review publishes on GitHub, and the project's code is not there: {reason}"
        )));
    }
    let project = value
        .get("project")
        .and_then(Value::as_str)
        .and_then(|slug| ProjectSlug::parse(slug).ok());
    let route = match GlobalIndex::load_entries(home) {
        Ok(Some(index)) => routes_review(&index.entries, project.as_ref()),
        Ok(None) => Err("no documentation is published on this computer".to_owned()),
        Err(error) => Err(format!("the published index cannot be read: {error}")),
    };
    if let Err(reason) = route {
        found.push(finding(format!(
            "the published set does not route `{REVIEW_NEEDED_EVENT}` to `{REVIEW_PROCEDURE}` under role `{REVIEW_ROLE}` alone: {reason}; publish that procedure and no other subscriber to the type"
        )));
    }
    Ok(found)
}

/// Whether the active published `entries` that reach `project` route
/// `runner.review_needed` to the review procedure and to nothing else: why
/// not when they do not.
pub(crate) fn routes_review(
    entries: &[IndexEntry],
    project: Option<&ProjectSlug>,
) -> Result<(), String> {
    let subscribed: Vec<(&IndexEntry, Vec<crate::event_type::TypePattern>)> = entries
        .iter()
        .filter(|entry| !entry.subscriptions.is_empty() && entry.delivered_by.is_empty())
        .filter(|entry| entry.subscription_home == Some(SubscriptionHome::Project))
        .filter(|entry| entry.scope == IndexScope::Global || Some(&entry.source_project) == project)
        .filter_map(|entry| {
            subscriber_patterns(entry)
                .ok()
                .map(|patterns| (entry, patterns))
        })
        .filter(|(_, patterns)| patterns.iter().any(|p| p.matches(REVIEW_NEEDED_EVENT)))
        .collect();
    let [(entry, _)] = subscribed.as_slice() else {
        return Err(if subscribed.is_empty() {
            "no active procedure subscribes to it".to_owned()
        } else {
            format!(
                "{} procedures subscribe to it ({})",
                subscribed.len(),
                subscribed
                    .iter()
                    .map(|(entry, _)| entry.id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });
    };
    if entry.id.to_string() != REVIEW_PROCEDURE {
        return Err(format!("`{}` subscribes to it in its place", entry.id));
    }
    if !entry
        .subscriptions
        .iter()
        .any(|text| text == REVIEW_NEEDED_EVENT)
    {
        return Err("it subscribes by a pattern, not by the exact type".to_owned());
    }
    if entry.role.as_deref() != Some(REVIEW_ROLE) {
        return Err(format!(
            "it declares role `{}`",
            entry.role.as_deref().unwrap_or_default()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, role: &str, subscriptions: &[&str]) -> IndexEntry {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "description": "d",
            "type": "procedure",
            "scope": "global",
            "domain": "sdlc",
            "path": format!("{id}.md"),
            "source_project": "instrukt",
            "role": role,
            "subscriptions": subscriptions,
            "subscription_home": "project",
        }))
        .unwrap()
    }

    fn project() -> ProjectSlug {
        ProjectSlug::parse("demo").unwrap()
    }

    #[test]
    fn the_review_procedure_alone_by_the_exact_type_and_its_role_routes_review() {
        let entries = [entry(REVIEW_PROCEDURE, REVIEW_ROLE, &[REVIEW_NEEDED_EVENT])];
        assert_eq!(routes_review(&entries, Some(&project())), Ok(()));
    }

    #[test]
    fn only_a_patch_from_absent_or_off_to_on_switches_automatic_review_on() {
        let doc = |text: &str| serde_yaml_ng::from_str::<Value>(text).unwrap();
        let on = doc("review: {automatic: true}");
        assert!(switches_on(&doc("project: demo"), &on));
        assert!(switches_on(&doc("review: {automatic: false}"), &on));
        assert!(!switches_on(&on, &on));
    }

    #[test]
    fn a_pattern_that_merely_matches_the_type_does_not_route_review() {
        let entries = [entry(REVIEW_PROCEDURE, REVIEW_ROLE, &["runner.*"])];
        assert!(routes_review(&entries, Some(&project())).is_err());
    }

    #[test]
    fn another_procedure_in_its_place_does_not_route_review() {
        let entries = [entry(
            "sdlc/procedure/lifecycle/work/review-other",
            REVIEW_ROLE,
            &[REVIEW_NEEDED_EVENT],
        )];
        assert!(routes_review(&entries, Some(&project())).is_err());
    }

    #[test]
    fn the_review_procedure_under_another_role_does_not_route_review() {
        let entries = [entry(REVIEW_PROCEDURE, "builder", &[REVIEW_NEEDED_EVENT])];
        assert!(routes_review(&entries, Some(&project())).is_err());
    }

    #[test]
    fn a_second_subscriber_to_the_type_does_not_route_review() {
        let entries = [
            entry(REVIEW_PROCEDURE, REVIEW_ROLE, &[REVIEW_NEEDED_EVENT]),
            entry("sdlc/procedure/watch", "watcher", &["runner.*"]),
        ];
        assert!(routes_review(&entries, Some(&project())).is_err());
    }
}
