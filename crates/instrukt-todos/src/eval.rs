//! The evaluation report (`local/projects/instrukt/spec/feature/work/eval-report`):
//! delivery outcomes aggregated from the work record by project and
//! evaluation window, with the model a separate dimension inside each
//! window and human acts in a row of their own, rendered as HTML.
//!
//! Everything here is pure: the CLI hands in each project's records as
//! work makes them available, the per-commit fingerprint records
//! publication keeps, and the time range; it writes the HTML where the
//! developer asked. Nothing is invented: a missing observation is counted
//! as missing and named as a limitation of the comparison it affects.

use std::collections::{BTreeMap, BTreeSet};

use instrukt_core::ids::{CommitSha, ProjectSlug};
use instrukt_core::publication::EvalFingerprint;
use jiff::Timestamp;

use crate::evidence::{
    Actor, ApprovedBy, BuildVerdict, ModelProvenance, ReviewVerdict, StampFact, StampRecord,
};
use crate::work::TodoRecord;

/// One project as the report reads it.
#[derive(Debug, Clone)]
pub struct ProjectInput<'a> {
    pub project: ProjectSlug,
    /// The records available for it.
    pub records: &'a [TodoRecord],
    /// What limits its record: why it, or part of it, could not be read.
    pub limitations: Vec<String>,
}

/// What the report is built from.
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    /// Every project, in report order (the current one first).
    pub projects: &'a [ProjectInput<'a>],
    /// The fingerprint each published commit was recorded with.
    pub fingerprints: &'a [EvalFingerprint],
    /// The start of the history range, when one was asked for.
    pub since: Option<Timestamp>,
    /// When the report is made.
    pub now: Timestamp,
}

/// The evaluation report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub generated: Timestamp,
    pub since: Option<Timestamp>,
    /// Per project, in report order.
    pub projects: Vec<ProjectReport>,
    /// Limitations on every comparison in the report.
    pub limitations: Vec<String>,
}

impl Report {
    /// Whether no recorded work entered the report.
    pub fn is_empty(&self) -> bool {
        self.projects.iter().all(|p| p.windows.is_empty())
    }
}

/// One project's outcomes, window by window, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectReport {
    pub project: ProjectSlug,
    pub windows: Vec<Window>,
    pub limitations: Vec<String>,
}

/// One evaluation window: the stamps recorded under one eval fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// The fingerprint; `None` for stamps that copied none.
    pub fingerprint: Option<String>,
    /// The first and last stamp in the window.
    pub first: Timestamp,
    pub last: Timestamp,
    /// The published commits that changed the eval set to this one.
    pub changed_by: Vec<(ProjectSlug, CommitSha)>,
    /// Outcomes per model, in model order; human acts last.
    pub models: Vec<ModelRow>,
    /// Limitations on this window's comparison.
    pub limitations: Vec<String>,
}

/// Who produced a stamp: a model as recorded, or a human.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Model {
    Reported {
        model: String,
        version: String,
    },
    /// An agent session whose runtime could not say.
    Unknown,
    /// An agent act recorded outside an agent session.
    NoSession,
    /// A human act: outside the model dimension.
    Human,
}

/// One row's outcomes within a window (`eval-report`, "Outcome measures").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tally {
    /// Todos with a stamp in the window.
    pub todos: usize,
    pub stamps: usize,
    /// Review verdicts: approving, and not approving (`needs_work`,
    /// `changes_requested`) — counted here and nowhere else.
    pub approved: usize,
    pub not_approved: usize,
    /// Distinct reviews (todo and review phase) and their rounds, one per
    /// verdict.
    pub reviews: usize,
    pub rounds: usize,
    /// Distinct findings reports the verdicts bound.
    pub findings: usize,
    /// Kickbacks and approvers' requests for changes.
    pub returns: usize,
    /// Stamps marked human-involved, human approvals, change requests, and
    /// acts of a person or GitHub actor.
    pub human: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRow {
    pub model: Model,
    pub tally: Tally,
}

