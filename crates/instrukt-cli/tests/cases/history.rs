//! History is exercised against fixture source stores in temporary homes.
#![allow(clippy::unwrap_used, reason = "isolated CLI fixtures")]

use crate::common;
use crate::fixtures::history as fixture;
use common::TestEnv;
use serde_json::Value;
use std::time::{Duration, Instant};

fn stable_sources(mut result: Value) -> Value {
    for found in result["matches"].as_array_mut().unwrap() {
        let kind = found["source"].as_str().unwrap().split_once(':').unwrap().0;
        found["source"] = Value::String(format!("{kind}:[SOURCE]"));
    }
    result
}

fn search(env: &TestEnv, query: &str) -> Value {
    let out = env.run(env.home(), ["history", "search", query, "--json"]);
    let out = out.success();
    serde_json::from_slice(&out.get_output().stdout).unwrap()
}

// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn extracted_conversations_survive_restart_and_source_deletion() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    env.run(env.home(), ["history", "extract", "--json"])
        .success();
    let first = search(&env, "jewels");
    assert_eq!(first["matches"].as_array().unwrap().len(), 2);
    source.remove_threads();
    let retained = search(&env, "jewels");
    assert_eq!(retained["matches"].as_array().unwrap().len(), 2);
    insta::assert_json_snapshot!("extracted-history", stable_sources(retained));
}

// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn search_refreshes_active_children_even_when_shell_transcript_events_are_suppressed() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    source.active_child(false);
    assert_eq!(
        search(&env, "jewels")["matches"].as_array().unwrap().len(),
        2
    );
    source.active_child(true);
    assert_eq!(
        search(&env, "Unreported")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

// spec: project/instrukt/spec/feature/runner/history UC-HI2
#[test]
fn legacy_import_keeps_aliases_once_without_a_continuing_mirror_dependency() {
    let env = TestEnv::new();
    let t3 = fixture::T3Store::new(&env);
    let mirror = fixture::mirror(&env);
    let original = std::fs::read(&mirror).unwrap();
    env.run(env.home(), ["history", "import-teleclaude", "--json"])
        .success();
    assert_eq!(std::fs::read(&mirror).unwrap(), original);
    std::fs::remove_file(mirror).unwrap();
    env.run(env.home(), ["history", "import-teleclaude", "--json"])
        .success();
    t3.message(
        "child",
        "after-import",
        "Postimport native message.",
        "2026-11-08T12:00:00.000Z",
        false,
    );
    assert_eq!(
        search(&env, "Postimport")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let found = search(&env, "archive");
    assert_eq!(found["matches"].as_array().unwrap().len(), 1);
    let source = found["matches"][0]["source"].as_str().unwrap();
    let out = env
        .run(env.home(), ["history", "show", source, "source-alias"])
        .success();
    let document: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(document["sources"].as_array().unwrap().len(), 1);
    insta::assert_json_snapshot!("legacy-history", stable_sources(found));
}

// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn runner_database_notifications_keep_history_current_without_a_search_or_tick() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    let found = search(&env, "jewels");
    let source_id = found["matches"][0]["source"].as_str().unwrap();
    let redis = common::redis::RedisServer::new(&env);
    fixture::configure_cluster(&env, &redis.url);
    source.message(
        "child",
        "startup-marker",
        "Startup marker.",
        "2026-11-08T10:30:00.000Z",
        false,
    );
    let _runner = fixture::runner(&env);
    let startup_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let out = env
            .run(env.home(), ["history", "show", source_id, "child"])
            .success();
        let document: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
        if document["messages"].as_array().unwrap().len() == 2 {
            break;
        }
        assert!(
            Instant::now() < startup_deadline,
            "startup marker was not retained"
        );
        std::thread::yield_now();
    }
    source.complete();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let out = env
            .run(env.home(), ["history", "show", source_id, "child"])
            .success();
        let document: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
        if document["messages"].as_array().unwrap().len() == 3 {
            break;
        }
        assert!(Instant::now() < deadline, "completion was not extracted");
        std::thread::yield_now();
    }
}

