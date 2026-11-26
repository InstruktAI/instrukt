//! What one computer reads for an evaluation report: every project its
//! publication state knows, each project's record from this computer's work
//! record, and the per-commit fingerprint records publication keeps. What
//! limits a read is a typed cause with two wordings: the local one a report
//! on this computer shows, and the public one an answer to another computer
//! carries, which holds no URL, credential, path or error text
//! (`project/instrukt/spec/feature/work/eval-report`, "Computers in a
//! report").

use std::path::{Path, PathBuf};

use instrukt_core::config::{self, Module};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::publication::{
    EvalFingerprint, PublicationError, PublicationStateError, PublishedRecord,
};

use crate::work::{TodoRecord, Work};

/// Why a project's record, or part of it, could not be read through work's
/// normal read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// Its checkout is not on this computer.
    CheckoutMissing { checkout: PathBuf },
    /// Its checkout has no project configuration.
    NoConfiguration { checkout: PathBuf },
    /// It switched work tracking off.
    TrackingOff,
    /// Its configuration cannot be read.
    ConfigurationUnreadable { error: String },
    /// Its work record cannot be read.
    RecordUnreadable { error: String },
}

/// Whether a limited project's record on this computer could be read in
/// its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    /// It is shown in place of the normal read.
    Shown,
    /// It cannot be read either; [`Limitation::StoredUnreadable`] says why.
    Unread,
}

/// What limits a computer's reading, with the detail it was formed from.
/// Each has a local wording, a public wording and a log detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Limitation {
    /// A project's record is limited by `why`.
    Project { why: Why, stored: Stored },
    /// A limited project's record on this computer cannot be read either.
    StoredUnreadable { error: String },
    /// Which projects this computer publishes cannot be read.
    PublicationUnreadable { error: String },
    /// This computer's recorded eval fingerprints cannot be read.
    FingerprintsUnreadable { error: String },
    /// `count` published commits recorded an eval fingerprint without its
    /// time.
    UntimedFingerprints { count: usize },
    /// The request asked for a format this computer does not know.
    UnknownRequestFormat,
}

const STORED_SHOWN: &str =
    " Only its record on this computer is shown; work reflected only to GitHub is not read.";
const INSPECT: &str = " Inspect this computer's logs for the detail.";

