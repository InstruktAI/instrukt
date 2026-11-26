//! Pooling by eval set (`project/instrukt/spec/feature/work/eval-report`,
//! "Computers in a report"): a report covering any other computer shows,
//! within each project, one window per eval set holding the outcomes of
//! every answering computer, spanning the earliest to the latest stamp
//! under that set across them, with its measures added per model and in
//! the human row and the commits those computers record setting it. Its
//! windows are listed by earliest stamp and claim no sequence: no process
//! change and no restoration is named between them.

use std::collections::{BTreeMap, BTreeSet};

use instrukt_core::ids::{CommitSha, ProjectSlug};
use instrukt_core::machine::MachineId;
use jiff::Timestamp;

use super::_answer::{Answer, SetAnswer};
use super::{
    Model, ModelRow, ORDER_UNKNOWN, ProjectReport, Report, ReportComputer, Tally, UNFINGERPRINTED,
    Window, unknown_model,
};

/// The limitation of a pooled window whose setting commit no answering
/// computer records.
pub(super) const SETTING_UNKNOWN: &str = "No answering computer records a commit setting this eval set: the commit setting it is unknown.";

/// One computer's answer in a pooled report.
#[derive(Debug, Clone)]
pub struct Part {
    pub computer: MachineId,
    /// Whether it is this computer's own part.
    pub this: bool,
    pub answer: Answer,
}

/// What a pooled report is built from.
#[derive(Debug, Clone)]
pub struct Pooling<'a> {
    /// Every answer the report holds, this computer's first when it is
    /// covered.
    pub parts: &'a [Part],
    /// The project the report is asked from, listed first.
    pub current: Option<&'a ProjectSlug>,
    /// Limitations of the whole report, such as the range and the computers
    /// without an answer.
    pub limitations: Vec<String>,
    pub since: Option<Timestamp>,
    pub now: Timestamp,
}

/// A limitation `computer` reported, under its identity.
pub(super) fn under(computer: &MachineId, limitation: &str) -> String {
    format!("Computer {computer}: {limitation}")
}

/// The pooled report.
pub fn pool(pooling: Pooling<'_>) -> Report {
    let mut limitations = pooling.limitations;
    for part in pooling.parts {
        limitations.extend(
            part.answer
                .limitations
                .iter()
                .map(|limitation| under(&part.computer, limitation)),
        );
    }
    Report {
        generated: pooling.now,
        since: pooling.since,
        projects: order(pooling.parts, pooling.current)
            .into_iter()
            .map(|project| pooled_project(&project, pooling.parts))
            .collect(),
        limitations,
        computers: pooling
            .parts
            .iter()
            .map(|part| ReportComputer {
                computer: part.computer.clone(),
                this: part.this,
            })
            .collect(),
        pooled: true,
    }
}

/// The projects of every answer, each once: the current one first, then
/// this computer's in its reading order, then those only other computers
/// hold, by slug.
fn order(parts: &[Part], current: Option<&ProjectSlug>) -> Vec<ProjectSlug> {
    let mut known: BTreeSet<&ProjectSlug> = BTreeSet::new();
    for part in parts {
        known.extend(part.answer.projects.iter().map(|p| &p.project));
    }
    let mut ordered: Vec<ProjectSlug> = Vec::new();
    let push = |project: &ProjectSlug, ordered: &mut Vec<ProjectSlug>| {
        if !ordered.contains(project) {
            ordered.push(project.clone());
        }
    };
    if let Some(current) = current.filter(|current| known.contains(current)) {
        push(current, &mut ordered);
    }
    for part in parts.iter().filter(|part| part.this) {
        for project in &part.answer.projects {
            push(&project.project, &mut ordered);
        }
    }
    for project in known {
        push(project, &mut ordered);
    }
    ordered
}

/// One eval set's pooled window, while it is gathered.
#[derive(Default)]
struct Gathered {
    models: BTreeMap<Model, (Timestamp, Timestamp, Tally)>,
    changed_by: BTreeSet<(ProjectSlug, CommitSha)>,
    order_unknown: Vec<MachineId>,
}

