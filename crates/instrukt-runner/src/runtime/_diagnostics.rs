//! Safe native failure evidence: typed causes and opaque references, never native payloads.
use crate::{process::ProcessError, sessions::SessionsError};
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeCause {
    TargetGone,
    Timeout {
        seconds: u64,
    },
    Io {
        phase: &'static str,
        kind: std::io::ErrorKind,
        os_code: Option<i32>,
    },
    InvalidResponse {
        category: &'static str,
        line: usize,
        column: usize,
    },
    ServerNotRunning,
    ServerUnreachable {
        code: Option<String>,
    },
    Transport {
        code: String,
    },
    IncompatibleResponse,
    Unsupported,
    InvalidConfiguration,
    Unrecognized,
}
impl fmt::Display for NativeCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetGone => f.write_str("native target is no longer available"),
            Self::Timeout { seconds } => write!(f, "native command timed out after {seconds}s"),
            Self::Io {
                phase,
                kind,
                os_code,
            } => write!(f, "native {phase} failed ({kind:?}, OS code {os_code:?})"),
            Self::InvalidResponse {
                category,
                line,
                column,
            } => write!(
                f,
                "native response is invalid ({category} at {line}:{column})"
            ),
            Self::ServerNotRunning => f.write_str("native server is not running"),
            Self::ServerUnreachable { code: Some(code) } => {
                write!(f, "native server unreachable ({code})")
            }
            Self::ServerUnreachable { code: None } => f.write_str("native server unreachable"),
            Self::Transport { code } => write!(f, "native transport failure ({code})"),
            Self::IncompatibleResponse => {
                f.write_str("native response schema is incompatible; update the native CLI")
            }
            Self::Unsupported => f.write_str("native capability is unavailable"),
            Self::InvalidConfiguration => f.write_str("native configuration is invalid"),
            Self::Unrecognized => {
                f.write_str("native operation rejected (unrecognized diagnostic)")
            }
        }
    }
}
#[derive(Debug, Clone)]
pub struct NativeDiagnostic {
    pub runtime: String,
    pub operation: String,
    pub cause: NativeCause,
    pub reference: String,
}
impl fmt::Display for NativeDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}: {} [diagnostic:{}]",
            self.runtime, self.operation, self.cause, self.reference
        )
    }
}
pub(super) fn reference(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
pub(super) fn diagnose(
    runtime: &str,
    operation: &str,
    error: &SessionsError,
    native: Option<NativeCause>,
) -> NativeDiagnostic {
    let cause = native.unwrap_or_else(|| match error {
        SessionsError::Gone(_) => NativeCause::TargetGone,
        SessionsError::Process(ProcessError::Timeout { timeout, .. }) => NativeCause::Timeout {
            seconds: timeout.as_secs(),
        },
        SessionsError::Process(ProcessError::Spawn { source, .. }) => NativeCause::Io {
            phase: "process start",
            kind: source.kind(),
            os_code: source.raw_os_error(),
        },
        SessionsError::Process(ProcessError::Io { source, .. }) => NativeCause::Io {
            phase: "process I/O",
            kind: source.kind(),
            os_code: source.raw_os_error(),
        },
        SessionsError::InvalidResponse {
            category,
            line,
            column,
            ..
        } => NativeCause::InvalidResponse {
            category,
            line: *line,
            column: *column,
        },
        SessionsError::MissingCapability { .. } => NativeCause::Unsupported,
        SessionsError::Config(_) => NativeCause::InvalidConfiguration,
        _ => NativeCause::Unrecognized,
    });
    let operation = match error {
        SessionsError::Failed { operation, .. }
        | SessionsError::InvalidResponse { operation, .. } => operation.as_str(),
        _ => operation,
    };
    NativeDiagnostic {
        runtime: runtime.into(),
        operation: operation.into(),
        cause,
        reference: reference(error.to_string().as_bytes()),
    }
}