/// Builds the report from `inputs`.
pub fn report(inputs: Inputs<'_>) -> Report {
    let mut limitations = Vec::new();
    if let Some(since) = inputs.since {
        limitations.push(format!(
            "History before {} is outside the requested range.",
            day(since)
        ));
    }
    let changes = change_points(inputs.fingerprints);
    if changes.untimed > 0 {
        limitations.push(format!(
            "{} published commit{} recorded an eval fingerprint without its time; where {} in the order of process changes is unknown, so {} not named as changing the eval set.",
            changes.untimed,
            if changes.untimed == 1 { "" } else { "s" },
            if changes.untimed == 1 { "its place" } else { "their place" },
            if changes.untimed == 1 { "it is" } else { "they are" },
        ));
    }
    let projects = inputs
        .projects
        .iter()
        .map(|input| {
            let stamps: Vec<(&TodoRecord, &StampRecord)> = input
                .records
                .iter()
                .flat_map(|record| record.stamps.iter().map(move |stamp| (record, stamp)))
                .filter(|(_, stamp)| inputs.since.is_none_or(|since| stamp.at >= since))
                .collect();
            ProjectReport {
                project: input.project.clone(),
                windows: windows(&stamps, &changes),
                limitations: input.limitations.clone(),
            }
        })
        .collect();
    Report {
        generated: inputs.now,
        since: inputs.since,
        projects,
        limitations,
    }
}

/// Where the eval set changed, from the timed fingerprint records in time
/// order across every project: for each fingerprint, the commits whose
/// record differs from the one before it.
struct ChangePoints {
    by_fingerprint: BTreeMap<String, Vec<(ProjectSlug, CommitSha)>>,
    /// Fingerprints with at least one record of unknown time.
    untimed_fingerprints: BTreeSet<String>,
    untimed: usize,
}

fn change_points(fingerprints: &[EvalFingerprint]) -> ChangePoints {
    let mut timed: Vec<(&Timestamp, &EvalFingerprint)> = fingerprints
        .iter()
        .filter_map(|f| f.recorded_at.as_ref().map(|at| (at, f)))
        .collect();
    timed.sort_by(|a, b| (a.0, &a.1.commit).cmp(&(b.0, &b.1.commit)));
    let mut by_fingerprint: BTreeMap<String, Vec<(ProjectSlug, CommitSha)>> = BTreeMap::new();
    let mut previous: Option<&str> = None;
    for (_, record) in timed {
        if previous != Some(record.fingerprint.as_str()) {
            by_fingerprint
                .entry(record.fingerprint.clone())
                .or_default()
                .push((record.project.clone(), record.commit.clone()));
        }
        previous = Some(record.fingerprint.as_str());
    }
    let untimed: Vec<&EvalFingerprint> = fingerprints
        .iter()
        .filter(|f| f.recorded_at.is_none())
        .collect();
    ChangePoints {
        by_fingerprint,
        untimed_fingerprints: untimed.iter().map(|f| f.fingerprint.clone()).collect(),
        untimed: untimed.len(),
    }
}

/// The project's stamps grouped into windows, oldest first.
fn windows(stamps: &[(&TodoRecord, &StampRecord)], changes: &ChangePoints) -> Vec<Window> {
    let mut grouped: BTreeMap<Option<&str>, Vec<(&TodoRecord, &StampRecord)>> = BTreeMap::new();
    for (record, stamp) in stamps {
        grouped
            .entry(stamp.eval_fingerprint.as_deref())
            .or_default()
            .push((record, stamp));
    }
    let mut out: Vec<Window> = grouped
        .into_iter()
        .filter_map(|(fingerprint, stamps)| {
            let first = stamps.iter().map(|(_, s)| s.at).min()?;
            let last = stamps.iter().map(|(_, s)| s.at).max()?;
            let changed_by = fingerprint
                .and_then(|f| changes.by_fingerprint.get(f))
                .cloned()
                .unwrap_or_default();
            let models = rows(&stamps);
            let mut limitations = Vec::new();
            match fingerprint {
                None => limitations.push(
                    "These stamps copied no eval fingerprint: they were recorded before any publication, so their process-document set is unknown."
                        .to_owned(),
                ),
                Some(f) if changed_by.is_empty() && changes.untimed_fingerprints.contains(f) => {
                    limitations.push(
                        "The commits recorded with this eval set carry no recording time: which one changed it is unknown."
                            .to_owned(),
                    );
                }
                Some(_) if changed_by.is_empty() => limitations.push(
                    "No published commit on this computer is recorded with this eval set: the commit that changed it is unknown here."
                        .to_owned(),
                ),
                Some(_) => {}
            }
            let unknown: usize = models
                .iter()
                .filter(|row| row.model == Model::Unknown)
                .map(|row| row.tally.stamps)
                .sum();
            if unknown > 0 {
                limitations.push(format!(
                    "{unknown} stamp{} by an agent session whose runtime did not report its model.",
                    if unknown == 1 { "" } else { "s" }
                ));
            }
            Some(Window {
                fingerprint: fingerprint.map(str::to_owned),
                first,
                last,
                changed_by,
                models,
                limitations,
            })
        })
        .collect();
    out.sort_by(|a, b| (a.first, &a.fingerprint).cmp(&(b.first, &b.fingerprint)));
    out
}

