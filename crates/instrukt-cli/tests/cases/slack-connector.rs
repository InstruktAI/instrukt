//! The Slack connector through the real runner and the Slack stand-in at the
//! network line: an outward Socket Mode connection, what reaches the agent and
//! what accompanies it, and where and how answers are posted.
#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]

use crate::fixtures::conversing::{Company, company_home, slack_settings, wait};
use crate::fixtures::slack::{APP_TOKEN, BOT_TOKEN, Post};
use instrukt_company::conversations::{ConnectionState, Status};
use instrukt_connector_slack::{API_URL_SETTING, WORKING};
use instrukt_core::ids::ProjectSlug;

/// What `instrukt run` says on standard error as it refuses to start. It
/// refuses before it serves anything: a runner that starts instead is
/// stopped once the wait's bound has passed, and the test fails naming the
/// refusal that did not come.
fn refused_at_start(c: &Company) -> String {
    use std::io::Read as _;
    /// The runner this test started; still running when the test fails, it
    /// is killed, and only it.
    struct Started(std::process::Child);
    impl Drop for Started {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let mut run = Started(
        c.env
            .instrukt_process(c.env.home())
            .arg("run")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    wait(
        "`instrukt run` to refuse a remote Slack address at its start",
        || run.0.try_wait().unwrap().is_some(),
    );
    assert!(!run.0.wait().unwrap().success());
    let mut stderr = String::new();
    run.0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    stderr
}

// spec: project/instrukt/spec/feature/company/slack-connector UC-SL1
#[test]
fn addressed_messages_arrive_over_an_outward_connection_and_refused_tokens_stay_visible() {
    let mut c = Company::new("");
    company_home(
        &c.env,
        "bravo",
        "xapp-refused",
        "xoxb-refused",
        &slack_settings("{}", ""),
    );
    // Tokens go to Slack's own hosts, or to a stand-in on this computer only.
    let stand_in = c.slack.api_url.clone();
    c.env
        .set_env(API_URL_SETTING, "http://slack.example.com/api");
    insta::assert_snapshot!("remote-slack-address-refused", refused_at_start(&c));
    c.env.set_env(API_URL_SETTING, &stand_in);
    c.run();
    let bravo = ProjectSlug::parse("bravo").unwrap();
    let state = |company: &ProjectSlug| {
        let status: Status = serde_json::from_value(c.status()["conversations"].clone()).unwrap();
        status
            .connectors
            .into_iter()
            .find(|connector| ProjectSlug::parse(&connector.company).as_ref() == Ok(company))
            .map(|connector| connector.state)
    };
    wait("the refused connector's failure", || {
        state(&bravo) == Some(ConnectionState::Failed)
    });
    assert_eq!(state(&c.company()), Some(ConnectionState::Connected));
    // A company that declares a connector while the run goes on is not
    // hosted by it, and shows as stopped beside the ones it hosts.
    company_home(
        &c.env,
        "charlie",
        APP_TOKEN,
        BOT_TOKEN,
        &slack_settings("{}", ""),
    );
    let charlie = ProjectSlug::parse("charlie").unwrap();
    assert_eq!(state(&charlie), Some(ConnectionState::Stopped));
    assert_eq!(state(&c.company()), Some(ConnectionState::Connected));
    // Slack's acknowledgements wait for no reading of what accompanies a
    // message: while the first message's read is held, a second message is
    // retained and acknowledged too.
    c.slack.hold_reads();
    c.slack
        .mention("C1", "USTEVEN", "<@UAGENT> Are you there?", None);
    wait("the first acknowledgement", || {
        c.slack.acknowledged().len() == 1
    });
    wait("the first message's held read", || c.slack.held() > 0);
    c.slack.mention("C2", "UANNA", "<@UAGENT> And here?", None);
    wait("the second acknowledgement", || {
        c.slack.acknowledged().len() == 2
    });
    c.slack.release_reads();
    wait("both conversations' sessions", || c.sessions().len() == 2);
    let mut inputs: Vec<String> = c
        .sessions()
        .iter()
        .map(|session| {
            assert_eq!(c.delivered(session, 0).empty, 0);
            c.input(session, 0)
        })
        .collect();
    inputs.sort();
    insta::assert_snapshot!("both-conversations-delivered", inputs.join("\n---\n"));
    // Stopped by a signal, the runner records nothing: no running runner
    // hosts a connector, so each shows as stopped.
    c.kill_runner();
    let status: Status = serde_json::from_value(c.status()["conversations"].clone()).unwrap();
    assert!(!status.connectors.is_empty());
    for connector in status.connectors {
        assert_eq!(connector.state, ConnectionState::Stopped);
    }
}

// spec: project/instrukt/spec/feature/company/slack-connector UC-SL2
#[test]
fn in_a_channel_only_a_tag_reaches_the_agent_with_the_most_recent_messages_since_the_previous_tag()
{
    let mut c = Company::new(", context_messages: 2");
    c.run();
    // Before the tag: a message, Slack's own notice of a person joining, and
    // a message in a form the connector does not know.
    let root = c.slack.say("C1", "UANNA", "Shall we ship on Friday?", None);
    c.slack.notice(
        "C1",
        "channel_join",
        "UBRAM",
        "<@UBRAM> has joined the channel",
        Some(&root),
    );
    c.slack.notice(
        "C1",
        "huddle_thread",
        "UBRAM",
        "A huddle with <@USTEVEN> happened.",
        Some(&root),
    );
    c.slack.mention(
        "C1",
        "USTEVEN",
        "<@UAGENT> What do you think, <@UANNA|anna>? Friday works for me.",
        Some(&root),
    );
    let session = c.session();
    let first = c.delivered(&session, 0);
    assert_eq!(first.accompanying, 2);
    assert!(!first.left_out);
    assert_eq!(first.empty, 0);
    // Speakers by the names Slack shows and mentions written as `@name`.
    insta::assert_snapshot!("first-tag-delivered", c.input(&session, 0));
    c.finish_turn(&session, 0, "Friday works.");
    wait("the answer's post", || !c.slack.posts().is_empty());
    let agreed = c.slack.say("C1", "UANNA", "Agreed.", Some(&root));
    c.slack.say(
        "C1",
        "USTEVEN",
        "Then QA runs on Thursday, <@UBRAM>.",
        Some(&root),
    );
    c.slack.notice(
        "C1",
        "channel_join",
        "UBRAM",
        "<@UBRAM> has joined the channel",
        Some(&root),
    );
    // A tag an edit adds to an earlier message starts nothing.
    c.slack
        .edit_into_mention("C1", "UANNA", &agreed, "Agreed, <@UAGENT>.", Some(&root));
    // A tag in a message that carries a file reaches the agent with its text.
    c.slack.mention_with_file(
        "C1",
        "UANNA",
        "<@UAGENT> Anything to add to the attached plan?",
        Some(&root),
    );
    wait("the tagged message", || {
        c.runtime.message_count(&session) == 2
    });
    // The untagged messages, the edited one among them, reached the agent
    // only as what accompanies the tag. Since the previous tag there were
    // three — the agent's own answer, then both — so the most recent two
    // accompany it, read across more than one page, and the context says
    // earlier ones were left out. The notice after them counts for nothing.
    let delivered = c.delivered(&session, 1);
    assert_eq!(delivered.members, 1);
    assert_eq!(delivered.accompanying, 2);
    assert!(delivered.left_out);
    assert_eq!(delivered.empty, 0);
    insta::assert_snapshot!("second-tag-delivered", c.input(&session, 1));
}

// spec: project/instrukt/spec/feature/company/slack-connector UC-SL3
#[test]
fn a_first_tag_brings_the_most_recent_messages_before_it() {
    let mut c = Company::new(", context_messages: 2");
    c.run();
    // A tag that starts its thread: the most recent of the channel.
    for user in ["USTEVEN", "UANNA", "UBRAM"] {
        c.slack.say("C1", user, "Earlier talk.", None);
    }
    c.slack
        .mention("C1", "USTEVEN", "<@UAGENT> Can you summarise?", None);
    let first = c.session();
    let delivered = c.delivered(&first, 0);
    assert_eq!(delivered.accompanying, 2);
    assert!(delivered.left_out);
    assert_eq!(delivered.empty, 0);
    insta::assert_snapshot!("first-tag-in-a-channel-delivered", c.input(&first, 0));
    // A first tag inside a long thread: the most recent of that thread.
    let root = c.slack.say("C2", "UBRAM", "A long thread.", None);
    for user in ["UANNA", "UBRAM", "USTEVEN"] {
        c.slack.say("C2", user, "More talk.", Some(&root));
    }
    c.slack
        .mention("C2", "UANNA", "<@UAGENT> Where are we?", Some(&root));
    wait("the second conversation's session", || {
        c.sessions().len() == 2
    });
    let second = c
        .sessions()
        .into_iter()
        .find(|session| *session != first)
        .unwrap();
    let delivered = c.delivered(&second, 0);
    assert_eq!(delivered.accompanying, 2);
    assert!(delivered.left_out);
    assert_eq!(delivered.empty, 0);
    insta::assert_snapshot!("first-tag-in-a-thread-delivered", c.input(&second, 0));

    // The reply event comes first. The root is already visible in Slack, but
    // only its own event should bring the channel history before the root.
    let history = "The channel chose a release date.";
    let channel = "C3";
    for user in ["USTEVEN", "UANNA", "UBRAM"] {
        c.slack.say(channel, user, history, None);
    }
    let late_root = "<@UAGENT> What did the channel decide?";
    let root = c.slack.say(channel, "USTEVEN", late_root, None);
    c.slack
        .mention(channel, "UANNA", "<@UAGENT> Any objections?", Some(&root));
    wait("the late-root conversation starts from the reply", || {
        c.sessions().len() == 3
    });
    let third = c
        .sessions()
        .into_iter()
        .find(|session| *session != first && *session != second)
        .unwrap();
    assert!(!c.input(&third, 0).contains(history));
    c.finish_turn(&third, 0, "NOOP");
    let conversation = format!("{channel}/{root}");
    wait(
        "the reply has completed before the root event arrives",
        || {
            c.conversations()
                .conversations
                .iter()
                .all(|row| row.conversation != conversation)
        },
    );
    c.slack
        .notify_mention(&root, channel, "USTEVEN", late_root, None);
    wait("the late root starts its own turn", || {
        c.runtime.message_count(&third) == 2
    });
    let delivered = c.delivered(&third, 1);
    assert_eq!(delivered.accompanying, 2);
    assert!(delivered.left_out);
    assert!(c.input(&third, 1).contains(history));

    let channel = "C4";
    let late_root = "<@UAGENT> What needs a decision?";
    let root = c.slack.say(channel, "USTEVEN", late_root, None);
    let intervening = "The team can meet on Friday.";
    c.slack.say(channel, "UBRAM", intervening, Some(&root));
    c.slack.refuse_replies(true);
    let question = c.slack.mention(
        channel,
        "UANNA",
        "<@UAGENT> What should we do?",
        Some(&root),
    );
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    wait(
        "the reply's first context read fails before the root arrives",
        || {
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE message=?1
                AND state='received' AND failure LIKE '%thread_not_found%')",
                [&question],
                |row| row.get::<_, bool>(0),
            )
            .unwrap()
        },
    );
    let previous_sessions = c.sessions();
    c.slack
        .notify_mention(&root, channel, "USTEVEN", late_root, None);
    wait("the root is delivered before Slack recovers", || {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE message=?1 AND state='sent')",
            [&root],
            |row| row.get::<_, bool>(0),
        )
        .unwrap()
    });
    let session = c
        .sessions()
        .into_iter()
        .find(|session| !previous_sessions.contains(session))
        .unwrap();
    assert!(c.input(&session, 0).contains("What needs a decision?"));
    // Drop replies requests begun before the root's acceptance. Recovery
    // must choose its range from retained delivery state.
    c.runner_dies();
    c.slack.refuse_replies(false);
    c.run();
    wait(
        "the reply captures its context while the root's turn is still running",
        || {
            db.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE message=?1 AND state='pending')",
            [&question], |row| row.get::<_, bool>(0),
        ).unwrap()
        },
    );
    c.finish_turn(&session, 0, "NOOP");
    wait("the retried reply reaches the agent", || {
        c.runtime.message_count(&session) == 2
    });
    let input = c.input(&session, 1);
    assert!(!input.contains("What needs a decision?"), "{input}");
    assert!(input.contains(intervening), "{input}");
    assert_eq!(c.delivered(&session, 1).accompanying, 1);
}

