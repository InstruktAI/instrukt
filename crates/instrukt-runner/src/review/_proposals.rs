//! A company home's knowledge proposals in the review observation
//! (`project/instrukt/spec/feature/runner/pull-request-review`, Knowledge
//! proposals in a company home): opening a pull request for each admitted
//! proposal on a remote-tracking branch, handing an approved one to the
//! standing integrator as `runner.proposal_approved`, and raising what the
//! operator must see. Whether a head is admitted is the company context's
//! answer; GitHub is reached through work's boundary. A failure for one head
//! is warned and that head skipped; the next tick tries again, and nothing
//! here writes state but the readiness cause and the event.

use std::collections::BTreeSet;
use std::sync::Mutex;

use instrukt_core::ids::CommitSha;
use instrukt_core::interfaces::ReviewReadiness;

use super::{REMOTE, ReviewPass};
use crate::events::Event;
use crate::interfaces::{CompanyPorts, ProposalOpening, ProposalPull, ProposalStoreCondition};

/// The references this running process already raised as anomalies, so
/// each is raised once per process.
#[derive(Debug, Default)]
pub struct ProposalReports(Mutex<BTreeSet<String>>);

impl ProposalReports {
    /// Whether `reference` is raised for the first time in this process.
    fn first(&self, reference: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(reference.to_owned())
    }
}

/// The proposal heads among the remote-tracking branches `listing` names
/// (`<refname> <sha>` per line): every branch of the remote but the
/// canonical branch, the integration branch and `HEAD`.
fn proposal_heads(listing: &str, canonical: &str) -> Vec<(String, CommitSha)> {
    let prefix = format!("refs/remotes/{REMOTE}/");
    listing
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter_map(|(name, sha)| {
            let branch = name.strip_prefix(prefix.as_str())?;
            if branch == canonical || branch == crate::sources::INTEGRATE_BRANCH || branch == "HEAD"
            {
                return None;
            }
            Some((branch.to_owned(), CommitSha::parse(sha.trim()).ok()?))
        })
        .collect()
}

/// Steps one to four of a company home's pass, before the listing of open
/// pull requests: the proposal credential's readiness asked once, the
/// pruning fetch that stands in for the pass's own, and a pull request
/// opened for each admitted proposal head no pull request has had as its
/// head, when the credential is met. A failed fetch ends the pass, which
/// the next tick tries again.
pub(super) fn open(pass: &ReviewPass<'_>, company: &dyn CompanyPorts) -> Result<(), String> {
    let project = &pass.project.slug;
    let root = &pass.project.root;
    let readiness = pass
        .port
        .proposal_readiness(pass.repository, (pass.clock)());
    let met = match readiness {
        ReviewReadiness::Met { .. } => true,
        ReviewReadiness::Unmet(finding) => {
            tracing::warn!(event = "proposal_readiness_unmet", project = %project, finding = %finding, "proposal pull requests are not opened until the finding is repaired");
            false
        }
        ReviewReadiness::CannotAsk(cause) => {
            if let Err(error) =
                crate::ci::record_source_cause(pass.state, project, &cause, (pass.clock)())
            {
                tracing::warn!(event = "ci_observation_state_failed", error = %error);
            }
            tracing::warn!(event = "proposal_readiness_unavailable", project = %project, cause = %cause);
            false
        }
    };
    instrukt_core::git::run(root, ["fetch", "--quiet", "--prune", REMOTE])
        .map_err(|error| error.to_string())?;
    if !met {
        return Ok(());
    }
    let canonical = instrukt_core::git::default_branch(root, REMOTE).map_err(|e| e.to_string())?;
    let tracked = format!("refs/remotes/{REMOTE}/{canonical}");
    let prefix = format!("refs/remotes/{REMOTE}");
    let listing = instrukt_core::git::run(
        root,
        [
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            prefix.as_str(),
        ],
    )
    .map_err(|error| error.to_string())?;
    for (branch, head) in proposal_heads(&listing, &canonical) {
        let opened = company
            .admitted_proposal(pass.home, root, &head)
            .map_err(|error| error.0)
            .and_then(|admitted| {
                if !admitted || pass.port.heads_pull(pass.repository, &head, pass.clock)? {
                    return Ok(None);
                }
                pass.port
                    .open_proposal(
                        &ProposalPull {
                            root,
                            repository: pass.repository,
                            branch: &branch,
                            base: &canonical,
                            canonical: &tracked,
                            head: &head,
                        },
                        pass.clock,
                    )
                    .map(Some)
            });
        match opened {
            Ok(None) => {}
            Ok(Some(ProposalOpening::Opened(number))) => {
                tracing::info!(event = "proposal_pull_opened", project = %project, branch = %branch, head = %head, pull = number);
            }
            Ok(Some(ProposalOpening::AlreadyOpen)) => {
                tracing::debug!(event = "proposal_pull_open_already", project = %project, branch = %branch, head = %head);
            }
            Err(error) => {
                tracing::warn!(event = "proposal_opening_failed", project = %project, branch = %branch, head = %head, error = %error, "the next tick tries again");
            }
        }
    }
    Ok(())
}

