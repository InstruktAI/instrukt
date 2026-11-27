use std::collections::BTreeMap;
use std::path::PathBuf;

use instrukt_core::ids::ProjectSlug;
use instrukt_runner::mcp::{McpError, Speaker};

use super::{Admission, Holding, Recorded, SpeakerDecision, admission, decide_speaker, retired};

fn acme() -> ProjectSlug {
    ProjectSlug::parse("acme").unwrap()
}

fn speaker(company: &str, is_admin: bool, is_direct: bool) -> Speaker {
    Speaker {
        company: company.to_owned(),
        identity: Some("U1".to_owned()),
        name: "Ada".to_owned(),
        email: None,
        is_admin,
        is_direct,
        connector: Some("slack".to_owned()),
        conversation: Some("C1".to_owned()),
        message: Some("1.0".to_owned()),
    }
}

#[test]
fn an_administrator_of_the_company_proposes_in_a_direct_message_and_a_channel_alike() {
    for is_direct in [true, false] {
        let asked = speaker("acme", true, is_direct);
        assert_eq!(
            decide_speaker(Ok(Some(asked.clone())), &acme()),
            SpeakerDecision::Proposal(asked)
        );
    }
}

#[test]
fn no_speaker_another_companys_administrator_and_a_member_propose_nothing() {
    assert_eq!(
        decide_speaker(Ok(None), &acme()),
        SpeakerDecision::NotAProposal
    );
    assert_eq!(
        decide_speaker(Ok(Some(speaker("globex", true, true))), &acme()),
        SpeakerDecision::NotAProposal
    );
    assert_eq!(
        decide_speaker(Ok(Some(speaker("acme", false, true))), &acme()),
        SpeakerDecision::NotAProposal
    );
}

#[test]
fn a_lookup_that_cannot_answer_is_asked_again() {
    for error in [
        McpError::Unbound {
            company: "acme".to_owned(),
        },
        McpError::Turn {
            cause: "the runtime has no turn of this session in progress".to_owned(),
        },
    ] {
        assert!(matches!(
            decide_speaker(Err(error), &acme()),
            SpeakerDecision::Retry(_)
        ));
    }
}

fn recorded(entries: &[(&str, Recorded)]) -> BTreeMap<String, Recorded> {
    entries
        .iter()
        .map(|(commit, recorded)| ((*commit).to_owned(), recorded.clone()))
        .collect()
}

fn commits(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

fn paths(list: &[&str]) -> Vec<PathBuf> {
    list.iter().map(PathBuf::from).collect()
}

#[test]
fn a_head_of_recorded_proposals_changing_knowledge_alone_is_admitted() {
    let store = recorded(&[("a1", Recorded::Proposal), ("a2", Recorded::Proposal)]);
    assert_eq!(
        admission(
            &commits(&["a2", "a1"]),
            &store,
            &paths(&["docs/a.md", "AGENTS.md"])
        ),
        Admission::Admitted
    );
}

#[test]
fn one_unrecorded_or_unestablished_commit_leaves_the_head_waiting() {
    let store = recorded(&[
        ("a1", Recorded::Proposal),
        (
            "u1",
            Recorded::Unestablished("the runtime did not answer".to_owned()),
        ),
    ]);
    assert_eq!(
        admission(&commits(&["b1", "a1"]), &store, &paths(&["docs/a.md"])),
        Admission::NotAProposal("b1".to_owned())
    );
    assert_eq!(
        admission(&commits(&["u1"]), &store, &paths(&["docs/a.md"])),
        Admission::NotAProposal("u1".to_owned())
    );
}

#[test]
fn one_path_outside_the_knowledge_leaves_the_head_waiting() {
    let store = recorded(&[("a1", Recorded::Proposal)]);
    assert_eq!(
        admission(
            &commits(&["a1"]),
            &store,
            &paths(&["docs/a.md", "instrukt.yaml"])
        ),
        Admission::OutsideKnowledge(PathBuf::from("instrukt.yaml"))
    );
}

#[test]
fn a_head_adding_nothing_is_not_admitted() {
    assert_eq!(
        admission(&[], &recorded(&[]), &[]),
        Admission::NothingToAdmit
    );
}

#[test]
fn an_entry_retires_once_canonical_contains_it_or_nothing_holds_it() {
    let facts = vec![
        (
            "merged".to_owned(),
            Holding {
                contained: true,
                held: true,
            },
        ),
        (
            "worktree".to_owned(),
            Holding {
                contained: false,
                held: true,
            },
        ),
        (
            "abandoned".to_owned(),
            Holding {
                contained: false,
                held: false,
            },
        ),
    ];
    assert_eq!(
        retired(&facts),
        vec!["merged".to_owned(), "abandoned".to_owned()]
    );
}