// spec: project/instrukt/spec/feature/company/slack-connector UC-SL4
#[test]
fn every_direct_message_reaches_the_agent_without_a_tag() {
    let mut c = Company::new("");
    c.run();
    let channel = "D1";
    let first = c
        .slack
        .direct(channel, "UANNA", "Can you check the invoice?");
    let session = c.session();
    // While the agent works, Slack shows it on the addressed message.
    wait("the status on the direct message", || {
        c.slack.status(channel, &first).as_deref() == Some(WORKING)
    });
    // Written with `/me`.
    let second = c.slack.direct_as(
        channel,
        "UANNA",
        "is waiting for the invoice",
        Some("me_message"),
        false,
    );
    wait("the status on the second direct message", || {
        c.slack.status(channel, &second).as_deref() == Some(WORKING)
    });
    let replies = [
        "The invoice is paid.",
        "Noted.",
        "The receipt matches.",
        "The file is here.",
    ];
    c.finish_turn(&session, 0, replies[0]);
    wait("the second direct message", || {
        c.runtime.message_count(&session) == 2
    });
    wait("the finished message's status cleared", || {
        c.slack.status(channel, &first).is_none()
    });
    // Each later message is written once the answers before it are posted,
    // so what accompanies it is fixed.
    wait("the first answer's post", || c.slack.posts().len() == 1);
    // A file with text, which Slack sends without a subtype, and a file
    // alone, which it sends as `file_share` with no text.
    c.slack
        .direct_as(channel, "UANNA", "Here is the receipt.", None, true);
    c.slack
        .direct_as(channel, "UANNA", "", Some("file_share"), true);
    // An edit and a deletion of earlier messages start nothing.
    c.slack
        .edit_direct(channel, "UANNA", &first, "Can you check both invoices?");
    c.slack.delete_direct(channel, &second);
    c.finish_turn(&session, 1, replies[1]);
    wait("the message with a file", || {
        c.runtime.message_count(&session) == 3
    });
    c.finish_turn(&session, 2, replies[2]);
    wait("the message with only a file", || {
        c.runtime.message_count(&session) == 4
    });
    c.finish_turn(&session, 3, replies[3]);
    wait("the answers' posts", || {
        c.slack.posts().len() == replies.len()
    });
    // A message written after the edit and the deletion is the next turn.
    c.slack.direct(channel, "UANNA", "Thanks.");
    wait("the message after the edit and the deletion", || {
        c.runtime.message_count(&session) == 5
    });
    let empty: Vec<usize> = (0..5).map(|n| c.delivered(&session, n).empty).collect();
    assert_eq!(empty, [0, 0, 0, 1, 0]);
    insta::assert_snapshot!(
        "direct-messages-delivered",
        (0..5)
            .map(|n| c.input(&session, n))
            .collect::<Vec<_>>()
            .join("\n---\n")
    );
    let answer = |reply: &str| Post {
        channel: channel.to_owned(),
        thread: None,
        text: None,
        markdown: Some(reply.to_owned()),
    };
    assert_eq!(
        c.slack.posts(),
        replies
            .iter()
            .map(|reply| answer(reply))
            .collect::<Vec<_>>()
    );
}

