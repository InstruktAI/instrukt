//! Work's part in worktree reclamation (`project/instrukt/spec/domain/work`,
//! "Work → runner (worktree reclamation)").
//!
//! A section of the work record in which no role attempt opens on a todo's
//! worktree: opening an attempt is a write under the same writer lock, so
//! inside the section none can. The runner is admitted to it only while no
//! role attempt is open; a session the assignment binds, while no other
//! session holds the worktree, so its own open attempt does not refuse it.
//! The guard reads the record, never the caller's rank
//! (`general/principle/trust-and-guardrails`).
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
    DeliveredRelease, DeliveredRemains, DeliveredResource, Lane, ProjectRef, SectionAdmission,
    SectionOutcome, TodoWorktree, WorkInterfaceError, WorktreeHolder, WorktreeReclamation,
    WorktreeStanding,
};

use super::_error::WorkError;
use super::_model::{Binding, TodoId, TodoRecord};
use super::_store::Store;
use super::_worktrees::{self, Kept, Worktree, ref_tip, worktrees};
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
        todo: &TodoId,
        admission: &SectionAdmission,
        action: &mut dyn FnMut(),
    ) -> Result<SectionOutcome, WorkInterfaceError> {
        Ok(
            Store::new(self.home(), &project.slug).hold_record(todo, |record| {
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
        delivered: &TodoWorktree,
    ) -> Result<DeliveredRelease, WorkInterfaceError> {
        let WorktreeStanding::Delivered(remains) = delivered.standing else {
            return Err(WorkError::refused(format!(
                "{} is not a delivered todo's worktree",
                delivered.todo
            ))
            .into());
        };
        let todo = &delivered.identity;
        let Some(_lock) = LockGuard::try_acquire(&self.home().delivered_release_lock_file())
            .map_err(WorkError::from)?
        else {
            return Ok(DeliveredRelease::Busy);
        };
        let store = Store::new(self.home(), &project.slug);
        let record = store
            .item(todo)?
            .map(|item| item.record)
            .filter(|record| record.status == TodoStatus::Delivered)
            .ok_or_else(|| WorkError::refused(format!("{todo} is not a delivered todo here")))?;
        let mut release = Release {
            root: &project.root,
            store: &store,
            record: &record,
            attributed: delivered.worktree.clone(),
            branch: delivered.branch.clone(),
            remains,
            released: Vec::new(),
            kept: Vec::new(),
        };
        // The worktree goes first: while it stays, so does its branch.
        if release.worktree()? {
            release.branches()?;
        }
        Ok(DeliveredRelease::Done {
            released: release.released,
            kept: release.kept,
        })
    }

    fn holds(&self, project: &ProjectRef, todo: &TodoId) -> Result<bool, WorkInterfaceError> {
        Ok(Store::new(self.home(), &project.slug).item(todo)?.is_some())
    }
}

/// One delivered release in progress: what it released and what stays.
struct Release<'a> {
    root: &'a Path,
    store: &'a Store,
    record: &'a TodoRecord,
    /// The worktree workflow attributes to the todo: at a path its
    /// bindings recorded.
    attributed: Option<PathBuf>,
    /// The work branch workflow attributes to the todo; `None` when a live
    /// todo holds that name.
    branch: Option<String>,
    /// What git listed of the todo when workflow answered.
    remains: DeliveredRemains,
    released: Vec<DeliveredResource>,
    kept: Vec<(DeliveredResource, String)>,
}

impl Release<'_> {
    fn keep(&mut self, resource: DeliveredResource, cause: impl Into<String>) {
        self.kept.push((resource, cause.into()));
    }

    /// The todo's worktree as git lists it: at the path workflow attributes
    /// to it, or at the staging location of a path its bindings recorded,
    /// where an earlier release moved it. The primary checkout and a
    /// worktree git reports prunable are never it. With the original path
    /// it was released from.
    fn locate(&self) -> Result<Option<(Worktree, PathBuf)>, WorkError> {
        let id = &self.record.id;
        let staged_from: Vec<(PathBuf, PathBuf)> = self
            .record
            .bindings
            .iter()
            .filter_map(|binding| binding.worktree.as_ref())
            .filter_map(|path| {
                _worktrees::staging_path(path, STAGING_PREFIX, id)
                    .map(|staging| (staging, path.clone()))
            })
            .collect();
        let found = worktrees(self.root)?
            .into_iter()
            .skip(1)
            .filter(|listed| !listed.prunable)
            .find_map(|listed| {
                let original = if self.attributed.as_ref() == Some(&listed.path) {
                    listed.path.clone()
                } else {
                    staged_from
                        .iter()
                        .find(|(staging, _)| *staging == listed.path)
                        .map(|(_, original)| original.clone())?
                };
                Some((listed, original))
            });
        Ok(found)
    }

    /// Why `worktree` is no longer the todo's: it does not have the work
    /// branch workflow attributes to the todo checked out.
    fn not_ours(&self, worktree: &Worktree) -> Option<String> {
        match (&self.branch, &worktree.branch) {
            (Some(branch), Some(checked_out)) if branch == checked_out => None,
            (Some(branch), Some(checked_out)) => Some(format!(
                "it has {checked_out} checked out, not the work branch {branch}"
            )),
            (Some(branch), None) => {
                Some(format!("it is detached, not on the work branch {branch}"))
            }
            (None, _) => Some("no work branch is attributed to the todo".to_owned()),
        }
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
                .find(|binding| binding.is_open_role_attempt())
                .map(holder)
        });
        Ok(open.map(|holder| format!("{holder} is open on it")))
    }

    /// Releases the worktree, first moving it aside to its staging
    /// location, unless an earlier release left it there already. Whether
    /// git no longer lists it.
    fn worktree(&mut self) -> Result<bool, WorkError> {
        let Some((found, original)) = self.locate()? else {
            return Ok(true);
        };
        let resource = DeliveredResource::Worktree(original.clone());
        let Some(staging) = _worktrees::staging_path(&original, STAGING_PREFIX, &self.record.id)
        else {
            self.keep(
                resource,
                "it has no directory beside it to be released from",
            );
            return Ok(false);
        };
        if let Some(cause) = self.not_ours(&found) {
            self.keep(resource, cause);
            return Ok(false);
        }
        if found.path == original {
            if let Some(cause) = self.held(&found)? {
                self.keep(resource, cause);
                return Ok(false);
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
                return Ok(false);
            }
            git::run(
                self.root,
                worktree_command("move", &original, Some(&staging)),
            )?;
        }
        // From here the worktree may be away from its path: every outcome
        // either removes it or restores it there, or reports where it is.
        let removal = (|| -> Result<(), String> {
            let staged = self
                .locate()
                .map_err(|error| format!("git's worktree list could not be read: {error}"))?
                .filter(|(listed, _)| listed.path == staging)
                .map(|(listed, _)| listed)
                .ok_or_else(|| "git no longer lists it where it was moved to".to_owned())?;
            if let Some(cause) = self.not_ours(&staged) {
                return Err(cause);
            }
            if let Some(cause) = self
                .held(&staged)
                .map_err(|error| format!("it could not be checked again: {error}"))?
            {
                return Err(cause);
            }
            git::run(self.root, worktree_command("remove", &staging, None))
                .map(|_| ())
                .map_err(|error| error.to_string())
        })();
        match removal {
            Ok(()) => {
                self.released.push(resource);
                return Ok(true);
            }
            Err(cause) => {
                let back = match _worktrees::exists(&original) {
                    Ok(true) => format!(
                        "; it stays at {} since something is at its path",
                        staging.display()
                    ),
                    Ok(false) => match git::run(
                        self.root,
                        worktree_command("move", &staging, Some(&original)),
                    ) {
                        Ok(_) => String::new(),
                        Err(error) => format!(
                            "; moving it back from {} failed, so it stays there: {error}",
                            staging.display()
                        ),
                    },
                    Err(error) => format!(
                        "; whether its path is free could not be read ({error}), so it stays at {}",
                        staging.display()
                    ),
                };
                self.keep(resource, format!("{cause}{back}"));
            }
        }
        Ok(false)
    }

    /// Deletes the work branch locally and on origin, each only where it
    /// holds no commit beyond the head the todo handed to integration.
    fn branches(&mut self) -> Result<(), WorkError> {
        let Some(branch) = self.branch.clone() else {
            return Ok(());
        };
        let handed = handed_head(self.record);
        let local = DeliveredResource::LocalBranch(branch.clone());
        let listed_local = if self.remains.local_branch {
            ref_tip(self.root, &format!("refs/heads/{branch}"))?
        } else {
            None
        };
        if let Some(tip) = listed_local {
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
        let listed_origin = if self.remains.origin_branch {
            ref_tip(self.root, &tracking)?
        } else {
            None
        };
        let Some(listed) = listed_origin else {
            return Ok(());
        };
        // Origin's own answer decides, not the tracking ref: another
        // computer may have published the branch since this one fetched.
        let remote_ref = format!("refs/heads/{branch}");
        let advertised = match git::run(self.root, ["ls-remote", REMOTE, remote_ref.as_str()]) {
            Ok(advertised) => advertised,
            Err(error) => {
                self.keep(origin, format!("origin could not be read: {error}"));
                return Ok(());
            }
        };
        let on_origin = advertised.lines().find_map(|line| {
            let (sha, name) = line.split_once('\t')?;
            (name == remote_ref)
                .then(|| CommitSha::parse(sha).ok())
                .flatten()
        });
        let Some(tip) = on_origin else {
            // Already gone on origin: the stale tracking ref goes so git
            // stops listing it.
            git::run(self.root, ["update-ref", "-d", &tracking, listed.as_str()])?;
            self.released.push(origin);
            return Ok(());
        };
        if tip != listed && !git::answers(self.root, ["cat-file", "-e", tip.as_str()])? {
            let refspec = format!("+{remote_ref}:{tracking}");
            if let Err(error) = git::run(self.root, ["fetch", "--quiet", REMOTE, refspec.as_str()])
            {
                self.keep(
                    origin,
                    format!("origin's tip {tip} could not be fetched: {error}"),
                );
                return Ok(());
            }
        }
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
            // A successful delete also removes the tracking ref.
            Ok(_) => self.released.push(origin),
            Err(error) => self.keep(origin, error.to_string()),
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

/// The todo's work branch: the one its record names, else the branch of
/// its latest work- or preparation-lane attempt that was given one.
pub(super) fn work_branch(record: &TodoRecord) -> Option<String> {
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

/// Why `admission` admits no section on `record`'s worktree; `None` when it
/// admits one. The runner is refused while any role attempt is open there
/// or the record moved past the revision it observed; a session is refused
/// only while another session holds the worktree: a role attempt open there
/// that is not its own, or, with none open, a claim it does not hold.
fn refusal(record: &TodoRecord, admission: &SectionAdmission) -> Option<SectionOutcome> {
    let mut open = record
        .bindings
        .iter()
        .filter(|binding| binding.is_open_role_attempt());
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

fn holder(binding: &Binding) -> WorktreeHolder {
    WorktreeHolder::Attempt {
        role: binding.role.clone(),
        attempt: binding.attempt.number,
        session: binding.session.clone(),
    }
}
