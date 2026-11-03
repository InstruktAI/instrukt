//! `local/projects/instrukt/spec/feature/runner/task-timing`: `instrukt
//! install` registers the generic wake-up — `instrukt tick` every minute —
//! with the machine's scheduler, and the wake-up it registers is what
//! `instrukt ticks status` then reports.
//!
//! The scheduler's own commands (`launchctl`, `systemctl`, `crontab`) are
//! recording stubs the test puts first on `PATH`; the registration file
//! lands in the temporary home. Nothing reaches the real scheduler.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

mod common;
#[path = "support/documented.rs"]
mod documented;
#[path = "support/flavor.rs"]
mod flavor;
#[path = "support/runtime.rs"]
mod runtime;

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use common::TestEnv;
use flavor::Tracked;
use jiff::{SignedDuration, Timestamp, ToSpan as _, Zoned};
use runtime::{FakeRuntime, global_layer};

/// Recording stand-ins for the schedulers' commands, in `<test>/registrar`.
fn registrar_stubs(env: &mut TestEnv) -> PathBuf {
    let dir = env.path("registrar");
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("calls");
    for name in ["launchctl", "systemctl", "crontab"] {
        let script = format!(
            "#!/bin/sh\nprintf '%s' \"{name}\" >> '{log}'\nfor a in \"$@\"; do printf ' %s' \"$a\" >> '{log}'; done\necho >> '{log}'\n\
             if [ \"{name}\" = crontab ] && [ \"${{1:-}}\" = - ]; then cat > '{dir}/crontab'; fi\n\
             if [ \"{name}\" = crontab ] && [ \"${{1:-}}\" = -l ]; then cat '{dir}/crontab' 2>/dev/null || exit 1; fi\n",
            log = log.display(),
            dir = dir.display(),
        );
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    env.set_env("PATH", &path);
    dir
}

/// What the registration runs, with the environment it declares.
struct Registered {
    program: Vec<String>,
    env: Vec<(String, String)>,
}

fn between<'t>(text: &'t str, open: &str, close: &str) -> Vec<&'t str> {
    text.split(open)
        .skip(1)
        .filter_map(|part| part.split_once(close).map(|(inner, _)| inner))
        .collect()
}

#[cfg(target_os = "macos")]
fn registered(env: &TestEnv, calls: &str) -> Registered {
    let plist_path = env
        .home()
        .join("Library/LaunchAgents/ai.instrukt.tick.plist");
    let plist = std::fs::read_to_string(&plist_path).unwrap();
    insta::assert_snapshot!(
        calls.replace(plist_path.to_str().unwrap(), "<plist>"),
        @r"
    launchctl unload <plist>
    launchctl load -w <plist>
    "
    );
    let interval: u32 = between(&plist, "<key>StartInterval</key>", "</integer>")[0]
        .trim()
        .trim_start_matches("<integer>")
        .parse()
        .unwrap();
    assert_eq!(interval, 60, "every minute");
    let arguments = between(&plist, "<key>ProgramArguments</key>", "</array>")[0];
    let program = between(arguments, "<string>", "</string>")
        .into_iter()
        .map(str::to_owned)
        .collect();
    let variables = between(&plist, "<key>EnvironmentVariables</key>", "</dict>")[0];
    let keys = between(variables, "<key>", "</key>");
    let values = between(variables, "<string>", "</string>");
    Registered {
        program,
        env: keys
            .into_iter()
            .zip(values)
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    }
}

