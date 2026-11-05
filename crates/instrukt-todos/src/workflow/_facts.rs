//! The fact vocabulary: names and types the provider may supply
//! (`docs/project/spec/domain/workflow.md`, fact table), and the fact set an
//! evaluation reads.

use std::collections::BTreeMap;
use std::fmt;

use instrukt_core::ids::{CommitSha, SessionRef};

/// The members enum-typed facts take, named once: the provider supplies
/// them and its tests compare with them.
pub(crate) mod member {
    pub const FEATURE: &str = "feature";
    pub const CHANGE: &str = "change";
    pub const BUG: &str = "bug";
    pub const TASK: &str = "task";
    pub const APPROVE: &str = "approve";
    pub const NEEDS_WORK: &str = "needs_work";
    pub const FRESH: &str = "fresh";
    pub const PENDING: &str = "pending";
    pub const COSMETIC: &str = "cosmetic";
    pub const SUBSTANTIVE: &str = "substantive";
    pub const CHANGES_REQUESTED: &str = "changes_requested";
    pub const EXACT_TARGET: &str = "exact_target";
    pub const CLEAN_UPSTREAM_MERGES: &str = "clean_upstream_merges";
    pub const TODO_CHANGED: &str = "todo_changed";
    pub const REPORT_CHANGED: &str = "report_changed";
    pub const UNBOUND: &str = "unbound";
    pub const UNDETERMINED: &str = "undetermined";
    pub const READY: &str = "ready";
    pub const HUMAN: &str = "human";
    pub const PEER: &str = "peer";
}

/// Members of the `kind` fact.
const KINDS: &[&str] = &[member::FEATURE, member::CHANGE, member::BUG, member::TASK];
/// Members of a preparation review verdict.
const REVIEW_VERDICTS: &[&str] = &[member::APPROVE, member::NEEDS_WORK];
/// Members of a drift classification.
const DRIFT: &[&str] = &[
    member::FRESH,
    member::PENDING,
    member::COSMETIC,
    member::SUBSTANTIVE,
];
/// Members of a build-review verdict.
const BUILD_VERDICTS: &[&str] = &[member::APPROVE, member::CHANGES_REQUESTED];
/// Members of a build-review binding.
const BINDINGS: &[&str] = &[
    member::EXACT_TARGET,
    member::CLEAN_UPSTREAM_MERGES,
    member::TODO_CHANGED,
    member::REPORT_CHANGED,
    member::UNBOUND,
    member::UNDETERMINED,
];
/// Members of the finalize status.
const FINALIZE: &[&str] = &[member::PENDING, member::READY];
/// Members of a task approver.
const APPROVERS: &[&str] = &[member::HUMAN, member::PEER];

/// The type a fact's value has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FactType {
    Bool,
    /// A count; informational where the table says so.
    Int,
    /// Free text: digests, reasons, paths.
    Text,
    /// A full commit id.
    Sha,
    /// One of a closed set of members.
    Enum(&'static [&'static str]),
    /// A list of commit ids.
    ShaList,
    /// A list of runtime/id session bindings.
    SessionList,
}

impl fmt::Display for FactType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FactType::Bool => f.write_str("bool"),
            FactType::Int => f.write_str("int"),
            FactType::Text => f.write_str("string"),
            FactType::Sha => f.write_str("SHA"),
            FactType::Enum(members) => f.write_str(&members.join("|")),
            FactType::ShaList => f.write_str("list of SHA"),
            FactType::SessionList => f.write_str("list of runtime/id session bindings"),
        }
    }
}

