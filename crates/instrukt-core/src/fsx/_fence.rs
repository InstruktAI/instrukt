//! The write fence: real locations this process refuses to write.
//!
//! The launching process names them in `INSTRUKT_WRITE_FENCE`, read once at
//! startup through the typed environment, together with a place of its own
//! where each refusal is recorded. A write whose target lies under a fenced
//! location, as the file system stands when the write is checked, is refused
//! before it happens: a line in the record, a line on stderr, a [`Refusal`]
//! error that travels through the operation's own error and rollback path,
//! and exit status [`EXIT`] when the binary ends. The record is the one way a
//! refusal reaches the launcher: a refusal the record cannot keep removes the
//! record instead, which the launcher reads as a failure just the same. The
//! fence only refuses; it never redirects a write or permits one. Unset, each
//! write costs one check of an empty cell.
//!
//! It is not a sandbox. Programs this process starts do not read it, and a
//! path that another process retargets between the check and the write is
//! outside what it stops.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write as _};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// The variable naming the fenced locations, separated as `PATH` is. One
/// entry may instead be `record=<directory>`: the launcher's directory
/// holding the [`RECORD`] file each refusal is appended to.
pub const VARIABLE: &str = "INSTRUKT_WRITE_FENCE";
/// The prefix of the entry naming the record's directory.
pub const RECORD_ENTRY: &str = "record=";
/// The record file in that directory. The launcher creates it; this process
/// only appends to it.
pub const RECORD: &str = "refusals";
/// The exit status of a process that refused a write at its fence.
pub const EXIT: u8 = 86;

/// How many symbolic links one path may pass through before it is taken
/// as written; the operating system refuses deeper chains itself.
const MAX_LINKS: usize = 40;

/// What [`VARIABLE`] says: the fenced locations and where refusals go.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Setting {
    pub roots: Vec<PathBuf>,
    pub record: Option<PathBuf>,
}

impl Setting {
    /// Reads the variable's value; a relative fenced entry is ignored, as
    /// it names no real location.
    pub fn parse(value: &str) -> Self {
        let mut setting = Self::default();
        for entry in std::env::split_paths(value) {
            match entry
                .to_str()
                .and_then(|text| text.strip_prefix(RECORD_ENTRY))
            {
                Some(record) => setting.record = Some(PathBuf::from(record)),
                None if entry.is_absolute() => setting.roots.push(entry),
                None => {}
            }
        }
        setting
    }
}

/// A write the fence refused.
#[derive(Debug, thiserror::Error)]
#[error(
    "write refused at the fence: {} resolves to {}, under {} which {VARIABLE} keeps off limits",
    path.display(),
    target.display(),
    root.display()
)]
pub struct Refusal {
    path: PathBuf,
    target: PathBuf,
    root: PathBuf,
}

/// The fence's record could not be opened, so nothing is run.
#[derive(Debug, thiserror::Error)]
#[error("the write fence's record {} cannot be used: {source}", path.display())]
pub struct RecordUnavailable {
    path: PathBuf,
    source: io::Error,
}

struct Fence {
    roots: Vec<PathBuf>,
    record: Option<Record>,
}

/// The record, open for appending, and the directory it was reached in.
struct Record {
    directory: rustix::fd::OwnedFd,
    file: Mutex<std::fs::File>,
}

static FENCE: OnceLock<Fence> = OnceLock::new();
static REFUSED: AtomicBool = AtomicBool::new(false);

/// Fences `setting` for the rest of this process. No roots install
/// nothing; a second installation changes nothing. A named record must
/// already exist as a regular file in a directory outside every fenced
/// location; it is opened here, once, without following a link, so a
/// refusal never looks a path up again.
pub fn install(setting: &Setting) -> Result<(), RecordUnavailable> {
    if setting.roots.is_empty() {
        return Ok(());
    }
    let record = match &setting.record {
        Some(directory) => Some(open_record(&setting.roots, directory)?),
        None => None,
    };
    let _ = FENCE.set(Fence {
        roots: setting.roots.clone(),
        record,
    });
    Ok(())
}

