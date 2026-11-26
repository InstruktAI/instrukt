//! Which computers a report covers, and how it is made from their answers
//! (`project/instrukt/spec/feature/work/eval-report`, "Computers in a
//! report"): the whole computer fleet unless computers are named, naming
//! narrowing it to exactly those; every covered computer appears among
//! those whose answer the report holds or as a limitation with the report's
//! reason. The fleet query and its membership record are the runner's
//! (`instrukt_runner::computer_fleet`); the wording of each missing answer
//! is the report's.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use instrukt_core::config::Configuration;
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::machine::{self, MachineError, MachineId};
use instrukt_core::publication::{PublicationError, PublicationStateError};
use instrukt_runner::computer_fleet::{
    self, LastSeen, MAX_DEADLINE, Members, MembersUnread, Outcome, Unopened,
};
use jiff::Timestamp;

use super::_answer::{ANSWER_FORMAT, Request, Wording, answer_known, decode};
use super::_pool::{Part, Pooling, pool, under};
use super::_read::read;
use super::{Inputs, ProjectInput, Report, minute, outside_range, report};
use crate::work::Work;

/// What a report is asked for.
pub struct Asked<'a> {
    pub home: &'a Home,
    pub work: &'a Work,
    /// The project the report is asked from, and its checkout.
    pub current: (ProjectSlug, PathBuf),
    /// The computers named; none means the whole computer fleet.
    pub computers: &'a [MachineId],
    pub since: Option<Timestamp>,
    pub now: Timestamp,
}