/// The window's outcomes per model, and the human row.
fn rows(stamps: &[(&TodoRecord, &StampRecord)]) -> Vec<ModelRow> {
    let mut by_model: BTreeMap<Model, Vec<(&TodoRecord, &StampRecord)>> = BTreeMap::new();
    for (record, stamp) in stamps {
        by_model
            .entry(model_of(stamp))
            .or_default()
            .push((record, stamp));
    }
    by_model
        .into_iter()
        .map(|(model, stamps)| ModelRow {
            model,
            tally: tally(&stamps),
        })
        .collect()
}

fn model_of(stamp: &StampRecord) -> Model {
    if human_act(stamp) {
        return Model::Human;
    }
    match (&stamp.model, stamp.model_provenance) {
        (Some(identity), _) => Model::Reported {
            model: identity.model.clone(),
            version: identity.version.clone(),
        },
        (None, ModelProvenance::OutsideSession) => Model::NoSession,
        (None, _) => Model::Unknown,
    }
}

/// A human's own act: a human approval (whatever session stated it), an
/// approver's change request, or an act of a person or GitHub actor.
fn human_act(stamp: &StampRecord) -> bool {
    matches!(
        stamp.fact,
        StampFact::Approval {
            by: ApprovedBy::Human,
            ..
        } | StampFact::ChangesRequested { .. }
    ) || matches!(stamp.actor, Actor::Person { .. } | Actor::GitHub { .. })
}

fn tally(stamps: &[(&TodoRecord, &StampRecord)]) -> Tally {
    let mut todos = BTreeSet::new();
    let mut reviews = BTreeSet::new();
    let mut findings = BTreeSet::new();
    let mut tally = Tally {
        stamps: stamps.len(),
        ..Tally::default()
    };
    for (record, stamp) in stamps {
        todos.insert(&record.id);
        let verdict = match &stamp.fact {
            StampFact::PreparationVerdict {
                phase,
                verdict,
                report,
                ..
            } => Some((
                format!("{phase:?}"),
                *verdict == ReviewVerdict::Approve,
                report,
            )),
            StampFact::BuildVerdict {
                verdict, report, ..
            } => Some((
                "build".to_owned(),
                *verdict == BuildVerdict::Approve,
                report,
            )),
            _ => None,
        };
        if let Some((phase, approved, report)) = verdict {
            tally.rounds += 1;
            if approved {
                tally.approved += 1;
            } else {
                tally.not_approved += 1;
            }
            reviews.insert((&record.id, phase));
            findings.insert((&record.id, report));
        }
        if matches!(
            stamp.fact,
            StampFact::Kickback { .. } | StampFact::ChangesRequested { .. }
        ) {
            tally.returns += 1;
        }
        if stamp.human_involved == Some(true) || human_act(stamp) {
            tally.human += 1;
        }
    }
    tally.todos = todos.len();
    tally.reviews = reviews.len();
    tally.findings = findings.len();
    tally
}

fn day(at: Timestamp) -> String {
    at.strftime("%Y-%m-%d").to_string()
}

fn minute(at: Timestamp) -> String {
    at.strftime("%Y-%m-%d %H:%M UTC").to_string()
}

/// The report as a self-contained HTML page.
pub fn render_html(report: &Report) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "<h1>Evaluation report</h1>\n<p class=\"meta\">Generated {}{}.</p>\n",
        minute(report.generated),
        report
            .since
            .map(|since| format!(", from {}", day(since)))
            .unwrap_or_default()
    ));
    if report.is_empty() {
        body.push_str(
            "<p class=\"empty\">No recorded work: there is nothing to compare yet. Outcomes appear here once work passes through the record.</p>\n",
        );
    }
    body.push_str(&limits("Limitations", &report.limitations));
    for project in &report.projects {
        body.push_str(&format!(
            "<section class=\"project\">\n<h2>Project {}</h2>\n",
            escape(project.project.as_str())
        ));
        body.push_str(&limits("Limitations", &project.limitations));
        if project.windows.is_empty() {
            body.push_str("<p class=\"empty\">No recorded work.</p>\n");
        }
        let mut previous: Option<&Window> = None;
        for (number, window) in project.windows.iter().enumerate() {
            body.push_str(&render_window(number + 1, window, previous));
            previous = Some(window);
        }
        body.push_str("</section>\n");
    }
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>Evaluation report</title>\n<style>{STYLE}</style>\n</head>\n<body>\n{body}</body>\n</html>\n"
    )
}

