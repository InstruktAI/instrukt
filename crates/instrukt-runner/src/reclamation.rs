//! Worktree reclamation (`project/instrukt/spec/feature/runner/worktree-reclamation`):
//! the runner's mechanical per-invocation work, beside canonical checkout
//! maintenance and the expired-record purge, that returns the disk todo
//! worktrees hold once it no longer serves work.
//!
//! The removal of a project's declared regenerable output from one worktree
//! is written once here and used by the runner and by `instrukt repo
//! reclaim`:
//!
//! 1. each declared path is walked from the worktree's root through real
//!    directories only;
//! 2. git, in that worktree, ignores it, tracks nothing beneath it and
//!    reports nothing beneath it untracked;
//! 3. inside work's section free of role attempts, each admitted path is
//!    renamed, in one rename, to a staging location of its own beside the
//!    worktree, under a fresh name;
//! 4. outside the section, holding nothing, the staging locations are
//!    deleted.
//!
//! A staging location is complete and nothing but removable output from the
//! instant the rename creates it, and deleting it is idempotent, so any
//! process may delete one at any time and two that meet on one do no harm.
//! Workflow answers each worktree's standing and work owns the section and
//! the delivered release; the runner derives none of their answers. No state
//! is kept: what is owed is read again on every invocation.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use instrukt_core::config::RegenerablePath;
use instrukt_core::fsx;
use instrukt_core::git;
use instrukt_core::ids::{ProjectSlug, SessionRef, TodoId};

use crate::interfaces::{
    DeliveredRelease, ProjectRef, RunnerWiring, SectionAdmission, SectionOutcome,
    WorkInterfaceError, WorktreeHolder, WorktreeReclamation, WorktreeStanding, WorktreeStandings,
};
use crate::runner::Considered;

/// The prefix of a staging location holding output moved out of a todo
/// worktree; the todo's identity and a fresh suffix follow it.
const STAGING_PREFIX: &str = ".instrukt-rg-";

/// Why a declared path stays in a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Left {
    /// Nothing is at it.
    Absent,
    /// It is reached through the symbolic link at this path.
    ThroughLink(PathBuf),
    /// It names a `.git` directory or a location under one.
    GitDirectory,
    /// Git does not ignore it.
    NotIgnored,
    /// Git tracks something beneath it.
    Tracked,
    /// Git reports something beneath it untracked or changed.
    Untracked,
    /// Git or the filesystem could not tell.
    Unanswered(String),
    /// Moving it out failed.
    NotMoved(String),
}

impl std::fmt::Display for Left {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Absent => f.write_str("nothing is there"),
            Self::ThroughLink(link) => {
                write!(
                    f,
                    "it is reached through the symbolic link {}",
                    link.display()
                )
            }
            Self::GitDirectory => f.write_str("it is a `.git` directory or under one"),
            Self::NotIgnored => f.write_str("git does not ignore it"),
            Self::Tracked => f.write_str("git tracks something beneath it"),
            Self::Untracked => f.write_str("git reports something beneath it untracked or changed"),
            Self::Unanswered(why) => write!(f, "it could not be examined: {why}"),
            Self::NotMoved(why) => write!(f, "moving it out failed: {why}"),
        }
    }
}

/// What moving one worktree's declared output out came to.
#[derive(Debug)]
pub struct Staged {
    /// Each declared path moved out, with the staging location it went to.
    pub moved: Vec<(RegenerablePath, PathBuf)>,
    /// Each declared path left, with why.
    pub left: Vec<(RegenerablePath, Left)>,
    /// What work's section answered; `None` when no path was admitted and
    /// no section was asked for.
    pub section: Option<SectionOutcome>,
}

/// The worktree whose declared output is moved out, and who asks.
pub struct Target<'a> {
    pub project: &'a ProjectRef,
    pub todo: &'a TodoId,
    pub worktree: &'a Path,
    pub admission: SectionAdmission,
}

