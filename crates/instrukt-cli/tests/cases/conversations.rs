//! Conversations through the real runner: a company member addresses the
//! company's agent through the Slack stand-in, the agent's session runs in the
//! fake T3 host, and its answer is posted back.
#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]

use crate::common;
use crate::fixtures::conversing::{COMPANY, Company, patch_company, wait};
use crate::fixtures::documented;
use instrukt_runner::connectors::{SLACK, TURN_AWAY, WORKING};
use instrukt_runner::conversations::{self, SLACK_MESSAGE_LIMIT};
use instrukt_runner::transport::EVENT_STREAM;

/// Where each post went: its channel and thread.
fn destinations(c: &Company) -> Vec<(String, Option<String>)> {
    c.slack
        .posts()
        .into_iter()
        .map(|post| (post.channel, post.thread))
        .collect()
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV1
#[test]
fn a_members_question_is_answered_in_that_conversation_and_nowhere_else() {
    // History is off, so the conversation relay is the stand-in's only
    // source reader while the runner runs.
    let mut c = Company::without_procedure(common::TestEnv::new(), ", history: false", "", "");
    // The company starts its sessions on its own order of profiles.
    let profile = instrukt_core::ids::ProfileName::parse("antigravity").unwrap();
    patch_company(&c.env, COMPANY, &format!("profiles: [{profile}]"));
    // The computer runs other work: many other threads, one of them
    // streaming and rewritten while instrukt reads.
    let mut elsewhere = c.store.elsewhere(200, "Elsewhere chatter.");
    elsewhere.stream();
    let gate = c.store.gate();
    c.run();
    // The relay runs, so its subscription to T3's database exists; watching
    // it holds no open file on T3's data folder.
    let t3_folder = c.store.file.parent().unwrap().to_path_buf();
    assert_eq!(c.conversations().relay_failure, None);
    assert_eq!(c.runner_files_on(&t3_folder), 0);
    let channel = "C1";
    // No published procedure declares the role conversations start under:
    // the question waits, no session starts, and the cause names the role.
    let question = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> When do we ship?", None);
    wait("the waiting question's cause", || {
        c.conversations()
            .conversations
            .iter()
            .any(|pending| !pending.failures.is_empty())
    });
    let waiting = c.status()["conversations"]["conversations"].clone();
    insta::assert_snapshot!(
        "no-procedure-status",
        serde_json::to_string_pretty(&waiting).unwrap()
    );
    assert!(c.sessions().is_empty());
    // Once the procedure is published, the runner starts the session and
    // dies before the runtime accepted the question's opening.
    c.runtime.flag("die-before-start", true);
    c.publish_procedure();
    c.runner_ended();
    assert!(c.sessions().is_empty());
    // Restarted, the question stays pending with its cause: a start that
    // remains unresolved, its key and the command that resolves it. Its
    // asker is admitted, so Slack shows the agent working on the thread.
    c.run();
    wait("the working status on the thread", || {
        c.slack.status(channel, &question).as_deref() == Some(WORKING)
    });
    wait("the pending question's new cause", || {
        c.status()["conversations"]["conversations"] != waiting
    });
    insta::assert_snapshot!(
        "unresolved-opening-status",
        serde_json::to_string_pretty(&c.status()["conversations"]["conversations"]).unwrap()
    );
    assert!(c.sessions().is_empty());
    let key = conversations::key(&c.company(), SLACK, &format!("{channel}/{question}"));
    // Nothing was created: the start is resolved as such, and the retry dies
    // after the runtime accepted the opening and before the runner bound it.
    c.runtime.flag("die-after-start", true);
    c.env
        .run(c.env.home(), ["sessions", "resolve-start", &key, "--none"])
        .success();
    c.runner_ended();
    let session = c.session();
    assert_eq!(c.runtime.message_count(&session), 1);
    // Restarted while the opening turn runs, with the source otherwise
    // quiet: recovery binds the session and reads its thread mid-turn. The
    // runner dies once that read is retained, as its next source read
    // begins, and the turn finishes while it is down.
    c.store
        .thread(&session, None, None, "2026-11-08T10:00:00.000Z");
    elsewhere.settle();
    gate.hold();
    c.run();
    gate.admit();
    elsewhere.nudge();
    gate.held();
    c.runner_dies();
    gate.open();
    // Someone types in the session's thread directly, and both turns finish
    // while the runner is down: the typed one comes first in the thread, so
    // it is read before the answer's, and it is not posted. An answer longer
    // than one Slack message arrives as its pieces.
    c.store.turn(
        &session,
        "direct",
        0,
        "A note typed in the thread.",
        "Noted.",
    );
    let reply = "x".repeat(SLACK_MESSAGE_LIMIT + 1);
    c.finish_turn(&session, 0, &reply);
    c.run();
    // The session opens with its procedure's loader under the role, then the
    // question; its recorded assignment is the role in the company's home.
    insta::assert_snapshot!("opening-from-procedure", c.opening(&session));
    let home = instrukt_core::home::Home::at_user_home(c.env.home().into());
    let caller = instrukt_core::ids::SessionRef::parse(&format!(
        "{}:{session}",
        instrukt_core::config::Runtime::T3.name()
    ))
    .unwrap();
    let resolved = || {
        instrukt_runner::assignments::resolve(
            &home,
            &caller,
            instrukt_todos::work::recorded_assignment,
        )
        .unwrap()
    };
    wait("the session's recorded binding", || resolved().is_some());
    let assignment = resolved().unwrap();
    assert_eq!(
        assignment.role.as_str(),
        instrukt_core::config::COMPANY_AGENT_ROLE
    );
    assert_eq!(assignment.company, Some(c.company()));
    wait("the answer's pieces", || c.slack.posts().len() >= 2);
    wait("nothing left to post", || {
        c.conversations().conversations.is_empty()
    });
    // While the other thread streams again, a later catch-up of this
    // conversation completes: the next source read stands at the gate
    // after it. Its outcome leaves no cause.
    elsewhere.stream();
    gate.hold();
    gate.admit();
    gate.held();
    let status = c.conversations();
    assert!(
        status.conversations.is_empty(),
        "{:?}",
        status.conversations
    );
    assert_eq!(status.relay_failure, None);
    gate.open();
    assert_eq!(
        destinations(&c),
        [
            (channel.to_owned(), Some(question.clone())),
            (channel.to_owned(), Some(question.clone()))
        ]
    );
    // Together the parts carry the whole answer, as Markdown.
    let posts = c.slack.posts();
    assert!(posts.iter().all(|post| post.text.is_none()));
    assert_eq!(
        posts
            .iter()
            .filter_map(|post| post.markdown.as_deref())
            .collect::<String>(),
        reply
    );
    // The session was started on the company's profile.
    assert_eq!(
        c.runtime.provider(&session),
        instrukt_core::config::Agents::load(&home).unwrap().profiles[&profile].instance
    );
    // One session, which received one opening input.
    assert_eq!(c.sessions(), [session.as_str()]);
    assert_eq!(c.runtime.message_count(&session), 1);
    // The turn ended: the status is cleared.
    wait("the status cleared", || {
        c.slack.status(channel, &question).is_none()
    });
    // The database holding the conversation, and the files beside it, are
    // its owner's alone.
    use std::os::unix::fs::PermissionsExt as _;
    for side in ["", "-wal", "-shm"] {
        let file = c
            .env
            .instrukt_home()
            .join(format!("state/instrukt.db{side}"));
        if let Ok(meta) = std::fs::metadata(&file) {
            assert_eq!(meta.permissions().mode() & 0o777, 0o600, "{side}");
        }
    }
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV3
#[test]
fn two_members_in_one_conversation_each_have_their_own_turn_in_order() {
    let mut c = Company::new("");
    c.run();
    let root = c
        .slack
        .mention("C1", "USTEVEN", "<@UAGENT> Is the build green?", None);
    let session = c.session();
    // While the first turn runs, two more arrive out of the order they were
    // written in, within one second. The later-written one is taken in
    // before the earlier one arrives: its lookup waits until then.
    c.slack.hold_lookups();
    c.slack.mention_at(
        "1700000500.000100",
        "C1",
        "USTEVEN",
        "<@UAGENT> And the changelog?",
        Some(&root),
    );
    wait("the later message taken in", || c.slack.held_lookups() > 0);
    c.slack.mention_at(
        "1700000500.000000",
        "C1",
        "UANNA",
        "<@UAGENT> And the release notes?",
        Some(&root),
    );
    wait("all three messages acknowledged", || {
        c.slack.acknowledged().len() == 3
    });
    c.slack.release_lookups();
    // The runner dies right after the runtime accepted the second message
    // and before it recorded that, and that turn finishes while it is down.
    c.runtime.flag("die-after-send", true);
    c.finish_turn(&session, 0, "Green.");
    c.runner_ended();
    assert_eq!(c.runtime.message_count(&session), 2);
    c.finish_turn(&session, 1, "Written.");
    c.run();
    // Restarted, it posts that answer once and the third message follows,
    // with no second copy of the second.
    wait("the third turn", || c.runtime.message_count(&session) == 3);
    assert_eq!(c.slack.posts().len(), 2);
    c.finish_turn(&session, 2, "Done.");
    wait("nothing left to post", || {
        c.conversations().conversations.is_empty() && c.slack.posts().len() == 3
    });
    assert_eq!(c.runtime.message_count(&session), 3);
    // Each names the person who wrote it, in written order. The third was
    // taken in before the second arrived, so what accompanies it holds the
    // second too.
    insta::assert_snapshot!(
        "three-turns-delivered",
        (0..3)
            .map(|n| c.input(&session, n))
            .collect::<Vec<_>>()
            .join("\n---\n")
    );
    for n in 0..3 {
        let delivered = c.delivered(&session, n);
        assert_eq!(delivered.members, 1, "input {n}");
        assert_eq!(delivered.empty, 0, "input {n}");
        // Each started its own turn: none arrived while a turn was running,
        // as the runtime recorded on its arrival.
        assert!(!c.queued(&session, n), "input {n}");
    }
    assert_eq!(c.sessions().len(), 1);
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV4
#[test]
fn a_person_who_is_not_one_of_the_companys_own_receives_one_fixed_reply_and_no_session_starts() {
    let mut c = Company::new("");
    // The runner's clock stands still, so the wait Slack names does not pass
    // while it runs.
    c.env.set_env("INSTRUKT_NOW", "2030-01-01T09:00:00Z");
    c.run();
    let channel = "C1";
    // Slack limits the lookup of who wrote it and names a wait: the message
    // waits, nothing is delivered or replied, and the cause shows. No lookup
    // is made for this workspace while the wait lasts, also not for a
    // message that arrives meanwhile.
    let waiting = |c: &Company| {
        c.conversations()
            .conversations
            .iter()
            .filter(|pending| pending.pending_messages == 1 && !pending.failures.is_empty())
            .count()
    };
    c.slack.limit_lookup(30);
    let asked = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Are you there?", None);
    wait("the waiting message's cause", || waiting(&c) == 1);
    let looked_up = c.slack.lookups();
    c.slack.direct("D1", "UANNA", "Are you there too?");
    wait("the second waiting message's cause", || waiting(&c) == 2);
    assert_eq!(c.slack.lookups(), looked_up);
    assert!(c.sessions().is_empty());
    assert!(c.slack.posts().is_empty());
    // Restarted, the runner learns any wait again from Slack, which now
    // answers: both are answered.
    c.kill_runner();
    c.env.remove_env("INSTRUKT_NOW");
    c.run();
    wait("both conversations' sessions", || c.sessions().len() == 2);
    let mut inputs: Vec<String> = c
        .sessions()
        .iter()
        .map(|session| c.input(session, 0))
        .collect();
    inputs.sort();
    insta::assert_snapshot!("waited-messages-delivered", inputs.join("\n---\n"));
    // A guest gets one fixed reply in their conversation, however often
    // they write.
    let first = c
        .slack
        .mention(channel, "USTRANGER", "<@UAGENT> Who are you?", None);
    c.slack
        .mention(channel, "USTRANGER", "<@UAGENT> Hello?", Some(&first));
    // A person answered before is decided again for each message: deactivated
    // since, they are not answered.
    c.slack.change("USTEVEN", |person| person.deleted = true);
    c.slack
        .mention(channel, "USTEVEN", "<@UAGENT> Still there?", Some(&asked));
    wait("both replies", || c.slack.posts().len() == 2);
    assert_eq!(
        destinations(&c),
        [
            (channel.to_owned(), Some(first)),
            (channel.to_owned(), Some(asked.clone()))
        ]
    );
    // The fixed reply goes as plain text.
    for post in c.slack.posts() {
        assert_eq!(post.text.as_deref(), Some(TURN_AWAY));
        assert_eq!(post.markdown, None);
    }
    let sessions = c.sessions();
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions
            .iter()
            .map(|session| c.runtime.message_count(session))
            .sum::<usize>(),
        2
    );
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV5
#[test]
fn a_conversation_message_activates_no_subscribed_procedure() {
    let env = common::TestEnv::new();
    let redis = common::redis::RedisServer::new(&env);
    let project = documented::project(
        &env,
        "fork",
        &[(
            "instrukt.yaml",
            format!(
                "schema_version: {}\nproject: fork\n",
                instrukt_core::config::CURRENT_SCHEMA_VERSION
            ),
        )],
    );
    project.write(
        "docs/domain/reactions/procedure/everything.md",
        &documented::snippet(
            "procedure",
            "Everything",
            "Answer every fact.",
            "subscription_home: project\nsubscriptions: ['*']\nrole: responder\n",
            &[],
            "Answer every event.\n",
        ),
    );
    project.commit("docs: answer every event");
    documented::sync(&env, &project);
    let mut c = Company::on(
        env,
        ", events: true",
        &format!("redis: {{cluster: {{url: '{}'}}}}\n", redis.url),
        "",
    );
    c.run();
    c.slack
        .mention("C1", "USTEVEN", "<@UAGENT> Anything new?", None);
    let session = c.session();
    insta::assert_snapshot!("conversation-delivered", c.input(&session, 0));
    c.finish_turn(&session, 0, "Nothing new.");
    wait("the answer's post", || !c.slack.posts().is_empty());
    // An event after the conversation: once the procedure answered it, the
    // event reader has passed everything before it.
    let computer = std::fs::read_to_string(c.env.instrukt_home().join("state/machine-id")).unwrap();
    let barrier = "barrier-event";
    let event = serde_json::json!({"id": barrier, "type": "example.changed", "source": computer.trim(), "subject": "todo:demo", "home": "project:fork", "home_owner": "unresolved", "time": "2026-11-05T00:00:00Z", "data": {}});
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg(event.to_string())
        .query::<String>(&mut redis.connection())
        .unwrap();
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let activated = || -> Vec<String> {
        db.prepare("SELECT event_id FROM procedure_activations")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    wait("the event reader's activation", || !activated().is_empty());
    assert_eq!(activated(), [barrier]);
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV12
#[test]
fn a_member_is_answered_while_a_projects_event_responses_are_not_admitted() {
    // History is off, so the conversation relay is the stand-in's only
    // source reader while the runner runs. The machine has no Redis settings.
    let mut c = Company::on(common::TestEnv::new(), ", history: false", "", "");
    let profile = instrukt_core::ids::ProfileName::parse("antigravity").unwrap();
    patch_company(&c.env, COMPANY, &format!("profiles: [{profile}]"));
    // A published project switches event responses on through its committed
    // layer; this computer has no event transport for them.
    let events = documented::project(
        &c.env,
        "reacting",
        &[(
            "instrukt.yaml",
            format!(
                "schema_version: {}\nproject: reacting\nmodules: {{history: false, events: true}}\n",
                instrukt_core::config::CURRENT_SCHEMA_VERSION
            ),
        )],
    );
    documented::sync(&c.env, &events);
    // The runner starts under a clock file while a tick holds the dispatch
    // lock: its wake waits for the lock on that clock.
    let clock = c.env.path("clock");
    std::fs::write(&clock, "2026-11-18T12:00:00Z").unwrap();
    c.env
        .set_env("INSTRUKT_NOW", &format!("@{}", clock.display()));
    let lock = c.env.instrukt_home().join("state/runtime-dispatch.lock");
    let held = instrukt_core::fsx::LockGuard::try_acquire(&lock)
        .unwrap()
        .unwrap();
    c.run();
    // Past the run-lock wait the wake is skipped, shown in status, and the
    // runner goes on.
    std::fs::write(&clock, "2026-11-18T12:03:00Z").unwrap();
    let workers = |c: &Company| -> Vec<String> {
        c.status()["workers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["worker"].as_str().unwrap().to_owned())
            .collect()
    };
    wait("the skipped wake in status", || {
        workers(&c).contains(&"run-loop".to_owned())
    });
    assert!(c.runner_running());
    drop(held);
    // A member is answered.
    let channel = "C1";
    c.slack
        .mention(channel, "USTEVEN", "<@UAGENT> When do we ship?", None);
    let first = c.session();
    c.finish_turn(&first, 0, "On Friday.");
    wait("the first answer", || c.slack.posts().len() == 1);
    // Status names the project's event responses as not admitted.
    let findings = |c: &Company| -> Vec<(std::path::PathBuf, String)> {
        c.status()["admission"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    std::path::PathBuf::from(f["document"].as_str().unwrap()),
                    f["key"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    let project_events = (
        events.path().join("instrukt.yaml"),
        "modules.events".to_owned(),
    );
    assert_eq!(findings(&c), std::slice::from_ref(&project_events));
    // While the runner runs, the company's document is broken by hand and its
    // bot token's secret blanked.
    let company_dir = c.env.instrukt_home().join(format!("companies/{COMPANY}"));
    let document = company_dir.join("instrukt.yaml");
    let mut text = std::fs::read_to_string(&document).unwrap();
    text.push_str("bogus: 1\n");
    std::fs::write(&document, text).unwrap();
    let secrets = company_dir.join("secrets.env");
    let blanked: String = std::fs::read_to_string(&secrets)
        .unwrap()
        .lines()
        .map(|line| {
            if line.starts_with("SLACK_BOT_TOKEN=") {
                "SLACK_BOT_TOKEN=\"\"\n".to_owned()
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    std::fs::write(&secrets, blanked).unwrap();
    wait("the company's findings in status", || {
        findings(&c).len() == 3
    });
    let mut found = findings(&c);
    found.sort();
    let mut expected = vec![
        project_events,
        (document.clone(), "bogus".to_owned()),
        (document.clone(), "connectors.slack.bot_token".to_owned()),
    ];
    expected.sort();
    assert_eq!(found, expected);
    // The member's next question still starts the agent on the company's
    // profile from the last valid generation, and is answered over the last
    // valid connection.
    c.slack
        .mention(channel, "USTEVEN", "<@UAGENT> And the release notes?", None);
    wait("the second session", || c.sessions().len() == 2);
    let second = c
        .sessions()
        .into_iter()
        .find(|session| *session != first)
        .unwrap();
    let home = instrukt_core::home::Home::at_user_home(c.env.home().into());
    assert_eq!(
        c.runtime.provider(&second),
        instrukt_core::config::Agents::load(&home.for_inspection())
            .unwrap()
            .profiles[&profile]
            .instance
    );
    c.finish_turn(&second, 0, "They are ready.");
    wait("the second answer", || c.slack.posts().len() == 2);
}
