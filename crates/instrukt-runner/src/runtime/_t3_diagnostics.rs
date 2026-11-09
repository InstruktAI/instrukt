//! Parse the T3 CLI's declared failure formats, not arbitrary server/tool prose.
use super::NativeCause;

// The native error constructors own these wire formats: apps/server/src/cli/thread.ts
// at T3 6417151c093e54dbba0e6c824c69d03983793b9a. Opaque detail stays fingerprinted.
pub(super) fn cause(detail: &str) -> Option<NativeCause> {
    let line = detail.lines().next()?.trim();
    if line.starts_with("No running T3 Code server uses ")
        && line.ends_with(". Start T3 Code, then retry.")
    {
        return Some(NativeCause::ServerNotRunning);
    }
    if let Some(rest) = line.strip_prefix("Could not reach the T3 Code server at ") {
        let reason = rest
            .split_once(". ")
            .map(|(_, reason)| reason)
            .unwrap_or("");
        return Some(NativeCause::ServerUnreachable {
            code: transport_code(reason),
        });
    }
    if let Some(code) = transport_code(line) {
        return Some(NativeCause::Transport { code });
    }
    if line.starts_with("The server sent an unexpected t3_")
        && line.ends_with(" result. Update the t3 CLI to match the server.")
    {
        return Some(NativeCause::IncompatibleResponse);
    }
    if let Some(id) = line
        .strip_prefix("Thread ")
        .and_then(|s| s.strip_suffix(" is no longer available."))
        && instrukt_core::ids::SessionId::parse(id).is_ok()
    {
        return Some(NativeCause::TargetGone);
    }
    None
}
fn transport_code(value: &str) -> Option<String> {
    let mut fields = value.split_whitespace();
    if !matches!(fields.next()?, "connect" | "read" | "write" | "getaddrinfo") {
        return None;
    }
    let code = fields.next()?;
    if matches!(
        code,
        "ECONNREFUSED"
            | "ECONNRESET"
            | "ECONNABORTED"
            | "ETIMEDOUT"
            | "ENETUNREACH"
            | "EHOSTUNREACH"
            | "ENOTFOUND"
            | "EAI_AGAIN"
            | "EPIPE"
            | "EACCES"
    ) {
        Some(code.into())
    } else {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::cause;
    use crate::runtime::NativeCause;
    #[test]
    fn declared_transport_codes_survive_while_payload_and_authorization_lines_are_excluded() {
        assert_eq!(
            cause(
                "Could not reach the T3 Code server at http://localhost. connect ECONNREFUSED localhost\nAuthorization: Bearer PRIVATE\nTranscript body"
            ),
            Some(NativeCause::ServerUnreachable {
                code: Some("ECONNREFUSED".into())
            })
        );
    }
    #[test]
    fn opaque_tool_detail_does_not_become_diagnostic_prose() {
        assert_eq!(
            cause("PRIVATE MODEL OUTPUT\nAuthorization: Bearer PRIVATE"),
            None
        );
    }
}