#[cfg(not(target_os = "macos"))]
fn registered(env: &TestEnv, calls: &str) -> Registered {
    insta::assert_snapshot!(calls, @r"
    systemctl --user daemon-reload
    systemctl --user enable --now instrukt-tick.timer
    ");
    let units = env.home().join(".config/systemd/user");
    let timer = std::fs::read_to_string(units.join("instrukt-tick.timer")).unwrap();
    let on_calendar = between(&timer, "OnCalendar=", "\n")[0];
    insta::assert_snapshot!(on_calendar, @"*-*-* *:*:00");
    let service = std::fs::read_to_string(units.join("instrukt-tick.service")).unwrap();
    let exec = between(&service, "ExecStart=", "\n")[0];
    let (program, rest) = exec[1..].split_once('"').unwrap();
    let path = between(&service, "Environment=PATH=", "\n")[0];
    Registered {
        program: std::iter::once(program.to_owned())
            .chain(rest.split_whitespace().map(str::to_owned))
            .collect(),
        env: vec![
            ("PATH".into(), path.to_owned()),
            ("HOME".into(), env.home().display().to_string()),
        ],
    }
}

fn run_registered(registered: &Registered, cwd: &Path) {
    let (program, args) = registered.program.split_first().unwrap();
    let output = std::process::Command::new(program)
        .args(args)
        .env_clear()
        .envs(registered.env.iter().map(|(k, v)| (k, v)))
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the registered wake-up failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn ticks_status(env: &TestEnv, cwd: &Path) -> serde_json::Value {
    let out = env.run(cwd, ["ticks", "status", "--json"]).success();
    serde_json::from_slice(&out.get_output().stdout).unwrap()
}

// spec: local/projects/instrukt/spec/feature/runner/task-timing UC-TT3
#[test]
fn the_installed_wake_up_runs_the_runner_every_minute_without_an_interactive_runner() {
    let mut env = TestEnv::new();
    let stubs = registrar_stubs(&mut env);
    let rt = FakeRuntime::new(&env, "rt", false);
    global_layer(&env, "todos: true, events: true", &[&rt]);
    let cwd = env.path("");

    let before = ticks_status(&env, &cwd);
    assert!(before["last_wakeup"].is_null(), "no wake-up yet: {before}");

    env.run(&cwd, ["install"]).success();
    let calls = std::fs::read_to_string(stubs.join("calls")).unwrap();
    let registered = registered(&env, &calls);
    assert_eq!(
        registered.program.last().map(String::as_str),
        Some("tick"),
        "{:?}",
        registered.program
    );

    // The scheduler fires the registered wake-up, with nobody at a runner.
    run_registered(&registered, env.home());
    let after = ticks_status(&env, &cwd);
    let woke: jiff::Timestamp = after["last_wakeup"].as_str().unwrap().parse().unwrap();
    assert!(woke <= jiff::Timestamp::now());
    assert_eq!(after["stale"], serde_json::Value::Bool(false));
}

/// Now, to the second: a schedule a developer writes.
fn this_second() -> Timestamp {
    Timestamp::from_second(Timestamp::now().as_second()).unwrap()
}

// spec: local/projects/instrukt/spec/feature/runner/task-timing UC-TT1
#[test]
fn an_authorized_task_is_dispatched_when_a_wake_up_arrives_at_its_time() {
    let project = Tracked::new(true);
    let tomorrow = this_second() + SignedDuration::from_hours(24);
    project.authorized_task("weekly-status", &tomorrow.to_string(), &[]);

    // Before its time, a wake-up leaves the task alone.
    project.tick();
    assert!(project.rt.calls().is_empty(), "{:?}", project.rt.calls());

    // The developer moves the task to the present moment; the next
    // wake-up is the one at its time.
    project.todo([
        "update",
        "weekly-status",
        "--schedule",
        &this_second().to_string(),
    ]);
    project.tick();

    assert_eq!(project.rt.calls().len(), 1, "{:?}", project.rt.calls());
    insta::assert_snapshot!("task-dispatched", project.rt.opening("s1"));
}

/// A cron schedule matching every minute of the day except this local
/// minute and the next two, so several occurrences lie behind the next
/// wake-up and none is on time at it.
fn every_minute_but_now() -> String {
    let now = Zoned::now();
    let minute = i32::from(now.minute());
    let skipped: Vec<i32> = (0..3).map(|offset| (minute + offset) % 60).collect();
    let minutes: Vec<String> = (0..60)
        .filter(|m| !skipped.contains(m))
        .map(|m| m.to_string())
        .collect();
    format!("cron:{} * * * *", minutes.join(","))
}

/// The computer was off: its last wake-up was three hours ago.
fn last_woke_three_hours_ago(env: &TestEnv) {
    let then = Zoned::now().checked_sub(3.hours()).unwrap().timestamp();
    let path = env.instrukt_home().join("state/runner/last-wakeup");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("{then}\n")).unwrap();
}

fn missed_occurrences(policy: &str) -> usize {
    let project = Tracked::new(true);
    project.authorized_task(
        "hourly-sync",
        &every_minute_but_now(),
        &["--catch-up", policy],
    );
    last_woke_three_hours_ago(&project.env);
    project.tick();
    project.rt.calls().len()
}

// spec: local/projects/instrukt/spec/feature/runner/task-timing UC-TT2
#[test]
fn missed_occurrences_with_one_catch_up_dispatch_one_execution() {
    assert_eq!(missed_occurrences("once"), 1);
}

// spec: local/projects/instrukt/spec/feature/runner/task-timing UC-TT2
#[test]
fn missed_occurrences_with_skip_dispatch_nothing() {
    assert_eq!(missed_occurrences("skip"), 0);
}
