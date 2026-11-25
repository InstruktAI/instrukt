//! Work's part in worktree reclamation (`project/instrukt/spec/domain/work`,
//! "Work → runner (worktree reclamation)").
//!
//! A section of the work record in which no role attempt is open on a
//! todo's worktree and none opens: opening an attempt is a write under the
//! same writer lock, so inside the section none can. The guard reads the
//! record, never the caller's rank (`general/principle/trust-and-guardrails`).
//!
//! The release of a delivered todo's worktree and work branch
//! (`project/instrukt/spec/feature/work/todos`, "Parking, withdrawal and
//! delivery"), through the sequence withdrawal uses for a worktree: checked
//! at its path, moved aside with `git worktree move`, checked again, then
//! removed with `git worktree remove` or moved back. It records no intent
//! and writes no marker: git shows of the worktree and of each branch
//! whether it is still there, so a release that stopped part-way is
//! continued by the next from what git shows. One release runs at a time on
//! a computer, under the delivered-release lock taken without waiting.

use std::path::{Path, PathBuf};

use instrukt_core::fsx::LockGuard;
use instrukt_core::git;
use instrukt_core::ids::CommitSha;
use instrukt_runner::interfaces::{
    AssignmentKind, DeliveredRelease, DeliveredResource, Lane, ProjectRef, SectionAdmission,
    SectionOutcome, TodoIdentity, WorkInterfaceError, WorktreeHolder, WorktreeReclamation,
};

use super::_error::WorkError;
use super::_model::{Binding, TodoId, TodoRecord};
use super::_store::Store;
use super::_worktrees::{self, Kept, Worktree, worktrees};
use super::Work;
use crate::evidence::{StampFact, TodoStatus};

/// The prefix of a delivered release's staging location beside a worktree.
const STAGING_PREFIX: &str = ".instrukt-dr-";

/// The remote work branches are published to.
const REMOTE: &str = "origin";

impl WorktreeReclamation for Work {
    fn attempt_free(
        &self,
        project: &ProjectRef,
        todo: &TodoIdentity,
        admission: &SectionAdmission,
        action: &mut dyn FnMut(),
    ) -> Result<SectionOutcome, WorkInterfaceError> {
        let Some(id) = TodoId::parse(todo.as_str()) else {
            return Ok(SectionOutcome::NotRecorded);
        };
        Ok(
            Store::new(self.home(), &project.slug).hold_record(&id, |record| {
                let Some(record) = record else {
                    return SectionOutcome::NotRecorded;
                };
                match refusal(record, admission) {
                    Some(refused) => refused,
                    None => {
                        action();
                        SectionOutcome::Ran
                    }
                }
            })?,
        )
    }

    fn release_delivered(
        &self,
        project: &ProjectRef,
        todo: &TodoIdentity,
    ) -> Result<DeliveredRelease, WorkInterfaceError> {
        let Some(_lock) = LockGuard::try_acquire(&self.home().delivered_release_lock_file())
            .map_err(WorkError::from)?
        else {
            return Ok(DeliveredRelease::Busy);
        };
        let store = Store::new(self.home(), &project.slug);
        let record = TodoId::parse(todo.as_str())
            .map(|id| store.item(&id))
            .transpose()?
            .flatten()
            .map(|item| item.record)
            .filter(|record| record.status == TodoStatus::Delivered)
            .ok_or_else(|| WorkError::refused(format!("{todo} is not a delivered todo here")))?;
        let mut release = Release {
            root: &project.root,
            store: &store,
            record: &record,
            branch: work_branch(&record),
            released: Vec::new(),
            kept: Vec::new(),
        };
        release.worktree()?;
        release.branches()?;
        Ok(DeliveredRelease::Done {
            released: release.released,
            kept: release.kept,
        })
    }
}

/// One delivered release in progress: what it released and what stays.
struct Release<'a> {
    root: &'a Path,
    store: &'a Store,
    record: &'a TodoRecord,
    branch: Option<String>,
    released: Vec<DeliveredResource>,
    kept: Vec<(DeliveredResource, String)>,
}