/// Moves the declared output `declared` of `target`'s worktree out to
/// staging locations beside it, inside the section `work` grants under the
/// target's admission (steps 1–3). Nothing is deleted.
pub fn stage(
    target: &Target<'_>,
    declared: &[RegenerablePath],
    work: &dyn WorktreeReclamation,
) -> Result<Staged, WorkInterfaceError> {
    let (worktree, todo) = (target.worktree, target.todo);
    let mut left = Vec::new();
    let mut admitted = Vec::new();
    for entry in declared {
        match examine(worktree, entry) {
            Ok(()) => admitted.push(entry.clone()),
            Err(why) => left.push((entry.clone(), why)),
        }
    }
    // The runner asks for no section with nothing to move. A session always
    // asks: whether another session holds the worktree is its answer
    // whatever there is to move.
    if admitted.is_empty() && matches!(target.admission, SectionAdmission::Runner { .. }) {
        return Ok(Staged {
            moved: Vec::new(),
            left,
            section: None,
        });
    }
    let mut moved = Vec::new();
    let mut not_moved = Vec::new();
    let outcome = work.attempt_free(target.project, todo, &target.admission, &mut || {
        for entry in &admitted {
            let from = worktree.join(entry.as_path());
            match fresh_staging(worktree, todo).and_then(|to| fsx::rename(&from, &to).map(|()| to))
            {
                Ok(to) => moved.push((entry.clone(), to)),
                Err(error) => not_moved.push((entry.clone(), Left::NotMoved(error.to_string()))),
            }
        }
    })?;
    left.extend(not_moved);
    Ok(Staged {
        moved,
        left,
        section: Some(outcome),
    })
}

/// Whether the declared path `entry` of the worktree at `worktree` may be
/// moved out: reached through real directories, ignored by git there, with
/// nothing beneath it tracked or untracked.
fn examine(worktree: &Path, entry: &RegenerablePath) -> Result<(), Left> {
    if entry
        .as_path()
        .components()
        .any(|part| part.as_os_str().eq_ignore_ascii_case(".git"))
    {
        return Err(Left::GitDirectory);
    }
    let names: Vec<&OsStr> = entry
        .as_path()
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    let mut at = worktree.to_path_buf();
    for (index, name) in names.iter().enumerate() {
        at.push(name);
        let leaf = index + 1 == names.len();
        match std::fs::symlink_metadata(&at) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err(Left::Absent),
            Err(error) => return Err(Left::Unanswered(error.to_string())),
            Ok(found) if !leaf && found.file_type().is_symlink() => {
                return Err(Left::ThroughLink(at));
            }
            Ok(found) if !leaf && !found.is_dir() => return Err(Left::Absent),
            Ok(_) => {}
        }
    }
    let path = entry.as_path().as_os_str();
    let unanswered = |error: git::GitError| Left::Unanswered(error.to_string());
    let separator = OsStr::new("--");
    if !git::answers(
        worktree,
        [
            OsStr::new("check-ignore"),
            OsStr::new("-q"),
            separator,
            path,
        ],
    )
    .map_err(unanswered)?
    {
        return Err(Left::NotIgnored);
    }
    if !git::run(worktree, [OsStr::new("ls-files"), separator, path])
        .map_err(unanswered)?
        .trim()
        .is_empty()
    {
        return Err(Left::Tracked);
    }
    if !git::run(
        worktree,
        [
            OsStr::new("status"),
            OsStr::new("--porcelain"),
            OsStr::new("--untracked-files=all"),
            separator,
            path,
        ],
    )
    .map_err(unanswered)?
    .trim()
    .is_empty()
    {
        return Err(Left::Untracked);
    }
    Ok(())
}

/// A staging location beside the worktree at `worktree`, in the same
/// directory, under a name no existing entry has: the prefix, the todo's
/// identity, and a suffix fresh to this process and moment.
fn fresh_staging(worktree: &Path, todo: &TodoId) -> std::io::Result<PathBuf> {
    let parent = worktree.parent().ok_or_else(|| {
        std::io::Error::other(format!("{} has no directory beside it", worktree.display()))
    })?;
    let moment = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    for attempt in 0u32.. {
        let candidate = parent.join(format!(
            "{STAGING_PREFIX}{todo}-{}-{moment}-{attempt}",
            std::process::id()
        ));
        match std::fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Err(error) => return Err(error),
            Ok(_) => {}
        }
    }
    Err(std::io::Error::other("no fresh staging name"))
}