// spec: project/instrukt/spec/feature/company/slack-connector UC-SL5
#[test]
fn the_agents_answer_is_posted_as_markdown_it_wrote_and_a_long_one_in_parts() {
    let mut c = Company::new("");
    c.run();
    let channel = "D1";
    c.slack
        .direct(channel, "UANNA", "How do I renew the certificate?");
    let session = c.session();
    // Its leading indentation and final line break are part of what it
    // wrote.
    let formatted = "    certbot renew --dry-run\n\n**Renew it** in three steps:\n\n1. Open [the portal](https://portal.example.com).\n2. Run:\n\n```sh\ncertbot renew\n```\n3. Check the new date.\n";
    c.finish_turn(&session, 0, formatted);
    wait("the formatted answer", || c.slack.posts().len() == 1);
    c.slack.direct(channel, "UANNA", "And the whole log?");
    wait("the second question", || {
        c.runtime.message_count(&session) == 2
    });
    // Longer than Slack takes at once: two line breaks lie in the second half
    // of the first 12,000 characters, and the first part ends after the
    // later one; the remainder follows unchanged, its final line break
    // included.
    let opening = format!("{}\n{}\n", "a".repeat(6_500), "b".repeat(3_000));
    let remainder = format!("{}\n", "c".repeat(7_000));
    c.finish_turn(&session, 1, &format!("{opening}{remainder}"));
    wait("the long answer's parts", || c.slack.posts().len() == 3);
    let answer = |markdown: &str| Post {
        channel: channel.to_owned(),
        thread: None,
        text: None,
        markdown: Some(markdown.to_owned()),
    };
    assert_eq!(
        c.slack.posts(),
        [answer(formatted), answer(&opening), answer(&remainder)]
    );
}

