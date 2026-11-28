//! Safe native failure evidence: typed causes and opaque references, never native payloads.
use std::fmt;
use std::path::PathBuf;

use instrukt_core::ids::SessionRef;
use instrukt_core::process::ProcessError;
use sha2::{Digest, Sha256};

use crate::{ExecutionRootCause, RuntimeError};

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
    ExecutionRootUnavailable {
        checkout: PathBuf,
        cause: ExecutionRootCause,
    },
    ThreadMisplaced {
        session: SessionRef,
        checkout: PathBuf,
        idle: bool,
    },
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
            } => match os_code {
                Some(code) => write!(f, "native {phase} failed ({kind}, OS code {code})"),
                None => write!(f, "native {phase} failed ({kind})"),
            },
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
            Self::ExecutionRootUnavailable { checkout, cause } => write!(
                f,
                "no runtime project at {} can be used ({cause}); retried on later passes, or run `instrukt install`",
                checkout.display()
            ),
            Self::ThreadMisplaced {
                session,
                checkout,
                idle,
            } => write!(
                f,
                "thread {session} runs in a runtime project not rooted at {}{}",
                checkout.display(),
                if *idle {
                    "; it rolls over to the checkout's own project"
                } else {
                    "; it rolls over once idle or archived"
                }
            ),
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

pub fn reference(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn diagnose(
    runtime: &str,
    operation: &str,
    error: &RuntimeError,
    native: Option<NativeCause>,
) -> NativeDiagnostic {
    let cause = native.unwrap_or_else(|| match error {
        RuntimeError::ThreadNotFound { .. } => NativeCause::TargetGone,
        RuntimeError::Process(ProcessError::Timeout { timeout, .. }) => NativeCause::Timeout {
            seconds: timeout.as_secs(),
        },
        RuntimeError::Process(ProcessError::Spawn { source, .. }) => NativeCause::Io {
            phase: "process start",
            kind: source.kind(),
            os_code: source.raw_os_error(),
        },
        RuntimeError::Process(ProcessError::Io { source, .. }) => NativeCause::Io {
            phase: "process I/O",
            kind: source.kind(),
            os_code: source.raw_os_error(),
        },
        RuntimeError::InvalidResponse {
            category,
            line,
            column,
            ..
        } => NativeCause::InvalidResponse {
            category,
            line: *line,
            column: *column,
        },
        RuntimeError::MissingCapability { .. } | RuntimeError::Unsupported(_) => {
            NativeCause::Unsupported
        }
        RuntimeError::ExecutionRootUnavailable { checkout, cause } => {
            NativeCause::ExecutionRootUnavailable {
                checkout: checkout.clone(),
                cause: *cause,
            }
        }
        RuntimeError::ThreadMisplaced {
            session,
            checkout,
            idle,
            ..
        } => NativeCause::ThreadMisplaced {
            session: session.clone(),
            checkout: checkout.clone(),
            idle: *idle,
        },
        _ => NativeCause::Unrecognized,
    });
    let operation = match error {
        RuntimeError::Failed { operation, .. }
        | RuntimeError::InvalidResponse { operation, .. } => operation.as_str(),
        _ => operation,
    };
    NativeDiagnostic {
        runtime: runtime.into(),
        operation: operation.into(),
        cause,
        reference: reference(error.to_string().as_bytes()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionRef {
        SessionRef::parse("t3:thread-1").unwrap()
    }

    #[test]
    fn an_unavailable_execution_root_keeps_its_checkout_and_cause() {
        let error = RuntimeError::ExecutionRootUnavailable {
            checkout: PathBuf::from("/home/.instrukt/companies/acme"),
            cause: ExecutionRootCause::AdditionRefused,
        };
        let diagnostic = diagnose("t3", "conversation delivery", &error, None);
        assert_eq!(
            diagnostic.cause,
            NativeCause::ExecutionRootUnavailable {
                checkout: PathBuf::from("/home/.instrukt/companies/acme"),
                cause: ExecutionRootCause::AdditionRefused,
            }
        );
    }

    #[test]
    fn a_misplaced_thread_keeps_its_thread_checkout_and_idleness() {
        let error = RuntimeError::ThreadMisplaced {
            key: Some("instrukt:conversation:acme".into()),
            session: session(),
            checkout: PathBuf::from("/home/.instrukt/companies/acme"),
            idle: false,
        };
        let diagnostic = diagnose("t3", "conversation delivery", &error, None);
        assert_eq!(
            diagnostic.cause,
            NativeCause::ThreadMisplaced {
                session: session(),
                checkout: PathBuf::from("/home/.instrukt/companies/acme"),
                idle: false,
            }
        );
    }

    #[test]
    fn an_opaque_native_failure_stays_unrecognized() {
        let error = RuntimeError::Failed {
            runtime: "t3".into(),
            operation: "send".into(),
            detail: "Authorization: Bearer secret".into(),
        };
        let diagnostic = diagnose("t3", "conversation delivery", &error, None);
        assert_eq!(diagnostic.cause, NativeCause::Unrecognized);
    }
}
