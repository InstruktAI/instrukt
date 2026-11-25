//! `project/instrukt/spec/feature/runner/worktree-reclamation`: instrukt
//! reclaims the regenerable output todo worktrees hold, itself.
#![allow(clippy::unwrap_used, reason = "isolated CLI setup")]
use std::path::{Path, PathBuf};

use crate::fixtures::workflow::Crew;
use instrukt_core::config::CURRENT_SCHEMA_VERSION;

const BUILD: &str = "crew/procedure/build";

/// The crew's project, as published on this computer, declaring as
/// regenerable `build/out` and `latest`, which it ignores; `out`, which it
/// ignores but in which it tracks a file; `vendored`, which it tracks; and
/// `gen`, which it neither tracks nor ignores. It also ignores `target` and
/// `cache` without declaring them.
fn declaring() -> Crew {
    let crew = Crew::new();
    let repo = &crew.project.repo;
    repo.write(
        "instrukt.yaml",
        &format!(
            "schema_version: {CURRENT_SCHEMA_VERSION}\nproject: demo\nboard: acme/1\nmodules:\n  todos: true\ngithub:\n  repository: {}\nworkflow:\n  domain: crew\nregenerable:\n  - build/out\n  - out\n  - gen\n  - vendored\n  - latest\n",
            crate::fixtures::work::REPOSITORY
        ),
    );
    repo.write(".gitignore", "/build\n/out\n/latest\n/target\n/cache\n");
    repo.write("vendored/lib.txt", "vendored source");
    repo.write("out/keep.txt", "tracked inside ignored output");
    repo.git(["add", "--force", "out/keep.txt"]);
    repo.commit("declare the regenerable output");
    repo.git(["push", "--quiet", "origin", "main"]);
    crew.project.env.run(crew.root(), ["sync"]).success();
    crew
}

/// The staging locations left beside `worktree`.
fn staged_beside(worktree: &Path) -> Vec<String> {
    std::fs::read_dir(worktree.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".instrukt-rg-"))
        .collect()
}