/// What stops a report covering only this computer from being made, as
/// today's single-computer report refuses.
#[derive(Debug, thiserror::Error)]
pub enum Refused {
    #[error("cannot read which projects this home publishes")]
    Publication(#[source] PublicationStateError),
    #[error("cannot read the recorded eval fingerprints")]
    Fingerprints(#[source] PublicationError),
    #[error("cannot read this computer's identity")]
    Machine(#[source] MachineError),
}

const FLEET_UNREACHABLE: &str = "the fleet could not be reached";
const UNREADABLE: &str = "answering in a form it cannot read";

/// The reason a covered computer has no usable answer, as this report words
/// what the fleet query observed.
fn reason(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Answered(_) => UNREADABLE,
        Outcome::Unreachable => FLEET_UNREACHABLE,
        Outcome::NotDelivered => "the request could not be delivered",
        Outcome::StoreFailed => "the store failed while its answer was awaited",
        Outcome::NoAnswer => "answering after the wait ends",
    }
}

/// The report covering what `asked` names, or the whole computer fleet.
pub fn covering(asked: &Asked<'_>) -> Result<Report, Refused> {
    let named = !asked.computers.is_empty();
    // A configuration that cannot be read leaves the fleet unreachable, as
    // a connection that cannot be used does.
    let fleet = Configuration::load_global(asked.home)
        .map_err(|error| tracing::warn!(event = "eval_fleet_unusable", %error))
        .and_then(|config| match computer_fleet::open(&config, asked.home) {
            Ok(fleet) => Ok(Some(fleet)),
            Err(Unopened::NotConfigured) => Ok(None),
            Err(Unopened::Unusable(error)) => {
                tracing::warn!(event = "eval_fleet_unusable", %error);
                Err(())
            }
        });
    let fleet = match fleet {
        // No cluster connection configured: today's report, never touching
        // Redis.
        Ok(None) if !named => return single(asked, Vec::new()),
        Ok(None) => None,
        Ok(Some(fleet)) => Some(fleet),
        Err(()) if !named => return single(asked, vec![only_this(FLEET_UNREACHABLE)]),
        Err(()) => None,
    };
    let own = machine::machine_id(asked.home).map_err(Refused::Machine)?;
    let mut limitations = Vec::new();
    let mut seen: BTreeMap<MachineId, LastSeen> = BTreeMap::new();
    let mut covered: BTreeSet<MachineId> = asked.computers.iter().cloned().collect();
    // Whether the other covered computers can be asked at all.
    let mut reachable = fleet.is_some();
    // With only this computer named, no other computer is asked and the
    // record has nothing to add.
    let others_named = asked.computers.iter().any(|computer| *computer != own);
    if let Some(fleet) = fleet.as_ref().filter(|_| !named || others_named) {
        match fleet.members() {
            Ok(Members {
                members,
                unidentified,
            }) => {
                if unidentified > 0 {
                    limitations.push(format!(
                        "{unidentified} {} on the fleet membership record name{} no computer identity; nothing was asked of {}.",
                        if unidentified == 1 { "entry" } else { "entries" },
                        if unidentified == 1 { "s" } else { "" },
                        if unidentified == 1 { "it" } else { "them" },
                    ));
                }
                for member in members {
                    if !named {
                        covered.insert(member.computer.clone());
                    }
                    seen.insert(member.computer, member.last_seen);
                }
            }
            Err(MembersUnread::Unreachable) if !named => {
                return single(asked, vec![only_this(FLEET_UNREACHABLE)]);
            }
            Err(MembersUnread::Unreachable) => reachable = false,
            Err(MembersUnread::Refused) if !named => {
                return single(
                    asked,
                    vec![only_this("the fleet membership record could not be read")],
                );
            }
            Err(MembersUnread::Refused) => {
                limitations.push("The fleet membership record could not be read.".to_owned());
            }
        }
    }
    if !named {
        covered.insert(own.clone());
    }
    let others: Vec<MachineId> = covered.iter().filter(|c| **c != own).cloned().collect();
    if others.is_empty() {
        return single(asked, limitations);
    }
    if let Some(since) = asked.since {
        limitations.insert(0, outside_range(since));
    }
    let request = Request {
        format: ANSWER_FORMAT,
        since: asked.since,
    };
    // This computer's part is read while the others are awaited; the
    // deadline runs from the ask's start.
    let (own_part, observed) = std::thread::scope(|scope| {
        let own_part = covered.contains(&own).then(|| {
            scope.spawn(|| {
                let reading = read(asked.home, asked.work, Some(asked.current.clone()));
                answer_known(&request, &reading, Wording::Local)
            })
        });
        let observed: BTreeMap<MachineId, Outcome> = match fleet.as_ref().filter(|_| reachable) {
            Some(fleet) => fleet
                .ask(computer_fleet::EVALUATION, &others, &request, MAX_DEADLINE)
                .unwrap_or_else(|error| {
                    tracing::warn!(event = "eval_fleet_unasked", %error);
                    others
                        .iter()
                        .map(|computer| (computer.clone(), Outcome::NotDelivered))
                        .collect()
                }),
            None => others
                .iter()
                .map(|computer| (computer.clone(), Outcome::Unreachable))
                .collect(),
        };
        let own_part = own_part.map(|handle| handle.join());
        (own_part, observed)
    });
    let mut parts = Vec::new();
    match own_part {
        Some(Ok(answer)) => parts.push(Part {
            computer: own.clone(),
            this: true,
            answer,
        }),
        Some(Err(_)) => limitations.push(under(&own, "its own part could not be read.")),
        None => {}
    }
    for (computer, outcome) in observed {
        let unusable = match &outcome {
            Outcome::Answered(body) => match decode(body) {
                Ok(answer) => {
                    parts.push(Part {
                        computer,
                        this: false,
                        answer,
                    });
                    continue;
                }
                Err(error) => {
                    tracing::warn!(event = "eval_answer_unreadable", %computer, %error);
                    UNREADABLE
                }
            },
            other => reason(other),
        };
        let last_seen = match seen.get(&computer) {
            Some(LastSeen::At(at)) => format!("; last seen {}", minute(*at)),
            Some(LastSeen::Unknown) => "; last seen at an unknown time".to_owned(),
            None => String::new(),
        };
        limitations.push(under(&computer, &format!("{unusable}{last_seen}.")));
    }
    Ok(pool(Pooling {
        parts: &parts,
        current: Some(&asked.current.0),
        limitations,
        since: asked.since,
        now: asked.now,
    }))
}

/// A report-level limitation that leaves the report covering only this
/// computer.
fn only_this(reason: &str) -> String {
    let mut reason = reason.to_owned();
    if let Some(first) = reason.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    format!("{reason}, so this report covers only this computer.")
}

/// The report covering only this computer: today's report, read and
/// windowed as it always was, with `limitations` of its coverage added.
fn single(asked: &Asked<'_>, limitations: Vec<String>) -> Result<Report, Refused> {
    let reading = read(asked.home, asked.work, Some(asked.current.clone()));
    reading.published.map_err(Refused::Publication)?;
    let fingerprints = reading.fingerprints.map_err(Refused::Fingerprints)?;
    let projects: Vec<ProjectInput<'_>> = reading
        .projects
        .iter()
        .map(|read| ProjectInput {
            project: read.project.clone(),
            records: &read.records,
            limitations: read
                .limitations
                .iter()
                .map(super::Limitation::local)
                .collect(),
        })
        .collect();
    let mut made = report(Inputs {
        projects: &projects,
        fingerprints: &fingerprints,
        since: asked.since,
        now: asked.now,
    });
    made.limitations.extend(limitations);
    Ok(made)
}
