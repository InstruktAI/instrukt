//! `project/instrukt/spec/feature/runner/thread-retention`: the runner
//! archives long-quiet finished threads of registered projects and keeps
//! pinned, standing and open-work threads.
#![allow(clippy::unwrap_used, reason = "isolated CLI setup")]

use crate::common::redis::RedisServer;
use crate::common::t3_readiness;
use crate::fixtures::documented;
use crate::fixtures::t3::FakeT3;
use crate::fixtures::workflow::Crew;
use instrukt_core::config::CURRENT_SCHEMA_VERSION;
use instrukt_runner::transport::EVENT_STREAM;

/// When the runner next runs its maintenance.
const NOW: &str = "2026-12-01T00:00:00Z";
/// Longer ago than the window, as of [`NOW`].
const LONG_AGO: &str = "2026-11-07T00:00:00Z";
/// Within the window, as of [`NOW`].
const RECENTLY: &str = "2026-11-28T00:00:00Z";

/// The crew's project with event responses on and a published procedure
/// that a fact starts a standing responsibility for, on a computer with a
/// retention window set; and the event transport it uses.
fn retaining() -> (Crew, RedisServer) {
    let crew = Crew::new();
    let redis = RedisServer::new(&crew.project.env);
    std::fs::write(
        crew.project.env.instrukt_home().join("instrukt.yaml"),
        format!(
            "schema_version: {CURRENT_SCHEMA_VERSION}\ngithub:\n  api_url: {}\nrunner: {{runtime: t3, notify: 'off', archive_finished_after: 7d}}\nmodules: {{events: true}}\nredis:\n  cluster:\n    url: '{}'\n",
            crew.project.github.url(),
            redis.url
        ),
    )
    .unwrap();
    crew.project.repo.write(
        "docs/domain/reactions/procedure/respond.md",
        &documented::snippet(
            "procedure",
            "Respond",
            "Handle external facts.",
            "role: responder\nsubscription_home: project\nsubscriptions: [example.changed]\n",
            &[],
            "Inspect current subject state and reconcile the event.\n",
        ),
    );
    crew.project.repo.commit("docs: publish a reaction");
    crew.project.repo.git(["push", "--quiet", "origin", "main"]);
    documented::sync(&crew.project.env, &crew.project.repo);
    (crew, redis)
}

/// A fact about the project, which starts its standing responsibility.
fn fact(crew: &Crew, redis: &RedisServer) {
    let computer =
        std::fs::read_to_string(crew.project.env.instrukt_home().join("state/machine-id")).unwrap();
    let event = serde_json::json!({"id":"changed","type":"example.changed","source":computer.trim(),"subject":"todo:demo","home":"project:demo","home_owner":"unresolved","time":"2026-11-05T00:00:00Z","data":{}});
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg(event.to_string())
        .query::<String>(&mut redis.connection())
        .unwrap();
}

/// A todo `slug` the session of thread `thread` claimed to work on.
fn claimed(crew: &mut Crew, slug: &str, thread: &str) {
    crew.project.env.set_env("T3CODE_THREAD_ID", thread);
    crew.project.create(slug, "change", "Keep list depth.");
    crew.write(slug, "implementation-plan", "Track depth.");
    crew.write(slug, "plan-review-findings", "No findings.");
    crew.stamp(slug, &["--verdict", "approve", "--phase", "plan_review"]);
    crew.next("work", slug);
}

// spec: project/instrukt/spec/feature/runner/thread-retention UC-TR1
#[test]
fn long_quiet_finished_threads_leave_the_thread_list_on_their_own() {
    let (mut crew, redis) = retaining();
    let t3 = FakeT3::create(&crew.project.env);
    t3.migrate_projection();
    let root = crew.root().to_path_buf();
    crew.project.env.run(&root, ["tick"]).success();
    fact(&crew, &redis);
    crew.project.env.run(&root, ["tick"]).success();
    let standing = t3.native_id("s1");

    t3.next_thread_id("lead-open");
    let open = t3.developer_starts_in(&root);
    claimed(&mut crew, "open", &open);
    let plain = t3.developer_starts_in(&root);
    let pinned = t3.developer_starts_in(&root);
    let recent = t3.developer_starts_in(&root);
    for thread in [&standing, &open, &plain, &pinned, &recent] {
        t3.settle_at(thread, LONG_AGO);
    }
    t3.pin_at(&pinned, LONG_AGO);
    t3.opened_at(&recent, RECENTLY);

    crew.project.env.set_env("INSTRUKT_NOW", NOW);
    crew.project.env.run(&root, ["tick"]).success();

    assert_eq!(t3_readiness::archived(&crew.project.env), [plain]);
    let mut listed = t3.listed(&root);
    listed.sort();
    let mut kept = vec![standing, open, pinned, recent];
    kept.sort();
    assert_eq!(listed, kept);
}
