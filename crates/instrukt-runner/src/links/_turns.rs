//! Render typed completed turns; framing and delivery policy belong to links.
use crate::{
    envelopes::{self, Span},
    runtime::conversations::{AuthorKind, CompletedTurn, TurnContribution},
};
fn human_contributions(
    source: &str,
    target: &str,
    contribution: &TurnContribution,
) -> Result<Vec<String>, crate::sessions::SessionsError> {
    let mut result = Vec::new();
    for span in envelopes::segment(&contribution.text) {
        match span {
            Span::Ordinary(body) if !body.trim().is_empty() => result.push(envelopes::peer(
                source,
                &envelopes::speaker(
                    contribution.author.kind,
                    contribution.author.identity.as_deref(),
                ),
                &envelopes::minute(contribution.occurred_at),
                body,
            )),
            Span::Identity {
                source,
                author,
                at,
                body,
            } if source != target => result.push(envelopes::peer(source, author, at, body)),
            // A company member's message keeps its own attribution; it is not
            // this thread's human. It crosses as a contribution of the thread
            // it was addressed in, so the inherited-source exclusion keeps it
            // from returning there; a fresh token frames it for the peer.
            Span::Member {
                connector,
                conversation,
                member,
                at,
                body,
                ..
            } => {
                let token =
                    envelopes::delivery_boundary(&[connector, conversation, member, at, body])?;
                result.push(envelopes::peer(
                    source,
                    &envelopes::speaker(AuthorKind::Human, Some(member)),
                    at,
                    &envelopes::member_at(connector, conversation, member, at, &token, body),
                ));
            }
            _ => {}
        }
    }
    Ok(result)
}
pub(super) fn finished(
    turns: &[CompletedTurn],
    thread: &str,
    source: &str,
    target: &str,
) -> Result<Vec<(String, String)>, crate::sessions::SessionsError> {
    let mut turns = turns
        .iter()
        .filter(|turn| turn.conversation.as_str() == thread)
        .collect::<Vec<_>>();
    turns.sort_by_key(|turn| turn.order);
    turns
        .into_iter()
        .map(|turn| {
            let mut rendered = Vec::new();
            for contribution in &turn.contributions {
                match contribution.author.kind {
                    AuthorKind::Human => {
                        rendered.extend(human_contributions(source, target, contribution)?)
                    }
                    AuthorKind::Agent
                        if contribution.text.trim() != "NOOP"
                            && !contribution.text.trim().is_empty() =>
                    {
                        rendered.push(envelopes::peer(
                            source,
                            &envelopes::speaker(
                                contribution.author.kind,
                                contribution.author.identity.as_deref(),
                            ),
                            &envelopes::minute(contribution.occurred_at),
                            &contribution.text,
                        ))
                    }
                    _ => {}
                }
            }
            Ok((turn.native_turn.to_string(), rendered.join("\n\n")))
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::human_contributions;
    use crate::{
        envelopes::{self, Span},
        runtime::conversations::{AuthorEvidence, AuthorKind, TurnContribution},
    };
    #[test]
    fn inherited_identity_is_not_echoed_to_its_source() {
        let contribution=TurnContribution {order:0,author:AuthorEvidence{kind:AuthorKind::Human,identity:None},occurred_at:None,text:"clarification\n[Peer (t3:scope:b · agent:codex/model · unknown)]\npeer reply\n[/Peer (t3:scope:b · agent:codex/model · unknown)]\ntrailing words".into()};
        let output = human_contributions("t3:scope:a", "t3:scope:b", &contribution).unwrap();
        assert_eq!(output.len(), 2);
        assert!(output.iter().all(|text| matches!(&envelopes::segment(text)[0],Span::Identity{source,..} if *source=="t3:scope:a")));
    }
    #[test]
    fn a_member_message_crosses_the_link_once_with_its_own_attribution() {
        let token = "0123456789abcdef0123456789abcdef";
        let framed = format!(
            "{}\n{}",
            envelopes::context(token, &[], false),
            envelopes::member("slack", "C1/1.2", "steven", None, token, "a question")
        );
        let human = |text: String| TurnContribution {
            order: 0,
            author: AuthorEvidence {
                kind: AuthorKind::Human,
                identity: None,
            },
            occurred_at: None,
            text,
        };
        // Addressed in thread a, it crosses to b from a, with its member
        // attribution intact inside.
        let output = human_contributions("t3:scope:a", "t3:scope:b", &human(framed)).unwrap();
        assert_eq!(output.len(), 1);
        let spans = envelopes::segment(&output[0]);
        let [Span::Identity { source, body, .. }] = spans.as_slice() else {
            panic!("one peer contribution");
        };
        assert_eq!(*source, "t3:scope:a");
        assert!(matches!(
            envelopes::segment(body).as_slice(),
            [Span::Member { connector: "slack", conversation: "C1/1.2", member: "steven", boundary, .. }]
                if *boundary != token
        ));
        // Arriving in b as part of b's input, it does not return to a.
        let back =
            human_contributions("t3:scope:b", "t3:scope:a", &human(output[0].clone())).unwrap();
        assert!(back.is_empty());
    }
}
