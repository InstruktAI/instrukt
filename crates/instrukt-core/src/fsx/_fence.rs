//! The write fence: real locations this process refuses to write.
//!
//! The launching process names them in `INSTRUKT_WRITE_FENCE`, read once at
//! startup through the typed environment, together with a place of its own
//! where each refusal is recorded. A write whose target lies under a fenced
//! location, as the file system stands when the write is checked, is refused
//! before it happens: a [`Refusal`] error that travels through the operation's
//! own error and rollback path, a line on stderr, a line in the record, and
//! exit status [`EXIT`] when the binary ends. The fence only refuses; it never
//! redirects a write or permits one. Unset, each write costs one check of an
//! empty cell.
//!
//! It is not a sandbox. Programs this process starts do not read it, and a
//! path that another process retargets between the check and the write is
//! outside what it stops.

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
    record: Option<Mutex<std::fs::File>>,
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
        Some(directory) => Some(Mutex::new(open_record(&setting.roots, directory)?)),
        None => None,
    };
    let _ = FENCE.set(Fence {
        roots: setting.roots.clone(),
        record,
    });
    Ok(())
}

fn open_record(roots: &[PathBuf], directory: &Path) -> Result<std::fs::File, RecordUnavailable> {
    use rustix::fs::{Mode, OFlags};
    let path = directory.join(RECORD);
    let unavailable = |source: io::Error| RecordUnavailable {
        path: path.clone(),
        source,
    };
    let place = resolved(directory);
    if let Some(root) = roots.iter().find(|root| fenced(&place, root)) {
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
    Ok(file)
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
pub fn admit(path: &Path) -> io::Result<()> {
    let Some(fence) = FENCE.get() else {
        return Ok(());
    };
    let absolute = absolute(path);
    // The entry itself (what a rename or removal changes) and, through a
    // final symbolic link, what a write into it changes.
    let entry = match (absolute.parent(), absolute.file_name()) {
        (Some(parent), Some(name)) => {
            let parent = resolved(parent);
            Resolved {
                existing: parent.existing,
                path: parent.path.join(name),
            }
        }
        _ => resolved(&absolute),
    };
    for target in [entry, resolved(&absolute)] {
        if let Some(root) = fence.roots.iter().find(|root| fenced(&target, root)) {
            return Err(refuse(fence, path, &target.path, root));
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
    let _ = writeln!(io::stderr(), "{line}");
    if let Some(record) = &fence.record {
        let appended = record
            .lock()
            .map_err(|_| io::Error::other("the record's lock is poisoned"))
            .and_then(|mut file| writeln!(file, "{line}"));
        // A refusal the record cannot keep ends the process at once with the
        // fence's status, so the launcher sees it whatever else happens.
        if let Err(error) = appended {
            let _ = writeln!(
                io::stderr(),
                "instrukt: the write fence's record failed: {error}"
            );
            std::process::exit(i32::from(EXIT));
        }
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

/// Whether `target` lies under the fenced `root`, both as the file system
/// stands now: under where the root leads, or, for a fenced link, under the
/// link itself.
fn fenced(target: &Resolved, root: &Path) -> bool {
    let fold = target.folds_case();
    let leads = resolved(root);
    let absolute = absolute(root);
    let entry = match (absolute.parent(), absolute.file_name()) {
        (Some(parent), Some(name)) => {
            let parent = resolved(parent);
            Resolved {
                existing: parent.existing,
                path: parent.path.join(name),
            }
        }
        _ => leads.clone(),
    };
    under(target, &leads, fold) || under(target, &entry, fold)
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
    let target_parts: Vec<Component<'_>> = target.path.components().collect();
    let root_parts: Vec<Component<'_>> = root.path.components().collect();
    root_parts.len() <= target_parts.len()
        && root_parts
            .iter()
            .zip(&target_parts)
            .enumerate()
            .all(|(at, (r, t))| {
                r == t
                    || (fold
                        && (at >= target.existing || at >= root.existing)
                        && same_ignoring_case(r.as_os_str(), t.as_os_str()))
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
/// taken as written.
pub(super) fn resolved(path: &Path) -> Resolved {
    let mut pending = absolute(path);
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
