//! What a computer answers to an evaluation request
//! (`project/instrukt/spec/feature/work/eval-report`, "Computers in a
//! report"): per project, its outcomes per eval set and model across all of
//! its stamps under that set, each todo, review and findings report counted
//! once per set, with only the earliest and latest stamp time of each row,
//! the commits it records setting each set, and the limitations it found.
//! No stamp, artifact or conversation leaves the computer.

use std::collections::{BTreeMap, BTreeSet};

use instrukt_core::config::Env;
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use instrukt_runner::interfaces::EvaluationAnswers;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::_read::{Limitation, Reading, read};
use super::{ChangePoints, Model, Tally, change_points, model_of, tally};
use crate::evidence::StampRecord;
use crate::work::{Clock, TodoRecord, Work};

/// The one request and answer format this build knows.
pub const ANSWER_FORMAT: u32 = 1;

/// An evaluation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub format: u32,
    /// The start of the requested range.
    pub since: Option<Timestamp>,
}

/// A computer's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub format: u32,
    /// What limits this computer's whole answer.
    pub limitations: Vec<String>,
    /// Every project this computer knows, in its reading order.
    pub projects: Vec<ProjectAnswer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAnswer {
    pub project: ProjectSlug,
    pub limitations: Vec<String>,
    pub sets: Vec<SetAnswer>,
}

/// One eval set's outcomes in a project, across all of its stamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetAnswer {
    /// `None` for stamps that copied no eval fingerprint.
    pub fingerprint: Option<String>,
    /// Every timed change point this computer records for the set.
    pub changed_by: Vec<(ProjectSlug, CommitSha)>,
    /// Whether this computer recorded the set with a commit of unknown
    /// time, so it cannot say which commit set it.
    pub order_unknown: bool,
    pub models: Vec<ModelAnswer>,
}

/// One model's outcomes within a set, with its earliest and latest stamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelAnswer {
    pub model: Model,
    pub first: Timestamp,
    pub last: Timestamp,
    pub tally: Tally,
}

/// Which wording an answer's limitations take: local for this computer's
/// own report, public for an answer that leaves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wording {
    Local,
    Public,
}

/// What a computer answers to the request `request` as it arrived, from
/// what it read: a request of a format this computer does not know is
/// answered in the known format with that limitation.
pub fn answer(request: &Value, reading: &Reading, wording: Wording) -> Answer {
    match serde_json::from_value::<Request>(request.clone()) {
        Ok(request) if request.format == ANSWER_FORMAT => answer_known(&request, reading, wording),
        _ => Answer {
            format: ANSWER_FORMAT,
            limitations: vec![Words { wording }.computer(&Limitation::UnknownRequestFormat)],
            projects: Vec::new(),
        },
    }
}

/// What a computer answers to `request`, of the known format, from what it
/// read.
pub fn answer_known(request: &Request, reading: &Reading, wording: Wording) -> Answer {
    let mut words = Words { wording };
    let mut limitations = Vec::new();
    if let Err(error) = &reading.published {
        limitations.push(words.computer(&Limitation::PublicationUnreadable {
            error: error.to_string(),
        }));
    }
    let changes = match &reading.fingerprints {
        Ok(fingerprints) => change_points(fingerprints),
        Err(error) => {
            limitations.push(words.computer(&Limitation::FingerprintsUnreadable {
                error: error.to_string(),
            }));
            change_points(&[])
        }
    };
    if changes.untimed > 0 {
        limitations.push(words.computer(&Limitation::UntimedFingerprints {
            count: changes.untimed,
        }));
    }
    let projects = reading
        .projects
        .iter()
        .map(|read| {
            let mut sets: BTreeMap<Option<&str>, Vec<(&TodoRecord, &StampRecord)>> =
                BTreeMap::new();
            for record in &read.records {
                for stamp in &record.stamps {
                    if request.since.is_none_or(|since| stamp.at >= since) {
                        sets.entry(stamp.eval_fingerprint.as_deref())
                            .or_default()
                            .push((record, stamp));
                    }
                }
            }
            ProjectAnswer {
                project: read.project.clone(),
                limitations: read
                    .limitations
                    .iter()
                    .map(|limitation| words.project(&read.project, limitation))
                    .collect(),
                sets: sets
                    .into_iter()
                    .map(|(fingerprint, stamps)| set(fingerprint, &stamps, &changes))
                    .collect(),
            }
        })
        .collect();
    Answer {
        format: ANSWER_FORMAT,
        limitations,
        projects,
    }
}