impl Release<'_> {
    fn keep(&mut self, resource: DeliveredResource, cause: impl Into<String>) {
        self.kept.push((resource, cause.into()));
    }

    /// The todo's worktree as git lists it: its work branch checked out, at
    /// a path its bindings recorded or at that path's staging location.
    /// The primary checkout and a worktree git reports prunable are never
    /// it. With the original path it was released from.
    fn locate(&self) -> Result<Option<(Worktree, PathBuf)>, WorkError> {
        let recorded: Vec<&PathBuf> = self
            .record
            .bindings
            .iter()
            .filter_map(|binding| binding.worktree.as_ref())
            .collect();
        let id = &self.record.id;
        let found = worktrees(self.root)?
            .into_iter()
            .skip(1)
            .filter(|listed| !listed.prunable)
            .find_map(|listed| {
                let original = recorded.iter().find(|path| {
                    **path == &listed.path
                        || _worktrees::staging_path(path, STAGING_PREFIX, id).as_ref()
                            == Some(&listed.path)
                })?;
                let ours = self.branch.is_some() && listed.branch == self.branch;
                ours.then(|| (listed, (*original).clone()))
            });
        Ok(found)
    }

    /// Why the worktree must stay where it is now; `None` when it may go.
    fn held(&self, worktree: &Worktree) -> Result<Option<String>, WorkError> {
        match _worktrees::kept(worktree)? {
            Some(Kept::Locked(reason)) if reason.is_empty() => {
                return Ok(Some("it is locked".to_owned()));
            }
            Some(Kept::Locked(reason)) => return Ok(Some(format!("it is locked ({reason})"))),
            Some(Kept::Uncommitted) => {
                return Ok(Some("it holds uncommitted changes".to_owned()));
            }
            None => {}
        }
        let open = self.store.item(&self.record.id)?.and_then(|item| {
            item.record
                .bindings
                .iter()
                .find(|binding| open_attempt(binding))
                .map(holder)
        });
        Ok(open.map(|holder| format!("{holder} is open on it")))
    }

    /// Releases the worktree, first moving it aside to its staging
    /// location, unless an earlier release left it there already.
    fn worktree(&mut self) -> Result<(), WorkError> {
        let Some((found, original)) = self.locate()? else {
            return Ok(());
        };
        let resource = DeliveredResource::Worktree(original.clone());
        let Some(staging) = _worktrees::staging_path(&original, STAGING_PREFIX, &self.record.id)
        else {
            self.keep(
                resource,
                "it has no directory beside it to be released from",
            );
            return Ok(());
        };
        if found.path == original {
            if let Some(cause) = self.held(&found)? {
                self.keep(resource, cause);
                return Ok(());
            }
            if _worktrees::exists(&staging).map_err(|source| WorkError::Io {
                path: staging.clone(),
                source,
            })? {
                self.keep(
                    resource,
                    format!(
                        "something is at {}, where it is moved to",
                        staging.display()
                    ),
                );
                return Ok(());
            }
            git::run(
                self.root,
                worktree_command("move", &original, Some(&staging)),
            )?;
        }
        let Some((staged, _)) = self.locate()?.filter(|(listed, _)| listed.path == staging) else {
            self.keep(resource, "git no longer lists it where it was moved to");
            return Ok(());
        };
        let removal = match self.held(&staged)? {
            Some(cause) => Err(cause),
            None => git::run(self.root, worktree_command("remove", &staging, None))
                .map(|_| ())
                .map_err(|error| error.to_string()),
        };
        match removal {
            Ok(()) => self.released.push(resource),
            Err(cause) => {
                let back = if _worktrees::exists(&original).unwrap_or(true) {
                    format!(
                        "; it stays at {} since something is at its path",
                        staging.display()
                    )
                } else {
                    git::run(
                        self.root,
                        worktree_command("move", &staging, Some(&original)),
                    )
                    .err()
                    .map(|error| {
                        format!(
                            "; moving it back from {} failed: {error}",
                            staging.display()
                        )
                    })
                    .unwrap_or_default()
                };
                self.keep(resource, format!("{cause}{back}"));
            }
        }
        Ok(())
    }

    /// Deletes the work branch locally and on origin, each only where it
    /// holds no commit beyond the head the todo handed to integration.
    fn branches(&mut self) -> Result<(), WorkError> {
        let Some(branch) = self.branch.clone() else {
            return Ok(());
        };
        let handed = handed_head(self.record);
        let local = DeliveredResource::LocalBranch(branch.clone());
        if let Some(tip) = ref_tip(self.root, &format!("refs/heads/{branch}"))? {
            match worktrees(self.root)?
                .iter()
                .find(|listed| listed.branch.as_deref() == Some(branch.as_str()))
            {
                Some(listed) => {
                    let at = listed.path.display().to_string();
                    self.keep(local, format!("it is checked out in {at}"));
                }
                None => match self.beyond(&tip, handed)? {
                    Some(cause) => self.keep(local, cause),
                    None => {
                        let name = format!("refs/heads/{branch}");
                        match git::run(self.root, ["update-ref", "-d", &name, tip.as_str()]) {
                            Ok(_) => self.released.push(local),
                            Err(error) => self.keep(local, error.to_string()),
                        }
                    }
                },
            }
        }
        let origin = DeliveredResource::OriginBranch(branch.clone());
        let tracking = format!("refs/remotes/{REMOTE}/{branch}");
        if let Some(tip) = ref_tip(self.root, &tracking)? {
            if let Some(cause) = self.beyond(&tip, handed)? {
                self.keep(origin, cause);
                return Ok(());
            }
            let lease = format!("--force-with-lease={branch}:{tip}");
            match git::run(
                self.root,
                [
                    "push",
                    "--quiet",
                    lease.as_str(),
                    REMOTE,
                    "--delete",
                    &branch,
                ],
            ) {
                Ok(_) => {
                    // A successful delete also removes the tracking ref.
                    self.released.push(origin);
                }
                Err(pushed) => {
                    let listed = format!("refs/heads/{branch}");
                    match git::run(self.root, ["ls-remote", REMOTE, listed.as_str()]) {
                        Ok(remote) if remote.trim().is_empty() => {
                            // Already gone on origin: the stale tracking ref
                            // goes so git stops listing it.
                            git::run(self.root, ["update-ref", "-d", &tracking, tip.as_str()])?;
                            self.released.push(origin);
                        }
                        _ => self.keep(origin, pushed.to_string()),
                    }
                }
            }
        }
        Ok(())
    }

    /// Why the branch at `tip` must stay: it holds a commit beyond the head
    /// handed to integration, or there is no such head to compare with.
    fn beyond(
        &self,
        tip: &CommitSha,
        handed: Option<&CommitSha>,
    ) -> Result<Option<String>, WorkError> {
        let Some(handed) = handed else {
            return Ok(Some(
                "the record names no head handed to integration".to_owned(),
            ));
        };
        let within = git::answers(
            self.root,
            ["merge-base", "--is-ancestor", tip.as_str(), handed.as_str()],
        )?;
        Ok((!within)
            .then(|| format!("it holds commits beyond {handed}, the head handed to integration")))
    }
}

