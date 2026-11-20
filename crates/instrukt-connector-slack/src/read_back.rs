//! Slack history traversal with a restartable, depth-first page position.
use super::{
    Incoming, Me, SLACK, Slack, SlackError, newly_written, own_message, ts_micros, ts_time,
};
use instrukt_connector::{ReadBackError, ReadBackPage};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize)]
enum Listing {
    Channels,
    History {
        channel: String,
        direct: bool,
    },
    Replies {
        channel: String,
        direct: bool,
        parent: String,
    },
}

#[derive(Serialize, Deserialize)]
struct Request {
    listing: Listing,
    cursor: String,
}

impl Slack {
    pub(super) fn history_page(
        &self,
        from: Timestamp,
        position: Option<String>,
    ) -> Result<ReadBackPage, ReadBackError> {
        let mut pending: Vec<Request> = match position {
            Some(position) => serde_json::from_str(&position).map_err(failed)?,
            None => vec![Request {
                listing: Listing::Channels,
                cursor: String::new(),
            }],
        };
        let request = pending
            .pop()
            .ok_or_else(|| failed("empty Slack read-back position"))?;
        let oldest = format!(
            "{}.{:06}",
            from.as_second(),
            from.subsec_nanosecond() / 1000
        );
        let (method, collection, mut form) = match &request.listing {
            Listing::Channels => (
                "users.conversations",
                "channels",
                vec![("types", "public_channel,private_channel,im")],
            ),
            Listing::History { channel, .. } => (
                "conversations.history",
                "messages",
                vec![("channel", channel.as_str())],
            ),
            Listing::Replies {
                channel, parent, ..
            } => (
                "conversations.replies",
                "messages",
                vec![
                    ("channel", channel.as_str()),
                    ("ts", parent.as_str()),
                    ("oldest", oldest.as_str()),
                    ("inclusive", "true"),
                ],
            ),
        };
        form.push(("limit", "200"));
        if !request.cursor.is_empty() {
            form.push(("cursor", &request.cursor));
        }
        let me = self.me().map_err(web_error)?;
        let body = match self.api.call(method, false, &form) {
            Err(SlackError::Api { cause, .. })
                if !request.cursor.is_empty() && cause == "invalid_cursor" =>
            {
                // Slack can expire a cursor while the runner is absent. Only
                // that listing starts over; the saved scan window still holds.
                form.pop();
                self.api.call(method, false, &form)
            }
            result => result,
        };
        let body = match body {
            Err(SlackError::Api { cause, .. })
                if (method == "conversations.history" && cause == "channel_not_found")
                    || (method == "conversations.replies"
                        && matches!(cause.as_str(), "channel_not_found" | "thread_not_found")) =>
            {
                return Ok(ReadBackPage {
                    items: Vec::new(),
                    next: continuation(&pending)?,
                });
            }
            result => result.map_err(web_error)?,
        };
        let page = body[collection]
            .as_array()
            .ok_or_else(|| failed(format!("{method} omitted {collection}")))?;
        let next = body["response_metadata"]["next_cursor"]
            .as_str()
            .unwrap_or_default();
        if next.is_empty() && body["has_more"] == true {
            return Err(failed(format!("{method} omitted the next cursor")));
        }
        if !next.is_empty()
            && next == request.cursor
            && form.last().is_some_and(|(name, _)| *name == "cursor")
        {
            return Err(failed(format!("{method} repeated its cursor")));
        }
        let mut items = Vec::new();
        let mut children = Vec::new();
        for item in page {
            match &request.listing {
                Listing::Channels => children.push(Listing::History {
                    channel: field(item, "id")?.to_owned(),
                    direct: item["is_im"].as_bool().unwrap_or(false),
                }),
                Listing::History { channel, direct } => {
                    if let Some(message) = recover(&me, channel, None, *direct, item, from)? {
                        items.push(message);
                    }
                    let ts = field(item, "ts")?;
                    // Broadcast replies are entries on this page, but do not
                    // supply a separate parent to traverse.
                    if item["thread_ts"]
                        .as_str()
                        .is_some_and(|parent| parent != ts)
                    {
                        continue;
                    }
                    let has_replies = item["reply_count"].as_u64().unwrap_or(0) > 0;
                    if item["latest_reply"]
                        .as_str()
                        .and_then(ts_time)
                        .map_or(has_replies, |latest| latest >= from)
                    {
                        children.push(Listing::Replies {
                            channel: channel.clone(),
                            direct: *direct,
                            parent: ts.to_owned(),
                        });
                    }
                }
                Listing::Replies {
                    channel,
                    parent,
                    direct,
                } => {
                    if item["ts"].as_str() != Some(parent)
                        && let Some(message) =
                            recover(&me, channel, Some(parent), *direct, item, from)?
                    {
                        items.push(message);
                    }
                }
            }
        }
        if !next.is_empty() {
            pending.push(Request {
                listing: request.listing,
                cursor: next.to_owned(),
            });
        }
        pending.extend(children.into_iter().rev().map(|listing| Request {
            listing,
            cursor: String::new(),
        }));
        Ok(ReadBackPage {
            items,
            next: continuation(&pending)?,
        })
    }
}

fn continuation(pending: &[Request]) -> Result<Option<String>, ReadBackError> {
    if pending.is_empty() {
        Ok(None)
    } else {
        serde_json::to_string(pending).map(Some).map_err(failed)
    }
}

fn failed(error: impl std::fmt::Display) -> ReadBackError {
    ReadBackError {
        cause: error.to_string(),
        retry_after: None,
    }
}

fn web_error(error: SlackError) -> ReadBackError {
    let retry_after = match &error {
        SlackError::Limited { seconds, .. } => Some(*seconds),
        _ => None,
    };
    ReadBackError {
        cause: error.to_string(),
        retry_after,
    }
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, ReadBackError> {
    value[name]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| failed(format!("Slack read-back omitted {name}")))
}

fn recover(
    me: &Me,
    channel: &str,
    parent: Option<&str>,
    direct: bool,
    message: &Value,
    from: Timestamp,
) -> Result<Option<(Incoming, String)>, ReadBackError> {
    let Some(sender) = message["user"].as_str().filter(|user| !user.is_empty()) else {
        return Ok(None);
    };
    if !newly_written(message) || own_message(message, me) || message.get("bot_id").is_some() {
        return Ok(None);
    }
    let text = message["text"].as_str().unwrap_or_default();
    if !direct && !text.contains(&format!("<@{}>", me.user)) {
        return Ok(None);
    }
    let ts = field(message, "ts")?;
    let written_at =
        ts_time(ts).ok_or_else(|| failed("Slack read-back found an invalid message time"))?;
    if written_at < from {
        return Ok(None);
    }
    let timestamp =
        ts_micros(ts).ok_or_else(|| failed("Slack read-back found an invalid message time"))?;
    let thread = message["thread_ts"].as_str().or(parent).unwrap_or(ts);
    Ok(Some((
        Incoming {
            connector: SLACK.into(),
            conversation: if direct {
                channel.to_owned()
            } else {
                format!("{channel}/{thread}")
            },
            message: ts.to_owned(),
            sender: sender.to_owned(),
            written_at,
            text: text.to_owned(),
        },
        format!("https://app.slack.com/archives/{channel}/p{timestamp}"),
    )))
}