// spec: project/instrukt/spec/feature/runner/history UC-HI3
#[test]
fn merged_search_identifies_each_computer_and_reports_an_unreachable_one() {
    let local = TestEnv::new();
    let remote = TestEnv::new();
    let _local_source = fixture::T3Store::new(&local);
    let _remote_source = fixture::T3Store::new(&remote);
    let redis = common::redis::RedisServer::new(&local);
    fixture::configure_cluster(&local, &redis.url);
    fixture::configure_cluster(&remote, &redis.url);
    fixture::computer(&remote, "bbbbbbbbbbbbbbbb");
    let worker = fixture::runner(&remote);
    let out = local
        .run(
            local.home(),
            [
                "history",
                "search",
                "jewels",
                "--computer",
                "bbbbbbbbbbbbbbbb",
                "--computer",
                "cccccccccccccccc",
                "--json",
            ],
        )
        .failure();
    let found: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(found["matches"].as_array().unwrap().len(), 4);
    assert_eq!(found["failures"].as_array().unwrap().len(), 1);
    insta::assert_json_snapshot!("merged-history", stable_sources(found));
    drop(worker);
}

// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn older_messages_survive_a_later_conversation_refresh() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    for index in 0..257 {
        source.message(
            "child",
            &format!("older-padding-{index:03}"),
            "Retained older entry.",
            "2025-01-01T00:00:00.000Z",
            false,
        );
    }
    source.message(
        "child",
        "older-message",
        "Ancienttoken retained.",
        "2025-01-01T00:00:00.000Z",
        false,
    );
    assert_eq!(
        search(&env, "Ancienttoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    source.complete();
    assert_eq!(
        search(&env, "Finished")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "Ancienttoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn catch_up_keeps_threads_archived_during_an_extraction_gap() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    search(&env, "jewels");
    source.archived_after_gap();
    assert_eq!(
        search(&env, "Archivedgap")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn streaming_messages_are_picked_up_after_completion_even_behind_the_cursor() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    source.message(
        "child",
        "streaming",
        "Drafttoken text.",
        "2026-11-08T10:00:00.000Z",
        true,
    );
    source.complete();
    assert_eq!(
        search(&env, "Drafttoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    source.message(
        "child",
        "streaming",
        "Drafttoken text.",
        "2026-11-08T10:00:00.000Z",
        false,
    );
    assert_eq!(
        search(&env, "Drafttoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn source_shape_refusal_does_not_consume_new_rows_and_release_numbers_are_diagnostics() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    source.release_and_migration();
    search(&env, "jewels");
    source.message(
        "child",
        "recoverable",
        "Recoverabletoken text.",
        "2026-11-08T12:00:00.000Z",
        false,
    );
    source.break_shape();
    env.run(env.home(), ["history", "extract", "--json"])
        .failure();
    let failed = env
        .run(env.home(), ["history", "search", "jewels", "--json"])
        .failure();
    let failed: Value = serde_json::from_slice(&failed.get_output().stdout).unwrap();
    insta::assert_json_snapshot!("safe-source-failure", stable_sources(failed));
    source.restore_shape();
    assert_eq!(
        search(&env, "Recoverabletoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn development_history_reads_the_machine_runtime_instead_of_its_output_home() {
    let mut env = TestEnv::new();
    let _source = fixture::T3Store::new(&env);
    let development = env.home().join("development");
    std::fs::create_dir_all(&development).unwrap();
    std::fs::write(
        development.join("instrukt.yaml"),
        "schema_version: 1\nrunner: {runtime: t3}\n",
    )
    .unwrap();
    env.set_env("INSTRUKT_HOME", development.to_str().unwrap())
        .set_env("INSTRUKT_DEVELOPMENT", "1");
    assert_eq!(
        search(&env, "jewels")["matches"].as_array().unwrap().len(),
        2
    );
}

// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn upgrading_history_preserves_previously_retained_text_absent_from_the_runtime() {
    let env = TestEnv::new();
    let _source = fixture::T3Store::new(&env);
    let found = search(&env, "jewels");
    let source_id = found["matches"][0]["source"].as_str().unwrap();
    let file = env.instrukt_home().join("state/instrukt.db");
    std::fs::remove_file(&file).unwrap();
    let db = instrukt_core::state_db::rusqlite::Connection::open(&file).unwrap();
    db.execute_batch(include_str!("../fixtures/state-v6.sql"))
        .unwrap();
    db.execute_batch(include_str!("../fixtures/history-v7.sql"))
        .unwrap();
    let document = serde_json::json!({"thread":{"id":"child","title":"Retained child","lineage":{"parentThreadId":"parent"}},"messages":[{"id":"pre-upgrade","threadId":"child","role":"assistant","text":"Upgradetoken retained only in history."}],"turnItems":[{"id":"retained-tool","threadId":"child","type":"command_execution","text":"Textonlytoken","input":"Inputonlytoken","output":"Outputonlytoken","markdown":"Markdownonlytoken","diffStr":"Diffonlytoken"}]});
    db.execute("INSERT INTO history_conversations(source,native_id,computer,title,parent,aliases,document,body,limitations) VALUES(?1,'child','aaaaaaaaaaaaaaaa','Retained child','parent','[]',?2,'Upgradetoken retained only in history. Textonlytoken Inputonlytoken Outputonlytoken Markdownonlytoken Diffonlytoken','[]')",instrukt_core::state_db::rusqlite::params![source_id,document.to_string()]).unwrap();
    drop(db);
    assert_eq!(
        search(&env, "Upgradetoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "Textonlytoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "Inputonlytoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "Outputonlytoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "Markdownonlytoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "Diffonlytoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "jewels")["matches"].as_array().unwrap().len(),
        2
    );
}

// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn later_hydrated_older_messages_and_tied_pages_are_discovered() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    search(&env, "jewels");
    for index in 0..259 {
        source.message(
            "child",
            &format!("late-{index:03}"),
            &format!("Latetoken{index:03} hydrated."),
            "2025-01-01T00:00:00.000Z",
            false,
        );
    }
    assert_eq!(
        search(&env, "Latetoken000")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "Latetoken258")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn same_time_lower_ids_and_in_place_edits_change_the_conversation_fingerprint() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    let at = "2026-11-08T12:00:00.000Z";
    source.message("child", "z-seen", "Firsttoken value.", at, false);
    search(&env, "Firsttoken");
    source.message("child", "a-late", "Tiedtoken value.", at, false);
    assert_eq!(
        search(&env, "Tiedtoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    source.message("child", "z-seen", "Editedtoken value.", at, false);
    assert_eq!(
        search(&env, "Editedtoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn conforming_turn_items_runs_and_subagents_are_retained_without_message_roles() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    source.native_records();
    assert_eq!(
        search(&env, "Useritemtoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "Assistantitemtoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "Toolitemtoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "Reasonitemtoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let result = search(&env, "Subagenttoken");
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    let source_id = result["matches"][0]["source"].as_str().unwrap();
    let out = env
        .run(env.home(), ["history", "show", source_id, "child"])
        .success();
    let document: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(document["turnItems"].as_array().unwrap().len(), 4);
    assert_eq!(document["runs"].as_array().unwrap().len(), 1);
    assert_eq!(document["subagents"].as_array().unwrap().len(), 1);
    source.complete();
    assert_eq!(
        search(&env, "Finished")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn repairing_v8_recovers_retained_only_output_and_diff_without_source_refresh() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    let found = search(&env, "jewels");
    let source_id = found["matches"][0]["source"].as_str().unwrap();
    let file = env.instrukt_home().join("state/instrukt.db");
    std::fs::remove_file(&file).unwrap();
    let db = instrukt_core::state_db::rusqlite::Connection::open(&file).unwrap();
    db.execute_batch(include_str!("../fixtures/state-v6.sql"))
        .unwrap();
    db.execute_batch(include_str!("../fixtures/history-v7.sql"))
        .unwrap();
    let native = serde_json::json!({"id":"retained-output","threadId":"child","type":"command_execution","text":"Textonlytoken","input":"Inputonlytoken","output":"Outputonlytoken","markdown":"Markdownonlytoken","diffStr":"Diffonlytoken"});
    let document = serde_json::json!({"thread":{"id":"child","title":"Retained child","lineage":{"parentThreadId":"parent"}},"turnItems":[native]});
    db.execute("INSERT INTO history_conversations(source,native_id,computer,title,parent,aliases,document,body,limitations) VALUES(?1,'child','aaaaaaaaaaaaaaaa','Retained child','parent','[]',?2,'Textonlytoken','[]')",instrukt_core::state_db::rusqlite::params![source_id,document.to_string()]).unwrap();
    db.execute_batch(include_str!("../fixtures/history-v8.sql"))
        .unwrap();
    drop(db);
    source.remove_threads();
    assert_eq!(
        search(&env, "Textonlytoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "Inputonlytoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "Outputonlytoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "Markdownonlytoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(&env, "Diffonlytoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn a_delivered_timestamp_position_migrates_to_complete_conversation_detection() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    let found = search(&env, "jewels");
    let source_id = found["matches"][0]["source"].as_str().unwrap();
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    db.execute("UPDATE history_source_positions SET position_json=?2 WHERE source=?1",instrukt_core::state_db::rusqlite::params![source_id,serde_json::json!({"version":1,"cursors":{"messages":{"updated_at":"2099-01-01T00:00:00.000Z","native_id":"z-seen"}},"pending":[],"pending_after":null}).to_string()]).unwrap();
    drop(db);
    source.message(
        "child",
        "hydrated-after-upgrade",
        "Checkpointtoken restored.",
        "2025-01-01T00:00:00.000Z",
        false,
    );
    assert_eq!(
        search(&env, "Checkpointtoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn a_baseline_v2_position_preserves_loaded_observations_when_adding_the_current_field() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    source.thread("retired", None, None, "2026-11-08T10:00:00.000Z");
    source.message(
        "retired",
        "retired-message",
        "Retained historical content.",
        "2026-11-08T10:00:00.000Z",
        false,
    );
    env.run(env.home(), ["history", "extract", "--json"])
        .success();
    let native = instrukt_core::state_db::rusqlite::Connection::open(&source.file).unwrap();
    native.execute_batch("DELETE FROM orchestration_v2_projection_messages WHERE thread_id='retired';DELETE FROM orchestration_v2_projection_threads WHERE thread_id='retired';").unwrap();
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let old: Value =
        serde_json::from_str(include_str!("../fixtures/history-source-v2.json")).unwrap();
    db.execute(
        "UPDATE history_source_positions SET position_json=?1",
        [old.to_string()],
    )
    .unwrap();
    let result = env
        .run(env.home(), ["history", "extract", "--json"])
        .success();
    let result: Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert_eq!(result["loaded"], 0);
    assert_eq!(result["position"]["loaded"], old["loaded"]);
    let current: String = db
        .query_row(
            "SELECT position_json FROM history_source_positions",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&current).unwrap(),
        result["position"]
    );
    insta::assert_json_snapshot!("upgraded-baseline-v2-position", result["position"]);
    assert_eq!(
        search(&env, "jewels")["matches"].as_array().unwrap().len(),
        3
    );
}

// spec: project/instrukt/spec/feature/runner/history UC-HI1
#[test]
fn disabled_history_search_preserves_retained_conversations() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    assert_eq!(
        search(&env, "jewels")["matches"].as_array().unwrap().len(),
        2
    );
    let config = env.instrukt_home().join("instrukt.yaml");
    let original = std::fs::read_to_string(&config).unwrap().replacen(
        "schema_version: 1",
        &format!(
            "schema_version: {}",
            instrukt_core::config::CURRENT_SCHEMA_VERSION
        ),
        1,
    );
    std::fs::write(
        &config,
        format!("{original}\nmodules: {{history: false}}\n"),
    )
    .unwrap();
    let result = env
        .run(env.home(), ["history", "search", "jewels"])
        .failure();
    insta::assert_snapshot!(String::from_utf8_lossy(&result.get_output().stderr), @"instrukt: the history module is off; switch it on with `modules.history: true` in instrukt.yaml");
    source.remove_threads();
    std::fs::write(&config, original).unwrap();
    assert_eq!(
        search(&env, "jewels")["matches"].as_array().unwrap().len(),
        2
    );
}