// spec: project/instrukt/spec/feature/company/slack-connector UC-SL6
#[test]
fn a_member_is_answered_when_slack_cannot_read_the_earlier_messages() {
    std::thread::scope(|scope| {
        scope.spawn(readable_context_survives_a_held_speaker_lookup);
        scope.spawn(a_retry_keeps_context_the_agent_has_never_seen);
        scope.spawn(a_retry_excludes_a_queued_question);
        scope.spawn(a_refused_send_reselects_context);
        scope.spawn(uncertain_send_keeps_its_input_and_queue_position);
        scope.spawn(early_stop_preserves_older_replies_already_read);
        scope.spawn(|| {
            a_stalled_history_page_keeps_the_first_page(false);
            a_stalled_history_page_keeps_the_first_page(true);
        });
        scope.spawn(receipts_survive_out_of_order_delivery_and_pruning);
        unread_context_expires_during_an_unrelated_sender_lookup();
    });
}

const CONTEXT_LIMIT: jiff::SignedDuration = jiff::SignedDuration::from_secs(120);

fn set_context_clock(clock: &std::path::Path, at: jiff::Timestamp) {
    let next = clock.with_extension("next");
    std::fs::write(&next, at.to_string()).unwrap();
    std::fs::rename(next, clock).unwrap();
}

fn context_company(settings: &str) -> (Company, std::path::PathBuf, jiff::Timestamp) {
    let mut c = Company::new(settings);
    let clock = c.env.path("context-clock");
    let at = "2026-11-19T10:00:00Z".parse().unwrap();
    set_context_clock(&clock, at);
    c.env
        .set_env("INSTRUKT_NOW", &format!("@{}", clock.display()));
    c.env.set_env("INSTRUKT_LOG_LEVEL", "debug");
    let pass = c.env.path("context-pass");
    c.env
        .set_env("INSTRUKT_TEST_CONTEXT_PASS", pass.to_str().unwrap());
    (c, clock, at)
}