fn open_record(roots: &[PathBuf], directory: &Path) -> Result<Record, RecordUnavailable> {
    use rustix::fs::{Mode, OFlags};
    let path = directory.join(RECORD);
    let unavailable = |source: io::Error| RecordUnavailable {
        path: path.clone(),
        source,
    };
    let place = resolved(directory);
    let mut parents = Parents::default();
    let mut target = Target::new(&place);
    if let Some(root) = roots
        .iter()
        .find(|root| Root::at(root, &mut parents).holds(&mut target))
    {
        return Err(unavailable(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("it lies under the fenced {}", root.display()),
        )));
    }
    // The directory checked is reached again from the root one component at
    // a time, none of them followed through a link: the directory held open
    // is the one admitted, or the record is unavailable.
    let walk = OFlags::DIRECTORY | OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut dir =
        rustix::fs::open("/", walk, Mode::empty()).map_err(|error| unavailable(error.into()))?;
    for component in place.path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                dir = rustix::fs::openat(&dir, name, walk, Mode::empty())
                    .map_err(|error| unavailable(error.into()))?;
            }
            _ => {
                return Err(unavailable(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "its directory is not a plain absolute path",
                )));
            }
        }
    }
    // Opened relative to that directory, and never through a link, the
    // record cannot lead a refusal into a fenced location.
    let file = rustix::fs::openat(
        &dir,
        RECORD,
        OFlags::WRONLY | OFlags::APPEND | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| unavailable(error.into()))?;
    let file = std::fs::File::from(file);
    let meta = file.metadata().map_err(unavailable)?;
    if !meta.file_type().is_file() {
        return Err(unavailable(io::Error::new(
            io::ErrorKind::InvalidInput,
            "it is not a regular file",
        )));
    }
    Ok(Record {
        directory: dir,
        file: Mutex::new(file),
    })
}

/// Whether a fence is installed in this process.
pub fn active() -> bool {
    FENCE.get().is_some()
}

/// Whether this process refused a write at its fence.
pub fn refused() -> bool {
    REFUSED.load(Ordering::SeqCst)
}

/// Admits a write to `path`, or refuses it when the path itself or what
/// it resolves to lies under a fenced location.
///
/// Everything is resolved during this admission, as the file system stands
/// when the write is checked; what one admission needs more than once — each
/// fenced root, each parent directory, each target's case folding — is
/// resolved once and shared within it. The targets are checked in turn against the roots in their
/// setting's order, so the first that refuses names the refusal.
pub fn admit(path: &Path) -> io::Result<()> {
    let Some(fence) = FENCE.get() else {
        return Ok(());
    };
    let absolute = absolute(path);
    let mut parents = Parents::default();
    // The entry itself (what a rename or removal changes) and, through a
    // final symbolic link, what a write into it changes; the latter is
    // checked only where it leads somewhere else. Both share the parent's
    // resolution with the roots.
    let shared = split(&absolute).and_then(|(parent, name)| Some((parents.of(parent)?, name)));
    let (entry, whole) = match shared {
        Some((parent, name)) => (
            Resolved {
                path: parent.path.join(name),
                existing: parent.existing,
            },
            beneath(parent, name).unwrap_or_else(|| resolved(&absolute)),
        ),
        None => {
            let whole = resolved(&absolute);
            (entry(&absolute).unwrap_or_else(|| whole.clone()), whole)
        }
    };
    let roots: Vec<Root<'_>> = fence
        .roots
        .iter()
        .map(|root| Root::at(root, &mut parents))
        .collect();
    let second = (whole != entry).then_some(whole);
    for target in std::iter::once(entry).chain(second) {
        let mut target = Target::new(&target);
        if let Some(root) = roots.iter().find(|root| root.holds(&mut target)) {
            return Err(refuse(fence, path, &target.resolved.path, root.name));
        }
    }
    Ok(())
}

fn refuse(fence: &Fence, path: &Path, target: &Path, root: &Path) -> io::Error {
    REFUSED.store(true, Ordering::SeqCst);
    let refusal = Refusal {
        path: path.to_path_buf(),
        target: target.to_path_buf(),
        root: root.to_path_buf(),
    };
    let line = format!("instrukt: {refusal}");
    // The record first, before any diagnostic. A refusal it cannot keep
    // removes it, by its name in the directory reached at startup and never
    // through a link, so the launcher finds no record and fails just the
    // same; the refusal still returns through the refused operation.
    let unrecorded = fence.record.as_ref().and_then(|record| {
        let appended = record
            .file
            .lock()
            .map_err(|_| io::Error::other("the record's lock is poisoned"))
            .and_then(|mut file| writeln!(file, "{line}"));
        appended.err().map(|error| {
            let removed =
                rustix::fs::unlinkat(&record.directory, RECORD, rustix::fs::AtFlags::empty());
            (error, removed)
        })
    });
    let _ = writeln!(io::stderr(), "{line}");
    if let Some((error, removed)) = unrecorded {
        let outcome = match removed {
            Ok(()) => "the record was removed".to_owned(),
            Err(removal) => format!("the record could not be removed either: {removal}"),
        };
        let _ = writeln!(
            io::stderr(),
            "instrukt: the write fence's record could not keep this refusal: {error}; {outcome}"
        );
    }
    io::Error::new(io::ErrorKind::PermissionDenied, refusal)
}

