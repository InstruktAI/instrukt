//! The Slack connector through the real runner and the Slack stand-in at the
//! network line: an outward Socket Mode connection, what reaches the agent and
//! what accompanies it, and where and how answers are posted.
#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]

use crate::fixtures::conversing::{Company, company_home, slack_settings, wait};
use crate::fixtures::slack::{APP_TOKEN, BOT_TOKEN, Post};
use instrukt_core::ids::ProjectSlug;
use instrukt_runner::conversations::{ConnectionState, Status};
use instrukt_runner::slack::{API_URL_SETTING, WORKING};

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
    let refused = c.env.run(c.env.home(), ["run"]).failure();
    insta::assert_snapshot!(
        "remote-slack-address-refused",
        String::from_utf8_lossy(&refused.get_output().stderr)
    );
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
