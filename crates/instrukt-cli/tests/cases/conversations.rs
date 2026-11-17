//! Conversations through the real runner: a company member addresses the
//! company's agent through the Slack stand-in, the agent's session runs in the
//! fake T3 host, and its answer is posted back.
#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]

use crate::common;
use crate::fixtures::conversing::{Company, wait};
use crate::fixtures::documented;
use instrukt_runner::connectors::SLACK;
use instrukt_runner::conversations::{self, SLACK_MESSAGE_LIMIT, TURN_AWAY};
use instrukt_runner::transport::EVENT_STREAM;

/// Where each post went: its channel and thread.
fn destinations(c: &Company) -> Vec<(String, Option<String>)> {
    c.slack
        .posts()
        .into_iter()
        .map(|post| (post.channel, post.thread))
        .collect()
}

// spec: project/instrukt/spec/feature/runner/conversations UC-CV1
#[test]
fn a_members_question_is_answered_in_that_conversation_and_nowhere_else() {
    let mut c = Company::new("");
    c.run();
    let channel = "C1";
    // The runner dies before the runtime accepted the question's opening.
    c.runtime.flag("die-before-start", true);
    let question = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> When do we ship?", None);
    c.runner_ended();
    assert!(c.sessions().is_empty());
    // Restarted, the question stays pending with its cause: a start that
    // remains unresolved, its key and the command that resolves it.
    c.run();
    wait("the pending question's cause", || {
        c.conversations()
            .conversations
            .iter()
            .any(|pending| !pending.failures.is_empty())
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
    // A plain agent in the company's home under the runner-supplied role.
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
        instrukt_core::config::CONVERSATION_ROLE
    );
    assert_eq!(assignment.procedure, None);
    assert_eq!(assignment.company, Some(c.company()));
    wait("the answer's pieces", || c.slack.posts().len() >= 2);
    wait("nothing left to post", || {
        c.conversations().conversations.is_empty()
    });
    assert_eq!(
        destinations(&c),
        [
            (channel.to_owned(), Some(question.clone())),
            (channel.to_owned(), Some(question))
        ]
    );
    // Together the pieces carry the whole answer.
    assert_eq!(
        c.slack
            .posts()
            .iter()
            .map(|post| post.text.chars().count())
            .sum::<usize>(),
        reply.chars().count()
    );
    // One session, which received one opening input.
    assert_eq!(c.sessions(), [session.as_str()]);
    assert_eq!(c.runtime.message_count(&session), 1);
}

// spec: project/instrukt/spec/feature/runner/conversations UC-CV3
#[test]
fn two_members_in_one_conversation_each_have_their_own_turn_in_order() {
    let mut c = Company::new("");
    c.run();
    let root = c
        .slack
        .mention("C1", "USTEVEN", "<@UAGENT> Is the build green?", None);
    let session = c.session();
    // While the first turn runs, two more arrive out of the order they were
    // written in, within one second.
    c.slack.mention_at(
        "1700000500.000100",
        "C1",
        "USTEVEN",
        "<@UAGENT> And the changelog?",
        Some(&root),
    );
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
    let order = [c.member("USTEVEN"), c.member("UANNA"), c.member("USTEVEN")];
    for (n, member) in order.into_iter().enumerate() {
        let delivered = c.delivered(&session, n);
        assert_eq!(delivered.member, member, "input {n}");
        assert_eq!(delivered.members, 1, "input {n}");
        assert_eq!(delivered.empty, 0, "input {n}");
        // Each started its own turn: none arrived while a turn was running,
        // as the runtime recorded on its arrival.
        assert!(!c.queued(&session, n), "input {n}");
    }
    assert_eq!(c.sessions().len(), 1);
}

// spec: project/instrukt/spec/feature/runner/conversations UC-CV4
#[test]
fn an_unlisted_person_receives_one_fixed_reply_and_no_session_starts() {
    let mut c = Company::new("");
    c.run();
    let channel = "C1";
    let first = c
        .slack
        .mention(channel, "USTRANGER", "<@UAGENT> Who are you?", None);
    c.slack
        .mention(channel, "USTRANGER", "<@UAGENT> Hello?", Some(&first));
    // A member's direct message after both: once it reaches a session, the
    // connector has handled everything before it.
    c.slack.direct("D1", "USTEVEN", "Are you there?");
    let session = c.session();
    assert_eq!(c.delivered(&session, 0).member, c.member("USTEVEN"));
    let posts = c.slack.posts();
    assert_eq!(destinations(&c), [(channel.to_owned(), Some(first))]);
    assert_eq!(posts[0].text, TURN_AWAY);
    assert_eq!(c.sessions(), [session]);
}

// spec: project/instrukt/spec/feature/runner/conversations UC-CV5
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
    assert_eq!(c.delivered(&session, 0).member, c.member("USTEVEN"));
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
