//! The test harness for instrukt's command-line tests.
//!
//! Every test runs the real `instrukt` binary against a temporary `HOME`
//! and real git repositories inside a temporary directory
//! (`docs/project/policy/testing.md`). [`TestEnv`] owns that directory:
//!
//! - subprocesses (instrukt and git) run with a cleared environment: `PATH`,
//!   the temporary `HOME`, an isolated git configuration and whatever the
//!   test sets with [`TestEnv::set_env`] — no live session variable leaks in;
//! - on drop it fails the test when anything on the live machine that
//!   instrukt could write — `~/.instrukt`, the agent CLIs' entrypoints and
//!   settings, the user's git configuration — changed while it ran.
//!
//! [`github::FakeGitHub`] (or [`TestEnv::with_fake_github`]) stands in for
//! GitHub's API at the network line for the work, workflow and runner
//! tests.
#![allow(
    dead_code,
    reason = "each test crate uses the part of the harness it needs"
)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test harness: a failed setup step fails the test"
)]

pub mod github;

/// `text` with every published commit a source line names
/// (`published commit <sha>`) shown as `[SHA]`: commits made in a test
/// differ from run to run.
pub fn mask_published_commits(text: &str) -> String {
    const MARK: &str = "published commit ";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(MARK) {
        let after = at + MARK.len();
        out.push_str(&rest[..after]);
        let hex = rest[after..]
            .bytes()
            .take_while(u8::is_ascii_hexdigit)
            .count();
        if hex == 40 {
            out.push_str("[SHA]");
            rest = &rest[after + hex..];
        } else {
            rest = &rest[after..];
        }
    }
    out.push_str(rest);
    out
}

/// The repository root of the published position `domain` addresses:
/// `docs/general` for `general`, `docs/domain/<domain>` for any other.
pub fn position_dir(domain: &str) -> String {
    if domain == "general" {
        "docs/general".to_owned()
    } else {
        format!("docs/domain/{domain}")
    }
}

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::SystemTime;

use assert_cmd::Command;
use assert_cmd::assert::Assert;
use tempfile::TempDir;

/// The git identity every test's git configuration carries.
const DEVELOPER_NAME: &str = "Test Developer";
const DEVELOPER_EMAIL: &str = "developer@example.test";

/// A temporary machine: a user home, a place for repositories, and the
/// environment every subprocess of the test runs with.
pub struct TestEnv {
    root: TempDir,
    home: PathBuf,
    vars: BTreeMap<String, String>,
    guard: LiveGuard,
}

