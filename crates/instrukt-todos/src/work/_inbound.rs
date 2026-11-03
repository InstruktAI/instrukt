//! Approvers' commands on a todo's issue (`github-record`, "Writer and
//! convergence"): `/instrukt approve <prd|task> <digest-prefix>` and
//! `/instrukt request-changes <prd|task> <reason>`.

use crate::evidence::{ApprovalGate, ContentDigest};

/// What every command comment starts with.
pub(crate) const COMMAND: &str = "/instrukt ";

/// How many hex characters of an evidence digest the issue shows beside an
/// open gate, and the fewest an approval must give.
pub(crate) const SHOWN_DIGEST: usize = 12;
const FEWEST_DIGEST: usize = 7;

/// One command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    Approve { gate: ApprovalGate, prefix: String },
    RequestChanges { gate: ApprovalGate, reason: String },
}

/// The command a comment body states, or why it states none.
pub(crate) fn parse(body: &str) -> Result<Command, String> {
    let line = body.trim().lines().next().unwrap_or_default();
    let words: Vec<&str> = line.split_whitespace().collect();
    let gate = |word: Option<&&str>| match word.copied() {
        Some("prd") => Ok(ApprovalGate::Prd),
        Some("task") => Ok(ApprovalGate::Task),
        _ => Err("name the gate: `prd` or `task`".to_owned()),
    };
    match words.get(1).copied() {
        Some("approve") => {
            let gate = gate(words.get(2))?;
            let prefix = words
                .get(3)
                .map(|p| p.trim_start_matches("sha256:").to_ascii_lowercase())
                .filter(|p| {
                    p.len() >= FEWEST_DIGEST
                        && p.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
                })
                .ok_or_else(|| {
                    format!(
                        "approve the evidence you reviewed by its digest prefix (at least {FEWEST_DIGEST} characters, as the issue shows it)"
                    )
                })?;
            Ok(Command::Approve { gate, prefix })
        }
        Some("request-changes") => {
            let gate = gate(words.get(2))?;
            let reason = words.get(3..).map(|rest| rest.join(" ")).unwrap_or_default();
            if reason.trim().is_empty() {
                return Err("a change request states its reason".to_owned());
            }
            Ok(Command::RequestChanges { gate, reason })
        }
        _ => Err(
            "the commands are `/instrukt approve <prd|task> <digest-prefix>` and `/instrukt request-changes <prd|task> <reason>`"
                .to_owned(),
        ),
    }
}

/// The hex of a digest an approval prefix is compared with.
pub(crate) fn digest_hex(digest: &ContentDigest) -> &str {
    digest
        .as_str()
        .strip_prefix("sha256:")
        .unwrap_or(digest.as_str())
}

/// The prefix the issue shows beside an open gate.
pub(crate) fn shown(digest: &ContentDigest) -> &str {
    let hex = digest_hex(digest);
    &hex[..hex.len().min(SHOWN_DIGEST)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_approval_names_its_gate_and_a_long_enough_digest_prefix() {
        assert!(matches!(
            parse("/instrukt approve prd 0123abcd"),
            Ok(Command::Approve {
                gate: ApprovalGate::Prd,
                ..
            })
        ));
        assert!(parse("/instrukt approve prd 0123").is_err());
        assert!(parse("/instrukt approve plan 0123abcd").is_err());
    }

    #[test]
    fn a_change_request_needs_a_reason() {
        assert!(matches!(
            parse("/instrukt request-changes task the plan misses the API"),
            Ok(Command::RequestChanges {
                gate: ApprovalGate::Task,
                ..
            })
        ));
        assert!(parse("/instrukt request-changes task").is_err());
    }
}