/// `git worktree <verb> <path> [<to>]`.
fn worktree_command(verb: &str, path: &Path, to: Option<&Path>) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = vec!["worktree".into(), verb.into(), path.into()];
    if let Some(to) = to {
        args.push(to.into());
    }
    args
}

/// The todo's work branch: the one its record names, else the one its
/// latest work-lane attempt was given.
fn work_branch(record: &TodoRecord) -> Option<String> {
    record.branch.clone().or_else(|| {
        record
            .bindings
            .iter()
            .rev()
            .filter(|binding| matches!(binding.attempt.lane, Some(Lane::Work | Lane::Prepare)))
            .find_map(|binding| binding.attempt.branch.clone())
    })
}

/// The head the todo handed to integration: its latest finalized hand-off,
/// else its delivered commit.
fn handed_head(record: &TodoRecord) -> Option<&CommitSha> {
    record
        .stamps
        .iter()
        .rev()
        .find_map(|stamp| match &stamp.fact {
            StampFact::FinalizeReady { commit } => Some(commit),
            _ => None,
        })
        .or(record.delivery.as_ref().map(|delivery| &delivery.commit))
}

/// The commit `reference` points at in the repository at `root`; `None`
/// when it does not exist.
fn ref_tip(root: &Path, reference: &str) -> Result<Option<CommitSha>, WorkError> {
    let spec = format!("{reference}^{{commit}}");
    let output = git::run_output(root, ["rev-parse", "--verify", "--quiet", spec.as_str()])?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    CommitSha::parse(&text)
        .map(Some)
        .map_err(|_| WorkError::refused(format!("git printed `{text}` for {reference}")))
}

/// Why `admission` admits no section on `record`'s worktree; `None` when it
/// admits one. The runner is refused while any role attempt is open there
/// or the record moved past the revision it observed; a session is refused
/// only while another session holds the worktree: a role attempt open there
/// that is not its own, or, with none open, a claim it does not hold.
fn refusal(record: &TodoRecord, admission: &SectionAdmission) -> Option<SectionOutcome> {
    let mut open = record
        .bindings
        .iter()
        .filter(|binding| open_attempt(binding));
    match admission {
        SectionAdmission::Runner { revision } => match open.next() {
            Some(binding) => Some(SectionOutcome::Held(holder(binding))),
            None => (record.revision != *revision).then_some(SectionOutcome::RecordChanged),
        },
        SectionAdmission::Session(caller) => {
            let open: Vec<&Binding> = open.collect();
            if let Some(other) = open
                .iter()
                .find(|binding| binding.session.as_ref() != Some(caller))
            {
                return Some(SectionOutcome::Held(holder(other)));
            }
            match &record.claim {
                Some(claim) if open.is_empty() && claim.session.as_ref() != Some(caller) => {
                    Some(SectionOutcome::Held(WorktreeHolder::Claim {
                        session: claim.session.clone(),
                    }))
                }
                _ => None,
            }
        }
    }
}

/// Whether `binding` is a role attempt open on the todo's worktree: an
/// unreleased workspace or child assignment. A driver binding is not one.
fn open_attempt(binding: &Binding) -> bool {
    matches!(
        binding.attempt.assignment,
        AssignmentKind::Workspace | AssignmentKind::Child
    ) && binding.attempt.released_at.is_none()
}

fn holder(binding: &Binding) -> WorktreeHolder {
    WorktreeHolder::Attempt {
        role: binding.role.clone(),
        attempt: binding.attempt.number,
        session: binding.session.clone(),
    }
}
