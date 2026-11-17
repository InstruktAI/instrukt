//! Conversations through the real runner: a company member addresses the
//! company's agent through the Slack stand-in, the agent's session runs in the
//! fake T3 host, and its answer is posted back.
#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]

use crate::common;
use crate::fixtures::conversing::{Company, wait};
use crate::fixtures::documented;
use crate::fixtures::slack::Post;
use instrukt_runner::conversations::TURN_AWAY;

// spec: project/instrukt/spec/feature/runner/conversations UC-CV1
#[test]
fn a_members_question_is_answered_in_that_conversation_and_nowhere_else() {
    let mut c = Company::new("");
    c.run();
    let channel = "C1";
    let question = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> When do we ship?", None);
    let session = c.session();
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
    let reply = "We ship on Friday.";
    // Someone types in the session's thread directly: that turn is not posted.
    c.store.turn(
        &session,
        "direct",
        100,
        "A note typed in the thread.",
        "Noted.",
    );
    c.finish_turn(&session, 0, reply);
    wait("the answer's post", || !c.slack.posts().is_empty());
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(
        c.slack.posts(),
        [Post {
            channel: channel.into(),
            thread: Some(question),
            text: reply.into(),
        }]
    );
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
    let later = "<@UAGENT> And the release notes?";
    c.slack.mention("C1", "UANNA", later, Some(&root));
    wait("both messages acknowledged", || {
        c.slack.acknowledged().len() == 2
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(
        c.runtime.message_count(&session),
        1,
        "the second turn waits for the first"
    );
    c.finish_turn(&session, 0, "Green.");
    wait("the second turn", || c.runtime.message_count(&session) == 2);
    assert_eq!(c.addressed(&session, 0).0, c.member("USTEVEN"));
    assert_eq!(
        c.addressed(&session, 1),
        (c.member("UANNA"), later.to_owned())
    );
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
    wait("the fixed reply", || !c.slack.posts().is_empty());
    c.slack
        .mention(channel, "USTRANGER", "<@UAGENT> Hello?", Some(&first));
    wait("both messages acknowledged", || {
        c.slack.acknowledged().len() == 2
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(
        c.slack.posts(),
        [Post {
            channel: channel.into(),
            thread: Some(first),
            text: TURN_AWAY.into(),
        }]
    );
    assert!(c.sessions().is_empty());
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
    c.finish_turn(&session, 0, "Nothing new.");
    wait("the answer's post", || !c.slack.posts().is_empty());
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let count = |table: &str| -> i64 {
        db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    assert_eq!(count("procedure_activations"), 0);
    assert_eq!(count("procedure_responsibilities"), 0);
    assert_eq!(c.sessions(), [session]);
}
