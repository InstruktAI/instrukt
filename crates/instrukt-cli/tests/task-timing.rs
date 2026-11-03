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
#[path = "support/work.rs"]
mod work;

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use common::TestEnv;
use flavor::Tracked;

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
    // The developer is signed in to GitHub's CLI; the scheduler's wake-up
    // carries no token of its own.
    let gh = dir.join("gh");
    std::fs::write(
        &gh,
        "#!/bin/sh\nif [ \"$1 $2\" = \"auth token\" ]; then echo test-token; else exit 1; fi\n",
    )
    .unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
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

/// Fires the registered wake-up as the scheduler would at `at`: its
/// program, in the environment it declares, with the clock at `at`.
fn run_registered(registered: &Registered, cwd: &Path, at: &str) {
    let (program, args) = registered.program.split_first().unwrap();
    let output = std::process::Command::new(program)
        .args(args)
        .env_clear()
        .envs(registered.env.iter().map(|(k, v)| (k, v)))
        .env("INSTRUKT_NOW", at)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the registered wake-up failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// spec: local/projects/instrukt/spec/feature/runner/task-timing UC-TT3
#[test]
fn the_installed_wake_up_runs_due_work_under_its_missed_run_policy() {
    let mut project = Tracked::new(true);
    let stubs = registrar_stubs(&mut project.env);
    project.authorized_task(
        "quarter-hourly-sync",
        "cron:*/15 * * * *",
        &["--catch-up", "once"],
    );

    project.env.run(project.env.home(), ["install"]).success();
    let calls = std::fs::read_to_string(stubs.join("calls")).unwrap();
    let registered = registered(&project.env, &calls);

    // The scheduler fires the registered wake-up with nobody at a runner:
    // at 09:05 nothing is due; the computer is then off until 11:05, past
    // several occurrences, and one catch-up run is dispatched.
    run_registered(&registered, project.env.home(), "2030-01-06T09:05:00Z");
    assert!(project.rt.calls().is_empty(), "{:?}", project.rt.calls());
    run_registered(&registered, project.env.home(), "2030-01-06T11:05:00Z");
    assert_eq!(project.rt.calls().len(), 1, "{:?}", project.rt.calls());
    insta::assert_snapshot!("installed-catch-up", project.rt.opening("s1"));
}

// spec: local/projects/instrukt/spec/feature/runner/task-timing UC-TT1
#[test]
fn an_authorized_task_is_dispatched_when_a_wake_up_arrives_at_its_time() {
    let mut project = Tracked::new(true);
    project.authorized_task("weekly-status", "2030-01-06T09:00:00Z", &[]);

    // A wake-up before its time leaves the task alone.
    let early = project.tick_at("2030-01-06T08:59:20Z");
    insta::assert_snapshot!(
        String::from_utf8_lossy(&early.get_output().stderr),
        @"instrukt: clock overridden to 2030-01-06T08:59:20Z"
    );
    assert!(project.rt.calls().is_empty(), "{:?}", project.rt.calls());

    // The wake-up at its time dispatches it into the task runner.
    project.tick_at("2030-01-06T09:00:20Z");
    assert_eq!(project.rt.calls().len(), 1, "{:?}", project.rt.calls());
    insta::assert_snapshot!("task-dispatched", project.rt.opening("s1"));
}

/// A task recurring every quarter hour, a first wake-up at 09:05, then the
/// computer off until a wake-up at 11:05, between occurrences: the
/// occurrences from 09:15 to 11:00 were missed. Returns the executions
/// dispatched.
fn missed_occurrences(policy: &str) -> usize {
    let mut project = Tracked::new(true);
    project.authorized_task(
        "quarter-hourly-sync",
        "cron:*/15 * * * *",
        &["--catch-up", policy],
    );
    project.tick_at("2030-01-06T09:05:00Z");
    assert!(project.rt.calls().is_empty(), "{:?}", project.rt.calls());
    project.tick_at("2030-01-06T11:05:00Z");
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

// spec: local/projects/instrukt/spec/feature/runner/task-timing UC-TT4
#[test]
fn a_recurring_task_continues_after_success_and_holds_after_an_error() {
    let mut project = Tracked::new(true);
    project.authorized_task("quarter-hourly-sync", "cron:*/15 * * * *", &[]);

    project.tick_at("2030-01-06T09:00:20Z");
    assert_eq!(project.rt.calls().len(), 1, "{:?}", project.rt.calls());
    project.todo_in_session(
        "s1",
        ["stamp", "quarter-hourly-sync", "--task-outcome", "success"],
    );

    // Success: the next occurrence runs when its time arrives.
    project.tick_at("2030-01-06T09:15:20Z");
    assert_eq!(project.rt.calls().len(), 2, "{:?}", project.rt.calls());
    insta::assert_snapshot!("next-occurrence", project.rt.opening("s2"));

    // An error needing the developer's judgment: held and notified.
    project.todo_in_session(
        "s2",
        [
            "stamp",
            "quarter-hourly-sync",
            "--task-outcome",
            "error",
            "--reason",
            "the sync target refused our credentials",
        ],
    );
    project.rt.set_status("s2", "ended failed");
    project.tick_at("2030-01-06T09:16:20Z");
    insta::assert_snapshot!(project.rt.notifications().join("\n"), @"rt:s2");
    insta::assert_snapshot!(
        "held-after-error",
        work::redact(&project.todo(["show", "quarter-hourly-sync"]))
    );

    // No further occurrence runs until the developer resolves it.
    project.tick_at("2030-01-06T09:30:20Z");
    project.tick_at("2030-01-06T09:45:20Z");
    assert_eq!(project.rt.calls().len(), 2, "{:?}", project.rt.calls());
    assert_eq!(project.rt.notifications().len(), 1);
}