/// A titled list of limitations; nothing when there are none.
fn limits(title: &str, limitations: &[String]) -> String {
    if limitations.is_empty() {
        return String::new();
    }
    let mut out = format!("<section class=\"limits\">\n<h3>{title}</h3>\n<ul>\n");
    for limitation in limitations {
        out.push_str(&format!("<li>{}</li>\n", escape(limitation)));
    }
    out.push_str("</ul>\n</section>\n");
    out
}

fn render_window(number: usize, window: &Window, previous: Option<&Window>) -> String {
    let mut out = format!(
        "<article class=\"window\">\n<h3>Window {number}: {}</h3>\n<p class=\"meta\">{} – {}</p>\n",
        match &window.fingerprint {
            Some(fingerprint) => format!("eval set <code>{}</code>", escape(fingerprint)),
            None => "no eval fingerprint".to_owned(),
        },
        minute(window.first),
        minute(window.last)
    );
    if previous.is_some_and(|previous| previous.fingerprint != window.fingerprint) {
        out.push_str(
            "<p class=\"change\">Process change: the eval set differs from the previous window's.</p>\n",
        );
    }
    if !window.changed_by.is_empty() {
        out.push_str("<p>Eval set changed to this one by:</p>\n<ul class=\"commits\">\n");
        for (project, commit) in &window.changed_by {
            out.push_str(&format!(
                "<li>{} <code>{}</code></li>\n",
                escape(project.as_str()),
                escape(commit.as_str())
            ));
        }
        out.push_str("</ul>\n");
    }
    if let Some(previous) = previous {
        let models = |w: &Window| -> BTreeSet<Model> {
            w.models
                .iter()
                .map(|r| r.model.clone())
                .filter(|m| *m != Model::Human)
                .collect()
        };
        if models(previous) != models(window) {
            out.push_str("<p class=\"change\">Model change: the models used differ from the previous window's.</p>\n");
        }
    }
    out.push_str(
        "<table>\n<thead><tr><th>Model</th><th>Version</th><th>Todos</th><th>Stamps</th>\
         <th>Approved</th><th>Not approved</th><th>Reviews</th><th>Rounds</th>\
         <th>Findings reports</th><th>Returns for repair</th><th>Human intervention</th></tr></thead>\n<tbody>\n",
    );
    for row in &window.models {
        let (model, version) = match &row.model {
            Model::Reported { model, version } => (escape(model), escape(version)),
            Model::Unknown => ("unknown".to_owned(), "unknown".to_owned()),
            Model::NoSession => ("no agent session".to_owned(), "—".to_owned()),
            Model::Human => ("human".to_owned(), "—".to_owned()),
        };
        let t = &row.tally;
        out.push_str(&format!(
            "<tr><td>{model}</td><td>{version}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td>\
             <td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            t.todos,
            t.stamps,
            t.approved,
            t.not_approved,
            t.reviews,
            t.rounds,
            t.findings,
            t.returns,
            t.human
        ));
    }
    out.push_str("</tbody>\n</table>\n");
    if !window.limitations.is_empty() {
        out.push_str("<ul class=\"limits\">\n");
        for limitation in &window.limitations {
            out.push_str(&format!("<li>{}</li>\n", escape(limitation)));
        }
        out.push_str("</ul>\n");
    }
    out.push_str("</article>\n");
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const STYLE: &str = "\
:root{color-scheme:light dark;--fg:#1d1d1f;--bg:#fff;--muted:#6e6e73;--line:#d2d2d7;--note:#8a5a00}\
@media (prefers-color-scheme:dark){:root{--fg:#f5f5f7;--bg:#1d1d1f;--muted:#a1a1a6;--line:#424245;--note:#ffcc66}}\
body{font:15px/1.5 system-ui,sans-serif;color:var(--fg);background:var(--bg);margin:0 auto;max-width:72rem;padding:1rem}\
table{border-collapse:collapse;width:100%;overflow-x:auto;display:block}\
th,td{border-bottom:1px solid var(--line);padding:.3rem .5rem;text-align:left;white-space:nowrap}\
.meta{color:var(--muted)}.change,.limits{color:var(--note)}code{font-size:.9em}";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{ApprovalGate, ContentDigest, ModelIdentity, PreparationReview};
    use crate::work::TodoId;
    use instrukt_core::ids::{SessionRef, TodoSlug};
    use instrukt_runner::interfaces::{CatchUp, TodoKind};

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_second(1_900_000_000 + seconds).unwrap()
    }

    fn stamp(seconds: i64, fact: StampFact, fingerprint: &str, model: Option<&str>) -> StampRecord {
        StampRecord {
            fact,
            at: at(seconds),
            actor: Actor::Session(SessionRef::parse("agent-cli:s1").unwrap()),
            model: model.map(|m| ModelIdentity {
                model: m.into(),
                version: "1".into(),
            }),
            eval_fingerprint: Some(fingerprint.into()),
            role: None,
            human_involved: None,
            sequence: 0,
            model_provenance: if model.is_some() {
                ModelProvenance::Reported
            } else {
                ModelProvenance::Unknown
            },
        }
    }

    fn verdict(verdict: ReviewVerdict, report: &str) -> StampFact {
        StampFact::PreparationVerdict {
            phase: PreparationReview::PrdReview,
            verdict,
            target: ContentDigest::new("prd"),
            report: ContentDigest::new(report),
        }
    }

    fn record(fill: char, stamps: Vec<StampRecord>) -> TodoRecord {
        TodoRecord {
            schema: 1,
            id: TodoId::parse(&fill.to_string().repeat(32)).unwrap(),
            project: demo(),
            slug: TodoSlug::parse("depth").unwrap(),
            kind: TodoKind::Feature,
            status: crate::evidence::TodoStatus::Active,
            created: at(0),
            created_by: Actor::Person { name: "dev".into() },
            human_involved: None,
            revision: 1,
            sequence: 0,
            artifacts: Vec::new(),
            stamps,
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

    fn demo() -> ProjectSlug {
        ProjectSlug::parse("demo").unwrap()
    }

    fn of(records: &[TodoRecord]) -> Vec<ProjectInput<'_>> {
        vec![ProjectInput {
            project: demo(),
            records,
            limitations: Vec::new(),
        }]
    }

    fn inputs<'a>(projects: &'a [ProjectInput<'a>]) -> Inputs<'a> {
        Inputs {
            projects,
            fingerprints: &[],
            since: None,
            now: at(100),
        }
    }

    fn published(fingerprint: &str, fill: char, recorded: Option<i64>) -> EvalFingerprint {
        EvalFingerprint {
            fingerprint: fingerprint.into(),
            snippets: 1,
            project: ProjectSlug::parse("flows").unwrap(),
            commit: CommitSha::parse(&fill.to_string().repeat(40)).unwrap(),
            recorded_at: recorded.map(at),
        }
    }

    fn reported(name: &str) -> Model {
        Model::Reported {
            model: name.into(),
            version: "1".into(),
        }
    }

    #[test]
    fn windows_follow_the_fingerprint_and_models_stay_a_separate_dimension() {
        let records = [record(
            'a',
            vec![
                stamp(1, verdict(ReviewVerdict::NeedsWork, "r1"), "f1", Some("m1")),
                stamp(2, verdict(ReviewVerdict::Approve, "r2"), "f2", Some("m1")),
                stamp(3, verdict(ReviewVerdict::Approve, "r3"), "f2", Some("m2")),
            ],
        )];
        let projects = of(&records);
        let report = report(inputs(&projects));
        let shape: Vec<(Option<&str>, Vec<&Model>)> = report.projects[0]
            .windows
            .iter()
            .map(|w| {
                (
                    w.fingerprint.as_deref(),
                    w.models.iter().map(|r| &r.model).collect(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                (Some("f1"), vec![&reported("m1")]),
                (Some("f2"), vec![&reported("m1"), &reported("m2")])
            ]
        );
    }

    #[test]
    fn a_needs_work_round_is_a_verdict_not_also_a_return_for_repair() {
        let mut kickback = stamp(
            4,
            StampFact::Kickback {
                reason: "scope".into(),
                input: ContentDigest::new("in"),
                invalidated_plan: None,
                invalidated_reviews: Vec::new(),
            },
            "f1",
            Some("m1"),
        );
        kickback.human_involved = Some(true);
        let records = [record(
            'a',
            vec![
                stamp(1, verdict(ReviewVerdict::NeedsWork, "r1"), "f1", Some("m1")),
                stamp(2, verdict(ReviewVerdict::Approve, "r2"), "f1", Some("m1")),
                kickback,
            ],
        )];
        let projects = of(&records);
        let tally = report(inputs(&projects)).projects[0].windows[0].models[0]
            .tally
            .clone();
        assert_eq!(
            tally,
            Tally {
                todos: 1,
                stamps: 3,
                approved: 1,
                not_approved: 1,
                reviews: 1,
                rounds: 2,
                findings: 2,
                returns: 1,
                human: 1,
            }
        );
    }

    #[test]
    fn a_human_approval_stated_in_a_session_is_in_the_human_row() {
        let approval = stamp(
            2,
            StampFact::Approval {
                gate: ApprovalGate::Prd,
                by: ApprovedBy::Human,
                target: ContentDigest::new("prd"),
                carried_from: None,
                reasoning: None,
            },
            "f1",
            None,
        );
        let records = [record(
            'a',
            vec![
                stamp(1, verdict(ReviewVerdict::Approve, "r1"), "f1", Some("m1")),
                approval,
            ],
        )];
        let projects = of(&records);
        // The eval set's change is known, so any limitation would be the
        // model's.
        let fingerprints = [published("f1", 'a', Some(0))];
        let window = report(Inputs {
            fingerprints: &fingerprints,
            ..inputs(&projects)
        })
        .projects[0]
            .windows[0]
            .clone();
        let models: Vec<Model> = window.models.iter().map(|r| r.model.clone()).collect();
        assert_eq!(models, vec![reported("m1"), Model::Human]);
        assert!(window.limitations.is_empty(), "no unknown model");
    }

    #[test]
    fn stamps_before_the_range_are_left_out() {
        let records = [record(
            'a',
            vec![
                stamp(1, verdict(ReviewVerdict::NeedsWork, "r1"), "f1", Some("m1")),
                stamp(5, verdict(ReviewVerdict::Approve, "r2"), "f1", Some("m1")),
            ],
        )];
        let projects = of(&records);
        let report = report(Inputs {
            since: Some(at(3)),
            ..inputs(&projects)
        });
        let window = &report.projects[0].windows[0];
        assert_eq!((window.models[0].tally.rounds, window.first), (1, at(5)));
    }

    #[test]
    fn no_recorded_work_is_an_empty_report_that_still_lists_each_project() {
        let projects = of(&[]);
        let report = report(inputs(&projects));
        assert!(report.is_empty());
        assert_eq!(report.projects.len(), 1);
    }

    #[test]
    fn only_the_commit_that_changed_the_eval_set_is_named() {
        let records = [record(
            'a',
            vec![
                stamp(1, verdict(ReviewVerdict::Approve, "r1"), "f1", Some("m1")),
                stamp(5, verdict(ReviewVerdict::Approve, "r2"), "f2", Some("m1")),
            ],
        )];
        let projects = of(&records);
        // f1, republished unchanged, then f2.
        let fingerprints = [
            published("f2", 'c', Some(4)),
            published("f1", 'a', Some(0)),
            published("f1", 'b', Some(2)),
        ];
        let report = report(Inputs {
            fingerprints: &fingerprints,
            ..inputs(&projects)
        });
        let named: Vec<Vec<CommitSha>> = report.projects[0]
            .windows
            .iter()
            .map(|w| w.changed_by.iter().map(|(_, c)| c.clone()).collect())
            .collect();
        assert_eq!(
            named,
            vec![
                vec![fingerprints[1].commit.clone()],
                vec![fingerprints[0].commit.clone()]
            ]
        );
    }

    #[test]
    fn a_record_of_unknown_time_names_no_commit_and_is_a_limitation() {
        let records = [record(
            'a',
            vec![stamp(
                1,
                verdict(ReviewVerdict::Approve, "r1"),
                "f1",
                Some("m1"),
            )],
        )];
        let projects = of(&records);
        let fingerprints = [published("f1", 'a', None)];
        let report = report(Inputs {
            fingerprints: &fingerprints,
            ..inputs(&projects)
        });
        let window = &report.projects[0].windows[0];
        assert!(window.changed_by.is_empty());
        assert_eq!((report.limitations.len(), window.limitations.len()), (1, 1));
    }
}