/// Deletes the staging location `staged`, never following a symbolic link.
/// Deleting is idempotent: entries, or the location, found already gone
/// count as deleted, and it is done once the location no longer exists.
pub fn delete(staged: &Path) -> std::io::Result<()> {
    let gone = |result: std::io::Result<()>| match result {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    };
    match std::fs::symlink_metadata(staged) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(found) if found.is_dir() => gone(fsx::remove_dir_all(staged)),
        Ok(_) => gone(fsx::remove_file(staged)),
    }
}

/// The todo identity the staging location named `name` carries, when the
/// name has exactly the form [`fresh_staging`] gives one: the prefix, a
/// todo identity, then the process, the moment and the attempt, each
/// decimal. Any other name is no staging location.
fn staged_for(name: &OsStr) -> Option<TodoId> {
    let rest = name.to_str()?.strip_prefix(STAGING_PREFIX)?;
    let mut parts = rest.split('-');
    let identity = TodoId::parse(parts.next()?)?;
    let suffix: Vec<&str> = parts.collect();
    let decimal = |part: &&str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    (suffix.len() == 3 && suffix.iter().all(decimal)).then_some(identity)
}

/// The owed causes this process last logged, so a cause is logged when it
/// appears or changes rather than on every invocation.
#[derive(Debug, Default)]
pub struct Reported {
    owed: BTreeMap<(ProjectSlug, String), String>,
}

impl Reported {
    fn owed(&mut self, project: &ProjectSlug, what: &str, cause: &str) {
        if self.changed(project, what, cause) {
            tracing::warn!(event = "worktree_reclamation_owed", project = %project, what = %what, cause = %cause);
        }
    }

    /// Declared paths a worktree keeps, with why, logged when they appear
    /// or change.
    fn kept(&mut self, project: &ProjectSlug, worktree: &str, paths: &str) {
        if self.changed(project, &kept_key(worktree), paths) {
            tracing::info!(event = "worktree_reclamation_kept", project = %project, worktree = %worktree, paths = %paths);
        }
    }

    /// Records `cause` for `what`; whether it is new or changed.
    fn changed(&mut self, project: &ProjectSlug, what: &str, cause: &str) -> bool {
        let key = (project.clone(), what.to_owned());
        if self.owed.get(&key).map(String::as_str) == Some(cause) {
            return false;
        }
        self.owed.insert(key, cause.to_owned());
        true
    }

    fn settled(&mut self, project: &ProjectSlug, what: &str) {
        self.owed.remove(&(project.clone(), what.to_owned()));
    }
}

/// In the wake, after dispatch and the event pass: moves the declared
/// regenerable output of each idle or delivered todo worktree out to its
/// staging locations, for each considered project whose work tracking and
/// reclamation are admitted. A worktree under way is not touched; one
/// worktree's failure is logged and the pass goes on.
pub fn stage_idle(considered: &[Considered], wiring: &RunnerWiring, reported: &mut Reported) {
    let (Some(standings), Some(work)) = (&wiring.standings, &wiring.work) else {
        return;
    };
    for project in considered
        .iter()
        .filter(|project| project.todos && project.reclamation && !project.regenerable.is_empty())
    {
        let slug = &project.project.slug;
        let worktrees = match standings.worktree_standings(&project.project) {
            Ok(worktrees) => {
                reported.settled(slug, "worktree standing");
                worktrees
            }
            Err(error) => {
                reported.owed(slug, "worktree standing", &error.to_string());
                continue;
            }
        };
        for todo in worktrees {
            let Some(worktree) = &todo.worktree else {
                continue;
            };
            if todo.standing == WorktreeStanding::UnderWay {
                continue;
            }
            let what = worktree.display().to_string();
            let target = Target {
                project: &project.project,
                todo: &todo.identity,
                worktree,
                admission: SectionAdmission::Runner {
                    revision: todo.revision,
                },
            };
            let staged = stage(&target, &project.regenerable, work.reclamation.as_ref());
            match staged {
                Ok(staged) => report_staged(reported, slug, &what, &staged),
                Err(error) => reported.owed(slug, &what, &error.to_string()),
            }
        }
    }
}

