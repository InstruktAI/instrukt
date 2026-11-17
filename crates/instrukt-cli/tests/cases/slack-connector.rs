//! The Slack connector through the real runner and the Slack stand-in at the
//! network line: an outward Socket Mode connection, what reaches the agent and
//! what accompanies it, and where answers are posted.
#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]

use crate::fixtures::conversing::{Company, company_home, slack_settings, wait};
use crate::fixtures::slack::Post;
use instrukt_runner::conversations::{CONNECTED, FAILED};

fn texts(earlier: &[instrukt_runner::conversations::Earlier]) -> Vec<(String, bool)> {
    earlier.iter().map(|e| (e.text.clone(), e.own)).collect()
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
    c.run();
    let state = |company: &str| {
        c.status()["conversations"]["connectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|connector| connector["company"] == company)
            .map(|connector| connector["state"].as_str().unwrap().to_owned())
    };
    wait("the refused connector's failure", || {
        state("bravo").as_deref() == Some(FAILED)
    });
    assert_eq!(state("acme").as_deref(), Some(CONNECTED));
    let question = "<@UAGENT> Are you there?";
    let message = c.slack.mention("C1", "USTEVEN", question, None);
    let session = c.session();
    assert_eq!(
        c.addressed(&session, 0),
        (c.member("USTEVEN"), question.to_owned())
    );
    assert_eq!(c.slack.acknowledged(), [format!("envelope-{message}")]);
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
    let answer = "Friday works.";
    c.finish_turn(&session, 0, answer);
    wait("the answer's post", || !c.slack.posts().is_empty());
    let agreed = "Agreed.";
    let plan = "Then QA runs on Thursday.";
    c.slack.say("C1", "UANNA", agreed, Some(&root));
    c.slack.say("C1", "USTEVEN", plan, Some(&root));
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(c.runtime.message_count(&session), 1);
    let tag = c
        .slack
        .mention("C1", "UANNA", "<@UAGENT> Anything to add?", Some(&root));
    wait("the tagged message", || {
        c.runtime.message_count(&session) == 2
    });
    assert_eq!(
        texts(&c.accompanying(&format!("C1/{root}"), &tag)),
        [
            (answer.to_owned(), true),
            (agreed.to_owned(), false),
            (plan.to_owned(), false)
        ]
    );
}

// spec: project/instrukt/spec/feature/runner/slack-connector UC-SL3
#[test]
fn a_first_tag_brings_the_most_recent_messages_before_it() {
    let mut c = Company::new(", context_messages: 2");
    c.run();
    let said = ["The deploy failed.", "It was the cache.", "Retrying now."];
    for text in said {
        c.slack.say("C1", "UANNA", text, None);
    }
    let tag = c
        .slack
        .mention("C1", "USTEVEN", "<@UAGENT> Can you summarise?", None);
    c.session();
    assert_eq!(
        texts(&c.accompanying(&format!("C1/{tag}"), &tag)),
        [(said[1].to_owned(), false), (said[2].to_owned(), false)]
    );
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
    let reply = "The invoice is paid.";
    c.finish_turn(&session, 0, reply);
    let next = "And the next one?";
    c.slack.direct(channel, "UANNA", next);
    wait("the second direct message", || {
        c.runtime.message_count(&session) == 2
    });
    assert_eq!(
        c.addressed(&session, 1),
        (c.member("UANNA"), next.to_owned())
    );
    wait("the answer's post", || !c.slack.posts().is_empty());
    assert_eq!(
        c.slack.posts(),
        [Post {
            channel: channel.into(),
            thread: None,
            text: reply.into(),
        }]
    );
}
