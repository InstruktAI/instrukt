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

    pub fn is_framed(self, body: &str) -> bool {
        body.starts_with(&format!("[{}]\n", self.name()))
            && body.trim_end().ends_with(&format!("[/{}]", self.name()))
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
                    let open = format!("[{}]\n", kind.name());
                    let close = format!("\n[/{}]", kind.name());
                    let after_open = rest.strip_prefix(&open)?;
                    let (_, tail) = after_open.split_once(&close)?;
                    if tail.is_empty() || tail.starts_with('\n') {
                        Some(tail.trim_start())
                    } else {
                        None
                    }
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
    fn frame_wraps_once_and_trims_trailing_newlines() {
        let framed = Control::Notification.frame("instrukt delivery log\nline\n\n");
        assert_eq!(
            framed,
            "[Instrukt Notification]\ninstrukt delivery log\nline\n[/Instrukt Notification]"
        );
        assert_eq!(Control::Notification.frame(&framed), framed);
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
        let spoofed = "[Instrukt Assignment]\nbody\n[/Instrukt Assignment] trailing";
        assert!(!Control::is_control_only(spoofed));
    }
}
