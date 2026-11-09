//! The release-owned Graphify tool. Inspection executes only its informational
//! version command, with Python bytecode writes disabled; uv runs only on repair.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use instrukt_core::home::Home;
use instrukt_core::process::{self, ProcessError};

/// The sole release pin, also read by Renovate and the real-tool CI check.
pub fn graphify_version() -> &'static str {
    include_str!("../graphify.version").trim()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graphify {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphifyState {
    Missing,
    Ready,
    Drifted { detail: String },
}

#[derive(Debug, thiserror::Error)]
pub enum GraphifyError {
    #[error("cannot resolve the instrukt tool directory: {0}")]
    Location(#[source] std::io::Error),
    #[error("uv is required to install Graphify; install uv and rerun `instrukt install`: {0}")]
    Uv(#[source] ProcessError),
    #[error(
        "uv could not install Graphify; resolve the package prerequisite and rerun `instrukt install`: {0}"
    )]
    Install(String),
    #[error(
        "Graphify did not converge to {expected}: {0:?}; rerun `instrukt install`",
        expected = graphify_version()
    )]
    Verification(GraphifyState),
}

impl Graphify {
    pub fn new(home: &Home) -> Result<Self, GraphifyError> {
        Ok(Self {
            root: std::path::absolute(home.root().join("tools"))
                .map_err(GraphifyError::Location)?,
        })
    }

    /// The adapter's absolute program path; never resolved through PATH.
    pub fn binary(&self) -> PathBuf {
        self.root.join("bin/graphify")
    }

    pub fn inspect(&self) -> GraphifyState {
        let mut command = Command::new(self.binary());
        command.arg("--version").env("PYTHONDONTWRITEBYTECODE", "1");
        match process::output(&mut command, None, Duration::from_secs(15)) {
            Ok(output) if output.status.success() => {
                let actual = String::from_utf8_lossy(&output.stdout);
                if actual.trim() == format!("graphify {}", graphify_version()) {
                    GraphifyState::Ready
                } else {
                    GraphifyState::Drifted {
                        detail: format!(
                            "expected graphify {}, received {}",
                            graphify_version(),
                            actual.trim()
                        ),
                    }
                }
            }
            Err(ProcessError::Spawn { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                GraphifyState::Missing
            }
            Ok(output) => GraphifyState::Drifted {
                detail: format!(
                    "version command failed ({}): {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            },
            Err(error) => GraphifyState::Drifted {
                detail: error.to_string(),
            },
        }
    }

    /// Install or replace the exact release version. The caller inspects first
    /// so a matching environment is reused without contacting a package index.
    pub fn install(&self) -> Result<(), GraphifyError> {
        let mut command = Command::new("uv");
        command
            .args(["tool", "install"])
            .arg(format!("graphifyy=={}", graphify_version()))
            .arg("--reinstall")
            .env("UV_TOOL_DIR", self.root.join("uv"))
            .env("UV_TOOL_BIN_DIR", self.root.join("bin"));
        let output = process::output(&mut command, None, Duration::from_secs(600))
            .map_err(GraphifyError::Uv)?;
        if !output.status.success() {
            return Err(GraphifyError::Install(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        match self.inspect() {
            GraphifyState::Ready => Ok(()),
            other => Err(GraphifyError::Verification(other)),
        }
    }
}
