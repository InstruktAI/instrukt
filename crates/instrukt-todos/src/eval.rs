//! The evaluation report (`local/projects/instrukt/spec/feature/work/eval-report`):
//! delivery outcomes aggregated from the work record by project and
//! evaluation window, with the model a separate dimension inside each
//! window, rendered as HTML.
//!
//! Everything here is pure: the CLI hands in the records work makes
//! available, the per-commit fingerprint records publication keeps, and the
//! time range; it writes the HTML where the developer asked. Nothing is
//! invented: a missing observation is counted as missing and named as a
//! limitation of the comparison it affects.

use std::collections::{BTreeMap, BTreeSet};

use instrukt_core::ids::{CommitSha, ProjectSlug};
use instrukt_core::publication::EvalFingerprint;
use jiff::Timestamp;

use crate::evidence::{
    Actor, ApprovedBy, BuildVerdict, ModelProvenance, ReviewVerdict, StampFact, StampRecord,
};
use crate::work::TodoRecord;

/// What the report is built from.
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    /// Every work record available.
    pub records: &'a [TodoRecord],
    /// The fingerprint each published commit was recorded with.
    pub fingerprints: &'a [EvalFingerprint],
    /// The start of the history range, when one was asked for.
    pub since: Option<Timestamp>,
    /// Why reflected work could not be read, when it could not.
    pub unread_github: Option<&'a str>,
    /// When the report is made.
    pub now: Timestamp,
}

/// The evaluation report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub generated: Timestamp,
    pub since: Option<Timestamp>,
    /// Per project, in project order.
    pub projects: Vec<ProjectReport>,
    /// Limitations on every comparison in the report.
    pub limitations: Vec<String>,
}

impl Report {
    /// Whether any recorded work entered the report.
    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }
}

/// One project's outcomes, window by window, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectReport {
    pub project: ProjectSlug,
    pub windows: Vec<Window>,
}

/// One evaluation window: the stamps recorded under one eval fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// The fingerprint; `None` for stamps that copied none.
    pub fingerprint: Option<String>,
    /// The first and last stamp in the window.
    pub first: Timestamp,
    pub last: Timestamp,
    /// The published commits recorded with this fingerprint: the commits
    /// that set this eval set.
    pub commits: Vec<(ProjectSlug, CommitSha)>,
    /// Outcomes per model, in model order.
    pub models: Vec<ModelRow>,
    /// Limitations on this window's comparison.
    pub limitations: Vec<String>,
}

/// The model a stamp was produced with, as recorded.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Model {
    Reported {
        model: String,
        version: String,
    },
    /// An agent session whose runtime could not say.
    Unknown,
    /// Recorded outside an agent session.
    NoSession,
}

/// One model's outcomes within a window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tally {
    /// Todos with a stamp in the window.
    pub todos: usize,
    pub stamps: usize,
    /// Review verdicts: approving, and returning the work for more.
    pub approved: usize,
    pub not_approved: usize,
    /// Distinct reviews (todo and review phase) and their verdict rounds.
    pub reviews: usize,
    pub rounds: usize,
    /// Distinct findings reports the verdicts bound.
    pub findings: usize,
    /// Kickbacks and approvers' requests for changes.
    pub returns: usize,
    /// Stamps a human made or declared involvement in.
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
    if let Some(cause) = inputs.unread_github {
        limitations.push(format!(
            "Reflected work on GitHub could not be read ({cause}); work recorded only on other computers is missing."
        ));
    }
    let mut by_project: BTreeMap<ProjectSlug, Vec<(&TodoRecord, &StampRecord)>> = BTreeMap::new();
    for record in inputs.records {
        for stamp in &record.stamps {
            if inputs.since.is_none_or(|since| stamp.at >= since) {
                by_project
                    .entry(record.project.clone())
                    .or_default()
                    .push((record, stamp));
            }
        }
    }
    let projects = by_project
        .into_iter()
        .map(|(project, stamps)| ProjectReport {
            windows: windows(&stamps, inputs.fingerprints),
            project,
        })
        .collect();
    Report {
        generated: inputs.now,
        since: inputs.since,
        projects,
        limitations,
    }
}

/// The project's stamps grouped into windows, oldest first.
fn windows(
    stamps: &[(&TodoRecord, &StampRecord)],
    fingerprints: &[EvalFingerprint],
) -> Vec<Window> {
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
            let commits: Vec<(ProjectSlug, CommitSha)> = fingerprints
                .iter()
                .filter(|f| Some(f.fingerprint.as_str()) == fingerprint)
                .map(|f| (f.project.clone(), f.commit.clone()))
                .collect();
            let models = rows(&stamps);
            let mut limitations = Vec::new();
            match fingerprint {
                None => limitations.push(
                    "These stamps copied no eval fingerprint: they were recorded before any publication, so their process-document set is unknown."
                        .to_owned(),
                ),
                Some(_) if commits.is_empty() => limitations.push(
                    "No published commit on this computer is recorded with this fingerprint: the commits that set this eval set are unknown here."
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
                commits,
                models,
                limitations,
            })
        })
        .collect();
    out.sort_by(|a, b| (a.first, &a.fingerprint).cmp(&(b.first, &b.fingerprint)));
    out
}