fn unread_context_expires_during_an_unrelated_sender_lookup() {
    let (mut c, clock, at) = context_company("");
    c.slack.refuse_replies(true);
    c.run();

    let thread = c
        .slack
        .say("C1", "UANNA", "The private earlier context.", None);
    let question = c.slack.mention(
        "C1",
        "USTEVEN",
        "<@UAGENT> Where are the notes?",
        Some(&thread),
    );
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let recorded_failure = || {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversation_messages
             WHERE message=?1 AND state='received' AND sender_name IS NOT NULL
             AND failure LIKE '%thread_not_found%')",
            [&question],
            |row| row.get::<_, bool>(0),
        )
        .unwrap()
    };
    wait(
        "the Slack failure is retained or the question is prematurely delivered",
        || recorded_failure() || !c.sessions().is_empty(),
    );
    assert!(
        recorded_failure(),
        "the question must wait for context before the bound"
    );
    assert!(c.sessions().is_empty());
    assert!(c.slack.posts().is_empty());

    c.slack.hold_reads();
    c.slack.refuse_replies(false);
    let deadline = at + CONTEXT_LIMIT;
    let before = deadline - jiff::SignedDuration::from_secs(1);
    set_context_clock(&clock, before);
    wait(
        "a context retry is still in flight just before its deadline",
        || c.slack.held() > 0 || !c.sessions().is_empty(),
    );
    #[cfg(debug_assertions)]
    wait(
        "intake evaluated the full 119 seconds before expiry",
        || {
            std::fs::read_to_string(c.env.path("context-pass"))
                .is_ok_and(|seen| seen == before.to_string())
        },
    );
    let state: String = db
        .query_row(
            "SELECT state FROM conversation_messages WHERE message=?1",
            [&question],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "received", "context must wait the full two minutes");
    assert!(c.sessions().is_empty());

    c.slack.hold_lookups_for("UANNA");
    c.slack.direct("D99", "UANNA", "What changed?");
    wait("the unrelated sender lookup is held", || {
        c.slack.held_lookups() > 0
    });
    set_context_clock(&clock, deadline);
    crate::common::wait_until(
        "the saved question reaches the agent at expiry while both reads are held",
        std::time::Duration::from_secs(12),
        || !c.sessions().is_empty(),
    );
    let session = c.session();
    let delivered = c.delivered(&session, 0);
    assert_eq!(delivered.members, 1);
    assert_eq!(delivered.accompanying, 0);
    assert!(!delivered.left_out);
    insta::assert_snapshot!("unread-thread-context", c.input(&session, 0));
    let reply = "The notes are in the wiki.";
    c.finish_turn(&session, 0, reply);
    wait(
        "the member gets the answer while both lookups are held",
        || !c.slack.posts().is_empty(),
    );
    let expected = [Post {
        channel: "C1".into(),
        thread: Some(thread),
        text: None,
        markdown: Some(reply.into()),
    }];
    assert_eq!(c.slack.posts(), expected);

    c.slack.release_lookups();
    c.slack.release_reads();
    let log = c
        .env
        .home()
        .join(".local/state/instrukt-ai/instrukt/run.log");
    wait(
        "the late readable context has returned and been discarded",
        || {
            std::fs::read_to_string(&log).is_ok_and(|log| {
                log.lines().any(|line| {
                    line.contains("event=conversation_context_late_result")
                        && line.contains(&question)
                })
            })
        },
    );
    assert_eq!(c.runtime.message_count(&session), 1);
    assert_eq!(c.slack.posts(), expected);
}

fn readable_context_survives_a_held_speaker_lookup() {
    let (mut c, _, _) = context_company("");
    c.slack.hold_lookups_for("UANNA");
    c.run();
    let earlier = "The earlier message Slack already returned.";
    let thread = c.slack.say("C1", "UANNA", earlier, None);
    c.slack.mention(
        "C1",
        "USTEVEN",
        "<@UAGENT> What did Anna say?",
        Some(&thread),
    );
    wait(
        "the context was read and the earlier speaker's name lookup is held",
        || c.slack.held_lookups() > 0,
    );
    crate::common::wait_until(
        "the readable context reaches the agent while the speaker lookup is held",
        std::time::Duration::from_secs(12),
        || !c.sessions().is_empty(),
    );
    let session = c.session();
    let input = c.input(&session, 0);
    assert!(input.contains(earlier), "{input}");
    assert!(input.contains("--- UANNA ·"), "{input}");
    assert!(
        !input.contains("Earlier messages could not be read."),
        "{input}"
    );
    assert_eq!(c.delivered(&session, 0).accompanying, 1);
    c.slack.release_lookups();
}

fn context_database(c: &Company) -> instrukt_core::state_db::rusqlite::Connection {
    instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap()
}

fn wait_for_message_state(
    db: &instrukt_core::state_db::rusqlite::Connection,
    id: &str,
    expected: &str,
) {
    wait(&format!("the message {id} reaches {expected}"), || {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE message=?1 AND state=?2)",
            [id, expected],
            |row| row.get::<_, bool>(0),
        )
        .unwrap()
    });
}

fn wait_for_failed_context(db: &instrukt_core::state_db::rusqlite::Connection, id: &str) {
    wait("the context attempt fails before Slack recovers", || {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE message=?1
             AND state='received' AND failure LIKE '%thread_not_found%')",
            [id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap()
    });
}