impl TestEnv {
    pub fn new() -> Self {
        let guard = LiveGuard::over_live_machine();
        let root = tempfile::Builder::new()
            .prefix("instrukt-test-")
            .tempdir()
            .unwrap();
        // Canonical path: macOS's /var → /private/var would otherwise make
        // paths the binary reports differ from the ones the test built.
        let base = root.path().canonicalize().unwrap();
        let home = base.join("home");
        let tmp = base.join("tmp");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            home.join(".gitconfig"),
            format!(
                "[user]\n\tname = {DEVELOPER_NAME}\n\temail = {DEVELOPER_EMAIL}\n\
                 [init]\n\tdefaultBranch = main\n[commit]\n\tgpgsign = false\n\
                 [tag]\n\tgpgsign = false\n"
            ),
        )
        .unwrap();
        let mut vars = BTreeMap::new();
        vars.insert("PATH".into(), std::env::var("PATH").unwrap_or_default());
        vars.insert("HOME".into(), home.display().to_string());
        vars.insert("TMPDIR".into(), tmp.display().to_string());
        vars.insert(
            "XDG_CONFIG_HOME".into(),
            home.join(".config").display().to_string(),
        );
        vars.insert(
            "GIT_CONFIG_GLOBAL".into(),
            home.join(".gitconfig").display().to_string(),
        );
        vars.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
        vars.insert("LANG".into(), "C.UTF-8".into());
        Self {
            root,
            home,
            vars,
            guard,
        }
    }

    /// The temporary user home (`HOME` of every subprocess).
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// `$HOME/.instrukt` of this test.
    pub fn instrukt_home(&self) -> PathBuf {
        self.home.join(".instrukt")
    }

    /// A path inside the test's directory, outside `HOME`.
    pub fn path(&self, relative: &str) -> PathBuf {
        self.base().join(relative)
    }

    /// Sets an environment variable for every later subprocess, e.g. a
    /// runtime's session variable.
    pub fn set_env(&mut self, key: &str, value: &str) -> &mut Self {
        self.vars.insert(key.to_owned(), value.to_owned());
        self
    }

    pub fn remove_env(&mut self, key: &str) -> &mut Self {
        self.vars.remove(key);
        self
    }

    /// A new git repository at `<test dir>/<name>` on branch `main`, with
    /// no commits.
    pub fn git_repo(&self, name: &str) -> Repo {
        let path = self.base().join(name);
        std::fs::create_dir_all(&path).unwrap();
        let repo = Repo {
            path,
            vars: self.vars.clone(),
        };
        repo.git(["init", "--quiet", "--initial-branch=main"]);
        repo
    }

    /// A new bare repository at `<test dir>/<name>.git`, to push to and
    /// clone from.
    pub fn bare_remote(&self, name: &str) -> PathBuf {
        let path = self.base().join(format!("{name}.git"));
        std::fs::create_dir_all(&path).unwrap();
        run_git(
            &self.vars,
            &path,
            ["init", "--quiet", "--bare", "--initial-branch=main"],
        );
        path
    }

    /// Clones `remote` into `<test dir>/<name>`.
    pub fn clone_repo(&self, remote: &Path, name: &str) -> Repo {
        let path = self.base().join(name);
        run_git(
            &self.vars,
            &self.base(),
            [
                OsStr::new("clone"),
                OsStr::new("--quiet"),
                remote.as_os_str(),
                path.as_os_str(),
            ],
        );
        Repo {
            path,
            vars: self.vars.clone(),
        }
    }

    /// The built `instrukt` binary, ready to run in `cwd` with this test's
    /// environment.
    pub fn instrukt(&self, cwd: &Path) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_instrukt"));
        cmd.env_clear().envs(&self.vars).current_dir(cwd);
        cmd
    }

    /// `git` ready to run in `dir` with this test's environment, for a test
    /// that bounds how long git may take (`.timeout(…)`).
    pub fn git_command(&self, dir: &Path) -> Command {
        let mut cmd = Command::new("git");
        cmd.env_clear().envs(&self.vars).current_dir(dir);
        cmd
    }

    /// Runs `instrukt <args>` in `cwd` and returns the assertion handle.
    pub fn run<I, S>(&self, cwd: &Path, args: I) -> Assert
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.instrukt(cwd).args(args).assert()
    }

    /// Runs `git <args>` in `dir` with this test's environment; returns
    /// trimmed stdout and fails the test on a non-zero exit.
    pub fn git<I, S>(&self, dir: &Path, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_git(&self.vars, dir, args)
    }

    fn base(&self) -> PathBuf {
        self.home.parent().unwrap().to_path_buf()
    }
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let changed = self.guard.changed();
        assert!(
            changed.is_empty(),
            "the test wrote outside its temporary HOME ({}): {changed:#?}",
            self.root.path().display()
        );
    }
}

/// A git repository created by the harness.
pub struct Repo {
    path: PathBuf,
    vars: BTreeMap<String, String>,
}

impl Repo {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes `contents` to `relative`, creating parent directories.
    pub fn write(&self, relative: &str, contents: &str) -> &Self {
        let target = self.path.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, contents).unwrap();
        self
    }

    pub fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path.join(relative)).unwrap()
    }

    /// Stages everything and commits it; returns the new commit's SHA.
    pub fn commit(&self, message: &str) -> String {
        self.git(["add", "--all"]);
        self.git(["commit", "--quiet", "--allow-empty", "-m", message]);
        self.head()
    }

    pub fn head(&self) -> String {
        self.git(["rev-parse", "HEAD"])
    }

    /// Runs `git <args>` in the repository; returns trimmed stdout.
    pub fn git<I, S>(&self, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_git(&self.vars, &self.path, args)
    }
}

