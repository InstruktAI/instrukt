//! History is exercised against fixture source stores in temporary homes.
#![allow(clippy::unwrap_used, reason = "isolated CLI fixtures")]

use crate::common;
use crate::fixtures::history as fixture;
use common::TestEnv;
use serde_json::Value;
use std::time::Duration;

fn stable_sources(mut result: Value) -> Value {
    for found in result["matches"].as_array_mut().unwrap() {
        let kind = found["source"].as_str().unwrap().split_once(':').unwrap().0;
        found["source"] = Value::String(format!("{kind}:[SOURCE]"));
    }
    result
}

/// Removes the history store and its sidecars, so the next history operation
/// finds only what the state database holds, as after upgrading from a build
/// that kept history there.
fn remove_history_store(env: &TestEnv) {
    for name in ["history.db", "history.db-wal", "history.db-shm"] {
        let file = env.instrukt_home().join("state").join(name);
        if file.exists() {
            std::fs::remove_file(file).unwrap();
        }
    }
}

/// How many conversations one explicit extraction loaded.
fn extracted(env: &TestEnv) -> u64 {
    let out = env
        .run(env.home(), ["history", "extract", "--json"])
        .success();
    let report: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    report["loaded"].as_u64().unwrap()
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
    // Other conversations beside the scenario's own, one still streaming.
    let elsewhere = source.elsewhere(40, "Elsewheretoken chatter.");
    let gate = source.gate();
    // A conversation changes after discovery; its fresh read snapshot
    // includes the edit without making the catch-up incomplete.
    gate.hold();
    let searching = env
        .instrukt_process(env.home())
        .args(["history", "search", "Elsewheretoken", "--json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    gate.admit();
    gate.held();
    source.message(
        "elsewhere-0000",
        "elsewhere-0000-message",
        "Elsewheretoken changed.",
        "2026-11-08T09:30:00.000Z",
        false,
    );
    gate.open();
    let searched = searching.wait_with_output().unwrap();
    assert!(searched.status.success());
    let complete: Value = serde_json::from_slice(&searched.stdout).unwrap();
    assert_eq!(complete["matches"].as_array().unwrap().len(), 40);
    assert_eq!(complete["failures"].as_array().unwrap().len(), 0);
    // An unchanged catch-up retains the same conversations.
    let settled = search(&env, "Elsewheretoken");
    assert_eq!(settled["matches"].as_array().unwrap().len(), 40);
    assert_eq!(settled["failures"].as_array().unwrap().len(), 0);
    env.run(env.home(), ["history", "extract", "--json"])
        .success();
    let first = search(&env, "jewels");
    assert_eq!(first["matches"].as_array().unwrap().len(), 2);
    // The streaming answer, once finished, is extracted too.
    drop(elsewhere);
    assert_eq!(
        search(&env, "Elsewheretoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        41
    );
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
fn legacy_import_resumes_in_batches_and_keeps_aliases_once_without_a_continuing_mirror_dependency()
{
    let env = TestEnv::new();
    let t3 = fixture::T3Store::new(&env);
    let mirror = fixture::mirrors(&env);
    let import = |expect_success: bool| {
        let before = std::fs::read(&mirror).unwrap();
        let run = env.run(
            env.home(),
            [
                "history",
                "import-teleclaude",
                "--source",
                mirror.to_str().unwrap(),
                "--json",
            ],
        );
        if expect_success {
            run.success();
        } else {
            run.failure();
        }
        assert_eq!(std::fs::read(&mirror).unwrap(), before);
    };
    let batched = |env: &TestEnv, term: &str| -> Vec<String> {
        let out = env
            .run(
                env.home(),
                ["history", "search", term, "--limit", "1000", "--json"],
            )
            .success();
        let found: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
        let mut conversations: Vec<String> = found["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["conversation"].as_str().unwrap().to_owned())
            .collect();
        conversations.sort();
        conversations
    };
    // A current state database without history tables, and no history store;
    // nothing the import, a catch-up or a search does may take its writer lock.
    env.run(env.home(), ["history", "extract", "--json"])
        .success();
    remove_history_store(&env);
    let _writer = instrukt_core::fsx::LockGuard::acquire(
        &env.instrukt_home().join("state/writer.lock"),
        Duration::from_secs(5),
    )
    .unwrap();
    // The import stops at a mirror it cannot read; what it committed stays.
    import(false);
    let first = batched(&env, fixture::BATCHED_TERM);
    assert!(!first.is_empty());
    assert!(first.len() < fixture::batched_ids().count());
    // The first batch starts at the lowest mirror id, below zero.
    assert_eq!(
        batched(&env, fixture::NONPOSITIVE_TERM).len(),
        fixture::batched_ids().filter(|id| *id <= 0).count()
    );
    // A rerun continues after the committed mirrors without reading them.
    fixture::retitle(&mirror, fixture::UNREADABLE, Some("Repaired"));
    fixture::retitle(&mirror, -5, None);
    import(true);
    let mut all = batched(&env, fixture::BATCHED_TERM);
    let found = all.len();
    all.dedup();
    assert_eq!(
        (found, all.len()),
        (
            fixture::batched_ids().count(),
            fixture::batched_ids().count()
        )
    );
    std::fs::remove_file(&mirror).unwrap();
    env.run(
        env.home(),
        [
            "history",
            "import-teleclaude",
            "--source",
            mirror.to_str().unwrap(),
            "--json",
        ],
    )
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
    crate::common::wait_until(
        "startup marker to be retained",
        Duration::from_secs(10),
        || {
            let out = env
                .run(env.home(), ["history", "show", source_id, "child"])
                .success();
            let document: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
            document["messages"].as_array().unwrap().len() == 2
        },
    );
    source.complete();
    crate::common::wait_until(
        "completion to be extracted",
        Duration::from_secs(10),
        || {
            let out = env
                .run(env.home(), ["history", "show", source_id, "child"])
                .success();
            let document: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
            document["messages"].as_array().unwrap().len() == 3
        },
    );
}

// spec: project/instrukt/spec/feature/runner/history UC-HI4
#[test]
fn history_follows_a_t3_database_that_appears_while_the_runner_is_running() {
    let env = TestEnv::new();
    let redis = common::redis::RedisServer::new(&env);
    fixture::computer(&env, "aaaaaaaaaaaaaaaa");
    fixture::configure_cluster(&env, &redis.url);
    // The runner starts on a computer where T3's database does not exist.
    let mut runner = fixture::runner(&env);
    let pid = runner.running();
    assert!(pid.is_some(), "the runner is running");
    // The source's identity, read without a search: a search catches up by
    // itself.
    let home = instrukt_core::home::Home::at_user_home(env.home().into());
    let source_env =
        instrukt_core::config::Env::from_pairs([("HOME", env.home().display().to_string())]);
    let source_id = instrukt_runner::runtime::adapter(instrukt_core::config::Runtime::T3)
        .conversation_source(&home, &source_env)
        .unwrap()
        .identity()
        .to_owned();
    // How many of the conversation's messages history holds; a conversation
    // it does not hold yet is refused.
    let messages_of = |conversation: &str| {
        let out = env.run(env.home(), ["history", "show", &source_id, conversation]);
        let output = out.get_output();
        if !output.status.success() {
            return 0;
        }
        let document: Value = serde_json::from_slice(&output.stdout).unwrap();
        document["messages"].as_array().map_or(0, Vec::len)
    };
    let messages = || messages_of("child");
    // T3 creates its database with a conversation, and history holds it.
    let source = fixture::T3Store::create(&env);
    crate::common::wait_until(
        "the seeded conversation to be held",
        Duration::from_secs(20),
        || messages() == 1,
    );
    // Only then the developer completes the conversation, and history holds
    // the completion without a search or a tick.
    source.complete();
    crate::common::wait_until("the completion to be held", Duration::from_secs(20), || {
        messages() == 2
    });
    // T3's database is removed and created again while the runner runs; a
    // conversation begun after that is held, read without a search or a
    // tick, by the same runner process.
    source.remove();
    let recreated = fixture::T3Store::create(&env);
    recreated.thread("later", None, None, "2026-11-08T12:00:00.000Z");
    recreated.message(
        "later",
        "message-later",
        "Begun after the database came back.",
        "2026-11-08T12:00:00.000Z",
        false,
    );
    crate::common::wait_until(
        "the later conversation to be held",
        Duration::from_secs(20),
        || messages_of("later") == 1,
    );
    assert_eq!(runner.running(), pid, "the same runner process");
    // T3's database becomes unreadable for history while the runner runs: the
    // extraction worker shows its failure in status and goes on trying.
    let failing = || -> Vec<String> {
        let out = env
            .run(env.home(), ["events", "status", "--json"])
            .success();
        let status: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
        status["workers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["worker"].as_str().unwrap().to_owned())
            .collect()
    };
    let extraction = || failing().contains(&"history-extraction".to_owned());
    recreated.break_shape();
    crate::common::wait_until(
        "the extraction failure in status",
        Duration::from_secs(20),
        extraction,
    );
    // Meanwhile the machine layer is broken by hand, which stops every
    // command, status among them. T3's database reads again and a
    // conversation begins while it is broken; the runner holds on with the
    // configuration it admitted. Once the layer is mended, the same worker
    // has cleared its failure and holds what was begun.
    let machine = env.instrukt_home().join("instrukt.yaml");
    let admitted = std::fs::read_to_string(&machine).unwrap();
    std::fs::write(&machine, format!("{admitted}modules: [\n")).unwrap();
    env.run(env.home(), ["events", "status", "--json"])
        .failure()
        .code(1);
    recreated.restore_shape();
    recreated.thread("repaired", None, None, "2026-11-08T13:00:00.000Z");
    recreated.message(
        "repaired",
        "message-repaired",
        "Begun while the runner held on.",
        "2026-11-08T13:00:00.000Z",
        false,
    );
    std::fs::write(&machine, &admitted).unwrap();
    crate::common::wait_until(
        "the extraction failure to clear",
        Duration::from_secs(20),
        || !extraction(),
    );
    crate::common::wait_until(
        "the repaired conversation to be held",
        Duration::from_secs(20),
        || messages_of("repaired") == 1,
    );
    assert_eq!(runner.running(), pid, "the same runner process");
    // A failure one runner process recorded is cleared by the next one's
    // worker once it extracts.
    recreated.break_shape();
    crate::common::wait_until(
        "the extraction failure in status",
        Duration::from_secs(20),
        extraction,
    );
    drop(runner);
    recreated.restore_shape();
    let _next = fixture::runner(&env);
    crate::common::wait_until(
        "the earlier runner's failure to clear",
        Duration::from_secs(20),
        || !extraction(),
    );
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
    let source_id = found["matches"][0]["source"].as_str().unwrap().to_owned();
    let file = env.instrukt_home().join("state/instrukt.db");
    std::fs::remove_file(&file).unwrap();
    remove_history_store(&env);
    let db = instrukt_core::state_db::rusqlite::Connection::open(&file).unwrap();
    db.execute_batch(include_str!("../fixtures/state-v6.sql"))
        .unwrap();
    db.execute_batch(include_str!("../fixtures/history-v7.sql"))
        .unwrap();
    let document = serde_json::json!({"thread":{"id":"child","title":"Retained child","lineage":{"parentThreadId":"parent"}},"messages":[{"id":"pre-upgrade","threadId":"child","role":"assistant","text":"Upgradetoken retained only in history."}],"turnItems":[{"id":"retained-tool","threadId":"child","type":"command_execution","text":"Textonlytoken","input":"Inputonlytoken","output":"Outputonlytoken","markdown":"Markdownonlytoken","diffStr":"Diffonlytoken"}]});
    db.execute("INSERT INTO history_conversations(source,native_id,computer,title,parent,aliases,document,body,limitations) VALUES(?1,'child','aaaaaaaaaaaaaaaa','Retained child','parent','[]',?2,'Upgradetoken retained only in history. Textonlytoken Inputonlytoken Outputonlytoken Markdownonlytoken Diffonlytoken','[]')",instrukt_core::state_db::rusqlite::params![source_id,document.to_string()]).unwrap();
    // A TeleClaude import that completed before upgrading, whose mirror store
    // is gone since.
    let mirrors = env.home().join("retired/mirrors.db");
    let teleclaude = format!(
        "teleclaude:{}",
        hex::encode(<sha2::Sha256 as sha2::Digest>::digest(
            mirrors.to_str().unwrap().as_bytes()
        ))
    );
    db.execute("INSERT INTO history_conversations(source,native_id,computer,title,parent,aliases,document,body,limitations) VALUES(?1,'archive-session','old-computer','Retained archive',NULL,'[\"source-alias\"]','{}','Archivedtoken kept from the mirrors.','[]')",[&teleclaude]).unwrap();
    db.execute(
        "INSERT INTO history_sources(source,position,imported) VALUES(?1,1,1)",
        [&teleclaude],
    )
    .unwrap();
    drop(db);
    // The retained conversation keeps its thread and its message, its text
    // once, and nothing its tool records held.
    let out = env
        .run(env.home(), ["history", "show", &source_id, "child"])
        .success();
    let document: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    insta::assert_json_snapshot!("upgraded-document", document);
    assert_eq!(
        search(&env, "Upgradetoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // The moved tables and the pages they freed have left the state database.
    let length = std::fs::metadata(&file).unwrap().len();
    let db = instrukt_core::state_db::rusqlite::Connection::open(&file).unwrap();
    let (tables, free, pages, page): (i64, i64, u64, u64) = db
        .query_row(
            "SELECT (SELECT count(*) FROM sqlite_master WHERE name IN ('history_conversations','history_fts','history_projection_records','history_source_positions','history_sources')),(SELECT freelist_count FROM pragma_freelist_count),(SELECT page_count FROM pragma_page_count),(SELECT page_size FROM pragma_page_size)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    drop(db);
    assert_eq!((tables, free, pages * page), (0, 0, length));
    // The completed import is preserved without its mirror store.
    env.run(
        env.home(),
        [
            "history",
            "import-teleclaude",
            "--source",
            mirrors.to_str().unwrap(),
            "--json",
        ],
    )
    .success();
    assert_eq!(
        search(&env, "Archivedtoken")["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        search(
            &env,
            "Textonlytoken OR Inputonlytoken OR Outputonlytoken OR Markdownonlytoken OR Diffonlytoken"
        )["matches"]
            .as_array()
            .unwrap()
            .len(),
        0
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
fn only_conversation_records_and_messages_are_read_and_retained() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    source.native_records();
    // The messages are found; what only a tool call, a reasoning trace, a
    // run, a subagent or captured terminal, diff and page context held is not.
    for said in ["Useritemtoken", "Assistantitemtoken"] {
        assert_eq!(search(&env, said)["matches"].as_array().unwrap().len(), 1);
    }
    let result = search(
        &env,
        "Toolitemtoken OR Reasonitemtoken OR Subagenttoken OR Terminalonlytoken OR Diffonlytoken OR Htmlonlytoken",
    );
    assert_eq!(result["matches"].as_array().unwrap().len(), 0);
    let source_id = search(&env, "Useritemtoken")["matches"][0]["source"]
        .as_str()
        .unwrap()
        .to_owned();
    let out = env
        .run(env.home(), ["history", "show", &source_id, "child"])
        .success();
    let document: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    insta::assert_json_snapshot!("text-only-document", document);
    // A change to a run, a subagent or a turn item is no change to history.
    source.rewrite_runs_subagents_and_turn_items();
    assert_eq!(extracted(&env), 0);
    // Nor is the loss of their tables: no part of the read consumes them.
    source.remove_runs_subagents_and_turn_items();
    assert_eq!(extracted(&env), 0);
    // A new message is read, without them.
    source.complete();
    assert_eq!(extracted(&env), 1);
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
fn a_delivered_timestamp_position_migrates_to_complete_conversation_detection() {
    let env = TestEnv::new();
    let source = fixture::T3Store::new(&env);
    let found = search(&env, "jewels");
    let source_id = found["matches"][0]["source"].as_str().unwrap();
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        env.instrukt_home().join("state/history.db"),
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
        env.instrukt_home().join("state/history.db"),
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
