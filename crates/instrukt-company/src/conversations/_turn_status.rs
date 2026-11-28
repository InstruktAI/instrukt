//! Single-writer curated active-turn status files (`state/company-turn-status/<digest>.json`).
//! Only `instrukt company status` writes these files (atomically, `0600`, zero locks);
//! the connector host reads them lock-free and never unlinks or mutates them.
use std::path::{Path, PathBuf};
use std::time::Duration;

use instrukt_core::config::SessionScope;
use instrukt_core::fsx;
use instrukt_core::home::Home;
use instrukt_core::ids::SessionRef;
use instrukt_runner::runtime::RuntimeAdapter;
use instrukt_runner::state::StateError;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::_replies_db::validate_target_conversation;
use super::_speaker::{conversation_binding, resolve_active_turn_within};
use super::{ConversationError, LOG_TARGET};

/// Current schema version for `state/company-turn-status/<digest>.json`.
pub const TURN_STATUS_SCHEMA_VERSION: u32 = 1;

/// Maximum UTF-8 byte length of a curated status line.
pub const MAX_STATUS_BYTES: usize = 256;

/// Maximum wait budget for live active-turn verification during `instrukt company status`.
const STATUS_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Validated non-empty, single-line, `<= 256` UTF-8 byte progress status text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusText(String);

impl StatusText {
    pub fn parse(raw: &str) -> Result<Self, ConversationError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(ConversationError::Refused(
                "status text must not be empty".into(),
            ));
        }
        if trimmed
            .chars()
            .any(|ch| ch == '\n' || ch == '\r' || ch.is_control())
        {
            return Err(ConversationError::Refused(
                "status text must be a single line with no control characters".into(),
            ));
        }
        if trimmed.len() > MAX_STATUS_BYTES {
            return Err(ConversationError::Refused(format!(
                "status text exceeds the {MAX_STATUS_BYTES}-byte limit"
            )));
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Persisted curated status file for one conversation binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnStatusFile {
    pub schema_version: u32,
    pub company: String,
    pub connector: String,
    pub conversation: String,
    pub turn_message: String,
    pub turn_input_id: String,
    pub text: String,
    pub updated_at: String,
}

/// Summary returned by `instrukt company status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnStatusSummary {
    pub company: String,
    pub connector: String,
    pub conversation: String,
    pub turn_message: String,
    pub text: String,
}

/// Deterministic lowercase hex SHA-256 file path for a binding:
/// `state/company-turn-status/<sha256(company\0connector\0binding_conversation)>.json`.
pub(crate) fn turn_status_path(
    home: &Home,
    company: &str,
    connector: &str,
    binding_conversation: &str,
) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(company.as_bytes());
    hasher.update(b"\0");
    hasher.update(connector.as_bytes());
    hasher.update(b"\0");
    hasher.update(binding_conversation.as_bytes());
    let digest = hex::encode(hasher.finalize());
    home.company_turn_status_dir()
        .join(format!("{digest}.json"))
}

fn refuse_symlink(path: &Path) -> Result<(), ConversationError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(ConversationError::Refused(format!(
            "refusing to access symlink at {}",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ConversationError::Refused(format!(
            "cannot inspect {}: {error}",
            path.display()
        ))),
    }
}

/// Publish a curated status line for the caller's currently active addressed
/// company turn. Fails closed if the caller is not bound to an active company
/// conversation, has no active addressed turn (Outcome 1), or targets a
/// conversation other than the active addressed turn's conversation.
pub fn set_turn_status(
    home: &Home,
    caller: &SessionRef,
    adapter: &dyn RuntimeAdapter,
    requested_conversation: Option<&str>,
    text: &str,
    now: Timestamp,
) -> Result<TurnStatusSummary, ConversationError> {
    let status = StatusText::parse(text)?;
    let Some(binding) = conversation_binding(home, caller).map_err(StateError::from)? else {
        return Err(ConversationError::Refused(
            "this session is not bound to an active company conversation".into(),
        ));
    };
    let target = validate_target_conversation(home, &binding, requested_conversation)?;
    let active = resolve_active_turn_within(home, caller, adapter, STATUS_OBSERVATION_TIMEOUT)
        .map_err(|err| ConversationError::Refused(err.to_string()))?
        .ok_or_else(|| {
            ConversationError::Refused(
                "status updates require an active addressed company turn".into(),
            )
        })?;
    if binding.session_scope == SessionScope::Company && active.conversation != target {
        return Err(ConversationError::Refused(format!(
            "conversation `{target}` does not match the active addressed turn's conversation `{}`",
            active.conversation
        )));
    }
    let dir = home.company_turn_status_dir();
    refuse_symlink(&dir)?;
    let path = turn_status_path(
        home,
        binding.company.as_str(),
        &binding.connector,
        &binding.conversation,
    );
    refuse_symlink(&path)?;
    let record = TurnStatusFile {
        schema_version: TURN_STATUS_SCHEMA_VERSION,
        company: binding.company.to_string(),
        connector: binding.connector.clone(),
        conversation: active.conversation.clone(),
        turn_message: active.message.clone(),
        turn_input_id: active.input_id.to_string(),
        text: status.as_str().to_owned(),
        updated_at: format!("{now:.9}"),
    };
    let bytes = serde_json::to_vec(&record)?;
    #[cfg(unix)]
    let permissions = std::os::unix::fs::PermissionsExt::from_mode(0o600);
    #[cfg(not(unix))]
    let permissions = {
        let mut perms = std::fs::metadata(home.state_dir())
            .map_err(|err| ConversationError::Refused(err.to_string()))?
            .permissions();
        perms.set_readonly(false);
        perms
    };
    fsx::atomic_write_with_permissions(&path, &bytes, permissions)?;
    Ok(TurnStatusSummary {
        company: binding.company.to_string(),
        connector: binding.connector,
        conversation: active.conversation,
        turn_message: active.message,
        text: status.as_str().to_owned(),
    })
}