impl Limitation {
    /// The cause's token, for logs.
    pub fn cause(&self) -> &'static str {
        match self {
            Self::Project { why, .. } => match why {
                Why::CheckoutMissing { .. } => "checkout_missing",
                Why::NoConfiguration { .. } => "no_configuration",
                Why::TrackingOff => "tracking_off",
                Why::ConfigurationUnreadable { .. } => "configuration_unreadable",
                Why::RecordUnreadable { .. } => "record_unreadable",
            },
            Self::StoredUnreadable { .. } => "stored_record_unreadable",
            Self::PublicationUnreadable { .. } => "publication_unreadable",
            Self::FingerprintsUnreadable { .. } => "fingerprints_unreadable",
            Self::UntimedFingerprints { .. } => "untimed_fingerprints",
            Self::UnknownRequestFormat => "unknown_request_format",
        }
    }

    /// The wording a report on this computer shows, paths and errors
    /// included.
    pub fn local(&self) -> String {
        match self {
            Self::Project { why, stored } => {
                let why = match why {
                    Why::CheckoutMissing { checkout } => {
                        format!("Its checkout at {} is not available.", checkout.display())
                    }
                    Why::NoConfiguration { checkout } => format!(
                        "Its checkout at {} has no project configuration, so whether it tracks work cannot be read.",
                        checkout.display()
                    ),
                    Why::TrackingOff => "Work tracking is switched off in this project now.".into(),
                    Why::ConfigurationUnreadable { error } => {
                        format!("Its configuration cannot be read: {error}.")
                    }
                    Why::RecordUnreadable { error } => {
                        format!("Its work record cannot be read: {error}.")
                    }
                };
                match stored {
                    Stored::Shown => format!("{why}{STORED_SHOWN}"),
                    Stored::Unread => why,
                }
            }
            Self::StoredUnreadable { error } => {
                format!("Its record on this computer cannot be read either: {error}.")
            }
            Self::PublicationUnreadable { error } => {
                format!("Which projects this home publishes cannot be read: {error}.")
            }
            Self::FingerprintsUnreadable { error } => {
                format!("The recorded eval fingerprints cannot be read: {error}.")
            }
            Self::UntimedFingerprints { .. } | Self::UnknownRequestFormat => self.public(),
        }
    }

    /// The wording an answer to another computer carries: what is limited
    /// and where to look, with no URL, credential, path or error text.
    pub fn public(&self) -> String {
        match self {
            Self::Project { why, stored } => {
                let (why, detail) = match why {
                    Why::CheckoutMissing { .. } => {
                        ("Its checkout is not available on this computer.", true)
                    }
                    Why::NoConfiguration { .. } => (
                        "Its checkout on this computer has no project configuration, so whether it tracks work cannot be read.",
                        true,
                    ),
                    Why::TrackingOff => {
                        ("Work tracking is switched off in this project now.", false)
                    }
                    Why::ConfigurationUnreadable { .. } => {
                        ("Its configuration cannot be read on this computer.", true)
                    }
                    Why::RecordUnreadable { .. } => {
                        ("Its work record cannot be read on this computer.", true)
                    }
                };
                format!(
                    "{why}{}{}",
                    match stored {
                        Stored::Shown => STORED_SHOWN,
                        Stored::Unread => "",
                    },
                    if detail { INSPECT } else { "" }
                )
            }
            Self::StoredUnreadable { .. } => {
                format!("Its record on this computer cannot be read either.{INSPECT}")
            }
            Self::PublicationUnreadable { .. } => format!(
                "Which projects this computer publishes cannot be read, so none of its projects is counted.{INSPECT}"
            ),
            Self::FingerprintsUnreadable { .. } => format!(
                "This computer's recorded eval fingerprints cannot be read, so it names no commit setting an eval set.{INSPECT}"
            ),
            Self::UntimedFingerprints { count } => untimed(*count),
            Self::UnknownRequestFormat => {
                "The request asked for a format this computer does not know.".into()
            }
        }
    }

    /// The detail the public wording points to, logged on this computer:
    /// the local wording with every URL credential redacted.
    pub fn log_detail(&self) -> String {
        redact_credentials(&self.local())
    }
}

/// The limitation that `count` published commits recorded an eval
/// fingerprint without its time.
pub(super) fn untimed(count: usize) -> String {
    format!(
        "{} published commit{} recorded an eval fingerprint without its time; where {} in the order of process changes is unknown, so {} not named as changing the eval set.",
        count,
        if count == 1 { "" } else { "s" },
        if count == 1 {
            "its place"
        } else {
            "their place"
        },
        if count == 1 { "it is" } else { "they are" },
    )
}

/// `text` with the user information of every URL in it replaced by `***`,
/// keeping scheme and host.
fn redact_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (before, after) = rest.split_at(at + 3);
        out.push_str(before);
        let authority_end = after
            .find(|c: char| matches!(c, '/' | '?' | '#') || c.is_whitespace())
            .unwrap_or(after.len());
        match after[..authority_end].rfind('@') {
            Some(userinfo_end) => {
                out.push_str("***");
                rest = &after[userinfo_end..];
            }
            None => rest = after,
        }
    }
    out.push_str(rest);
    out
}

/// One project's record as this computer reads it.
#[derive(Debug, Clone)]
pub struct ProjectRead {
    pub project: ProjectSlug,
    pub records: Vec<TodoRecord>,
    pub limitations: Vec<Limitation>,
}

/// Everything one computer reads for a report.
#[derive(Debug)]
pub struct Reading {
    /// Every project the publication state knows with something to
    /// evaluate, the current one first, then in publication order.
    pub projects: Vec<ProjectRead>,
    /// Whether the publication state could be read; when it could not, only
    /// the current project is read.
    pub published: Result<(), PublicationStateError>,
    /// The fingerprint each published commit was recorded with.
    pub fingerprints: Result<Vec<EvalFingerprint>, PublicationError>,
}

