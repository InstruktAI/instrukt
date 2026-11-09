//! The shared vocabulary for native input framing and attributed contributions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Assignment,
    Notification,
}
impl Control {
    fn name(self) -> &'static str {
        match self {
            Self::Assignment => "Instrukt Assignment",
            Self::Notification => "Instrukt Notification",
        }
    }
    pub fn frame(self, body: &str) -> String {
        if body.starts_with(&format!("[{}]\n", self.name()))
            && body.trim_end().ends_with(&format!("[/{}]", self.name()))
        {
            return body.into();
        }
        format!(
            "[{}]\n{}\n[/{}]",
            self.name(),
            body.trim_end_matches('\n'),
            self.name()
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span<'a> {
    Ordinary(&'a str),
    Identity {
        source: &'a str,
        author: &'a str,
        at: &'a str,
        body: &'a str,
    },
    Control,
}
type PeerIdentity<'a> = (&'a str, &'a str, &'a str);
fn opening(line: &str) -> Option<(&str, Option<PeerIdentity<'_>>)> {
    let tag = line.strip_prefix('[')?.strip_suffix(']')?;
    if [Control::Assignment.name(), Control::Notification.name()].contains(&tag) {
        return Some((tag, None));
    }
    let fields = tag
        .strip_prefix("Peer (")?
        .strip_suffix(')')?
        .split(" · ")
        .collect::<Vec<_>>();
    if fields.len() != 3
        || instrukt_core::ids::SessionRef::parse(fields[0]).is_err()
        || !(fields[1] == "human"
            || fields[1] == "agent"
            || fields[1].starts_with("human:")
            || fields[1].starts_with("agent:"))
    {
        return None;
    }
    let at = fields[2];
    if at != "unknown"
        && at
            .strip_suffix(" UTC")
            .and_then(|s| s.replace(' ', "T").parse::<jiff::civil::DateTime>().ok())
            .map(|parsed| parsed.strftime("%Y-%m-%d %H:%M UTC").to_string())
            .as_deref()
            != Some(at)
    {
        return None;
    }
    Some((tag, Some((fields[0], fields[1], at))))
}
/// Only complete registered frames establish provenance. Bodies are opaque.
pub fn segment(text: &str) -> Vec<Span<'_>> {
    let mut spans = Vec::new();
    let mut ordinary = 0;
    let mut at = 0;
    while at < text.len() {
        let end = text[at..].find('\n').map(|n| at + n).unwrap_or(text.len());
        let line = text[at..end].trim_end_matches('\r');
        if let Some((tag, identity)) = opening(line) {
            let close = format!("[/{tag}]");
            let body_start = (end + 1).min(text.len());
            let mut scan = body_start;
            let mut closing = None;
            while scan < text.len() {
                let finish = text[scan..]
                    .find('\n')
                    .map(|n| scan + n)
                    .unwrap_or(text.len());
                if text[scan..finish].trim_end_matches('\r') == close {
                    closing = Some((scan, (finish + 1).min(text.len())));
                    break;
                }
                scan = (finish + 1).min(text.len());
            }
            if let Some((body_end, next)) = closing {
                if ordinary < at {
                    spans.push(Span::Ordinary(&text[ordinary..at]));
                }
                spans.push(match identity {
                    Some((source, author, at)) => Span::Identity {
                        source,
                        author,
                        at,
                        body: text[body_start..body_end]
                            .strip_suffix('\n')
                            .unwrap_or(&text[body_start..body_end]),
                    },
                    None => Span::Control,
                });
                at = next;
                ordinary = at;
                continue;
            }
        }
        at = (end + 1).min(text.len());
    }
    if ordinary < text.len() {
        spans.push(Span::Ordinary(&text[ordinary..]));
    }
    spans
}
pub fn peer(source: &str, author: &str, at: &str, body: &str) -> String {
    format!(
        "[Peer ({source} · {author} · {at})]\n{}\n[/Peer ({source} · {author} · {at})]",
        body
    )
}

pub fn minute(at: Option<jiff::Timestamp>) -> String {
    at.map(|at| {
        at.to_zoned(jiff::tz::TimeZone::UTC)
            .strftime("%Y-%m-%d %H:%M UTC")
            .to_string()
    })
    .unwrap_or_else(|| "unknown".into())
}
pub fn agent(provider: Option<&str>, model: Option<&str>) -> String {
    match provider.zip(model) {
        Some((provider, model)) => {
            format!("agent:{}/{}", provider.escape_debug(), model.escape_debug())
                .replace(" · ", " ")
        }
        None => "agent".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Control, Span, peer, segment};
    #[test]
    fn peer_frames_preserve_trailing_newlines_and_require_minute_precision() {
        let at = "2026-11-08 10:00 UTC";
        let body = "Keep both newlines.\n\n";
        let framed = peer("t3:scope:thread", "human", at, body);
        assert_eq!(
            segment(&framed),
            vec![Span::Identity {
                source: "t3:scope:thread",
                author: "human",
                at,
                body
            }]
        );
        let invalid = peer("t3:scope:thread", "human", "2026-11-08 10:00:33 UTC", body);
        assert_eq!(segment(&invalid), vec![Span::Ordinary(&invalid)]);
    }
    #[test]
    fn controls_are_opaque_and_incomplete_shapes_remain_ordinary() {
        let body = "[Peer (t3:a · human · unknown)]\nquoted\n[/Peer (t3:a · human · unknown)]";
        assert_eq!(
            segment(&Control::Notification.frame(body)),
            vec![Span::Control]
        );
        assert_eq!(
            segment("[Instrukt Assignment]\nmissing"),
            vec![Span::Ordinary("[Instrukt Assignment]\nmissing")]
        );
    }
}
