//! Lightweight outer framing for runner-authored assignments and notifications.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Assignment,
    Notification,
}

impl Control {
    pub fn name(self) -> &'static str {
        match self {
            Self::Assignment => "Instrukt Assignment",
            Self::Notification => "Instrukt Notification",
        }
    }

    /// If `text` begins with a complete frame of this kind (closing on its own
    /// line, followed by EOF or newline), returns `(complete_frame, body, remainder)`.
    pub fn split_leading(self, text: &str) -> Option<(&str, &str, &str)> {
        let open = format!("[{}]\n", self.name());
        let close = format!("\n[/{}]", self.name());
        let after_open = text.strip_prefix(&open)?;
        let (body, tail) = after_open.split_once(&close)?;
        if tail.is_empty() || tail.starts_with('\n') {
            let frame_len = open.len() + body.len() + close.len();
            Some((&text[..frame_len], body, tail))
        } else {
            None
        }
    }

    /// If `text` begins with any complete runner control frame, returns its inner body.
    pub fn unwrap_any(text: &str) -> Option<&str> {
        [Self::Notification, Self::Assignment]
            .into_iter()
            .find_map(|kind| kind.split_leading(text).map(|(_, body, _)| body))
    }

    pub fn is_framed(self, body: &str) -> bool {
        self.split_leading(body.trim_end())
            .is_some_and(|(_, _, tail)| tail.is_empty())
    }

    pub fn frame(self, body: &str) -> String {
        if self.is_framed(body) {
            return body.into();
        }
        format!(
            "[{}]\n{}\n[/{}]",
            self.name(),
            body.trim_end_matches('\n'),
            self.name()
        )
    }

    /// True when `body` consists solely of one or more complete control frames
    /// separated by optional whitespace (e.g. composed procedure launch blocks).
    pub fn is_control_only(body: &str) -> bool {
        let mut rest = body.trim();
        if rest.is_empty() {
            return false;
        }
        while !rest.is_empty() {
            let matched = [Self::Assignment, Self::Notification]
                .into_iter()
                .find_map(|kind| {
                    kind.split_leading(rest)
                        .map(|(_, _, tail)| tail.trim_start())
                });
            match matched {
                Some(next) => rest = next,
                None => return false,
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::Control;

    #[test]
    fn frame_wraps_once_and_round_trips_trimmed_body() {
        let framed = Control::Notification.frame("instrukt delivery log\nline\n\n");
        insta::assert_snapshot!("control-notification-frame", framed);
        assert_eq!(Control::Notification.frame(&framed), framed);
        assert_eq!(
            Control::Notification.split_leading(&framed),
            Some((framed.as_str(), "instrukt delivery log\nline", ""))
        );
        assert_eq!(
            Control::unwrap_any(&framed),
            Some("instrukt delivery log\nline")
        );
        assert_eq!(Control::Assignment.split_leading(&framed), None);
    }

    #[test]
    fn is_control_only_accepts_adjacent_control_frames() {
        let composed = format!(
            "{}\n{}",
            Control::Assignment.frame("loader"),
            Control::Assignment.frame("body")
        );
        assert!(Control::is_control_only(&composed));
        assert!(Control::is_control_only(
            &Control::Notification.frame("log")
        ));
    }

    #[test]
    fn is_control_only_rejects_unframed_or_trailing_text() {
        assert!(!Control::is_control_only(""));
        assert!(!Control::is_control_only("plain brief"));
        let mixed = format!("{}\nextra", Control::Assignment.frame("loader"));
        assert!(!Control::is_control_only(&mixed));
        let trailing = format!("{} trailing", Control::Assignment.frame("body"));
        assert!(!Control::is_control_only(&trailing));
        assert_eq!(Control::Assignment.split_leading(&trailing), None);
    }
}