/// Step five: for each open eligible pull request whose current head is an
/// admitted proposal holding a current approving verdict that covers the
/// canonical branch, `runner.proposal_approved` with that verdict's pinned
/// base.
pub(super) fn hand_off(
    pass: &ReviewPass<'_>,
    company: &dyn CompanyPorts,
    eligible: &[&super::_eligibility::Eligible],
) {
    let project = &pass.project.slug;
    for pull in eligible {
        let approved = company
            .admitted_proposal(pass.home, &pass.project.root, &pull.head)
            .map_err(|error| error.0)
            .and_then(|admitted| match admitted {
                true => pass.port.approved_head(project, &pull.head),
                false => Ok(None),
            });
        let base = match approved {
            Ok(Some(base)) => base,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(event = "proposal_handoff_failed", project = %project, pull = pull.number, head = %pull.head, error = %error, "the next tick tries again");
                continue;
            }
        };
        let handoff = Event::ProposalApproved {
            project: project.clone(),
            pull_request: pull.number,
            head_branch: pull.head_branch.clone(),
            base_branch: pull.base_branch.clone(),
            head: pull.head.clone(),
            base,
        };
        match pass.log.log(&handoff) {
            Ok(_) => {
                tracing::info!(event = "proposal_handed_off", project = %project, pull = pull.number, head = %pull.head);
            }
            Err(error) => {
                tracing::warn!(event = "proposal_handoff_failed", project = %project, pull = pull.number, head = %pull.head, error = %error, "the next tick tries again");
            }
        }
    }
}

/// Raises, once per running process, each unestablished entry a
/// remote-tracking branch holds and a proposal store that is not usable.
pub(super) fn report(pass: &ReviewPass<'_>, company: &dyn CompanyPorts) {
    let root = &pass.project.root;
    let project = &pass.project.slug;
    match company.unestablished_proposals(pass.home, root) {
        Ok(entries) => {
            for entry in entries {
                let reference = format!("knowledge-proposal:{}/{}", entry.company, entry.commit);
                if pass.reports.first(&reference) {
                    crate::anomaly::notify(pass.config, pass.env, &reference, None);
                }
            }
        }
        Err(error) => {
            tracing::warn!(event = "proposal_lookup_failed", project = %project, error = %error.0);
        }
    }
    match company.proposal_store(pass.home, root) {
        Ok(ProposalStoreCondition::Usable) => {}
        Ok(_) => {
            let company = project
                .as_str()
                .strip_prefix("company-")
                .unwrap_or(project.as_str());
            let reference = format!("knowledge-proposal-store:{company}");
            if pass.reports.first(&reference) {
                crate::anomaly::notify(pass.config, pass.env, &reference, None);
            }
        }
        Err(error) => {
            tracing::warn!(event = "proposal_lookup_failed", project = %project, error = %error.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::proposal_heads;

    #[test]
    fn only_remote_tracking_branches_other_than_canonical_integrate_and_head_are_proposals() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let listing = format!(
            "refs/remotes/origin/HEAD {a}\nrefs/remotes/origin/main {a}\nrefs/remotes/origin/integrate {a}\nrefs/remotes/origin/knowledge/pricing {b}\nrefs/remotes/upstream/other {b}\nrefs/heads/local {b}\n"
        );
        let heads = proposal_heads(&listing, "main");
        assert_eq!(heads.len(), 1);
        assert_eq!(heads[0].0, "knowledge/pricing");
        assert_eq!(heads[0].1.as_str(), b);
    }
}