fn a_retry_keeps_context_the_agent_has_never_seen() {
    let (mut c, _, _) = context_company(", context_messages: 2");
    // Keep the provider transcript fixed while the refusal is retried.
    c.slack.fail_posts();
    c.run();
    let channel = "C1";
    let root = c.slack.say(channel, "UBRAM", "Planning thread", None);
    let unseen = "The budget was approved earlier.";
    c.slack.say(channel, "UBRAM", unseen, Some(&root));
    let refused = "A suggestion from a guest.";
    let denied = c.slack.mention(channel, "USTRANGER", refused, Some(&root));
    let db = context_database(&c);
    wait_for_message_state(&db, &denied, "refused");
    for text in ["Recent note one.", "Recent note two."] {
        c.slack.say(channel, "UBRAM", text, Some(&root));
    }
    let late = c
        .slack
        .say(channel, "USTEVEN", "What can we review?", Some(&root));
    let retry = c
        .slack
        .say(channel, "UANNA", "What was the budget?", Some(&root));
    c.slack.refuse_replies_for(Some(&retry));
    c.slack.notify_mention(
        &retry,
        channel,
        "UANNA",
        "What was the budget?",
        Some(&root),
    );
    wait_for_failed_context(&db, &retry);

    c.slack.notify_mention(
        &late,
        channel,
        "USTEVEN",
        "What can we review?",
        Some(&root),
    );
    let session = c.session();
    let input = c.input(&session, 0);
    assert!(
        input.contains("Recent note one.") && input.contains("Recent note two."),
        "{input}"
    );
    assert!(!input.contains(unseen), "{input}");
    assert!(c.delivered(&session, 0).left_out);
    wait_for_message_state(&db, &late, "sent");
    c.slack.refuse_replies_for(None);
    wait_for_message_state(&db, &retry, "pending");
    c.finish_turn(&session, 0, "NOOP");
    wait("the retried budget question reaches the agent", || {
        c.runtime.message_count(&session) == 2
    });
    let input = c.input(&session, 1);
    assert!(input.contains(unseen) && input.contains(refused), "{input}");
    assert!(
        !input.contains("Recent note one.") && !input.contains("Recent note two."),
        "{input}"
    );
    assert!(c.delivered(&session, 1).left_out);
}

fn a_retry_excludes_a_queued_question() {
    let (mut c, _, _) = context_company("");
    c.run();
    let channel = "C1";
    let root = c
        .slack
        .mention(channel, "USTEVEN", "Start the review.", None);
    let session = c.session();
    let db = context_database(&c);
    let earlier = c.slack.say(
        channel,
        "USTEVEN",
        "Check the queued checklist.",
        Some(&root),
    );
    let detail = "The checklist is in the wiki.";
    c.slack.say(channel, "UBRAM", detail, Some(&root));
    let later = c
        .slack
        .say(channel, "UANNA", "Is the review complete?", Some(&root));
    // The later question arrives first and its read fails. The earlier
    // question takes its own place in the queue while that read waits.
    c.slack.refuse_replies_for(Some(&later));
    c.slack.notify_mention(
        &later,
        channel,
        "UANNA",
        "Is the review complete?",
        Some(&root),
    );
    wait_for_failed_context(&db, &later);
    c.slack.notify_mention(
        &earlier,
        channel,
        "USTEVEN",
        "Check the queued checklist.",
        Some(&root),
    );
    wait_for_message_state(&db, &earlier, "pending");
    c.slack.refuse_replies_for(None);
    wait_for_message_state(&db, &later, "pending");
    assert_eq!(c.runtime.message_count(&session), 1);
    c.finish_turn(&session, 0, "NOOP");
    wait("the earlier question takes its own turn", || {
        c.runtime.message_count(&session) == 2
    });
    assert!(c.input(&session, 1).contains("Check the queued checklist."));
    c.finish_turn(&session, 1, "NOOP");
    wait("the later question takes its own turn", || {
        c.runtime.message_count(&session) == 3
    });
    let input = c.input(&session, 2);
    assert!(!input.contains("Check the queued checklist."), "{input}");
    assert!(input.contains(detail), "{input}");
    assert_eq!(c.delivered(&session, 2).accompanying, 1);
    assert!(!c.delivered(&session, 2).left_out);
}

fn a_refused_send_reselects_context() {
    let (mut c, _, _) = context_company("");
    c.run();
    let channel = "C1";
    let root = c
        .slack
        .mention(channel, "USTEVEN", "Open the review.", None);
    let session = c.session();
    c.finish_turn(&session, 0, "NOOP");
    let earlier_text = "Review the earlier draft.";
    let earlier = c.slack.say(channel, "USTEVEN", earlier_text, Some(&root));
    let detail = "The review checklist is in the wiki.";
    c.slack.say(channel, "UBRAM", detail, Some(&root));
    let hold = format!("{session}.hold-send");
    c.runtime.flag(&hold, true);
    let later = c
        .slack
        .mention(channel, "UANNA", "Can we ship?", Some(&root));
    wait(
        "the later question's first send waits before acceptance",
        || c.runtime.held_send_running(&session),
    );
    let db = context_database(&c);
    let first_input: String = db.query_row(
        "SELECT content FROM conversation_messages WHERE message=?1 AND attempted_at IS NOT NULL",
        [&later], |row| row.get(0),
    ).unwrap();
    assert!(first_input.contains(earlier_text), "{first_input}");
    c.slack
        .notify_mention(&earlier, channel, "USTEVEN", earlier_text, Some(&root));
    wait_for_message_state(&db, &earlier, "pending");
    c.runtime.flag(&format!("{session}.fail-next-send"), true);
    c.runtime.flag(&hold, false);
    wait("the earlier question takes its own turn", || {
        c.runtime.message_count(&session) == 2
    });
    assert!(c.input(&session, 1).contains(earlier_text));
    c.finish_turn(&session, 1, "NOOP");
    wait("the refused later question is retried", || {
        c.runtime.message_count(&session) == 3
    });
    let retried = c.input(&session, 2);
    assert!(retried.contains(detail), "{retried}");
    assert!(!retried.contains(earlier_text), "{retried}");
}

