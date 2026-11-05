//! The paths this user's running processes hold: working directories,
//! executables and open files. A process runs as this user when its
//! effective user is this user; a process of another user is outside what
//! this user may inspect. Any such process whose holdings cannot all be
//! read makes the snapshot incomplete, and every checkout then counts as
//! in use.

use std::path::{Path, PathBuf};

/// What the processes hold, or why that could not be fully read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Snapshot {
    Complete(Vec<PathBuf>),
    /// Why some process's holdings could not be read.
    Incomplete(String),
}

impl Snapshot {
    /// Whether any held path lies inside `checkout`; `None` when the
    /// snapshot is incomplete.
    pub fn holds(&self, checkout: &Path) -> Option<bool> {
        match self {
            Snapshot::Complete(paths) => Some(paths.iter().any(|path| path.starts_with(checkout))),
            Snapshot::Incomplete(_) => None,
        }
    }
}

/// Whether the process `pid` is still running.
fn running(pid: i32) -> bool {
    match rustix::process::Pid::from_raw(pid) {
        Some(pid) => !matches!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        ),
        None => false,
    }
}

/// Reads what this user's processes hold.
pub fn snapshot() -> Snapshot {
    let uid = rustix::process::geteuid().as_raw();
    platform::snapshot(uid)
}

#[cfg(target_os = "linux")]
mod platform {
    //! Linux: each process's `/proc/<pid>` entries.

    use std::io::ErrorKind;
    use std::path::PathBuf;

    use super::{Snapshot, running};