/// Whether `error` is, or carries, a refusal at the fence.
pub fn is_refusal(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(error) = current {
        if error.is::<Refusal>()
            || error
                .downcast_ref::<io::Error>()
                .and_then(io::Error::get_ref)
                .is_some_and(|inner| inner.is::<Refusal>())
        {
            return true;
        }
        current = error.source();
    }
    false
}

/// A fenced root as the file system stands at one check: the root entry
/// itself, and where it leads, which differs from it for a fenced link.
struct Root<'a> {
    name: &'a Path,
    entry: Resolved,
    leads: Leads,
}

/// Where a fenced root leads, as far as one check needs it resolved.
enum Leads {
    /// Resolved as [`resolved`] resolves the root: a root that does not
    /// exist, a symbolic link, or one whose parent the walk decides.
    Place(Resolved),
    /// The root entry itself, which exists and is no link, under its
    /// resolved parent; only the name the file system stores for it is left
    /// unspelled. A target can lie under it only through that very directory
    /// entry, which no other entry of the parent shares this identity with.
    Unspelled(Identity),
}

impl<'a> Root<'a> {
    /// Resolves `name` now, for the check that uses it and no later one,
    /// sharing its parent's resolution with every root and target of the
    /// same admission that has the same parent.
    fn at(name: &'a Path, parents: &mut Parents) -> Self {
        let absolute = absolute(name);
        if let Some((parent, file)) = split(&absolute)
            && let Some(parent) = parents.of(parent)
        {
            let entry = Resolved {
                path: parent.path.join(file),
                existing: parent.existing,
            };
            // The walk's next step from that parent: a name that is missing,
            // or under a missing parent, is taken as written; a link is
            // followed as the walk follows it.
            let leads = if !parent.whole() {
                Leads::Place(entry.clone())
            } else {
                match std::fs::symlink_metadata(&entry.path) {
                    Err(_) => Leads::Place(entry.clone()),
                    Ok(meta) if meta.file_type().is_symlink() => Leads::Place(resolved(name)),
                    Ok(meta) => Leads::Unspelled(Identity::of(&meta)),
                }
            };
            return Self { name, entry, leads };
        }
        let leads = resolved(name);
        let entry = entry(&absolute).unwrap_or_else(|| leads.clone());
        Self {
            name,
            entry,
            leads: Leads::Place(leads),
        }
    }

    /// Whether `target` lies under this root: under the root entry itself
    /// or under where it leads. An unspelled root is spelled, during this
    /// check, only for a target that may pass through its entry, so every
    /// target that lies under it meets the full comparison.
    fn holds(&self, target: &mut Target<'_>) -> bool {
        if under(target.resolved, &self.entry, target.fold) {
            return true;
        }
        match &self.leads {
            Leads::Place(place) => under(target.resolved, place, target.fold),
            Leads::Unspelled(identity) => {
                self.may_pass_through(target, *identity)
                    && under(target.resolved, &resolved(self.name), target.fold)
            }
        }
    }

    /// Whether `target` may pass through this existing root entry: false
    /// only where the target's components before it differ from its parent's,
    /// or where the target exists to its depth as another directory entry,
    /// whose stored name then differs from the root's.
    fn may_pass_through(&self, target: &mut Target<'_>, identity: Identity) -> bool {
        let depth = self.entry.path.components().count();
        let Some(parent) = self.entry.path.parent() else {
            return true;
        };
        if target.resolved.path.components().count() < depth
            || !under_path(target.resolved, parent, depth, target.fold)
        {
            return false;
        }
        target.resolved.existing < depth
            || target
                .identity_at(depth)
                .is_none_or(|found| found == identity)
    }
}

/// A directory entry's identity: its device and inode, as `lstat` reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
}

impl Identity {
    fn of(meta: &std::fs::Metadata) -> Self {
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }
}

