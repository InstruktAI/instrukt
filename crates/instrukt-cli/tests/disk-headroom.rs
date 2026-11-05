//! `project/instrukt/spec/feature/runner/disk-headroom`: on its wake-ups
//! the runner keeps the shared volume above its headroom. Below the warn
//! level it tells the developer once, naming the largest regenerable
//! directories in their agents' linked checkouts; below the floor it
//! removes them, largest first, from checkouts no running process uses,
//! and removes nothing it cannot verify.
//!
//! The free space of the volume is real; the tests place it against the
//! guard's levels through the levels themselves (`runner.disk_headroom`).
//! The checkouts live under a configured root of the test directory, so
//! nothing outside it is ever a candidate.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;
#[path = "support/runtime.rs"]
mod runtime;

use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use common::TestEnv;
use runtime::{FakeRuntime, global_layer_with_headroom};

/// Warn far above any volume's free space, floor far below it: `warn`.
const WARNING: &str = "{warn: 1000000GiB, floor: 0.001GiB, roots: [ROOT]}";
/// Both levels far above any volume's free space: `floor`.
const AT_FLOOR: &str = "{warn: 1000000GiB, floor: 999999GiB, roots: [ROOT]}";

/// A developer's machine whose agents work in linked checkouts of one
/// primary clone under `workspace/`:
///
/// - `app` — the primary clone, with its own Rust build output;
/// - `app-idle` — a linked checkout with Rust build output (2 MiB), Node
///   dependencies (1 MiB), a Python environment, a tracked
///   `vendor/node_modules` and a nested repository (its `.git` a file)
///   with build output;
/// - `app-busy` — a linked checkout with Rust build output (3 MiB).
struct Machine {
    env: TestEnv,
    rt: FakeRuntime,
    workspace: PathBuf,
    holders: Vec<Child>,
}

impl Drop for Machine {
    fn drop(&mut self) {
        for holder in &mut self.holders {
            let _ = holder.kill();
            let _ = holder.wait();
        }
    }
}

/// `size` bytes of real data at `path`, so the file occupies its space.
fn fill(path: &Path, size: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![7u8; size]).unwrap();
}

/// Rust build output of `size` bytes at `dir`.
fn rust_output(dir: &Path, size: usize) {
    fill(&dir.join("CACHEDIR.TAG"), 43);
    fill(&dir.join("debug/app"), size);
}