    /// The link at `path` as a held path; `Ok(None)` when it names no
    /// file (a socket, a pipe) or is gone.
    fn held(path: &std::path::Path) -> std::io::Result<Option<PathBuf>> {
        match std::fs::read_link(path) {
            Ok(target) => {
                let text = target.to_string_lossy();
                let text = text.strip_suffix(" (deleted)").unwrap_or(text.as_ref());
                Ok(text.starts_with('/').then(|| PathBuf::from(text)))
            }
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub(super) fn snapshot(uid: u32) -> Snapshot {
        let entries = match std::fs::read_dir("/proc") {
            Ok(entries) => entries,
            Err(err) => return Snapshot::Incomplete(format!("cannot list processes: {err}")),
        };
        let mut paths = Vec::new();
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<i32>().ok())
            else {
                continue;
            };
            let dir = entry.path();
            let Ok(status) = std::fs::read_to_string(dir.join("status")) else {
                if running(pid) {
                    return Snapshot::Incomplete(format!("cannot read process {pid}"));
                }
                continue;
            };
            let effective = status
                .lines()
                .find_map(|line| line.strip_prefix("Uid:"))
                .and_then(|ids| ids.split_whitespace().nth(1))
                .and_then(|id| id.parse::<u32>().ok());
            if effective != Some(uid) {
                continue;
            }
            let mut read = || -> std::io::Result<()> {
                paths.extend(held(&dir.join("cwd"))?);
                paths.extend(held(&dir.join("exe"))?);
                match std::fs::read_dir(dir.join("fd")) {
                    Ok(fds) => {
                        for fd in fds {
                            paths.extend(held(&fd?.path())?);
                        }
                    }
                    Err(err) if err.kind() == ErrorKind::NotFound => {}
                    Err(err) => return Err(err),
                }
                Ok(())
            };
            if let Err(err) = read()
                && running(pid)
            {
                return Snapshot::Incomplete(format!(
                    "cannot read what process {pid} holds: {err}"
                ));
            }
        }
        Snapshot::Complete(paths)
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    //! macOS: the processes `ps` lists, read through `lsof`.

    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{Snapshot, running};
    use crate::process::Invocation;

    const PS: &str = "/bin/ps";
    const LSOF: &str = "/usr/sbin/lsof";
    const TIMEOUT: Duration = Duration::from_secs(60);

    /// One process `lsof` reported: its paths, and whether every record
    /// was read.
    #[derive(Debug, Default, PartialEq, Eq)]
    pub(super) struct Listed {
        pub(super) paths: Vec<PathBuf>,
        pub(super) fully_read: bool,
    }

    /// `lsof` types of descriptors backed by a filesystem object.
    const FILE_TYPES: [&str; 6] = ["FIFO", "CHR", "DIR", "BLK", "LINK", "REG"];

    fn file_backed(kind: &str) -> bool {
        FILE_TYPES.contains(&kind)
            || (kind.len() == 4 && kind.chars().all(|c| ('0'..='7').contains(&c)))
    }

    /// One descriptor record: its type and name fields, when present.
    #[derive(Default)]
    struct Record {
        kind: Option<String>,
        name: Option<String>,
    }

    /// Adds `record` to `process`: a path it names, or the mark that it
    /// was not read.
    fn close(process: Option<&mut Listed>, record: Option<Record>) {
        let (Some(process), Some(record)) = (process, record) else {
            return;
        };
        let name = record.name.unwrap_or_default();
        match record.kind {
            Some(_) if name.starts_with('/') => process.paths.push(PathBuf::from(name)),
            Some(kind) if !file_backed(&kind) => {}
            _ => process.fully_read = false,
        }
    }

    /// Folds `lsof -Fpftn` output into the processes it reports. A record
    /// with no type, or a file-backed record without an absolute path, is
    /// one `lsof` could not read.
    pub(super) fn parse(output: &str) -> BTreeMap<i32, Listed> {
        let mut processes: BTreeMap<i32, Listed> = BTreeMap::new();
        let mut current: Option<i32> = None;
        let mut record: Option<Record> = None;
        for line in output.lines() {
            let (field, value) = line.split_at(line.len().min(1));
            match field {
                "p" | "f" => {
                    close(
                        current.and_then(|pid| processes.get_mut(&pid)),
                        record.take(),
                    );
                    if field == "p" {
                        current = value.parse().ok();
                        if let Some(pid) = current {
                            processes.entry(pid).or_insert(Listed {
                                paths: Vec::new(),
                                fully_read: true,
                            });
                        }
                    } else {
                        record = Some(Record::default());
                    }
                }
                "t" => {
                    if let Some(record) = record.as_mut() {
                        record.kind = Some(value.to_owned());
                    }
                }
                "n" => {
                    if let Some(record) = record.as_mut() {
                        record.name = Some(value.to_owned());
                    }
                }
                _ => {}
            }
        }
        close(
            current.and_then(|pid| processes.get_mut(&pid)),
            record.take(),
        );
        processes
    }

    pub(super) fn snapshot(uid: u32) -> Snapshot {
        let listing = match Invocation::program(PS, &["-A", "-o", "pid=,uid="]).run(TIMEOUT) {
            Ok(done) if done.success => done.stdout,
            Ok(done) => {
                return Snapshot::Incomplete(format!(
                    "cannot list processes: {}",
                    done.stderr.trim()
                ));
            }
            Err(err) => return Snapshot::Incomplete(format!("cannot list processes: {err}")),
        };
        let mine: BTreeSet<i32> = listing
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let pid = fields.next()?.parse::<i32>().ok()?;
                let owner = fields.next()?.parse::<u32>().ok()?;
                (owner == uid).then_some(pid)
            })
            .collect();
        if mine.is_empty() {
            return Snapshot::Complete(Vec::new());
        }
        let pids = mine
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        // `lsof` exits non-zero when a listed process is gone; what it
        // printed is still what it read.
        let listed = match Invocation::program(LSOF, &["-w", "-n", "-P", "-p", &pids, "-Fpftn"])
            .run(TIMEOUT)
        {
            Ok(done) => parse(&done.stdout),
            Err(err) => return Snapshot::Incomplete(format!("cannot run lsof: {err}")),
        };
        let mut paths = Vec::new();
        for pid in mine {
            match listed.get(&pid) {
                Some(process) if process.fully_read => paths.extend(process.paths.iter().cloned()),
                _ if !running(pid) => {}
                _ => {
                    return Snapshot::Incomplete(format!("cannot read what process {pid} holds"));
                }
            }
        }
        Snapshot::Complete(paths)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_process_whose_records_all_name_paths_or_non_files_is_fully_read() {
            let listed = parse("p10\nfcwd\ntDIR\nn/work/a\nf3\ntIPv4\nnlocalhost:80\n");
            assert_eq!(
                listed.get(&10),
                Some(&Listed {
                    paths: vec![PathBuf::from("/work/a")],
                    fully_read: true,
                })
            );
        }

        #[test]
        fn a_record_without_a_type_marks_its_process_unread() {
            let listed = parse("p10\nfcwd\nn/work/a (readlink: Permission denied)\n");
            assert!(!listed[&10].fully_read);
        }

        #[test]
        fn a_file_record_without_an_absolute_path_marks_its_process_unread() {
            let listed = parse("p10\nf4\ntREG\nncan't read vnode\n");
            assert!(!listed[&10].fully_read);
        }
    }
}