/// Reads this computer's part of a report: `current`, when given, is the
/// project the report is asked from and its checkout, read first.
pub fn read(home: &Home, work: &Work, current: Option<(ProjectSlug, PathBuf)>) -> Reading {
    let current_project = current.as_ref().map(|(slug, _)| slug.clone());
    let mut checkouts: Vec<(ProjectSlug, PathBuf)> = current.into_iter().collect();
    let published = PublishedRecord::load_all(home).map(|records| {
        for published in records {
            if current_project.as_ref() != Some(&published.project) {
                checkouts.push((published.project, published.checkout));
            }
        }
    });
    let projects = checkouts
        .into_iter()
        .filter_map(|(project, checkout)| {
            let (records, limitations) = project_record(home, work, &project, &checkout)?;
            Some(ProjectRead {
                project,
                records,
                limitations,
            })
        })
        .collect();
    Reading {
        projects,
        published,
        fingerprints: EvalFingerprint::recorded(home),
    }
}

/// The records of project `slug`, checked out at `checkout`, and what
/// limits them. A project that tracks work is read through work's normal
/// read. Otherwise its record on this computer is what remains: a project
/// whose checkout or configuration is not available is listed with that
/// limitation, and one that switched tracking off keeps its recorded
/// history. `None` only for a project that tracks no work and never did
/// here, which has nothing to evaluate.
fn project_record(
    home: &Home,
    work: &Work,
    slug: &ProjectSlug,
    checkout: &Path,
) -> Option<(Vec<TodoRecord>, Vec<Limitation>)> {
    // The record on this computer alone, limited by `why`.
    let stored = |why: Why| -> (Vec<TodoRecord>, Vec<Limitation>) {
        match work.stored_work(slug) {
            Ok(records) => (
                records,
                vec![Limitation::Project {
                    why,
                    stored: Stored::Shown,
                }],
            ),
            Err(err) => (
                Vec::new(),
                vec![
                    Limitation::Project {
                        why,
                        stored: Stored::Unread,
                    },
                    Limitation::StoredUnreadable {
                        error: err.to_string(),
                    },
                ],
            ),
        }
    };
    if !checkout.is_dir() {
        return Some(stored(Why::CheckoutMissing {
            checkout: checkout.to_path_buf(),
        }));
    }
    if !checkout.join("instrukt.yaml").is_file() {
        return Some(stored(Why::NoConfiguration {
            checkout: checkout.to_path_buf(),
        }));
    }
    match config::module_enabled(home, Some(checkout), Module::Todos) {
        Ok(true) => {}
        Ok(false) => {
            let (records, limitations) = stored(Why::TrackingOff);
            return (!records.is_empty() || limitations.len() > 1)
                .then_some((records, limitations));
        }
        Err(err) => {
            return Some(stored(Why::ConfigurationUnreadable {
                error: err.to_string(),
            }));
        }
    }
    let recorded = work
        .project_at(checkout)
        .and_then(|project| work.recorded_work(&project));
    Some(match recorded {
        Ok(recorded) => (recorded.records, Vec::new()),
        Err(err) => stored(Why::RecordUnreadable {
            error: err.to_string(),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::{Limitation, Stored, Why};
    use std::path::PathBuf;

    #[test]
    fn every_cause_has_a_public_wording_and_a_redacted_log_detail() {
        let path = || PathBuf::from("/sentinel/checkout/path");
        let error = || "sentinel error text".to_owned();
        let url_error = || {
            "remote https://sentinel-user:sentinel-secret@host.example/org/repo.git is elsewhere"
                .to_owned()
        };
        let mut causes = Vec::new();
        for stored in [Stored::Shown, Stored::Unread] {
            for why in [
                Why::CheckoutMissing { checkout: path() },
                Why::NoConfiguration { checkout: path() },
                Why::TrackingOff,
                Why::ConfigurationUnreadable { error: error() },
                Why::RecordUnreadable { error: url_error() },
            ] {
                causes.push(Limitation::Project { why, stored });
            }
        }
        causes.extend([
            Limitation::StoredUnreadable { error: error() },
            Limitation::PublicationUnreadable { error: url_error() },
            Limitation::FingerprintsUnreadable { error: error() },
            Limitation::UntimedFingerprints { count: 2 },
            Limitation::UnknownRequestFormat,
        ]);
        let shown: Vec<String> = causes
            .iter()
            .map(|cause| {
                format!(
                    "{}\n  public: {}\n  log:    {}",
                    cause.cause(),
                    cause.public(),
                    cause.log_detail()
                )
            })
            .collect();
        insta::assert_snapshot!("limitation-wordings", shown.join("\n"));
    }
}