fn uncertain_send_keeps_its_input_and_queue_position() {
    let (mut c, _, _) = context_company("");
    c.run();
    let channel = "C1";
    let root = c
        .slack
        .mention(channel, "USTEVEN", "Open the review.", None);
    let session = c.session();
    c.finish_turn(&session, 0, "NOOP");
    let earlier_text = "Review the first draft.";
    let earlier = c.slack.say(channel, "USTEVEN", earlier_text, Some(&root));
    let hold = format!("{session}.hold-send");
    c.runtime.flag(&hold, true);
    c.runtime.flag(&format!("{session}.defer-next-send"), true);
    let later = c
        .slack
        .mention(channel, "UANNA", "Can we publish?", Some(&root));
    wait("the later handoff starts", || {
        c.runtime.held_send_running(&session)
    });
    let db = context_database(&c);
    let attempt = || -> (String, Option<String>, String) {
        db.query_row(
            "SELECT state,attempted_at,content FROM conversation_messages WHERE message=?1",
            [&later],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
    };
    let original = attempt();
    assert!(original.1.is_some());
    assert!(original.2.contains(earlier_text));
    c.slack
        .notify_mention(&earlier, channel, "USTEVEN", earlier_text, Some(&root));
    wait_for_message_state(&db, &earlier, "pending");
    c.runtime.flag(&hold, false);
    let ready = c
        .runtime
        .state_dir()
        .join(format!("{session}.deferred-ready-read"));
    wait(
        "the first lookup missed and the readiness read is held",
        || ready.exists(),
    );
    assert_eq!(attempt(), original);
    c.runtime
        .flag(&format!("{session}.release-deferred-input"), true);
    wait(
        "the handoff is reconciled or its attempt was cleared",
        || {
            let tracked = attempt();
            tracked.0 == "sent" || tracked.1.is_none() || c.runtime.message_count(&session) > 2
        },
    );
    assert_eq!(attempt(), ("sent".to_owned(), original.1, original.2));
    assert_eq!(c.runtime.message_count(&session), 2);
    assert_eq!(c.input(&session, 1).matches("Can we publish?").count(), 1);
    c.finish_turn(&session, 1, "NOOP");
    wait("the queued question takes its own turn", || {
        c.runtime.message_count(&session) == 3
    });
    assert!(c.input(&session, 2).contains(earlier_text));
    assert!(!c.input(&session, 2).contains("Can we publish?"));
}

fn early_stop_preserves_older_replies_already_read() {
    let (mut c, clock, at) = context_company(", context_messages: 1");
    c.run();
    let channel = "C1";
    let root = c
        .slack
        .mention(channel, "USTEVEN", "Open the project review.", None);
    let session = c.session();
    let older = "An older reply that was already read.";
    c.slack
        .say_at("1700000002.000100", channel, "UBRAM", older, Some(&root));
    let nearer = "Review the newer draft.";
    let newest = "Check the final draft.";
    let second = "1700000018.000100";
    let third = "1700000019.000100";
    let question = "1700000020.000100";
    for (ts, text) in [
        (second, nearer),
        (third, newest),
        (question, "What did I miss?"),
    ] {
        c.slack.say_at(ts, channel, "USTEVEN", text, Some(&root));
    }
    c.slack.notify_mention(
        question,
        channel,
        "USTEVEN",
        "What did I miss?",
        Some(&root),
    );
    let db = context_database(&c);
    wait_for_message_state(&db, question, "pending");
    // The newer replies take their own turns without any context from Slack.
    c.slack.refuse_replies(true);
    c.slack
        .notify_mention(second, channel, "USTEVEN", nearer, Some(&root));
    c.slack
        .notify_mention(third, channel, "USTEVEN", newest, Some(&root));
    wait_for_failed_context(&db, second);
    wait_for_failed_context(&db, third);
    set_context_clock(&clock, at + CONTEXT_LIMIT);
    wait_for_message_state(&db, second, "pending");
    wait_for_message_state(&db, third, "pending");
    c.finish_turn(&session, 0, "NOOP");
    for (i, text) in [(1, nearer), (2, newest)] {
        wait("a newer reply takes its own turn", || {
            c.runtime.message_count(&session) == i + 1
        });
        assert!(c.input(&session, i).contains(text));
        assert!(!c.input(&session, i).contains(older));
        c.finish_turn(&session, i, "NOOP");
    }
    wait("the question is delivered after expiry", || {
        c.runtime.message_count(&session) == 4
    });
    let input = c.input(&session, 3);
    assert!(input.contains(older), "{input}");
    assert!(!input.contains(nearer), "{input}");
    assert!(!input.contains(newest), "{input}");
}

fn a_stalled_history_page_keeps_the_first_page(first_page_is_enough: bool) {
    let (mut c, clock, at) = context_company(", context_messages: 1");
    let channel = "C1";
    c.slack
        .say(channel, "USTEVEN", "An older note on the held page.", None);
    if first_page_is_enough {
        c.slack
            .say(channel, "USTEVEN", "The next most recent note.", None);
    } else {
        c.slack
            .notice(channel, "channel_join", "UANNA", "A Slack notice.", None);
    }
    let latest = "The newest note belongs in the context.";
    c.slack.say(channel, "UBRAM", latest, None);
    c.slack.hold_later_history_pages();
    c.run();
    c.slack
        .mention(channel, "USTEVEN", "What did I miss?", None);
    if first_page_is_enough {
        crate::common::wait_until(
            "page one supplies context and proves omission without page two",
            std::time::Duration::from_secs(12),
            || !c.sessions().is_empty() || c.slack.held() > 0,
        );
        assert_eq!(
            c.slack.held(),
            0,
            "selection was known before the held page"
        );
    } else {
        wait("the unread second page is held", || c.slack.held() > 0);
        assert!(c.sessions().is_empty());
        set_context_clock(&clock, at + CONTEXT_LIMIT);
    }
    let session = c.session();
    let input = c.input(&session, 0);
    assert!(input.contains(latest), "{input}");
    assert_eq!(c.delivered(&session, 0).accompanying, 1);
    assert_eq!(c.delivered(&session, 0).left_out, first_page_is_enough);
    assert_eq!(
        input.contains("Earlier messages could not be read."),
        !first_page_is_enough,
        "{input}"
    );
    c.slack.release_reads();
}

fn receipts_survive_out_of_order_delivery_and_pruning() {
    let (mut c, clock, at) = context_company("");
    c.runtime.flag("separate-completion", true);
    c.store.thread("s1", None, None, "2026-11-08T10:00:00Z");
    c.run();
    let channel = "C1";
    let thread = c.slack.say_at(
        "1700000000.000100",
        channel,
        "USTEVEN",
        "Release thread",
        None,
    );
    let seen = "A release note delivered only as context.";
    c.slack
        .say_at("1700000002.000100", channel, "UBRAM", seen, Some(&thread));
    let failed = c.slack.mention_at(
        "1700000005.000100",
        channel,
        "USTEVEN",
        "Was the release blocked?",
        Some(&thread),
    );
    let session = c.session();
    let first = c.runtime.message(&session, 0);
    assert!(first.contains(seen));
    c.runtime.set_status(&session, "idle");
    c.runtime.project(&c.env, &session);
    c.store.failed_turn(&session, "failed-turn", 1, &first);
    let db = context_database(&c);
    wait_for_message_state(&db, &failed, "failed");

    c.slack.mention_at(
        "1700000004.000100",
        channel,
        "USTEVEN",
        "Has the changelog been written?",
        Some(&thread),
    );
    wait(
        "the older question is delivered after the newer question",
        || c.runtime.message_count(&session) == 2,
    );
    let older = c.input(&session, 1);
    assert!(!older.contains(seen), "{older}");
    c.finish_turn(&session, 1, "NOOP");
    wait("the successful turn supersedes the failure", || {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE message=?1 AND superseded_at IS NOT NULL)",
            [&failed], |row| row.get::<_, bool>(0),
        ).unwrap()
    });

    c.runner_dies();
    set_context_clock(
        &clock,
        at + jiff::SignedDuration::from_hours(7 * 24) + jiff::SignedDuration::from_secs(1),
    );
    c.run();
    wait("the superseded failure is pruned at startup", || {
        db.query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM conversation_messages WHERE message=?1)",
            [&failed],
            |row| row.get::<_, bool>(0),
        )
        .unwrap()
    });
    let fresh = "The release checklist is ready.";
    c.slack
        .say_at("1700000006.000100", channel, "UANNA", fresh, Some(&thread));
    c.slack.mention_at(
        "1700000007.000100",
        channel,
        "USTEVEN",
        "When do we ship?",
        Some(&thread),
    );
    wait("the next question reaches the agent after pruning", || {
        c.runtime.message_count(&session) == 3
    });
    let input = c.input(&session, 2);
    assert!(input.contains(fresh), "{input}");
    assert!(
        !input.contains(seen) && !input.contains("Was the release blocked?"),
        "{input}"
    );
    assert_eq!(c.delivered(&session, 2).accompanying, 1);

    // The grace window starts at pruning, not at the earlier delivery.
    let pruned_at: String = db
        .query_row(
            "SELECT retired_at FROM conversation_context_receipts WHERE message=?1",
            [&failed],
            |row| row.get(0),
        )
        .unwrap();
    let pruned_at: jiff::Timestamp = pruned_at.parse().unwrap();
    set_context_clock(
        &clock,
        pruned_at + jiff::SignedDuration::from_hours(7 * 24) + jiff::SignedDuration::from_secs(1),
    );
    wait("the receipt is pruned after its seven-day grace", || {
        db.query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM conversation_context_receipts WHERE message=?1)",
            [&failed],
            |row| row.get::<_, bool>(0),
        )
        .unwrap()
    });
}