/// A todo `slug` whose plan is approved and whose worktree its own
/// orchestrator session entered.
fn prepared(crew: &mut Crew, slug: &str) {
    crew.project
        .env
        .set_env("T3CODE_THREAD_ID", &format!("lead-{slug}"));
    crew.project.create(slug, "change", "Keep list depth.");
    crew.write(slug, "implementation-plan", "Track depth.");
    crew.write(slug, "plan-review-findings", "No findings.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "plan_review"]);
    crew.next("work", slug);
}

/// A role attempt of `slug` dispatched into its worktree under `procedure`,
/// bound to a session named for the todo: its session and worktree.
fn attempt(crew: &Crew, slug: &str, role: &str, procedure: &str) -> (String, PathBuf) {
    crew.next("work", slug);
    let out = crew
        .project
        .env
        .instrukt(crew.root())
        .args([
            "todo", "attempt", slug, "--role", role, "--brief", "-", "--json",
        ])
        .args(["--procedure", procedure, "--args", slug])
        .write_stdin("Do the assigned role.")
        .assert()
        .success();
    let value: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let id = format!("{slug}-{role}-{}", value["attempt"].as_u64().unwrap());
    crew.runtime.open_session(&id);
    let worktree = PathBuf::from(value["worktree"].as_str().unwrap());
    crew.project
        .env
        .instrukt(crew.root())
        .args(["todo", "bind", slug, "--role", role])
        .args(["--attempt", &value["attempt"].to_string()])
        .args(["--session", &format!("t3:{id}")])
        .args(["--worktree", worktree.to_str().unwrap()])
        .args(["--branch", value["branch"].as_str().unwrap()])
        .args(["--base", value["base"].as_str().unwrap()])
        .assert()
        .success();
    (format!("t3:{id}"), worktree)
}

/// `todo stamp <slug> <args>` as the session `session`, in `worktree`.
fn stamp_as(crew: &Crew, session: &str, worktree: &Path, slug: &str, args: &[&str]) {
    crew.project
        .env
        .instrukt(worktree)
        .env("T3CODE_THREAD_ID", session.strip_prefix("t3:").unwrap())
        .args(["todo", "stamp", slug])
        .args(args)
        .assert()
        .success();
}

/// A built worktree of `slug`: a commit on its work branch, published, and
/// build output under `target`.
fn built(crew: &Crew, session: &str, worktree: &Path, slug: &str) {
    std::fs::write(worktree.join("work.txt"), slug).unwrap();
    crew.project.env.git(worktree, ["add", "work.txt"]);
    crew.project
        .env
        .git(worktree, ["commit", "--quiet", "-m", "role work"]);
    crew.project.env.git(
        worktree,
        ["push", "--quiet", "origin", &format!("HEAD:{slug}/work")],
    );
    crew.project.repo.git(["fetch", "--quiet", "origin"]);
    stamp_as(crew, session, worktree, slug, &["--build-started"]);
    output(worktree);
}

/// Build output under the worktree's declared `build/out` and its
/// undeclared `target`.
fn output(worktree: &Path) {
    std::fs::create_dir_all(worktree.join("build/out")).unwrap();
    std::fs::write(worktree.join("build/out/app"), "binary").unwrap();
    std::fs::create_dir_all(worktree.join("target/debug")).unwrap();
    std::fs::write(worktree.join("target/debug/app"), "binary").unwrap();
}

/// What git says of `worktree`: its commit, branch and status.
fn git_state(crew: &Crew, worktree: &Path) -> (String, String, String) {
    let git = |args: &[&str]| crew.project.env.git(worktree, args);
    (
        git(&["rev-parse", "HEAD"]),
        git(&["branch", "--show-current"]),
        git(&["status", "--porcelain", "--untracked-files=all"]),
    )
}

// spec: project/instrukt/spec/feature/runner/worktree-reclamation UC-WR1
#[test]
fn the_runner_reclaims_declared_output_from_an_idle_worktree_only() {
    let mut crew = declaring();

    // Idle: its candidate is handed to integration.
    prepared(&mut crew, "handed");
    let (builder, handed) = attempt(&crew, "handed", "builder", BUILD);
    built(&crew, &builder, &handed, "handed");
    stamp_as(&crew, &builder, &handed, "handed", &["--build-complete"]);
    let (reviewer, _) = attempt(
        &crew,
        "handed",
        "build-reviewer",
        "crew/procedure/review-build",
    );
    crew.write("handed", "review-findings", "Approve the exact candidate.");
    stamp_as(
        &crew,
        &reviewer,
        &handed,
        "handed",
        &["--verdict", "approve", "--phase", "build_review"],
    );
    let (finalizer, _) = attempt(&crew, "handed", "finalizer", "crew/procedure/finalize");
    stamp_as(&crew, &finalizer, &handed, "handed", &["--role-complete"]);
    stamp_as(&crew, &finalizer, &handed, "handed", &["--finalize-ready"]);

    // Between its rounds: built, its review not yet started.
    prepared(&mut crew, "between");
    let (builder, between) = attempt(&crew, "between", "builder", BUILD);
    built(&crew, &builder, &between, "between");
    stamp_as(&crew, &builder, &between, "between", &["--build-complete"]);

    // Held, with a role attempt still open in its worktree.
    prepared(&mut crew, "working");
    let (builder, working) = attempt(&crew, "working", "builder", BUILD);
    built(&crew, &builder, &working, "working");
    crew.todo([
        "update",
        "working",
        "--hold",
        "--reason",
        "waiting on a decision",
    ]);

    let before = git_state(&crew, &handed);
    let threads = crew.runtime.thread_calls();
    crew.project.env.run(crew.root(), ["tick"]).success();

    assert!(!handed.join("build/out").exists());
    assert!(staged_beside(&handed).is_empty());
    assert_eq!(git_state(&crew, &handed), before);
    assert!(handed.join("target/debug/app").is_file());
    assert!(handed.join("out/keep.txt").is_file());
    assert!(between.join("build/out/app").is_file());
    assert!(working.join("build/out/app").is_file());
    assert_eq!(crew.runtime.thread_calls(), threads);
}

// spec: project/instrukt/spec/feature/runner/worktree-reclamation UC-WR2
#[test]
fn a_working_session_reclaims_only_its_own_worktrees_declared_output() {
    let mut crew = declaring();
    prepared(&mut crew, "own");
    let (builder, own) = attempt(&crew, "own", "builder", BUILD);
    output(&own);
    std::fs::create_dir_all(own.join("cache")).unwrap();
    std::fs::write(own.join("cache/kept"), "undeclared").unwrap();
    std::fs::create_dir_all(own.join("gen")).unwrap();
    std::fs::write(own.join("gen/new.txt"), "untracked work").unwrap();
    // An absolute referent, which still resolves once the link is staged.
    std::os::unix::fs::symlink(own.join("target"), own.join("latest")).unwrap();
    prepared(&mut crew, "other");
    let (_, other) = attempt(&crew, "other", "builder", BUILD);
    output(&other);
    let before = git_state(&crew, &own);
    let reclaim = |session: &str| {
        crew.project
            .env
            .instrukt(&crew.project.env.path(""))
            .env("T3CODE_THREAD_ID", session)
            .args(["repo", "reclaim"])
            .assert()
    };

    let reclaimed = reclaim(builder.strip_prefix("t3:").unwrap()).success();
    insta::assert_snapshot!(
        String::from_utf8_lossy(&reclaimed.get_output().stdout),
        @r"
    removed: build/out
    removed: latest
    left: out (git does not ignore it)
    left: gen (git does not ignore it)
    left: vendored (git does not ignore it)
    "
    );
    assert!(!own.join("build/out").exists());
    assert!(std::fs::symlink_metadata(own.join("latest")).is_err());
    assert!(staged_beside(&own).is_empty());
    assert_eq!(git_state(&crew, &own), before);
    assert!(own.join("target/debug/app").is_file());
    assert!(own.join("out/keep.txt").is_file());
    assert!(own.join("gen/new.txt").is_file());
    assert!(own.join("vendored/lib.txt").is_file());
    assert!(own.join("cache/kept").is_file());
    assert!(other.join("build/out/app").is_file());

    let refused = reclaim("unbound").failure();
    insta::assert_snapshot!(
        String::from_utf8_lossy(&refused.get_output().stderr),
        @"instrukt: no todo assignment binds this caller to a worktree; the todo attempt its dispatcher reserved would, once bound to this session in that attempt's worktree (`instrukt todo bind <slug> --role <role> --attempt <attempt> --session <session> --worktree <worktree> --branch <branch> --base <base>`, with the worktree, branch and base the reservation names)"
    );
}