/// The parent directories one admission resolves, each once, by
/// [`from_existing`]; `None` where the walk must decide.
#[derive(Default)]
struct Parents(HashMap<PathBuf, Option<Resolved>>);

impl Parents {
    fn of(&mut self, parent: &Path) -> Option<&Resolved> {
        if !self.0.contains_key(parent) {
            self.0.insert(parent.to_path_buf(), from_existing(parent));
        }
        self.0.get(parent).and_then(Option::as_ref)
    }
}

/// One target of a check, with what the check looks up about it at most
/// once: whether its volume folds case, and its existing prefixes' identities.
struct Target<'t> {
    resolved: &'t Resolved,
    fold: bool,
    identities: HashMap<usize, Option<Identity>>,
}

impl<'t> Target<'t> {
    fn new(resolved: &'t Resolved) -> Self {
        Self {
            resolved,
            fold: resolved.folds_case(),
            identities: HashMap::new(),
        }
    }

    /// The identity of the entry its first `depth` components name.
    fn identity_at(&mut self, depth: usize) -> Option<Identity> {
        let path = &self.resolved.path;
        *self.identities.entry(depth).or_insert_with(|| {
            let prefix: PathBuf = path.components().take(depth).collect();
            std::fs::symlink_metadata(prefix)
                .ok()
                .map(|meta| Identity::of(&meta))
        })
    }
}

/// A plain absolute path's parent and final name; `None` for any other.
fn split(absolute: &Path) -> Option<(&Path, &OsStr)> {
    if !plain(absolute) {
        return None;
    }
    Some((absolute.parent()?, absolute.file_name()?))
}

/// The entry an absolute path names, without following it when it is a
/// symbolic link: its parent resolved and its own name as written. `None`
/// for a path with no parent or name.
fn entry(absolute: &Path) -> Option<Resolved> {
    let (parent, name) = (absolute.parent()?, absolute.file_name()?);
    let parent = resolved(parent);
    Some(Resolved {
        existing: parent.existing,
        path: parent.path.join(name),
    })
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// A path as the file system stands: its first `existing` components exist
/// and carry the file system's own spelling, with every symbolic link in
/// them resolved; the rest do not exist and are taken as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Resolved {
    pub(super) path: PathBuf,
    pub(super) existing: usize,
}

impl Resolved {
    /// Whether every component of the path exists.
    fn whole(&self) -> bool {
        self.existing == self.path.components().count()
    }

    /// Whether the volume holding its existing part treats differently
    /// cased names as one, seen by looking an existing name up in another
    /// case.
    fn folds_case(&self) -> bool {
        let existing: PathBuf = self.path.components().take(self.existing).collect();
        for dir in existing.ancestors() {
            let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) else {
                continue;
            };
            let Some(other) = recase(name) else {
                continue;
            };
            let (Ok(here), Ok(there)) = (
                std::fs::symlink_metadata(dir),
                std::fs::symlink_metadata(parent.join(&other)),
            ) else {
                return false;
            };
            return here.dev() == there.dev() && here.ino() == there.ino();
        }
        false
    }
}

/// `name` with its first cased letter in the other case.
fn recase(name: &OsStr) -> Option<OsString> {
    let text = name.to_str()?;
    let (at, letter) = text
        .char_indices()
        .find(|(_, c)| c.is_lowercase() || c.is_uppercase())?;
    let other: String = if letter.is_lowercase() {
        letter.to_uppercase().collect()
    } else {
        letter.to_lowercase().collect()
    };
    Some(OsString::from(format!(
        "{}{other}{}",
        &text[..at],
        &text[at + letter.len_utf8()..]
    )))
}

/// Whether `target` lies at or under `root`. Existing components compare as
/// the file system spells them; where either side does not exist yet, a
/// volume that folds case compares the names without regard to case.
pub(super) fn under(target: &Resolved, root: &Resolved, fold: bool) -> bool {
    under_path(target, &root.path, root.existing, fold)
}

/// [`under`] for a root given as its path and how many of its components
/// exist.
fn under_path(target: &Resolved, root: &Path, root_existing: usize, fold: bool) -> bool {
    let mut target_parts = target.path.components();
    root.components().enumerate().all(|(at, r)| {
        target_parts.next().is_some_and(|t| {
            r == t
                || (fold
                    && (at >= target.existing || at >= root_existing)
                    && same_ignoring_case(r.as_os_str(), t.as_os_str()))
        })
    })
}