fn pooled_project(project: &ProjectSlug, parts: &[Part]) -> ProjectReport {
    let mut limitations = Vec::new();
    let mut sets: BTreeMap<Option<String>, Gathered> = BTreeMap::new();
    for part in parts {
        let Some(answered) = part.answer.projects.iter().find(|p| p.project == *project) else {
            continue;
        };
        limitations.extend(
            answered
                .limitations
                .iter()
                .map(|limitation| under(&part.computer, limitation)),
        );
        for set in &answered.sets {
            gather(
                sets.entry(set.fingerprint.clone()).or_default(),
                &part.computer,
                set,
            );
        }
    }
    let mut windows: Vec<Window> = sets
        .into_iter()
        .filter_map(|(fingerprint, gathered)| window(fingerprint, gathered))
        .collect();
    windows.sort_by(|a, b| (a.first, &a.fingerprint).cmp(&(b.first, &b.fingerprint)));
    ProjectReport {
        project: project.clone(),
        windows,
        limitations,
    }
}

fn gather(gathered: &mut Gathered, computer: &MachineId, set: &SetAnswer) {
    for row in &set.models {
        let entry = gathered
            .models
            .entry(row.model.clone())
            .or_insert_with(|| (row.first, row.last, Tally::default()));
        entry.0 = entry.0.min(row.first);
        entry.1 = entry.1.max(row.last);
        entry.2 += &row.tally;
    }
    if set.order_unknown {
        gathered.order_unknown.push(computer.clone());
    } else {
        gathered.changed_by.extend(set.changed_by.iter().cloned());
    }
}

