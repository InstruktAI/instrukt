//! The Slack connector through the real runner and the Slack stand-in at the
//! network line: an outward Socket Mode connection, what reaches the agent and
//! what accompanies it, and where answers are posted.
#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]

use crate::fixtures::conversing::{Company, company_home, slack_settings, wait};
use crate::fixtures::slack::BOT_USER;
use instrukt_runner::connectors::API_URL_SETTING;
use instrukt_runner::conversations::{ConnectionState, Status};

fn context(speakers: &[&str], own: &[bool]) -> Vec<(String, bool)> {
    speakers
        .iter()
        .zip(own)
        .map(|(speaker, own)| ((*speaker).to_owned(), *own))
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
    let state = |company: &str| {
        let status: Status = serde_json::from_value(c.status()["conversations"].clone()).unwrap();
        status
            .connectors
            .into_iter()
            .find(|connector| connector.company == company)
            .map(|connector| connector.state)
    };
    wait("the refused connector's failure", || {
        state("bravo") == Some(ConnectionState::Failed)
    });
    assert_eq!(state("acme"), Some(ConnectionState::Connected));
    // Slack's acknowledgement does not wait for reading what accompanies the
    // message.
    c.slack.hold_reads();
    let message = c
        .slack
        .mention("C1", "USTEVEN", "<@UAGENT> Are you there?", None);
    wait("the acknowledgement", || {
        c.slack.acknowledged() == [format!("envelope-{message}")]
    });
    c.slack.release_reads();
    let session = c.session();
    assert_eq!(c.delivered(&session, 0).member, c.member("USTEVEN"));
}

// spec: project/instrukt/spec/feature/runner/slack-connector UC-SL2
#[test]
fn in_a_channel_only_a_tag_reaches_the_agent_with_everything_since_the_previous_tag() {
    let mut c = Company::new("");
    c.run();
    let root = c.slack.say("C1", "UANNA", "Shall we ship on Friday?", None);
    c.slack
        .mention("C1", "USTEVEN", "<@UAGENT> What do you think?", Some(&root));
    let session = c.session();
    assert_eq!(
        c.delivered(&session, 0).context,
        context(&["UANNA"], &[false])
    );
    c.finish_turn(&session, 0, "Friday works.");
    wait("the answer's post", || !c.slack.posts().is_empty());
    c.slack.say("C1", "UANNA", "Agreed.", Some(&root));
    c.slack
        .say("C1", "USTEVEN", "Then QA runs on Thursday.", Some(&root));
    c.slack
        .mention("C1", "UANNA", "<@UAGENT> Anything to add?", Some(&root));
    wait("the tagged message", || {
        c.runtime.message_count(&session) == 2
    });
    // The untagged messages reached the agent only as what accompanies the
    // tag: the agent's own answer, then both, across more than one page.
    let delivered = c.delivered(&session, 1);
    assert_eq!(delivered.member, c.member("UANNA"));
    assert_eq!(delivered.members, 1);
    assert_eq!(
        delivered.context,
        context(&[BOT_USER, "UANNA", "USTEVEN"], &[true, false, false])
    );
}

// spec: project/instrukt/spec/feature/runner/slack-connector UC-SL3
#[test]
fn a_first_tag_brings_the_most_recent_messages_before_it() {
    let mut c = Company::new(", context_messages: 2");
    c.run();
    // A tag that starts its thread: the most recent of the channel.
    for user in ["U1", "U2", "U3"] {
        c.slack.say("C1", user, "Earlier talk.", None);
    }
    c.slack
        .mention("C1", "USTEVEN", "<@UAGENT> Can you summarise?", None);
    let first = c.session();
    let delivered = c.delivered(&first, 0);
    assert_eq!(delivered.member, c.member("USTEVEN"));
    assert_eq!(delivered.context, context(&["U2", "U3"], &[false, false]));
    // A first tag inside a long thread: the most recent of that thread.
    let root = c.slack.say("C2", "U1", "A long thread.", None);
    for user in ["U2", "U3", "U4"] {
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
    assert_eq!(delivered.context, context(&["U3", "U4"], &[false, false]));
}

// spec: project/instrukt/spec/feature/runner/slack-connector UC-SL4
#[test]
fn every_direct_message_reaches_the_agent_without_a_tag() {
    let mut c = Company::new("");
    c.run();
    let channel = "D1";
    c.slack
        .direct(channel, "UANNA", "Can you check the invoice?");
    let session = c.session();
    c.finish_turn(&session, 0, "The invoice is paid.");
    c.slack.direct(channel, "UANNA", "And the next one?");
    wait("the second direct message", || {
        c.runtime.message_count(&session) == 2
    });
    for n in 0..2 {
        assert_eq!(c.delivered(&session, n).member, c.member("UANNA"));
    }
    wait("the answer's post", || !c.slack.posts().is_empty());
    let posts = c.slack.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(
        (posts[0].channel.as_str(), posts[0].thread.as_deref()),
        (channel, None)
    );
}
