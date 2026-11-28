//! Neutral runtime errors, execution-root causes, and dispatch error phases.
use std::fmt;
use std::path::PathBuf;

use instrukt_core::ids::SessionRef;
use instrukt_core::process::ProcessError;

use crate::Capability;

/// Why an execution root's own runtime project cannot be had, in fixed words: never native text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionRootCause {
    Unreachable,
    AdditionRefused,
    MalformedMetadata,
    DuplicateRoots,
    Missing,
}

impl fmt::Display for ExecutionRootCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unreachable => "the runtime is unreachable",
            Self::AdditionRefused => "the runtime refused to add it",
            Self::MalformedMetadata => "the runtime's project metadata is malformed",
            Self::DuplicateRoots => "two live runtime projects share that root",
            Self::Missing => "no runtime project is rooted there",
        })
    }
}

/// Typed runtime-layer errors independent of runner or state-store errors.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("unknown compiled runtime adapter `{0}`")]
    UnknownRuntime(String),
    #[error("thread {thread_id} is no longer available")]
    ThreadNotFound { thread_id: String },
    #[error("command `{command_id}` was previously rejected: {detail}")]
    CommandPreviouslyRejected { command_id: String, detail: String },
    #[error("runtime `{runtime}` lacks the {capability} capability")]
    MissingCapability {
        runtime: String,
        capability: Capability,
    },
    #[error("runtime `{runtime}` {operation} failed: {detail}")]
    Failed {
        runtime: String,
        operation: String,
        detail: String,
    },
    #[error(
        "runtime `{runtime}` {operation} returned an invalid response ({category} at {line}:{column})"
    )]
    InvalidResponse {
        runtime: String,
        operation: String,
        category: &'static str,
        line: usize,
        column: usize,
    },
    #[error("thread {0} is still being placed: the runtime is preparing its requested worktree")]
    PlacementPreparing(SessionRef),
    #[error(
        "native project for {session} under `{key}` is unverified; restore its retained project evidence"
    )]
    ProjectUnverified { key: String, session: SessionRef },
    #[error(
        "no runtime project at {} can be used ({cause}); retried on later passes, or run `instrukt install`",
        checkout.display()
    )]
    ExecutionRootUnavailable {
        checkout: PathBuf,
        cause: ExecutionRootCause,
    },
    #[error(
        "thread {session} runs in a runtime project not rooted at {}{}",
        checkout.display(),
        if *idle { "; it rolls over to the checkout's own project" } else { "; it rolls over once idle or archived" }
    )]
    ThreadMisplaced {
        key: Option<String>,
        session: SessionRef,
        checkout: PathBuf,
        idle: bool,
    },
    #[error("runtime observation failed: {0}")]
    Observation(String),
    #[error("{0}")]
    Refused(String),
    #[error("unsupported runtime operation: {0}")]
    Unsupported(String),
    #[error(transparent)]
    Process(#[from] ProcessError),
}

impl RuntimeError {
    /// Whether this error indicates the target thread/session is absent or gone.
    pub fn is_gone(&self) -> bool {
        matches!(self, Self::ThreadNotFound { .. })
    }
}

/// Phase-qualified failure of one idempotent runtime dispatch.
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error(transparent)]
    BeforeWrite(RuntimeError),
    #[error(transparent)]
    BeforeFrame(RuntimeError),
    #[error(transparent)]
    AfterFrame(RuntimeError),
}

impl DispatchError {
    pub fn into_inner(self) -> RuntimeError {
        match self {
            Self::BeforeWrite(error) | Self::BeforeFrame(error) | Self::AfterFrame(error) => error,
        }
    }
}
