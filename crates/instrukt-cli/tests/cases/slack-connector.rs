//! The Slack connector through the real runner and the Slack stand-in at the
//! network line: an outward Socket Mode connection, what reaches the agent and
//! what accompanies it, and where answers are posted.
#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]

use crate::fixtures::conversing::{Company, company_home, slack_settings, wait};
use instrukt_core::ids::ProjectSlug;
use instrukt_runner::connectors::{API_URL_SETTING, WORKING};
use instrukt_runner::conversations::{ConnectionState, Status};

/// Accompanying messages by the people who wrote them, by the names Slack
/// shows, none the agent's own.
fn said_by(c: &Company, speakers: &[&str]) -> Vec<(String, bool)> {
    speakers
        .iter()
        .map(|speaker| (c.member(speaker), false))
        .collect()
}

// spec: project/instrukt/spec/feature/runner/slack-connector UC-SL1
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
    let mut members: Vec<String> = c
        .sessions()
        .iter()
        .map(|session| {
            let delivered = c.delivered(session, 0);
            assert_eq!(delivered.empty, 0);
            delivered.member
        })
        .collect();
    members.sort();
    let mut expected = vec![c.member("USTEVEN"), c.member("UANNA")];
    expected.sort();
    assert_eq!(members, expected);
    // Stopped by a signal, the runner records nothing: no running runner
    // hosts the connector, so it shows as stopped.
    c.kill_runner();
    let status: Status = serde_json::from_value(c.status()["conversations"].clone()).unwrap();
    assert_eq!(
        status
            .connectors
            .iter()
            .find(|connector| ProjectSlug::parse(&connector.company).as_ref() == Ok(&c.company()))
            .map(|connector| connector.state),
        Some(ConnectionState::Stopped)
    );
}

// spec: project/instrukt/spec/feature/runner/slack-connector UC-SL2
#[test]
fn in_a_channel_only_a_tag_reaches_the_agent_with_the_most_recent_messages_since_the_previous_tag()
{
    let mut c = Company::new(", context_messages: 2");
    c.run();
    let root = c.slack.say("C1", "UANNA", "Shall we ship on Friday?", None);
    // Slack's own notice of someone joining accompanies no message.
    c.slack.notice("C1", "channel_join", "UBRAM", Some(&root));
    c.slack.mention(
        "C1",
        "USTEVEN",
        "<@UAGENT> What do you think, <@UANNA|anna>?",
        Some(&root),
    );
    let session = c.session();
    let first = c.delivered(&session, 0);
    assert_eq!(first.context, said_by(&c, &["UANNA"]));
    assert!(!first.left_out);
    assert_eq!(first.empty, 0);
    // Mentions reach the agent as names.
    assert_eq!(first.markup, 0);
    c.finish_turn(&session, 0, "Friday works.");
    wait("the answer's post", || !c.slack.posts().is_empty());
    c.slack.say("C1", "UANNA", "Agreed.", Some(&root));
    c.slack.say(
        "C1",
        "USTEVEN",
        "Then QA runs on Thursday, <@UBRAM>.",
        Some(&root),
    );
    c.slack.notice("C1", "channel_join", "UBRAM", Some(&root));
    c.slack
        .mention("C1", "UANNA", "<@UAGENT> Anything to add?", Some(&root));
    wait("the tagged message", || {
        c.runtime.message_count(&session) == 2
    });
    // The untagged messages reached the agent only as what accompanies the
    // tag. Since the previous tag there were three — the agent's own answer,
    // then both — so the most recent two accompany it, read across more than
    // one page, and the context says earlier ones were left out. The notice
    // after them counts for nothing.
    let delivered = c.delivered(&session, 1);
    assert_eq!(delivered.member, c.member("UANNA"));
    assert_eq!(delivered.members, 1);
    assert_eq!(delivered.context, said_by(&c, &["UANNA", "USTEVEN"]));
    assert!(delivered.left_out);
    assert_eq!(delivered.empty, 0);
    assert_eq!(delivered.markup, 0);
}

// spec: project/instrukt/spec/feature/runner/slack-connector UC-SL3
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
    assert_eq!(delivered.member, c.member("USTEVEN"));
    assert_eq!(delivered.context, said_by(&c, &["UANNA", "UBRAM"]));
    assert!(delivered.left_out);
    assert_eq!(delivered.empty, 0);
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
    assert_eq!(delivered.member, c.member("UANNA"));
    assert_eq!(delivered.context, said_by(&c, &["UBRAM", "USTEVEN"]));
    assert!(delivered.left_out);
    assert_eq!(delivered.empty, 0);
}

// spec: project/instrukt/spec/feature/runner/slack-connector UC-SL4
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
    c.finish_turn(&session, 0, "The invoice is paid.");
    c.slack.direct(channel, "UANNA", "And the next one?");
    wait("the second direct message", || {
        c.runtime.message_count(&session) == 2
    });
    for n in 0..2 {
        let delivered = c.delivered(&session, n);
        assert_eq!(delivered.member, c.member("UANNA"));
        assert_eq!(delivered.empty, 0);
    }
    wait("the answer's post", || !c.slack.posts().is_empty());
    wait("the finished message's status cleared", || {
        c.slack.status(channel, &first).is_none()
    });
    let posts = c.slack.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(
        (posts[0].channel.as_str(), posts[0].thread.as_deref()),
        (channel, None)
    );
    assert!(!posts[0].text.is_empty());
}
