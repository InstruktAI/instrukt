//! Render typed completed turns; framing and delivery policy belong to links.
use crate::{
    envelopes::{self, Span},
    runtime::conversations::{AuthorKind, CompletedTurn, TurnContribution},
};
fn human_contributions(source: &str, target: &str, contribution: &TurnContribution) -> Vec<String> {
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
            _ => {}
        }
    }
    result
}
pub(super) fn finished(
    turns: &[CompletedTurn],
    thread: &str,
    source: &str,
    target: &str,
) -> Vec<(String, String)> {
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
                        rendered.extend(human_contributions(source, target, contribution))
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
            (turn.native_turn.to_string(), rendered.join("\n\n"))
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
        let output = human_contributions("t3:scope:a", "t3:scope:b", &contribution);
        assert_eq!(output.len(), 2);
        assert!(output.iter().all(|text| matches!(&envelopes::segment(text)[0],Span::Identity{source,..} if *source=="t3:scope:a")));
    }
}