fn report_staged(reported: &mut Reported, project: &ProjectSlug, what: &str, staged: &Staged) {
    match &staged.section {
        Some(SectionOutcome::Held(holder)) => {
            reported.owed(project, what, &format!("{holder} holds it"));
            return;
        }
        Some(SectionOutcome::RecordChanged) => {
            reported.owed(project, what, "its todo's record changed meanwhile");
            return;
        }
        Some(SectionOutcome::NotRecorded) => {
            reported.owed(project, what, "its todo is no longer in the record");
            return;
        }
        Some(SectionOutcome::Ran) | None => {}
    }
    let not_moved: Vec<String> = staged
        .left
        .iter()
        .filter(|(_, why)| matches!(why, Left::NotMoved(_) | Left::Unanswered(_)))
        .map(|(entry, why)| format!("{entry}: {why}"))
        .collect();
    if !not_moved.is_empty() {
        reported.owed(project, what, &not_moved.join("; "));
        return;
    }
    reported.settled(project, what);
    let kept: Vec<String> = staged
        .left
        .iter()
        .filter(|(_, why)| *why != Left::Absent)
        .map(|(entry, why)| format!("{entry}: {why}"))
        .collect();
    if kept.is_empty() {
        reported.settled(project, &kept_key(what));
    } else {
        reported.kept(project, what, &kept.join("; "));
    }
    if !staged.moved.is_empty() {
        let paths: Vec<String> = staged
            .moved
            .iter()
            .map(|(entry, _)| entry.to_string())
            .collect();
        tracing::info!(event = "worktree_reclaimed", project = %project, worktree = %what, paths = %paths.join(", "));
    }
}

/// The report key of the declared paths the worktree `worktree` keeps.
fn kept_key(worktree: &str) -> String {
    format!("{worktree} (kept paths)")
}

/// The work an invocation leaves for after its wake, which nothing waits
/// on: deleting the staging locations beside the todo worktrees of the
/// considered projects, and asking work to release each delivered todo's
/// worktree and branches.
pub struct Deferred {
    projects: Vec<ProjectRef>,
    standings: Arc<dyn WorktreeStandings>,
    reclamation: Arc<dyn WorktreeReclamation>,
}

impl Deferred {
    /// The deferred work for the considered projects whose work tracking is
    /// admitted, whatever stands at their declaration; `None` when there is
    /// none or workflow and work are not wired.
    pub fn prepare(considered: &[Considered], wiring: &RunnerWiring) -> Option<Self> {
        let standings = wiring.standings.clone()?;
        let reclamation = wiring.work.as_ref()?.reclamation.clone();
        let projects: Vec<ProjectRef> = considered
            .iter()
            .filter(|project| project.todos)
            .map(|project| project.project.clone())
            .collect();
        (!projects.is_empty()).then_some(Self {
            projects,
            standings,
            reclamation,
        })
    }

