//! Project completed native runs into conversational contributions only.
use super::LinkError;
use crate::{
    envelopes::{self, Span},
    runtime::conversations::{ConversationRecord, RecordKind, Role},
};
fn minute(at: Option<jiff::Timestamp>) -> String {
    at.map(|at| {
        at.to_zoned(jiff::tz::TimeZone::UTC)
            .strftime("%Y-%m-%d %H:%M UTC")
            .to_string()
    })
    .unwrap_or_else(|| "unknown".into())
}
pub fn human_contributions(
    source: &str,
    target: &str,
    text: &str,
    at: Option<jiff::Timestamp>,
) -> Vec<String> {
    let mut contributions = Vec::new();
    for span in envelopes::segment(text) {
        match span {
            Span::Ordinary(body) if !body.trim().is_empty() => {
                contributions.push(envelopes::peer(source, "human", &minute(at), body))
            }
            Span::Identity {
                source,
                author,
                at,
                body,
            } if source != target => contributions.push(envelopes::peer(source, author, at, body)),
            _ => {}
        }
    }
    contributions
}
fn shape(message: &str) -> LinkError {
    LinkError::Refused(format!(
        "unsupported or missing native turn evidence: {message}"
    ))
}
fn author(run: &ConversationRecord) -> String {
    let provider = run.native["providerInstanceId"].as_str();
    let model = run.native["modelSelection"]["model"].as_str();
    match provider.zip(model) {
        Some((provider, model)) => {
            format!("agent:{}/{}", provider.escape_debug(), model.escape_debug())
                .replace(" · ", " ")
        }
        None => "agent".into(),
    }
}
pub(super) fn finished(
    batch: &crate::runtime::conversations::ConversationChanges,
    thread: &str,
    source: &str,
    target: &str,
) -> Result<Vec<(String, String)>, LinkError> {
    let records = &batch.records;
    if batch.incomplete_runs.iter().any(|id| id == "*") {
        return Err(shape("streaming record has no native run identity"));
    }
    let mut runs = records
        .iter()
        .filter(|r| {
            r.conversation_id == thread
                && matches!(r.kind, RecordKind::Run)
                && r.completed
                && !batch.incomplete_runs.contains(&r.id)
        })
        .collect::<Vec<_>>();
    for run in &runs {
        if run.native["ordinal"].as_u64().is_none() {
            return Err(shape("run ordinal"));
        }
    }
    runs.sort_by_key(|r| r.native["ordinal"].as_u64());
    let mut result = Vec::new();
    for run in runs {
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
        for message in messages {
            let text = message.native["text"]
                .as_str()
                .ok_or_else(|| shape("message text"))?;
            match message.role {
                Some(Role::User) => contributions.extend(human_contributions(
                    source,
                    target,
                    text,
                    message.created_at,
                )),
                Some(Role::Assistant) if text.trim() != "NOOP" && !text.trim().is_empty() => {
                    contributions.push(envelopes::peer(
                        source,
                        &author(run),
                        &minute(message.created_at),
                        text,
                    ))
                }
                _ => {}
            }
        }
        result.push((run.id.clone(), contributions.join("\n\n")));
    }
    Ok(result)
}