fn run_git<I, S>(vars: &BTreeMap<String, String>, dir: &Path, args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<_> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
    let output = StdCommand::new("git")
        .env_clear()
        .envs(vars)
        .args(&args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

/// What the guard compares for one path: the path itself (a symlink's own
/// identity and where it points) and, followed, what it resolves to.
/// `None` parts do not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stat {
    link: Option<(u64, Option<SystemTime>, Option<PathBuf>)>,
    target: Option<(u64, Option<SystemTime>)>,
}

/// A snapshot of live-machine paths instrukt could write, compared on drop.
/// Symlinks are compared both as links and through their targets; any
/// inspection failure other than a missing path fails the test.
pub struct LiveGuard {
    entries: BTreeMap<PathBuf, Stat>,
    trees: Vec<PathBuf>,
}

impl LiveGuard {
    /// Guards the live user's `~/.instrukt` (whole tree), the user-scope
    /// entrypoints and settings of supported agent CLIs, and the user's git
    /// configuration.
    pub fn over_live_machine() -> Self {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return Self::over(Vec::new(), Vec::new());
        };
        let files = [
            ".gitconfig",
            ".claude/CLAUDE.md",
            ".claude/settings.json",
            ".codex/AGENTS.md",
            ".codex/config.toml",
            ".gemini/GEMINI.md",
            ".gemini/settings.json",
            ".gemini/antigravity-cli/AGENTS.md",
            ".gemini/antigravity-cli/settings.json",
            ".dsh/AGENTS.md",
            ".teleclaude/docs/index.yaml",
        ]
        .iter()
        .map(|rel| home.join(rel))
        .collect();
        Self::over(files, vec![home.join(".instrukt")])
    }

    /// Guards the given files and directory trees.
    pub fn over(files: Vec<PathBuf>, trees: Vec<PathBuf>) -> Self {
        let mut guard = Self {
            entries: BTreeMap::new(),
            trees,
        };
        for file in files {
            let state = stat(&file);
            guard.entries.insert(file, state);
        }
        for (path, state) in guard.walk_trees() {
            guard.entries.insert(path, state);
        }
        guard
    }

    /// Paths that appeared, disappeared or changed since the snapshot.
    pub fn changed(&self) -> Vec<PathBuf> {
        let mut now: BTreeMap<PathBuf, Stat> = self
            .entries
            .keys()
            .map(|path| (path.clone(), stat(path)))
            .collect();
        for (path, state) in self.walk_trees() {
            now.insert(path, state);
        }
        let absent = Stat {
            link: None,
            target: None,
        };
        now.into_iter()
            .filter(|(path, state)| self.entries.get(path).unwrap_or(&absent) != state)
            .map(|(path, _)| path)
            .collect()
    }

    fn walk_trees(&self) -> Vec<(PathBuf, Stat)> {
        let mut found = Vec::new();
        for tree in &self.trees {
            if stat(tree).link.is_none() {
                continue;
            }
            for entry in walkdir::WalkDir::new(tree).follow_links(true) {
                match entry {
                    Ok(entry) => found.push((entry.path().to_path_buf(), stat(entry.path()))),
                    Err(err) if is_not_found(err.io_error()) => {}
                    Err(err) => panic!("live guard cannot inspect {}: {err}", tree.display()),
                }
            }
        }
        found
    }
}

fn stat(path: &Path) -> Stat {
    let link = match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            let points_to = if meta.file_type().is_symlink() {
                Some(std::fs::read_link(path).unwrap_or_else(|err| {
                    panic!("live guard cannot read link {}: {err}", path.display())
                }))
            } else {
                None
            };
            Some((meta.len(), meta.modified().ok(), points_to))
        }
        Err(err) if is_not_found(Some(&err)) => None,
        Err(err) => panic!("live guard cannot inspect {}: {err}", path.display()),
    };
    let target = match std::fs::metadata(path) {
        Ok(meta) => Some((meta.len(), meta.modified().ok())),
        Err(err) if is_not_found(Some(&err)) => None,
        Err(err) => panic!("live guard cannot inspect {}: {err}", path.display()),
    };
    Stat { link, target }
}

fn is_not_found(err: Option<&std::io::Error>) -> bool {
    err.is_some_and(|err| err.kind() == std::io::ErrorKind::NotFound)
}