    /// Runs it, holding no lock of the runner's.
    pub fn run(self, reported: &mut Reported) {
        for project in &self.projects {
            let worktrees = match self.standings.worktree_standings(project) {
                Ok(worktrees) => {
                    reported.settled(&project.slug, "worktree standing");
                    worktrees
                }
                Err(error) => {
                    reported.owed(&project.slug, "worktree standing", &error.to_string());
                    continue;
                }
            };
            let beside: BTreeSet<PathBuf> = worktrees
                .iter()
                .filter_map(|todo| todo.worktree.as_ref()?.parent().map(Path::to_path_buf))
                .collect();
            let mut swept = Swept::default();
            for dir in beside {
                self.sweep(project, &dir, reported, &mut swept);
            }
            for todo in &worktrees {
                let WorktreeStanding::Delivered(remains) = todo.standing else {
                    continue;
                };
                if !remains.any() {
                    continue;
                }
                let what = format!("delivered todo {}", todo.todo);
                if swept.holds_back(todo) {
                    // The worktree stays where that output is found again.
                    reported.owed(
                        &project.slug,
                        &what,
                        "output moved out of its worktree is not deleted yet; the worktree stays until it is",
                    );
                    continue;
                }
                match self.reclamation.release_delivered(project, todo) {
                    Ok(DeliveredRelease::Busy) => reported.owed(
                        &project.slug,
                        &what,
                        "another delivered release runs on this computer; it is asked again on a later invocation",
                    ),
                    Ok(DeliveredRelease::Done { released, kept }) => {
                        for resource in &released {
                            tracing::info!(event = "delivered_worktree_released", project = %project.slug, todo = %todo.todo, released = %resource);
                        }
                        if kept.is_empty() {
                            reported.settled(&project.slug, &what);
                        } else {
                            let causes: Vec<String> = kept
                                .iter()
                                .map(|(resource, cause)| format!("{resource}: {cause}"))
                                .collect();
                            reported.owed(&project.slug, &what, &causes.join("; "));
                        }
                    }
                    Err(error) => reported.owed(&project.slug, &what, &error.to_string()),
                }
            }
        }
    }

    /// Deletes each staging location in `dir` whose todo identity is one
    /// this home's work record holds for `project`, noting in `swept` what
    /// stays undeleted and a directory it could not read in full.
    fn sweep(&self, project: &ProjectRef, dir: &Path, reported: &mut Reported, swept: &mut Swept) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) => {
                reported.owed(
                    &project.slug,
                    &dir.display().to_string(),
                    &error.to_string(),
                );
                swept.unread.insert(dir.to_path_buf());
                return;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    swept.unread.insert(dir.to_path_buf());
                    // The rest of the directory may still be read.
                    reported.owed(
                        &project.slug,
                        &dir.display().to_string(),
                        &format!("an entry could not be read: {error}"),
                    );
                    continue;
                }
            };
            let Some(todo) = staged_for(&entry.file_name()) else {
                continue;
            };
            match self.reclamation.holds(project, &todo) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    reported.owed(&project.slug, "staging locations", &error.to_string());
                    swept.remaining.insert(todo);
                    continue;
                }
            }
            let staged = entry.path();
            let what = staged.display().to_string();
            match delete(&staged) {
                Ok(()) => reported.settled(&project.slug, &what),
                Err(error) => {
                    reported.owed(&project.slug, &what, &error.to_string());
                    swept.remaining.insert(todo);
                }
            }
        }
    }
}

/// What a deferred pass's sweep left: todos whose output stays at a staging
/// location, and directories it could not read in full.
#[derive(Debug, Default)]
struct Swept {
    remaining: BTreeSet<TodoId>,
    unread: BTreeSet<PathBuf>,
}

impl Swept {
    /// Whether `todo`'s worktree must stay so output staged beside it is
    /// found again: some of that output is undeleted, or the directory
    /// beside its worktree could not be read in full.
    fn holds_back(&self, todo: &crate::interfaces::TodoWorktree) -> bool {
        self.remaining.contains(&todo.identity)
            || todo
                .worktree
                .as_ref()
                .and_then(|worktree| worktree.parent())
                .is_some_and(|dir| self.unread.contains(dir))
    }
}

/// A session's request to reclaim the worktree its todo assignment records
/// (`instrukt repo reclaim`).
pub struct Request<'a> {
    pub project: &'a ProjectSlug,
    pub todo: &'a TodoId,
    pub worktree: &'a Path,
    pub session: SessionRef,
}