fn window(fingerprint: Option<String>, gathered: Gathered) -> Option<Window> {
    let first = gathered.models.values().map(|(first, _, _)| *first).min()?;
    let last = gathered.models.values().map(|(_, last, _)| *last).max()?;
    let models: Vec<ModelRow> = gathered
        .models
        .into_iter()
        .map(|(model, (_, _, tally))| ModelRow { model, tally })
        .collect();
    let changed_by: Vec<(ProjectSlug, CommitSha)> = gathered.changed_by.into_iter().collect();
    let mut limitations = Vec::new();
    match &fingerprint {
        None => limitations.push(UNFINGERPRINTED.to_owned()),
        Some(_) => {
            limitations.extend(
                gathered
                    .order_unknown
                    .iter()
                    .map(|computer| under(computer, ORDER_UNKNOWN)),
            );
            if changed_by.is_empty() {
                limitations.push(SETTING_UNKNOWN.to_owned());
            }
        }
    }
    limitations.extend(unknown_model(&models));
    Some(Window {
        fingerprint,
        first,
        last,
        changed_by,
        models,
        limitations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::_answer::{ANSWER_FORMAT, ModelAnswer, ProjectAnswer};

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_second(1_900_000_000 + seconds).unwrap()
    }

    fn id(fill: char) -> MachineId {
        MachineId::parse(&fill.to_string().repeat(16)).unwrap()
    }

    fn slug(name: &str) -> ProjectSlug {
        ProjectSlug::parse(name).unwrap()
    }

    fn commit(fill: char) -> (ProjectSlug, CommitSha) {
        (
            slug("flows"),
            CommitSha::parse(&fill.to_string().repeat(40)).unwrap(),
        )
    }

    fn tally(todos: usize) -> Tally {
        Tally {
            todos,
            stamps: todos * 2,
            approved: todos,
            not_approved: 1,
            reviews: todos,
            rounds: todos + 1,
            findings: todos,
            returns: 1,
            human: 1,
        }
    }

    fn row(model: Model, first: i64, last: i64, todos: usize) -> ModelAnswer {
        counted(model, first, last, tally(todos))
    }

    fn counted(model: Model, first: i64, last: i64, tally: Tally) -> ModelAnswer {
        ModelAnswer {
            model,
            first: at(first),
            last: at(last),
            tally,
        }
    }

    /// A tally of the nine measures in their declared order.
    fn measures(m: [usize; 9]) -> Tally {
        Tally {
            todos: m[0],
            stamps: m[1],
            approved: m[2],
            not_approved: m[3],
            reviews: m[4],
            rounds: m[5],
            findings: m[6],
            returns: m[7],
            human: m[8],
        }
    }

    fn reported(name: &str) -> Model {
        Model::Reported {
            model: name.into(),
            version: "1".into(),
        }
    }

    fn set(
        fingerprint: Option<&str>,
        changed_by: Vec<(ProjectSlug, CommitSha)>,
        order_unknown: bool,
        models: Vec<ModelAnswer>,
    ) -> SetAnswer {
        SetAnswer {
            fingerprint: fingerprint.map(str::to_owned),
            changed_by,
            order_unknown,
            models,
        }
    }

    fn part(computer: MachineId, this: bool, projects: Vec<ProjectAnswer>) -> Part {
        Part {
            computer,
            this,
            answer: Answer {
                format: ANSWER_FORMAT,
                limitations: Vec::new(),
                projects,
            },
        }
    }

    fn project(name: &str, limitations: &[&str], sets: Vec<SetAnswer>) -> ProjectAnswer {
        ProjectAnswer {
            project: slug(name),
            limitations: limitations.iter().map(|l| (*l).to_owned()).collect(),
            sets,
        }
    }

    fn pooled(parts: &[Part], current: Option<&ProjectSlug>) -> Report {
        pool(Pooling {
            parts,
            current,
            limitations: Vec::new(),
            since: None,
            now: at(1000),
        })
    }

    #[test]
    fn measures_add_per_model_and_in_the_human_row_and_span_every_computer() {
        let parts = [
            part(
                id('a'),
                true,
                vec![project(
                    "demo",
                    &[],
                    vec![set(
                        Some("A"),
                        vec![commit('1')],
                        false,
                        vec![
                            counted(
                                reported("m1"),
                                10,
                                20,
                                measures([1, 2, 1, 1, 1, 2, 1, 1, 0]),
                            ),
                            counted(Model::Human, 15, 15, measures([1, 1, 0, 0, 0, 0, 0, 1, 1])),
                        ],
                    )],
                )],
            ),
            part(
                id('b'),
                false,
                vec![project(
                    "demo",
                    &[],
                    vec![set(
                        Some("A"),
                        vec![commit('1'), commit('2')],
                        false,
                        vec![
                            counted(reported("m1"), 5, 12, measures([2, 5, 2, 3, 2, 5, 4, 2, 3])),
                            counted(Model::Human, 30, 30, measures([1, 2, 0, 0, 0, 0, 0, 2, 2])),
                        ],
                    )],
                )],
            ),
        ];
        let report = pooled(&parts, None);
        let window = &report.projects[0].windows[0];
        assert_eq!((window.first, window.last), (at(5), at(30)));
        assert_eq!(
            window.models,
            vec![
                ModelRow {
                    model: reported("m1"),
                    tally: measures([3, 7, 3, 4, 3, 7, 5, 3, 3]),
                },
                ModelRow {
                    model: Model::Human,
                    tally: measures([2, 3, 0, 0, 0, 0, 0, 3, 3]),
                },
            ]
        );
        assert_eq!(window.changed_by, vec![commit('1'), commit('2')]);
        assert!(window.limitations.is_empty());
        assert_eq!(
            report.computers,
            vec![
                ReportComputer {
                    computer: id('a'),
                    this: true,
                },
                ReportComputer {
                    computer: id('b'),
                    this: false,
                },
            ]
        );
    }

    #[test]
    fn an_order_unknown_computer_names_no_commit_and_its_limitation_stays_under_it() {
        let parts = [
            part(
                id('a'),
                true,
                vec![project(
                    "demo",
                    &[],
                    vec![set(
                        Some("A"),
                        vec![commit('1')],
                        true,
                        vec![row(reported("m1"), 1, 2, 1)],
                    )],
                )],
            ),
            part(
                id('b'),
                false,
                vec![project(
                    "demo",
                    &[],
                    vec![set(
                        Some("A"),
                        vec![commit('2')],
                        false,
                        vec![row(reported("m1"), 1, 2, 1)],
                    )],
                )],
            ),
        ];
        let window = &pooled(&parts, None).projects[0].windows[0];
        assert_eq!(window.changed_by, vec![commit('2')]);
        assert_eq!(window.limitations, vec![under(&id('a'), ORDER_UNKNOWN)]);
    }

    #[test]
    fn with_no_commit_named_by_anyone_the_setting_commit_is_unknown() {
        let parts = [part(
            id('a'),
            false,
            vec![project(
                "demo",
                &[],
                vec![set(
                    Some("A"),
                    Vec::new(),
                    true,
                    vec![row(reported("m1"), 1, 2, 1)],
                )],
            )],
        )];
        let window = &pooled(&parts, None).projects[0].windows[0];
        assert_eq!(
            window.limitations,
            vec![under(&id('a'), ORDER_UNKNOWN), SETTING_UNKNOWN.to_owned()]
        );
    }

    #[test]
    fn unfingerprinted_stamps_pool_into_one_window_with_the_unknown_model_count_recomputed() {
        let parts = [
            part(
                id('a'),
                true,
                vec![project(
                    "demo",
                    &[],
                    vec![set(
                        None,
                        Vec::new(),
                        false,
                        vec![row(Model::Unknown, 1, 2, 1)],
                    )],
                )],
            ),
            part(
                id('b'),
                false,
                vec![project(
                    "demo",
                    &[],
                    vec![set(
                        None,
                        Vec::new(),
                        false,
                        vec![row(Model::Unknown, 3, 4, 1)],
                    )],
                )],
            ),
        ];
        let report = pooled(&parts, None);
        assert_eq!(report.projects[0].windows.len(), 1);
        let window = &report.projects[0].windows[0];
        assert_eq!(
            window.limitations,
            vec![
                UNFINGERPRINTED.to_owned(),
                unknown_model(&window.models).unwrap()
            ]
        );
        assert_eq!(window.models[0].tally.stamps, 4);
    }

    #[test]
    fn projects_list_the_current_first_then_this_computers_then_the_others_by_slug() {
        let parts = [
            part(
                id('a'),
                true,
                vec![
                    project("mine", &[], Vec::new()),
                    project("demo", &[], Vec::new()),
                ],
            ),
            part(
                id('b'),
                false,
                vec![
                    project("zeta", &[], Vec::new()),
                    project("beta", &[], Vec::new()),
                    project("mine", &[], Vec::new()),
                ],
            ),
        ];
        let report = pooled(&parts, Some(&slug("demo")));
        let order: Vec<&str> = report.projects.iter().map(|p| p.project.as_str()).collect();
        assert_eq!(order, vec!["demo", "mine", "beta", "zeta"]);
    }

    #[test]
    fn an_answers_limitations_stay_under_its_computer() {
        let mut b = part(
            id('b'),
            false,
            vec![project(
                "demo",
                &["Its checkout is not available."],
                Vec::new(),
            )],
        );
        b.answer
            .limitations
            .push("Its fingerprints cannot be read.".into());
        let report = pooled(&[b], None);
        assert_eq!(
            report.limitations,
            vec![under(&id('b'), "Its fingerprints cannot be read.")]
        );
        assert_eq!(
            report.projects[0].limitations,
            vec![under(&id('b'), "Its checkout is not available.")]
        );
    }

    #[test]
    fn windows_are_ordered_by_earliest_stamp_then_fingerprint() {
        let parts = [part(
            id('a'),
            false,
            vec![project(
                "demo",
                &[],
                vec![
                    set(
                        Some("B"),
                        vec![commit('1')],
                        false,
                        vec![row(reported("m"), 5, 9, 1)],
                    ),
                    set(
                        Some("A"),
                        vec![commit('1')],
                        false,
                        vec![row(reported("m"), 7, 8, 1)],
                    ),
                    set(
                        Some("C"),
                        vec![commit('1')],
                        false,
                        vec![row(reported("m"), 5, 6, 1)],
                    ),
                ],
            )],
        )];
        let report = pooled(&parts, None);
        let order: Vec<Option<&str>> = report.projects[0]
            .windows
            .iter()
            .map(|w| w.fingerprint.as_deref())
            .collect();
        assert_eq!(order, vec![Some("B"), Some("C"), Some("A")]);
    }
}