/// The fact table, one row per name.
const CATALOG: &[(&str, FactType)] = &[
    ("kind", FactType::Enum(KINDS)),
    ("deps.delivered", FactType::Bool),
    ("prd.digest", FactType::Text),
    ("architecture.digest", FactType::Text),
    ("diagnosis.digest", FactType::Text),
    ("implementation_plan.digest", FactType::Text),
    ("implementation_plan.seeded", FactType::Bool),
    ("prd_review_findings.digest", FactType::Text),
    ("architecture_review_findings.digest", FactType::Text),
    ("plan_review_findings.digest", FactType::Text),
    ("prd_review.verdict", FactType::Enum(REVIEW_VERDICTS)),
    ("prd_review.target_digest", FactType::Text),
    ("prd_review.report_digest", FactType::Text),
    ("prd_review.cleared_by_substantive_drift", FactType::Bool),
    ("prd_review.rounds", FactType::Int),
    (
        "architecture_review.verdict",
        FactType::Enum(REVIEW_VERDICTS),
    ),
    ("architecture_review.target_digest", FactType::Text),
    ("architecture_review.report_digest", FactType::Text),
    (
        "architecture_review.cleared_by_substantive_drift",
        FactType::Bool,
    ),
    ("architecture_review.rounds", FactType::Int),
    ("plan_review.verdict", FactType::Enum(REVIEW_VERDICTS)),
    ("plan_review.target_digest", FactType::Text),
    ("plan_review.report_digest", FactType::Text),
    ("plan_review.cleared_by_substantive_drift", FactType::Bool),
    ("plan_review.rounds", FactType::Int),
    ("prd_approval.digest", FactType::Text),
    ("prd_approval.by_human", FactType::Bool),
    ("architecture_skipped.reason", FactType::Text),
    ("diagnosis_settled.digest", FactType::Text),
    ("claim_commits.unreviewed", FactType::ShaList),
    ("drift.prd", FactType::Enum(DRIFT)),
    ("drift.architecture", FactType::Enum(DRIFT)),
    ("drift.implementation_plan", FactType::Enum(DRIFT)),
    ("build.start_base_sha", FactType::Text),
    ("build.plan_digest", FactType::Text),
    ("build.complete", FactType::Bool),
    ("build_review.verdict", FactType::Enum(BUILD_VERDICTS)),
    ("build_review.target_digest", FactType::Sha),
    ("build_review.binding", FactType::Enum(BINDINGS)),
    ("build_review.round", FactType::Int),
    ("build_review.baseline_commit", FactType::Sha),
    ("deferrals.pending", FactType::Bool),
    ("finalize.status", FactType::Enum(FINALIZE)),
    ("finalize.sha", FactType::Sha),
    ("worktree.exists", FactType::Bool),
    ("worktree.path", FactType::Text),
    ("claim.held", FactType::Bool),
    ("sessions.bound", FactType::SessionList),
    ("task_plan.digest", FactType::Text),
    ("task_approved.target_digest", FactType::Text),
    ("task_approved.by", FactType::Enum(APPROVERS)),
    ("worktree.clean", FactType::Bool),
    ("worktree.head", FactType::Sha),
    ("branch.head", FactType::Sha),
    ("remote.head", FactType::Sha),
    ("quality_checklist.digest", FactType::Text),
    ("delivery.commit", FactType::Sha),
];

/// A name from the fact table. Only the table's names exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FactName {
    name: &'static str,
    ty: FactType,
}

impl FactName {
    /// The table's name for `name`, or `None` when the table has no such fact.
    pub fn parse(name: &str) -> Option<Self> {
        CATALOG
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(name, ty)| FactName { name, ty: *ty })
    }

    pub fn as_str(self) -> &'static str {
        self.name
    }

    /// The type the table gives this fact.
    pub fn fact_type(self) -> FactType {
        self.ty
    }

    /// Every fact the table declares, in table order.
    pub fn all() -> impl Iterator<Item = FactName> {
        CATALOG.iter().map(|(name, ty)| FactName { name, ty: *ty })
    }
}

impl fmt::Display for FactName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

/// A fact's value as the provider supplies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactValue {
    Bool(bool),
    Int(i64),
    /// Free text, or an enum member for an enum-typed fact.
    Text(String),
    Sha(CommitSha),
    ShaList(Vec<CommitSha>),
    /// The runtime/id session bindings `sessions.bound` lists.
    Sessions(Vec<SessionRef>),
}

impl FactValue {
    /// Whether this value is empty evidence, which the table treats as absent.
    fn is_empty(&self) -> bool {
        match self {
            FactValue::Text(text) => text.is_empty(),
            FactValue::ShaList(list) => list.is_empty(),
            FactValue::Sessions(list) => list.is_empty(),
            FactValue::Bool(_) | FactValue::Int(_) | FactValue::Sha(_) => false,
        }
    }

    /// The text of a textual value (free text, enum member, SHA).
    pub(crate) fn as_text(&self) -> Option<&str> {
        match self {
            FactValue::Text(text) => Some(text),
            FactValue::Sha(sha) => Some(sha.as_str()),
            _ => None,
        }
    }

    fn fits(&self, ty: FactType) -> bool {
        match (self, ty) {
            (FactValue::Bool(_), FactType::Bool)
            | (FactValue::Int(_), FactType::Int)
            | (FactValue::Text(_), FactType::Text)
            | (FactValue::Sha(_), FactType::Sha)
            | (FactValue::ShaList(_), FactType::ShaList)
            | (FactValue::Sessions(_), FactType::SessionList) => true,
            (FactValue::Text(member), FactType::Enum(members)) => {
                members.contains(&member.as_str())
            }
            _ => false,
        }
    }