/// Lock-free read of the curated turn status for a currently active addressed
/// turn. Returns `None` on missing, malformed, symlinked, or mismatched files
/// without ever unlinking or mutating the file.
pub fn read_turn_status(
    home: &Home,
    company: &str,
    connector: &str,
    binding_conversation: &str,
    expected_conversation: &str,
    expected_turn_message: &str,
    expected_turn_input_id: &str,
) -> Option<String> {
    let dir = home.company_turn_status_dir();
    if let Err(error) = refuse_symlink(&dir) {
        tracing::warn!(target: LOG_TARGET, event = "company_turn_status_ignored", %error);
        return None;
    }
    let path = turn_status_path(home, company, connector, binding_conversation);
    if let Err(error) = refuse_symlink(&path) {
        tracing::warn!(target: LOG_TARGET, event = "company_turn_status_ignored", path = %path.display(), %error);
        return None;
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!(target: LOG_TARGET, event = "company_turn_status_unreadable", path = %path.display(), %error);
            return None;
        }
    };
    let parsed: TurnStatusFile = match serde_json::from_slice(&bytes) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(target: LOG_TARGET, event = "company_turn_status_malformed", path = %path.display(), %error);
            return None;
        }
    };
    if parsed.schema_version != TURN_STATUS_SCHEMA_VERSION {
        tracing::warn!(
            target: LOG_TARGET,
            event = "company_turn_status_version_mismatch",
            version = parsed.schema_version,
            expected = TURN_STATUS_SCHEMA_VERSION
        );
        return None;
    }
    if parsed.company != company
        || parsed.connector != connector
        || parsed.conversation != expected_conversation
        || parsed.turn_message != expected_turn_message
        || parsed.turn_input_id != expected_turn_input_id
    {
        return None;
    }
    match StatusText::parse(&parsed.text) {
        Ok(valid) => Some(valid.as_str().to_owned()),
        Err(error) => {
            tracing::warn!(target: LOG_TARGET, event = "company_turn_status_invalid_text", %error);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_home() -> (tempfile::TempDir, Home) {
        let dir = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf()).for_writing();
        (dir, home)
    }

    #[test]
    fn status_text_enforces_non_empty_single_line_and_byte_limit() {
        assert!(StatusText::parse("").is_err());
        assert!(StatusText::parse("   ").is_err());
        assert!(StatusText::parse("line1\nline2").is_err());
        assert!(StatusText::parse("line1\rline2").is_err());
        assert!(StatusText::parse("bad\x07bell").is_err());
        assert!(StatusText::parse(&"a".repeat(MAX_STATUS_BYTES + 1)).is_err());
        // Multi-byte UTF-8 counts bytes, not chars (86 * 3 = 258 > 256).
        assert!(StatusText::parse(&"€".repeat(86)).is_err());
        assert_eq!(
            StatusText::parse(&"€".repeat(85)).unwrap().as_str().len(),
            255
        );
        assert_eq!(
            StatusText::parse("  Checking invoices...  ")
                .unwrap()
                .as_str(),
            "Checking invoices..."
        );
    }

    #[test]
    fn turn_status_path_uses_nul_separated_sha256_digest() {
        let (_dir, home) = test_home();
        let a = turn_status_path(&home, "ab", "c", "d");
        let b = turn_status_path(&home, "a", "bc", "d");
        assert_ne!(a, b);
        assert_eq!(a.parent().unwrap(), home.company_turn_status_dir());
        let name = a.file_name().unwrap().to_str().unwrap();
        assert!(name.ends_with(".json"));
        assert_eq!(name.len(), 64 + ".json".len());
    }

    #[test]
    fn read_turn_status_returns_matching_text_and_ignores_malformed_or_stale_files_without_unlinking()
     {
        let (_dir, home) = test_home();
        let path = turn_status_path(&home, "acme", "slack", "C1");
        instrukt_core::fsx::create_dir_all(path.parent().unwrap()).unwrap();

        // Missing file returns None.
        assert_eq!(
            read_turn_status(&home, "acme", "slack", "C1", "C1", "m1", "cmd-1"),
            None
        );

        // Malformed JSON returns None and leaves file in place.
        instrukt_core::fsx::write(&path, b"{not json").unwrap();
        assert_eq!(
            read_turn_status(&home, "acme", "slack", "C1", "C1", "m1", "cmd-1"),
            None
        );
        assert!(path.exists());

        // Valid matching file returns text; stale input_id returns None without unlinking.
        let file = TurnStatusFile {
            schema_version: TURN_STATUS_SCHEMA_VERSION,
            company: "acme".into(),
            connector: "slack".into(),
            conversation: "C1".into(),
            turn_message: "m1".into(),
            turn_input_id: "cmd-1".into(),
            text: "Looking up order status...".into(),
            updated_at: "2026-11-20T10:00:00Z".into(),
        };
        instrukt_core::fsx::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        assert_eq!(
            read_turn_status(&home, "acme", "slack", "C1", "C1", "m1", "cmd-1").as_deref(),
            Some("Looking up order status...")
        );
        assert_eq!(
            read_turn_status(&home, "acme", "slack", "C1", "C1", "m1", "cmd-2"),
            None
        );
        assert!(path.exists());
    }
}