/// The window's outcomes per model.
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
    match (&stamp.model, stamp.model_provenance) {
        (Some(identity), _) => Model::Reported {
            model: identity.model.clone(),
            version: identity.version.clone(),
        },
        (None, ModelProvenance::OutsideSession) => Model::NoSession,
        (None, _) => Model::Unknown,
    }
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
        if human(stamp) {
            tally.human += 1;
        }
    }
    tally.todos = todos.len();
    tally.reviews = reviews.len();
    tally.findings = findings.len();
    tally
}

/// Whether a human acted in or declared involvement in `stamp`.
fn human(stamp: &StampRecord) -> bool {
    stamp.human_involved == Some(true)
        || matches!(
            stamp.fact,
            StampFact::Approval {
                by: ApprovedBy::Human,
                ..
            } | StampFact::ChangesRequested { .. }
        )
        || matches!(stamp.actor, Actor::Person { .. } | Actor::GitHub { .. })
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
    if !report.limitations.is_empty() {
        body.push_str("<section class=\"limits\">\n<h2>Limitations</h2>\n<ul>\n");
        for limitation in &report.limitations {
            body.push_str(&format!("<li>{}</li>\n", escape(limitation)));
        }
        body.push_str("</ul>\n</section>\n");
    }
    for project in &report.projects {
        body.push_str(&format!(
            "<section class=\"project\">\n<h2>Project {}</h2>\n",
            escape(project.project.as_str())
        ));
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
    if !window.commits.is_empty() {
        out.push_str(
            "<p>Published commits recorded with this eval set:</p>\n<ul class=\"commits\">\n",
        );
        for (project, commit) in &window.commits {
            out.push_str(&format!(
                "<li>{} <code>{}</code></li>\n",
                escape(project.as_str()),
                escape(commit.as_str())
            ));
        }
        out.push_str("</ul>\n");
    }
    if let Some(previous) = previous {
        let before: BTreeSet<&Model> = previous.models.iter().map(|r| &r.model).collect();
        let now: BTreeSet<&Model> = window.models.iter().map(|r| &r.model).collect();
        if before != now {
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
    use crate::evidence::{ContentDigest, ModelIdentity, PreparationReview};
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
            project: ProjectSlug::parse("demo").unwrap(),
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

    fn inputs(records: &[TodoRecord]) -> Inputs<'_> {
        Inputs {
            records,
            fingerprints: &[],
            since: None,
            unread_github: None,
            now: at(100),
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
        let report = report(inputs(&records));
        let windows = &report.projects[0].windows;
        let shape: Vec<(Option<&str>, Vec<&Model>)> = windows
            .iter()
            .map(|w| {
                (
                    w.fingerprint.as_deref(),
                    w.models.iter().map(|r| &r.model).collect(),
                )
            })
            .collect();
        let m = |name: &str| Model::Reported {
            model: name.into(),
            version: "1".into(),
        };
        assert_eq!(
            shape,
            vec![
                (Some("f1"), vec![&m("m1")]),
                (Some("f2"), vec![&m("m1"), &m("m2")])
            ]
        );
    }

    #[test]
    fn verdict_rounds_findings_and_returns_are_counted_per_review() {
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
        let tally = report(inputs(&records)).projects[0].windows[0].models[0]
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
    fn stamps_before_the_range_are_left_out() {
        let records = [record(
            'a',
            vec![
                stamp(1, verdict(ReviewVerdict::NeedsWork, "r1"), "f1", Some("m1")),
                stamp(5, verdict(ReviewVerdict::Approve, "r2"), "f1", Some("m1")),
            ],
        )];
        let report = report(Inputs {
            since: Some(at(3)),
            ..inputs(&records)
        });
        assert_eq!(report.projects[0].windows[0].models[0].tally.rounds, 1);
        assert_eq!(report.projects[0].windows[0].first, at(5));
    }

    #[test]
    fn no_recorded_work_is_an_empty_report() {
        assert!(report(inputs(&[])).is_empty());
    }

    #[test]
    fn a_window_names_the_commits_recorded_with_its_fingerprint() {
        let records = [record(
            'a',
            vec![stamp(1, verdict(ReviewVerdict::Approve, "r1"), "f1", None)],
        )];
        let commit = CommitSha::parse(&"c".repeat(40)).unwrap();
        let fingerprints = [EvalFingerprint {
            fingerprint: "f1".into(),
            snippets: 1,
            project: ProjectSlug::parse("flows").unwrap(),
            commit: commit.clone(),
        }];
        let window = &report(Inputs {
            fingerprints: &fingerprints,
            ..inputs(&records)
        })
        .projects[0]
            .windows[0];
        assert_eq!(
            (window.commits.clone(), window.models[0].model.clone()),
            (
                vec![(ProjectSlug::parse("flows").unwrap(), commit)],
                Model::Unknown
            )
        );
        assert_eq!(window.limitations.len(), 1, "the unknown model is named");
    }
}