impl Machine {
    /// The machine with the guard set to `headroom`, its `ROOT` the
    /// workspace.
    fn new(headroom: &str) -> Self {
        let env = TestEnv::new();
        let rt = FakeRuntime::new(&env, "rt", false);
        let workspace = env.path("workspace");
        let headroom = headroom.replace("ROOT", &format!("'{}'", workspace.display()));
        global_layer_with_headroom(&env, "events: true", &[&rt], &headroom);

        let app = env.git_repo("workspace/app");
        app.write("README.md", "app\n")
            .write("vendor/node_modules/kept.js", "tracked\n");
        app.commit("initial");
        rust_output(&app.path().join("target"), 1 << 20);
        for name in ["app-idle", "app-busy"] {
            app.git(["worktree", "add", "--quiet", &format!("../{name}")]);
        }

        let idle = workspace.join("app-idle");
        rust_output(&idle.join("target"), 2 << 20);
        fill(&idle.join("web/node_modules/lib/index.js"), 1 << 20);
        fill(&idle.join(".venv/pyvenv.cfg"), 64);
        fill(&idle.join(".venv/lib/site.py"), 256 << 10);
        // A nested repository whose `.git` is a file, as a submodule's is.
        let nested = idle.join("tools/nested");
        std::fs::create_dir_all(&nested).unwrap();
        let nested_git = env.path("nested.git");
        env.git(
            &nested,
            [
                "init".as_ref(),
                "--quiet".as_ref(),
                "--separate-git-dir".as_ref(),
                nested_git.as_os_str(),
            ],
        );
        rust_output(&nested.join("target"), 4 << 20);

        rust_output(&workspace.join("app-busy/target"), 3 << 20);
        Self {
            env,
            rt,
            workspace,
            holders: Vec::new(),
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.workspace.join(relative)
    }

    /// A running process of the developer working in `checkout`.
    fn work_in(&mut self, checkout: &str) {
        let holder = Command::new("sleep")
            .arg("600")
            .current_dir(self.path(checkout))
            .spawn()
            .unwrap();
        self.holders.push(holder);
    }

    /// A wake-up whose clock reads `at`.
    fn tick_at(&mut self, at: &str) {
        self.env.set_env("INSTRUKT_NOW", at);
        self.env.run(self.env.home(), ["tick"]).success();
        self.env.remove_env("INSTRUKT_NOW");
    }

    /// `instrukt ticks status` at `at`, with the test directory and every
    /// size redacted: free space is the real volume's.
    fn status_at(&mut self, at: &str) -> String {
        self.env.set_env("INSTRUKT_NOW", at);
        let out = self.env.run(self.env.home(), ["ticks", "status"]).success();
        self.env.remove_env("INSTRUKT_NOW");
        self.redact(&String::from_utf8(out.get_output().stdout.clone()).unwrap())
    }

    /// The notifications the developer received, redacted.
    fn notifications(&self) -> String {
        self.redact(&self.rt.notifications().join("\n"))
    }

    fn redact(&self, text: &str) -> String {
        let base = self.env.home().parent().unwrap().display().to_string();
        text.replace(&base, "[TMP]")
            .lines()
            .map(redact_sizes)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// `line` with every `<number> <unit>` size shown as `<size>`, keeping
/// the punctuation after the unit.
fn redact_sizes(line: &str) -> String {
    let words: Vec<&str> = line.split(' ').collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let unit = words.get(i + 1).map(|word| {
            let end = word
                .find(|c: char| !c.is_ascii_alphabetic())
                .unwrap_or(word.len());
            word.split_at(end)
        });
        match unit {
            Some((unit, rest))
                if words[i].parse::<f64>().is_ok()
                    && ["GiB", "MiB", "KiB", "B"].contains(&unit) =>
            {
                out.push(format!("<size>{rest}"));
                i += 2;
            }
            _ => {
                out.push(words[i].to_owned());
                i += 1;
            }
        }
    }
    out.join(" ")
}

// spec: project/instrukt/spec/feature/runner/disk-headroom UC-HR1
#[test]
fn below_the_warn_level_the_developer_is_told_once_and_nothing_is_removed() {
    let mut machine = Machine::new(WARNING);

    machine.tick_at("2030-01-06T09:00:00Z");
    // Within five minutes the guard does not run again; at five, it runs
    // and finds the level unchanged.
    machine.tick_at("2030-01-06T09:01:00Z");
    machine.tick_at("2030-01-06T09:05:00Z");

    insta::assert_snapshot!(machine.notifications(), @"disk headroom warn: <size> free; see `instrukt ticks status`");
    insta::assert_snapshot!("warned", machine.status_at("2030-01-06T09:05:30Z"));
    for kept in [
        "app-idle/target",
        "app-idle/web/node_modules",
        "app-busy/target",
    ] {
        assert!(machine.path(kept).is_dir(), "{kept} was removed");
    }
}

// spec: project/instrukt/spec/feature/runner/disk-headroom UC-HR2
#[test]
fn at_the_floor_build_output_is_removed_largest_first_from_idle_checkouts_only() {
    let mut machine = Machine::new(AT_FLOOR);
    machine.work_in("app-busy");

    machine.tick_at("2030-01-06T09:00:00Z");

    insta::assert_snapshot!(machine.notifications(), @"disk headroom floor: <size> free; see `instrukt ticks status`");
    insta::assert_snapshot!("reclaimed", machine.status_at("2030-01-06T09:00:30Z"));
    for removed in [
        "app-idle/target",
        "app-idle/web/node_modules",
        "app-idle/.venv",
    ] {
        assert!(!machine.path(removed).exists(), "{removed} was kept");
    }
    for kept in [
        "app-busy/target",
        "app/target",
        "app-idle/tools/nested/target",
        "app-idle/vendor/node_modules/kept.js",
        "app-idle/README.md",
    ] {
        assert!(machine.path(kept).exists(), "{kept} was removed");
    }
}

// spec: project/instrukt/spec/feature/runner/disk-headroom UC-HR2
#[test]
fn a_floor_run_that_removes_nothing_after_an_earlier_one_tells_the_developer_nothing() {
    let mut machine = Machine::new(AT_FLOOR);
    machine.work_in("app-busy");
    machine.tick_at("2030-01-06T09:00:00Z");

    machine.tick_at("2030-01-06T09:05:00Z");

    assert_eq!(machine.rt.notifications().len(), 1);
    assert!(machine.path("app-busy/target").is_dir());
}

// spec: project/instrukt/spec/feature/runner/disk-headroom UC-HR3
#[cfg(target_os = "linux")]
#[test]
fn a_process_whose_holdings_cannot_be_read_stops_every_removal() {
    let mut machine = Machine::new(AT_FLOOR);
    // A process of the developer that makes itself non-dumpable: what it
    // holds open cannot be read by its own user.
    let mut hidden = Command::new("python3")
        .args([
            "-c",
            "import ctypes, sys, time\n\
             ctypes.CDLL(None).prctl(4, 0, 0, 0, 0)\n\
             print('ready', flush=True)\n\
             time.sleep(600)\n",
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let pid = hidden.id().to_string();
    let mut ready = [0u8; 5];
    std::io::Read::read_exact(hidden.stdout.as_mut().unwrap(), &mut ready).unwrap();
    machine.holders.push(hidden);

    machine.tick_at("2030-01-06T09:00:00Z");

    for kept in [
        "app-idle/target",
        "app-idle/web/node_modules",
        "app-busy/target",
    ] {
        assert!(machine.path(kept).is_dir(), "{kept} was removed");
    }
    insta::assert_snapshot!(machine.notifications(), @"disk headroom floor: <size> free; see `instrukt ticks status`");
    insta::assert_snapshot!(
        "unverified",
        machine
            .status_at("2030-01-06T09:00:30Z")
            .replace(&pid, "<pid>")
    );
}

// spec: project/instrukt/spec/feature/runner/disk-headroom UC-HR4
#[test]
fn a_guard_switched_off_neither_reports_nor_removes() {
    let mut machine = Machine::new("'off'");

    machine.tick_at("2030-01-06T09:00:00Z");

    assert!(machine.rt.notifications().is_empty());
    assert!(machine.path("app-idle/target").is_dir());
    insta::assert_snapshot!(machine.status_at("2030-01-06T09:00:30Z"), @r"
    last wake-up: 2030-01-06T09:00:00Z
    disk headroom: off
    ");
}
