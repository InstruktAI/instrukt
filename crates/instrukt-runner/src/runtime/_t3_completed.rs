//! Translate T3 projection relationships into runtime-neutral completed-turn evidence.
use super::conversations::{
    AuthorEvidence, AuthorKind, CompletedTurn, ConversationRecord, RecordKind, Role, SourceError,
    TurnContribution,
};
use instrukt_core::ids::{NativeTurnRef, SessionId};
fn shape(message: &'static str) -> SourceError {
    SourceError::Shape(message)
}
pub(super) fn completed(
    records: &[ConversationRecord],
    incomplete: &[String],
) -> Result<Vec<CompletedTurn>, SourceError> {
    if incomplete.iter().any(|id| id == "*") {
        return Err(shape("streaming record has no native run identity"));
    }
    let mut runs = records
        .iter()
        .filter(|r| matches!(r.kind, RecordKind::Run) && r.completed && !incomplete.contains(&r.id))
        .collect::<Vec<_>>();
    for run in &runs {
        if run.native["ordinal"].as_u64().is_none() {
            return Err(shape("run ordinal"));
        }
    }
    runs.sort_by_key(|r| r.native["ordinal"].as_u64());
    let mut result = Vec::new();
    for run in runs {
        let thread = run.conversation_id.as_str();
        let mut entries = records
            .iter()
            .filter(|r| {
                r.conversation_id == thread
                    && matches!(r.kind, RecordKind::TurnItem)
                    && r.run_id.as_deref() == Some(run.id.as_str())
                    && matches!(
                        r.native["type"].as_str(),
                        Some("user_message" | "assistant_message")
                    )
            })
            .collect::<Vec<_>>();
        for item in &entries {
            if item.native["ordinal"].as_u64().is_none() {
                return Err(shape("message item ordinal"));
            }
        }
        entries.sort_by_key(|r| r.native["ordinal"].as_u64());
        let mut messages = Vec::new();
        if entries.is_empty() {
            let user = run.native["userMessageId"]
                .as_str()
                .ok_or_else(|| shape("run userMessageId"))?;
            let user = records
                .iter()
                .find(|r| {
                    r.id == user
                        && r.conversation_id == thread
                        && matches!(r.kind, RecordKind::Message)
                })
                .ok_or_else(|| shape("run's recorded human input"))?;
            messages.push(user);
            let replies = records
                .iter()
                .filter(|r| {
                    r.conversation_id == thread
                        && matches!(r.kind, RecordKind::Message)
                        && r.run_id.as_deref() == Some(run.id.as_str())
                        && matches!(r.role, Some(Role::Assistant))
                })
                .collect::<Vec<_>>();
            if replies.len() > 1 {
                return Err(shape("multiple replies need native message item ordering"));
            }
            messages.extend(replies);
        } else {
            if let Some(user_id) = run.native["userMessageId"].as_str()
                && entries
                    .iter()
                    .all(|item| item.native["messageId"].as_str() != Some(user_id))
            {
                let user = records
                    .iter()
                    .find(|record| {
                        record.id == user_id
                            && record.conversation_id == thread
                            && matches!(record.kind, RecordKind::Message)
                            && matches!(record.role, Some(Role::User))
                    })
                    .ok_or_else(|| shape("run's recorded human input"))?;
                messages.push(user);
            }
            for entry in entries {
                let id = entry.native["messageId"]
                    .as_str()
                    .ok_or_else(|| shape("message item identity"))?;
                let message = records
                    .iter()
                    .find(|r| {
                        r.id == id
                            && r.conversation_id == thread
                            && matches!(r.kind, RecordKind::Message)
                    })
                    .ok_or_else(|| shape("completed message content"))?;
                if !messages
                    .iter()
                    .any(|m: &&ConversationRecord| m.id == message.id)
                {
                    messages.push(message);
                }
            }
        }
        let mut contributions = Vec::new();
        let identity = run.native["providerInstanceId"]
            .as_str()
            .zip(run.native["modelSelection"]["model"].as_str())
            .map(|(provider, model)| format!("{provider}/{model}"));
        for message in messages {
            let author = match message.role {
                Some(Role::User) => AuthorEvidence {
                    kind: AuthorKind::Human,
                    identity: None,
                },
                Some(Role::Assistant) => AuthorEvidence {
                    kind: AuthorKind::Agent,
                    identity: identity.clone(),
                },
                _ => continue,
            };
            let text = message.native["text"]
                .as_str()
                .ok_or_else(|| shape("message text is unavailable"))?;
            contributions.push(TurnContribution {
                order: contributions.len() as u64,
                author,
                text: text.into(),
                occurred_at: message.created_at,
            });
        }
        result.push(CompletedTurn {
            conversation: SessionId::parse(thread)
                .map_err(|_| shape("invalid conversation reference"))?,
            native_turn: NativeTurnRef::parse(&run.id)
                .map_err(|_| shape("invalid native turn reference"))?,
            order: run.native["ordinal"]
                .as_u64()
                .ok_or_else(|| shape("run ordinal is unavailable"))?,
            contributions,
        });
    }
    Ok(result)
}
