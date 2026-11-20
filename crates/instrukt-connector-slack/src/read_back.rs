//! Slack history traversal for a connected app's offline messages.
use super::{
    ConnectorError, Incoming, Me, SLACK, Slack, failed, newly_written, own_message, ts_micros,
    ts_time,
};
use jiff::Timestamp;
use serde_json::Value;
use std::collections::BTreeSet;

impl Slack {
    pub(super) fn scan_history(
        &self,
        from: Timestamp,
        quit: &dyn Fn() -> bool,
        retain: &mut dyn FnMut(Incoming, String) -> Result<(), ConnectorError>,
    ) -> Result<(), ConnectorError> {
        let me = self.me().map_err(failed)?;
        let oldest = format!(
            "{}.{:06}",
            from.as_second(),
            from.subsec_nanosecond() / 1000
        );
        self.pages(
            "users.conversations",
            "channels",
            &[("types", "public_channel,private_channel,im")],
            quit,
            &mut |channel| {
                let id = field(channel, "id")?;
                let direct = channel["is_im"].as_bool().unwrap_or(false);
                self.pages(
                    "conversations.history",
                    "messages",
                    &[("channel", id)],
                    quit,
                    &mut |parent| {
                        let ts = field(parent, "ts")?;
                        recover(&me, id, None, direct, parent, from, retain)?;
                        let has_replies = parent["reply_count"].as_u64().unwrap_or(0) > 0;
                        let latest_reply = parent["latest_reply"].as_str().and_then(ts_time);
                        if latest_reply.map_or(has_replies, |latest| latest >= from) {
                            self.pages(
                                "conversations.replies",
                                "messages",
                                &[
                                    ("channel", id),
                                    ("ts", ts),
                                    ("oldest", &oldest),
                                    ("inclusive", "true"),
                                ],
                                quit,
                                &mut |reply| {
                                    if reply["ts"].as_str() != Some(ts) {
                                        recover(&me, id, Some(ts), direct, reply, from, retain)?;
                                    }
                                    Ok(())
                                },
                            )?;
                        }
                        Ok(())
                    },
                )
            },
        )
    }

    fn pages(
        &self,
        method: &str,
        collection: &str,
        args: &[(&str, &str)],
        quit: &dyn Fn() -> bool,
        visit: &mut dyn FnMut(&Value) -> Result<(), ConnectorError>,
    ) -> Result<(), ConnectorError> {
        let mut cursor = String::new();
        let mut seen = BTreeSet::new();
        loop {
            if quit() {
                return Err(failed("Slack read-back stopped"));
            }
            let mut form = args.to_vec();
            form.push(("limit", "200"));
            if !cursor.is_empty() {
                form.push(("cursor", &cursor));
            }
            let body = self.api.call(method, false, &form).map_err(failed)?;
            let page = body[collection]
                .as_array()
                .ok_or_else(|| failed(format!("{method} omitted {collection}")))?;
            for item in page {
                if quit() {
                    return Err(failed("Slack read-back stopped"));
                }
                visit(item)?;
            }
            let next = body["response_metadata"]["next_cursor"]
                .as_str()
                .unwrap_or_default();
            if next.is_empty() {
                if body["has_more"] == true {
                    return Err(failed(format!("{method} omitted the next cursor")));
                }
                return Ok(());
            }
            if !seen.insert(next.to_owned()) {
                return Err(failed(format!("{method} repeated its cursor")));
            }
            cursor = next.to_owned();
        }
    }
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, ConnectorError> {
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
    retain: &mut dyn FnMut(Incoming, String) -> Result<(), ConnectorError>,
) -> Result<(), ConnectorError> {
    let Some(sender) = message["user"].as_str().filter(|user| !user.is_empty()) else {
        return Ok(());
    };
    if !newly_written(message) || own_message(message, me) || message.get("bot_id").is_some() {
        return Ok(());
    }
    let text = message["text"].as_str().unwrap_or_default();
    if !direct && !text.contains(&format!("<@{}>", me.user)) {
        return Ok(());
    }
    let ts = field(message, "ts")?;
    let written_at =
        ts_time(ts).ok_or_else(|| failed("Slack read-back found an invalid message time"))?;
    if written_at < from {
        return Ok(());
    }
    let timestamp =
        ts_micros(ts).ok_or_else(|| failed("Slack read-back found an invalid message time"))?;
    let link = format!("https://app.slack.com/archives/{channel}/p{timestamp}");
    retain(
        Incoming {
            connector: SLACK.into(),
            conversation: if direct {
                channel.to_owned()
            } else {
                format!("{channel}/{}", parent.unwrap_or(ts))
            },
            message: ts.to_owned(),
            sender: sender.to_owned(),
            written_at,
            text: text.to_owned(),
        },
        link,
    )
}