    /// The value as placeholder text. Lists are space-separated, as the
    /// sdlc flavor's actions require for a SHA list argument.
    pub(crate) fn render(&self) -> String {
        match self {
            FactValue::Bool(value) => value.to_string(),
            FactValue::Int(value) => value.to_string(),
            FactValue::Text(text) => text.clone(),
            FactValue::Sha(sha) => sha.to_string(),
            FactValue::ShaList(list) => list
                .iter()
                .map(CommitSha::as_str)
                .collect::<Vec<_>>()
                .join(" "),
            FactValue::Sessions(list) => list
                .iter()
                .map(|session| format!("{}/{}", session.runtime, session.id))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

/// A fact could not be supplied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FactError {
    #[error("`{0}` is not a fact the workflow declares")]
    Unknown(String),
    #[error("fact `{name}` is {expected}, not {given:?}")]
    WrongType {
        name: FactName,
        expected: FactType,
        given: FactValue,
    },
}

/// The typed facts one evaluation reads. A name not supplied is absent; empty
/// text or an empty list is absent too, while `false` and `0` are values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    values: BTreeMap<FactName, FactValue>,
}

impl Facts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Supplies `name` with `value`, replacing an earlier value. Empty
    /// evidence makes the fact absent.
    pub fn insert(&mut self, name: &str, value: FactValue) -> Result<(), FactError> {
        let fact = FactName::parse(name).ok_or_else(|| FactError::Unknown(name.to_owned()))?;
        let expected = fact.fact_type();
        // Empty evidence is absent whatever the type, so an empty enum text
        // clears the fact rather than failing membership.
        let empty_text = matches!(&value, FactValue::Text(text) if text.is_empty())
            && matches!(expected, FactType::Text | FactType::Enum(_));
        if !empty_text && !value.fits(expected) {
            return Err(FactError::WrongType {
                name: fact,
                expected,
                given: value,
            });
        }
        if empty_text || value.is_empty() {
            self.values.remove(&fact);
        } else {
            self.values.insert(fact, value);
        }
        Ok(())
    }

    /// Builder form of [`Facts::insert`].
    pub fn with(mut self, name: &str, value: FactValue) -> Result<Self, FactError> {
        self.insert(name, value)?;
        Ok(self)
    }

    /// The present value of `name`, or `None` when absent.
    pub fn get(&self, name: FactName) -> Option<&FactValue> {
        self.values.get(&name)
    }

    /// The present facts, in name order.
    pub fn iter(&self) -> impl Iterator<Item = (FactName, &FactValue)> {
        self.values.iter().map(|(name, value)| (*name, value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_names_each_fact_once() {
        let mut names: Vec<_> = FactName::all().map(FactName::as_str).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn review_family_rows_expand_to_the_three_preparation_reviews() {
        for review in ["prd_review", "architecture_review", "plan_review"] {
            for field in [
                "verdict",
                "target_digest",
                "report_digest",
                "cleared_by_substantive_drift",
                "rounds",
            ] {
                assert!(FactName::parse(&format!("{review}.{field}")).is_some());
            }
        }
    }

    #[test]
    fn empty_evidence_is_absent_but_false_and_zero_are_values() {
        let facts = Facts::new()
            .with("prd.digest", FactValue::Text(String::new()))
            .unwrap()
            .with("claim_commits.unreviewed", FactValue::ShaList(vec![]))
            .unwrap()
            .with("build.complete", FactValue::Bool(false))
            .unwrap()
            .with("build_review.round", FactValue::Int(0))
            .unwrap();
        let get = |name| facts.get(FactName::parse(name).unwrap());
        assert_eq!(get("prd.digest"), None);
        assert_eq!(get("claim_commits.unreviewed"), None);
        assert_eq!(get("build.complete"), Some(&FactValue::Bool(false)));
        assert_eq!(get("build_review.round"), Some(&FactValue::Int(0)));
    }

    #[test]
    fn supplying_empty_evidence_removes_an_earlier_value() {
        let mut facts = Facts::new();
        facts
            .insert("prd.digest", FactValue::Text("d1".into()))
            .unwrap();
        facts
            .insert("prd.digest", FactValue::Text(String::new()))
            .unwrap();
        assert_eq!(facts.iter().count(), 0);
    }

    #[test]
    fn empty_enum_evidence_clears_an_earlier_member() {
        let verdict = FactName::parse("prd_review.verdict").unwrap();
        let mut facts = Facts::new();
        facts
            .insert("prd_review.verdict", FactValue::Text("approve".into()))
            .unwrap();
        assert_eq!(
            facts.insert("prd_review.verdict", FactValue::Text(String::new())),
            Ok(())
        );
        assert_eq!(facts.get(verdict), None);
    }

    #[test]
    fn empty_text_is_not_a_boolean() {
        assert!(matches!(
            Facts::new().insert("build.complete", FactValue::Text(String::new())),
            Err(FactError::WrongType { .. })
        ));
    }

    #[test]
    fn unknown_names_and_wrong_types_are_refused() {
        let mut facts = Facts::new();
        assert_eq!(
            facts.insert("prd.digst", FactValue::Text("d".into())),
            Err(FactError::Unknown("prd.digst".into()))
        );
        assert!(matches!(
            facts.insert("build.complete", FactValue::Text("true".into())),
            Err(FactError::WrongType { .. })
        ));
        assert!(matches!(
            facts.insert("kind", FactValue::Text("epic".into())),
            Err(FactError::WrongType { .. })
        ));
        assert!(facts.insert("kind", FactValue::Text("bug".into())).is_ok());
    }
}
