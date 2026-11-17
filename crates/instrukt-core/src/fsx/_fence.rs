//! The write fence: locations this process refuses to write.
//!
//! The launching process names them in `INSTRUKT_WRITE_FENCE`, read once at
//! startup through the typed environment. A write whose target, with every
//! symbolic link resolved, lies under one of them stops the process before
//! anything is written: a distinctive error on stderr, a record of it in the
//! launcher's temporary directory and exit status [`EXIT`]. The fence only
//! refuses; it never redirects a write or permits one. Unset, it costs one
//! check of an empty cell per write.

use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// The variable naming the fenced locations, separated as `PATH` is.
pub const VARIABLE: &str = "INSTRUKT_WRITE_FENCE";
/// The exit status of a process that refused a write at its fence.
pub const EXIT: i32 = 86;
/// The file, in the launcher's temporary directory (`TMPDIR`), that keeps
/// one line per refused write.
pub const RECORD: &str = "instrukt-write-refusals";

/// How many symbolic links one path may pass through before it is taken
/// as written; the operating system refuses deeper chains itself.
const MAX_LINKS: usize = 40;

struct Fence {
    roots: Vec<PathBuf>,
    record: Option<PathBuf>,
}

static FENCE: OnceLock<Fence> = OnceLock::new();

/// Fences `roots` for the rest of this process, recording refusals under
/// `temporary`. Empty `roots` install nothing; a second installation
/// changes nothing.
pub fn install(roots: &[PathBuf], temporary: Option<&Path>) {
    if roots.is_empty() {
        return;
    }
    let mut fenced = Vec::new();
    for root in roots {
        for form in [lexical(&absolute(root)), resolved(root)] {
            if !fenced.contains(&form) {
                fenced.push(form);
            }
        }
    }
    let record = temporary
        .map(|dir| resolved(&dir.join(RECORD)))
        .filter(|record| !fenced.iter().any(|root| record.starts_with(root)));
    let _ = FENCE.set(Fence {
        roots: fenced,
        record,
    });
}

/// Admits a write to `path`, or ends the process when the path itself or
/// what it resolves to lies under a fenced location.
pub fn admit(path: &Path) {
    let Some(fence) = FENCE.get() else {
        return;
    };
    let absolute = absolute(path);
    // The entry itself (what a rename or removal changes) and, through a
    // final symbolic link, what a write into it changes.
    let entry = match (absolute.parent(), absolute.file_name()) {
        (Some(parent), Some(name)) => resolved(parent).join(name),
        _ => resolved(&absolute),
    };
    for target in [entry, resolved(&absolute)] {
        if let Some(root) = fence.roots.iter().find(|root| target.starts_with(root)) {
            refuse(fence, path, &target, root);
        }
    }
}

fn refuse(fence: &Fence, path: &Path, target: &Path, root: &Path) -> ! {
    let line = format!(
        "instrukt: write refused at the fence: {} resolves to {}, under {} which {VARIABLE} keeps off limits",
        path.display(),
        target.display(),
        root.display()
    );
    let _ = writeln!(std::io::stderr(), "{line}");
    if let Some(record) = &fence.record {
        #[allow(
            clippy::disallowed_methods,
            reason = "the record lies outside every fenced location"
        )]
        let opened = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(record);
        if let Ok(mut file) = opened {
            let _ = writeln!(file, "{line}");
        }
    }
    std::process::exit(EXIT)
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// `path` with `.` and `..` applied as written.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `path` made absolute with every symbolic link in it resolved, a dangling
/// one included; parts that do not exist are taken as written.
pub(super) fn resolved(path: &Path) -> PathBuf {
    let mut pending = absolute(path);
    let mut links = 0;
    loop {
        let mut out = PathBuf::new();
        let mut through = None;
        let components: Vec<Component<'_>> = pending.components().collect();
        for (at, component) in components.iter().enumerate() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                }
                Component::Normal(name) => {
                    out.push(name);
                    let is_link = std::fs::symlink_metadata(&out)
                        .is_ok_and(|meta| meta.file_type().is_symlink());
                    if is_link
                        && links < MAX_LINKS
                        && let Ok(target) = std::fs::read_link(&out)
                    {
                        out.pop();
                        let mut next = out.join(target);
                        next.extend(&components[at + 1..]);
                        through = Some(next);
                        break;
                    }
                }
                other => out.push(other),
            }
        }
        match through {
            Some(next) => {
                links += 1;
                pending = next;
            }
            None => return out,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        reason = "pure path resolution over a scratch tree"
    )]
    #![allow(clippy::disallowed_methods, reason = "builds the scratch tree")]
    use super::*;

    #[test]
    fn links_resolve_wherever_they_sit_and_dangling_ones_too() {
        let scratch = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(scratch.path()).unwrap();
        let fenced = base.join("live");
        std::fs::create_dir_all(fenced.join("dir")).unwrap();
        std::os::unix::fs::symlink(&fenced, base.join("to-live")).unwrap();
        std::os::unix::fs::symlink("live/missing", base.join("dir-link")).unwrap();
        assert_eq!(
            resolved(&base.join("to-live/dir/new.yaml")),
            fenced.join("dir/new.yaml")
        );
        assert_eq!(resolved(&base.join("dir-link")), fenced.join("missing"));
        assert_eq!(
            resolved(&base.join("elsewhere/../to-live/./dir")),
            fenced.join("dir")
        );
        assert_eq!(lexical(&base.join("a/../b")), base.join("b"));
    }
}