/// What reclaiming on request came to.
#[derive(Debug)]
pub enum OnRequest {
    /// The project is not published on this computer.
    NotPublished,
    /// The project declares no regenerable output.
    NothingDeclared,
    /// The project's declaration is not admitted on this computer: the
    /// findings that withhold it.
    Withheld(Vec<String>),
    /// Another session holds the worktree.
    Held(WorktreeHolder),
    /// The project's work record holds no such todo.
    NotRecorded,
    /// Each declared path removed, and each left with why.
    Reclaimed {
        removed: Vec<RegenerablePath>,
        left: Vec<(RegenerablePath, Left)>,
    },
}

/// Reclaiming on request could not finish.
#[derive(Debug, thiserror::Error)]
pub enum ReclaimError {
    #[error(transparent)]
    Admission(#[from] instrukt_core::config::AdmissionError),
    #[error(transparent)]
    Config(#[from] instrukt_core::config::ConfigError),
    #[error(transparent)]
    Work(#[from] WorkInterfaceError),
    #[error("the output moved out to {} could not be deleted: {source}", path.display())]
    Delete {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Reclaims the declared regenerable output of the worktree `request`
/// names, by the declaration admitted for its project from that project's
/// checkout on this computer: moved out inside work's section under the
/// session's admission, then deleted before this returns. It waits on no
/// runner work; a runner deleting one of the same staging locations meanwhile
/// only shortens it.
pub fn on_request(
    home: &instrukt_core::home::Home,
    request: &Request<'_>,
    work: &dyn WorktreeReclamation,
) -> Result<OnRequest, ReclaimError> {
    let candidate = instrukt_core::config::Admitted::load(home)?;
    let Some(project) = candidate
        .generation
        .projects()
        .iter()
        .find(|project| &project.slug == request.project)
    else {
        return Ok(OnRequest::NotPublished);
    };
    if !project.reclamation {
        let document = project
            .checkout
            .join(instrukt_core::config::CONFIG_FILE_NAME);
        return Ok(OnRequest::Withheld(
            candidate
                .findings
                .iter()
                .filter(|finding| {
                    finding.document == document
                        && instrukt_core::config::withholds_reclamation(finding)
                })
                .map(ToString::to_string)
                .collect(),
        ));
    }
    let declared = project.config.regenerable()?;
    let project = ProjectRef {
        slug: project.slug.clone(),
        root: project.checkout.clone(),
    };
    let target = Target {
        project: &project,
        todo: request.todo,
        worktree: request.worktree,
        admission: SectionAdmission::Session(request.session.clone()),
    };
    let staged = stage(&target, &declared, work)?;
    match staged.section {
        Some(SectionOutcome::Held(holder)) => return Ok(OnRequest::Held(holder)),
        Some(SectionOutcome::NotRecorded | SectionOutcome::RecordChanged) => {
            return Ok(OnRequest::NotRecorded);
        }
        Some(SectionOutcome::Ran) | None => {}
    }
    if declared.is_empty() {
        return Ok(OnRequest::NothingDeclared);
    }
    let mut removed = Vec::with_capacity(staged.moved.len());
    for (entry, location) in staged.moved {
        delete(&location).map_err(|source| ReclaimError::Delete {
            path: location.clone(),
            source,
        })?;
        removed.push(entry);
    }
    Ok(OnRequest::Reclaimed {
        removed,
        left: staged.left,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_staging_name_carries_the_identity_of_its_todo() {
        let name = OsStr::new(".instrukt-rg-0123456789abcdef0123456789abcdef-42-17-0");
        assert_eq!(
            staged_for(name),
            TodoId::parse("0123456789abcdef0123456789abcdef")
        );
    }

    #[test]
    fn a_name_with_the_prefix_and_an_identity_but_another_suffix_is_no_staging_location() {
        let name = OsStr::new(".instrukt-rg-0123456789abcdef0123456789abcdef-copy");
        assert_eq!(staged_for(name), None);
    }

    #[test]
    fn a_name_without_the_staging_prefix_is_no_staging_location() {
        assert_eq!(staged_for(OsStr::new(".instrukt-wd-0123-abc")), None);
    }
}