/// One eval set's answer from all of its stamps in a project.
fn set(
    fingerprint: Option<&str>,
    stamps: &[(&TodoRecord, &StampRecord)],
    changes: &ChangePoints,
) -> SetAnswer {
    let mut by_model: BTreeMap<Model, Vec<(&TodoRecord, &StampRecord)>> = BTreeMap::new();
    for (record, stamp) in stamps {
        by_model
            .entry(model_of(stamp))
            .or_default()
            .push((record, stamp));
    }
    let models = by_model
        .into_iter()
        .filter_map(|(model, stamps)| {
            let first = stamps.iter().map(|(_, stamp)| stamp.at).min()?;
            let last = stamps.iter().map(|(_, stamp)| stamp.at).max()?;
            Some(ModelAnswer {
                model,
                first,
                last,
                tally: tally(&stamps),
            })
        })
        .collect();
    SetAnswer {
        fingerprint: fingerprint.map(str::to_owned),
        changed_by: fingerprint
            .and_then(|f| changes.by_fingerprint.get(f))
            .map(|points| {
                points
                    .iter()
                    .map(|(_, project, commit)| (project.clone(), commit.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        order_unknown: fingerprint.is_some_and(|f| changes.untimed_fingerprints.contains(f)),
        models,
    }
}

/// Words limitations; an answer leaving this computer logs the detail of
/// each where its public wording points.
struct Words {
    wording: Wording,
}

impl Words {
    fn computer(&mut self, limitation: &Limitation) -> String {
        match self.wording {
            Wording::Local => limitation.local(),
            Wording::Public => {
                tracing::warn!(
                    event = "eval_answer_limited",
                    cause = limitation.cause(),
                    scope = "computer",
                    detail = %limitation.log_detail()
                );
                limitation.public()
            }
        }
    }

    fn project(&mut self, project: &ProjectSlug, limitation: &Limitation) -> String {
        match self.wording {
            Wording::Local => limitation.local(),
            Wording::Public => {
                tracing::warn!(
                    event = "eval_answer_limited",
                    cause = limitation.cause(),
                    scope = "project",
                    project = %project,
                    detail = %limitation.log_detail()
                );
                limitation.public()
            }
        }
    }
}

/// An answer the requesting computer cannot read.
#[derive(Debug, thiserror::Error)]
pub enum Unreadable {
    #[error("the answer is not of a known form: {0}")]
    Form(String),
    #[error("the answer is of format {0}, which this computer does not know")]
    Format(u64),
    #[error("the answer holds {0} more than once")]
    Duplicate(String),
}

/// Reads an answer as it arrived: one of an unknown format, one that does
/// not decode, or one holding a project, an eval set within a project or a
/// model within a set more than once is unreadable.
pub fn decode(body: &str) -> Result<Answer, Unreadable> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| Unreadable::Form(error.to_string()))?;
    match value.get("format").and_then(Value::as_u64) {
        Some(format) if format == u64::from(ANSWER_FORMAT) => {}
        Some(format) => return Err(Unreadable::Format(format)),
        None => return Err(Unreadable::Form("no format".into())),
    }
    let answer: Answer =
        serde_json::from_value(value).map_err(|error| Unreadable::Form(error.to_string()))?;
    let mut projects = BTreeSet::new();
    for project in &answer.projects {
        if !projects.insert(&project.project) {
            return Err(Unreadable::Duplicate(format!(
                "project {}",
                project.project
            )));
        }
        let mut sets = BTreeSet::new();
        for set in &project.sets {
            if !sets.insert(&set.fingerprint) {
                return Err(Unreadable::Duplicate(format!(
                    "an eval set in project {}",
                    project.project
                )));
            }
            let mut models = BTreeSet::new();
            for model in &set.models {
                if !models.insert(&model.model) {
                    return Err(Unreadable::Duplicate(format!(
                        "a model in an eval set of project {}",
                        project.project
                    )));
                }
            }
        }
    }
    Ok(answer)
}

/// Work's answer to other computers' evaluation requests, hosted by the
/// runner: this home's reading, with no current project, in public
/// wording.
pub struct EvaluationAnswering {
    home: Home,
    env: Env,
    clock: Clock,
}

impl EvaluationAnswering {
    pub fn new(home: Home, env: Env, clock: Clock) -> Self {
        Self { home, env, clock }
    }
}

impl EvaluationAnswers for EvaluationAnswering {
    fn answer(&self, request: Value) -> Result<Value, serde_json::Error> {
        let work = Work::new(self.home.clone(), self.env.clone(), self.clock.clone());
        let reading = read(&self.home, &work, None);
        serde_json::to_value(answer(&request, &reading, Wording::Public))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use jiff::Timestamp;
    use serde_json::{Value, json};

    use instrukt_core::ids::{CommitSha, ProjectSlug, SessionRef, TodoSlug};
    use instrukt_core::publication::EvalFingerprint;
    use instrukt_runner::interfaces::{CatchUp, TodoKind};

    use super::{ANSWER_FORMAT, Answer, Wording, answer, decode};
    use crate::eval::_read::{Limitation, ProjectRead, Reading, Stored, Why};
    use crate::eval::Tally;
    use crate::evidence::{
        Actor, ContentDigest, ModelIdentity, ModelProvenance, PreparationReview, ReviewVerdict,
        StampFact, StampRecord,
    };
    use crate::work::{TodoId, TodoRecord};

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_second(1_900_000_000 + seconds).unwrap()
    }

    fn stamp(seconds: i64, report: &str, fingerprint: &str) -> StampRecord {
        StampRecord {
            source_comment: None,
            fact: StampFact::PreparationVerdict {
                phase: PreparationReview::PrdReview,
                verdict: ReviewVerdict::NeedsWork,
                target: ContentDigest::new("prd"),
                report: ContentDigest::new(report),
            },
            at: at(seconds),
            actor: Actor::Session(SessionRef::parse("agent-cli:s1").unwrap()),
            model: Some(ModelIdentity {
                model: "m1".into(),
                version: "1".into(),
            }),
            eval_fingerprint: Some(fingerprint.into()),
            role: None,
            human_involved: None,
            sequence: 0,
            model_provenance: ModelProvenance::Reported,
        }
    }

    fn record(fill: char, stamps: Vec<StampRecord>) -> TodoRecord {
        TodoRecord {
            schema: 1,
            id: TodoId::parse(&fill.to_string().repeat(32)).unwrap(),
            project: slug("demo"),
            slug: TodoSlug::parse("depth").unwrap(),
            prior_slugs: Vec::new(),
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

    fn slug(name: &str) -> ProjectSlug {
        ProjectSlug::parse(name).unwrap()
    }

    fn reading(projects: Vec<ProjectRead>, fingerprints: Vec<EvalFingerprint>) -> Reading {
        Reading {
            projects,
            published: Ok(()),
            fingerprints: Ok(fingerprints),
        }
    }

    fn project(name: &str, records: Vec<TodoRecord>, limitations: Vec<Limitation>) -> ProjectRead {
        ProjectRead {
            project: slug(name),
            records,
            limitations,
        }
    }

    fn request() -> Value {
        json!({"format": 1, "since": null})
    }

    #[test]
    fn a_restored_eval_set_is_counted_once_across_its_periods() {
        let records = vec![record(
            'a',
            vec![
                stamp(1, "r1", "A"),
                stamp(2, "r2", "B"),
                stamp(3, "r1", "A"),
            ],
        )];
        let answered = answer(
            &request(),
            &reading(vec![project("demo", records, Vec::new())], Vec::new()),
            Wording::Public,
        );
        let a = &answered.projects[0].sets[0];
        assert_eq!(a.fingerprint.as_deref(), Some("A"));
        assert_eq!(
            (a.models[0].first, a.models[0].last),
            (at(1), at(3)),
            "the set spans both periods"
        );
        assert_eq!(
            (
                a.models[0].tally.todos,
                a.models[0].tally.reviews,
                a.models[0].tally.findings,
                a.models[0].tally.rounds
            ),
            (1, 1, 1, 2),
            "one todo, review and findings report, two rounds"
        );
    }

    #[test]
    fn an_answer_holds_only_outcome_numbers() {
        let fingerprints = vec![EvalFingerprint {
            fingerprint: "A".into(),
            snippets: 1,
            prose: None,
            units: None,
            project: slug("flows"),
            commit: CommitSha::parse(&"c".repeat(40)).unwrap(),
            recorded_at: Some(at(0)),
        }];
        let records = vec![record('a', vec![stamp(1, "r1", "A")])];
        let answered = answer(
            &request(),
            &reading(vec![project("demo", records, Vec::new())], fingerprints),
            Wording::Public,
        );
        let wire = serde_json::to_value(&answered).unwrap();
        let keys = |value: &Value| -> BTreeSet<String> {
            value.as_object().unwrap().keys().cloned().collect()
        };
        let named = |names: &[&str]| -> BTreeSet<String> {
            names.iter().map(|name| (*name).to_owned()).collect()
        };
        let project = &wire["projects"][0];
        let set = &project["sets"][0];
        let row = &set["models"][0];
        for (value, fields) in [
            (&wire, named(&["format", "limitations", "projects"])),
            (project, named(&["project", "limitations", "sets"])),
            (
                set,
                named(&["fingerprint", "changed_by", "order_unknown", "models"]),
            ),
            (row, named(&["model", "first", "last", "tally"])),
            (&row["model"], named(&["kind", "model", "version"])),
            (
                &row["tally"],
                named(&[
                    "todos",
                    "stamps",
                    "approved",
                    "not_approved",
                    "reviews",
                    "rounds",
                    "findings",
                    "returns",
                    "human",
                ]),
            ),
        ] {
            assert_eq!(keys(value), fields);
        }
    }

    #[test]
    fn a_missing_checkout_is_a_public_limitation_in_an_answer_and_local_in_a_report() {
        let missing = |path: &str| Limitation::Project {
            why: Why::CheckoutMissing {
                checkout: path.into(),
            },
            stored: Stored::Shown,
        };
        let read = |path: &str| {
            reading(
                vec![project("demo", Vec::new(), vec![missing(path)])],
                Vec::new(),
            )
        };
        let one = "/sentinel/checkout/one";
        let public = answer(&request(), &read(one), Wording::Public);
        assert_eq!(public.projects[0].limitations, vec![missing(one).public()]);
        let elsewhere = answer(&request(), &read("/sentinel/checkout/two"), Wording::Public);
        assert_eq!(
            serde_json::to_value(&public).unwrap(),
            serde_json::to_value(&elsewhere).unwrap(),
            "the answer carries nothing of the checkout's path"
        );
        let local = answer(&request(), &read(one), Wording::Local);
        assert_eq!(local.projects[0].limitations, vec![missing(one).local()]);
    }

    #[test]
    fn a_known_project_with_no_stamps_in_range_is_answered_with_no_sets() {
        let records = vec![record('a', vec![stamp(1, "r1", "A")])];
        let answered = answer(
            &json!({"format": 1, "since": at(10)}),
            &reading(
                vec![
                    project("demo", records, Vec::new()),
                    project("other", Vec::new(), Vec::new()),
                ],
                Vec::new(),
            ),
            Wording::Public,
        );
        let shape: Vec<(&str, usize)> = answered
            .projects
            .iter()
            .map(|p| (p.project.as_str(), p.sets.len()))
            .collect();
        assert_eq!(shape, vec![("demo", 0), ("other", 0)]);
    }

    #[test]
    fn an_unknown_request_format_is_answered_in_the_known_format_with_its_limitation() {
        let answered = answer(
            &json!({"format": 2}),
            &reading(Vec::new(), Vec::new()),
            Wording::Public,
        );
        assert_eq!(
            answered,
            Answer {
                format: ANSWER_FORMAT,
                limitations: vec![Limitation::UnknownRequestFormat.public()],
                projects: Vec::new(),
            }
        );
    }

    #[test]
    fn decode_refuses_duplicates_and_unknown_formats() {
        let set = json!({"fingerprint": "A", "changed_by": [], "order_unknown": false, "models": [
            {"model": {"kind": "human"}, "first": at(1), "last": at(1), "tally": Tally::default()}
        ]});
        let body = |projects: Value| {
            json!({"format": 1, "limitations": [], "projects": projects}).to_string()
        };
        let project = |sets: Value| json!({"project": "demo", "limitations": [], "sets": sets});
        assert!(decode(&body(json!([project(json!([set.clone()]))]))).is_ok());
        let mut doubled_model = set.clone();
        doubled_model["models"] = json!([set["models"][0].clone(), set["models"][0].clone()]);
        for refused in [
            body(json!([project(json!([])), project(json!([]))])),
            body(json!([project(json!([set.clone(), set.clone()]))])),
            body(json!([project(json!([doubled_model]))])),
            json!({"format": 2, "limitations": [], "projects": []}).to_string(),
            "not json".to_owned(),
        ] {
            assert!(decode(&refused).is_err(), "{refused}");
        }
    }
}