fn same_ignoring_case(a: &OsStr, b: &OsStr) -> bool {
    match (a.to_str(), b.to_str()) {
        (Some(a), Some(b)) => a.to_lowercase() == b.to_lowercase(),
        _ => a == b,
    }
}

/// `path` made absolute and resolved as the file system stands: each
/// existing component in the file system's own spelling, every symbolic
/// link followed (a dangling one included), and parts that do not exist
/// taken as written. A path that exists whole is resolved by the operating
/// system in one step, and a plain path (the root and names only) from its
/// deepest ancestor that exists whole, each with the same result as the walk
/// below; any other path is walked a component at a time.
pub(super) fn resolved(path: &Path) -> Resolved {
    let mut pending = absolute(path);
    if plain(&pending) {
        if let Some(found) = from_existing(&pending) {
            return found;
        }
    } else if let Ok(path) = std::fs::canonicalize(&pending) {
        return Resolved {
            existing: path.components().count(),
            path,
        };
    }
    let mut links = 0;
    loop {
        let mut out = PathBuf::new();
        let mut existing = 0;
        let mut exists = true;
        let mut through = None;
        let components: Vec<Component<'_>> = pending.components().collect();
        for (at, component) in components.iter().enumerate() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                    existing = existing.min(out.components().count());
                }
                Component::Normal(name) => {
                    let next = out.join(name);
                    if !exists {
                        out = next;
                        continue;
                    }
                    match std::fs::symlink_metadata(&next) {
                        Ok(meta) if meta.file_type().is_symlink() && links < MAX_LINKS => {
                            if let Ok(target) = std::fs::read_link(&next) {
                                let mut redirected = out.join(target);
                                redirected.extend(&components[at + 1..]);
                                through = Some(redirected);
                                break;
                            }
                            out = next;
                            exists = false;
                        }
                        Ok(_) => {
                            out = std::fs::canonicalize(&next).unwrap_or(next);
                            existing = out.components().count();
                        }
                        Err(_) => {
                            out = next;
                            exists = false;
                        }
                    }
                }
                other => {
                    out.push(other);
                    if exists {
                        existing = out.components().count();
                    }
                }
            }
        }
        match through {
            Some(next) => {
                links += 1;
                pending = next;
            }
            None => {
                return Resolved {
                    path: out,
                    existing,
                };
            }
        }
    }
}

/// Whether `path` holds only the root and plain names: no `..`, which the
/// walk applies only after resolving what precedes it.
fn plain(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
}

/// A plain absolute path resolved from its deepest ancestor that exists
/// whole: that ancestor in one step, then each further name as the walk
/// takes it. `None` when a symbolic link lies past that ancestor, where the
/// walk alone decides.
fn from_existing(path: &Path) -> Option<Resolved> {
    if let Ok(found) = std::fs::canonicalize(path) {
        return Some(Resolved {
            existing: found.components().count(),
            path: found,
        });
    }
    let (parent, name) = (path.parent()?, path.file_name()?);
    beneath(&from_existing(parent)?, name)
}

