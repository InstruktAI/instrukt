//! History is exercised against fixture source stores in temporary homes.
#![allow(clippy::unwrap_used, reason = "isolated CLI fixtures")]
mod common;
#[path = "support/history.rs"]
mod fixture;

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
    let out = env
        .run(env.home(), ["history", "search", query, "--json"])
        .success();
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
    let _source = fixture::T3Store::new(&env);
    let mirror = fixture::mirror(&env);
    let original = std::fs::read(&mirror).unwrap();
    env.run(env.home(), ["history", "import-teleclaude", "--json"])
        .success();
    assert_eq!(std::fs::read(&mirror).unwrap(), original);
    std::fs::remove_file(mirror).unwrap();
    env.run(env.home(), ["history", "import-teleclaude", "--json"])
        .success();
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
fn runner_completion_keeps_history_current_without_a_search_or_tick() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    let found = search(&env, "jewels");
    let source_id = found["matches"][0]["source"].as_str().unwrap();
    let redis = common::redis::RedisServer::new(&env);
    fixture::configure_cluster(&env, &redis.url);
    let _runner = fixture::runner(&env);
    source.wait_for_readers(2);
    source.complete();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let out = env
            .run(env.home(), ["history", "show", source_id, "child"])
            .success();
        let document: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
        if document["messages"].as_array().unwrap().len() == 2 {
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