/// `name` in the already resolved `parent`, exactly as the walk continues
/// there: taken as written once something is missing, followed into the file
/// system's own spelling where it exists and is no link. `None` for a
/// symbolic link, whose step depends on how many links the walk has passed.
fn beneath(parent: &Resolved, name: &OsStr) -> Option<Resolved> {
    let next = parent.path.join(name);
    let as_written = |path| {
        Some(Resolved {
            path,
            existing: parent.existing,
        })
    };
    if !parent.whole() {
        return as_written(next);
    }
    match std::fs::symlink_metadata(&next) {
        Err(_) => as_written(next),
        Ok(meta) if meta.file_type().is_symlink() => None,
        Ok(_) => {
            let path = std::fs::canonicalize(&next).unwrap_or(next);
            Some(Resolved {
                existing: path.components().count(),
                path,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(path: &str, existing: usize) -> Resolved {
        Resolved {
            path: PathBuf::from(path),
            existing,
        }
    }

    #[test]
    fn links_resolve_wherever_they_sit_and_dangling_ones_too() {
        let scratch = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(scratch.path()).unwrap();
        let fenced = base.join("live");
        std::fs::create_dir_all(fenced.join("dir")).unwrap();
        std::os::unix::fs::symlink(&fenced, base.join("to-live")).unwrap();
        std::os::unix::fs::symlink("live/missing", base.join("dir-link")).unwrap();
        assert_eq!(
            resolved(&base.join("to-live/dir/new.yaml")).path,
            fenced.join("dir/new.yaml")
        );
        assert_eq!(
            resolved(&base.join("dir-link")).path,
            fenced.join("missing")
        );
        assert_eq!(
            resolved(&base.join("live/dir/../../to-live/./dir")).path,
            fenced.join("dir")
        );
    }

    #[test]
    fn a_path_counts_as_existing_as_far_as_the_file_system_holds_it() {
        let scratch = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(scratch.path()).unwrap();
        let dir = base.join("live/dir");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(base.join("live"), base.join("to-live")).unwrap();
        let depth = dir.components().count();
        assert_eq!(
            resolved(&base.join("to-live/dir/../dir")),
            Resolved {
                path: dir.clone(),
                existing: depth,
            }
        );
        assert_eq!(
            resolved(&base.join("to-live/dir/../dir/new/file.yaml")),
            Resolved {
                path: dir.join("new/file.yaml"),
                existing: depth,
            }
        );
    }

    #[test]
    fn a_path_whose_last_names_do_not_exist_resolves_from_its_deepest_existing_directory() {
        let scratch = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(scratch.path()).unwrap();
        let dir = base.join("live/dir");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&dir, base.join("to-dir")).unwrap();
        assert_eq!(
            resolved(&base.join("to-dir/new/file.yaml")),
            Resolved {
                path: dir.join("new/file.yaml"),
                existing: dir.components().count(),
            }
        );
    }

    /// Whether the fenced `root` holds `target`, as one check finds them.
    fn holds(root: &Path, target: &Path) -> bool {
        let target = resolved(target);
        Root::at(root, &mut Parents::default()).holds(&mut Target::new(&target))
    }

    #[test]
    fn a_fenced_link_holds_what_lies_where_it_leads() {
        let scratch = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(scratch.path()).unwrap();
        std::fs::create_dir(base.join("t")).unwrap();
        std::fs::create_dir(base.join("u")).unwrap();
        std::os::unix::fs::symlink(base.join("t"), base.join("l")).unwrap();
        assert!(holds(&base.join("l"), &base.join("t/new")));
        assert!(!holds(&base.join("l"), &base.join("u/new")));
    }

    #[test]
    fn a_fenced_entry_holds_only_what_passes_through_that_entry() {
        let scratch = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(scratch.path()).unwrap();
        std::fs::write(base.join("b"), "").unwrap();
        std::fs::hard_link(base.join("b"), base.join("a")).unwrap();
        std::fs::create_dir(base.join("d")).unwrap();
        std::fs::create_dir(base.join("e")).unwrap();
        assert!(holds(&base.join("b"), &base.join("b")));
        assert!(!holds(&base.join("b"), &base.join("a")));
        assert!(holds(&base.join("d"), &base.join("d/new")));
        assert!(!holds(&base.join("d"), &base.join("e/new")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_fenced_root_spelled_otherwise_than_stored_holds_what_lies_inside_it() {
        let scratch = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(scratch.path()).unwrap();
        let composed = base.join("caf\u{e9}");
        std::fs::create_dir(&composed).unwrap();
        assert!(holds(&base.join("cafe\u{301}"), &composed.join("new")));
    }

    #[test]
    fn names_that_do_not_exist_yet_compare_without_case_only_where_the_volume_folds_it() {
        let root = at("/home/person/.instrukt", 3);
        let other_case = at("/home/person/.INSTRUKT/locks", 3);
        assert!(under(&other_case, &root, true));
        assert!(!under(&other_case, &root, false));
        // Existing components carry the file system's own spelling, so they
        // compare exactly.
        assert!(!under(&at("/home/Person/.instrukt", 4), &root, true));
        assert!(under(&at("/home/person/.instrukt/locks", 4), &root, false));
        assert!(!under(&at("/home/person", 3), &root, true));
    }

    #[test]
    fn the_setting_names_its_record_beside_the_fenced_locations() {
        let setting = Setting::parse("/live/home:record=/test/fence:relative/entry:/live/.t3");
        assert_eq!(
            setting,
            Setting {
                roots: vec![PathBuf::from("/live/home"), PathBuf::from("/live/.t3")],
                record: Some(PathBuf::from("/test/fence")),
            }
        );
    }
}
