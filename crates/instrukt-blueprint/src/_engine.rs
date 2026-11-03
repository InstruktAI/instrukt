//! The engine adapter (`docs/local/spec/feature/blueprint/graph-engine.md`):
//! runs the selected engine against a checkout of one revision and
//! translates its output into the graph contract, and forwards the
//! questions only an engine can answer.
//!
//! Two engines exist:
//!
//! - **The shipped adapter** (`blueprint.engine` unset or `graphify`):
//!   Graphify's code-only `graphify update <checkout> --force`, whose
//!   `graphify-out/graph.json` is node-link JSON in the contract's shape.
//!   It offers `query` and `explain`, forwarded as
//!   `graphify query|explain <args…> --graph <graph>`.
//! - **A configured command**: run through `sh -c` with the checkout's path
//!   as its one argument; it writes contract JSON to stdout. It offers no
//!   question beyond the contract.
//!
//! An engine that cannot be found is reported with what to install;
//! instrukt installs nothing.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use instrukt_core::config::SHIPPED_BLUEPRINT_ENGINES;

use crate::_contract::{ContractError, Graph};

/// The `blueprint.engine` value naming the shipped adapter; configuration
/// owns the list of shipped adapter names a project layer may give.
pub const SHIPPED_ENGINE: &str = SHIPPED_BLUEPRINT_ENGINES[0];

/// The shipped adapter's program.
const GRAPHIFY: &str = "graphify";
/// What to install when the shipped adapter's program is missing.
const GRAPHIFY_INSTALL: &str = "install Graphify (the `graphifyy` package), e.g. `uv tool install graphifyy` or `pipx install graphifyy`";
/// Where Graphify writes its graph, relative to the mapped tree.
const GRAPHIFY_OUT: &str = "graphify-out";
const GRAPHIFY_GRAPH: &str = "graph.json";

/// How often a bounded wait looks whether the engine ended.
const POLL: Duration = Duration::from_millis(50);
/// The longest engine message an error keeps.
const MESSAGE_LIMIT: usize = 2000;

/// A question only an engine can answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Question {
    Query,
    Explain,
}

impl Question {
    pub fn name(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Explain => "explain",
        }
    }
}

/// The engine `blueprint.engine` selects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Engine {
    /// The shipped adapter (Graphify).
    Shipped,
    /// A project's own engine: a shell command line.
    Command(String),
}

/// The engine could not produce or answer.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("the graph engine `{program}` is not installed; {install}")]
    NotInstalled {
        program: &'static str,
        install: &'static str,
    },
    #[error("cannot run the graph engine `{program}`: {source}")]
    Spawn { program: String, source: io::Error },
    #[error("the graph engine {status}{}", message.as_deref().map(|m| format!(": {m}")).unwrap_or_default())]
    Failed {
        status: String,
        message: Option<String>,
    },
    #[error("the graph engine did not finish within {}s and was ended", .0.as_secs())]
    TimedOut(Duration),
    #[error("the graph engine produced no graph at {0}")]
    NoOutput(PathBuf),
    #[error("the graph engine's output at {0} is a symbolic link; it is not read")]
    LinkedOutput(PathBuf),
    #[error("the graph engine's output was refused: {0}")]
    Refused(#[from] ContractError),
    #[error("{what} {path}: {source}")]
    Io {
        what: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl Engine {
    /// The engine an effective `blueprint.engine` value selects; unset is
    /// the shipped adapter. A command reaches here only from the global
    /// layer: configuration drops a project layer's command.
    pub fn from_setting(setting: Option<&str>) -> Self {
        match setting.map(str::trim) {
            None | Some("") | Some(SHIPPED_ENGINE) => Self::Shipped,
            Some(command) => Self::Command(command.to_owned()),
        }
    }

    /// Whether the engine offers `question`.
    pub fn offers(&self, question: Question) -> bool {
        match (self, question) {
            (Self::Shipped, Question::Query | Question::Explain) => true,
            (Self::Command(_), _) => false,
        }
    }

    /// Maps the tree checked out at `tree` and returns its graph, validated
    /// against the contract. `scratch` is a private directory for the
    /// engine's captured output; `timeout` bounds the run.
    pub fn generate(
        &self,
        tree: &Path,
        scratch: &Path,
        timeout: Duration,
    ) -> Result<Graph, EngineError> {
        match self {
            Self::Shipped => {
                // A committed `graphify-out` (or a link of that name) is not
                // the engine's output and must not redirect its writes.
                let out = tree.join(GRAPHIFY_OUT);
                remove_entry(&out)?;
                let mut command = Command::new(GRAPHIFY);
                command
                    .arg("update")
                    .arg(tree)
                    .arg("--force")
                    .current_dir(tree);
                run(GRAPHIFY, command, scratch, timeout)?;
                let graph = out.join(GRAPHIFY_GRAPH);
                for path in [&out, &graph] {
                    match std::fs::symlink_metadata(path) {
                        Ok(meta) if meta.file_type().is_symlink() => {
                            return Err(EngineError::LinkedOutput(path.clone()));
                        }
                        Ok(_) => {}
                        Err(err) if err.kind() == io::ErrorKind::NotFound => {
                            return Err(EngineError::NoOutput(path.clone()));
                        }
                        Err(source) => {
                            return Err(EngineError::Io {
                                what: "cannot inspect",
                                path: path.clone(),
                                source,
                            });
                        }
                    }
                }
                let bytes = std::fs::read(&graph).map_err(|source| EngineError::Io {
                    what: "cannot read",
                    path: graph.clone(),
                    source,
                })?;
                Ok(Graph::from_json(&bytes)?)
            }
            Self::Command(line) => {
                let stdout = run(line, shell(line, tree), scratch, timeout)?;
                let bytes = std::fs::read(&stdout).map_err(|source| EngineError::Io {
                    what: "cannot read",
                    path: stdout.clone(),
                    source,
                })?;
                Ok(Graph::from_json(&bytes)?)
            }
        }
    }

    /// Forwards `question` with `args` unchanged to the engine, over the
    /// graph stored at `graph`; returns what the engine printed. `None`
    /// when the engine does not offer the question.
    pub fn ask(
        &self,
        question: Question,
        args: &[String],
        graph: &Path,
        scratch: &Path,
        timeout: Duration,
    ) -> Result<Option<String>, EngineError> {
        if !self.offers(question) {
            return Ok(None);
        }
        let mut command = Command::new(GRAPHIFY);
        command
            .arg(question.name())
            .args(args)
            .arg("--graph")
            .arg(graph)
            .current_dir(scratch);
        let stdout = run(GRAPHIFY, command, scratch, timeout)?;
        let bytes = std::fs::read(&stdout).map_err(|source| EngineError::Io {
            what: "cannot read",
            path: stdout.clone(),
            source,
        })?;
        Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
    }
}

/// `sh -c '<line> "$1"' instrukt-engine <tree>`, in the tree.
fn shell(line: &str, tree: &Path) -> Command {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(format!("{line} \"$1\""))
        .arg("instrukt-engine")
        .arg(tree)
        .current_dir(tree);
    command
}

/// Runs `command` with no input and its output captured in `scratch`,
/// waiting at most `timeout`; returns the captured stdout's path.
fn run(
    program: &str,
    mut command: Command,
    scratch: &Path,
    timeout: Duration,
) -> Result<PathBuf, EngineError> {
    let stdout_path = scratch.join("engine.stdout");
    let stderr_path = scratch.join("engine.stderr");
    let create = |path: &Path| {
        File::create(path).map_err(|source| EngineError::Io {
            what: "cannot create",
            path: path.to_path_buf(),
            source,
        })
    };
    command
        .stdin(Stdio::null())
        .stdout(create(&stdout_path)?)
        .stderr(create(&stderr_path)?);
    let mut child = command.spawn().map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound && program == GRAPHIFY {
            EngineError::NotInstalled {
                program: GRAPHIFY,
                install: GRAPHIFY_INSTALL,
            }
        } else {
            EngineError::Spawn {
                program: program.to_owned(),
                source,
            }
        }
    })?;
    let status = wait(&mut child, timeout)?;
    if !status.success() {
        let stderr = std::fs::read(&stderr_path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        return Err(EngineError::Failed {
            status: describe(status),
            message: last_line(&stderr),
        });
    }
    Ok(stdout_path)
}

fn wait(child: &mut Child, timeout: Duration) -> Result<ExitStatus, EngineError> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if started.elapsed() >= timeout => {
                if let Err(err) = child.kill() {
                    tracing::warn!(event = "blueprint_engine_kill_failed", error = %err);
                }
                if let Err(err) = child.wait() {
                    tracing::warn!(event = "blueprint_engine_reap_failed", error = %err);
                }
                return Err(EngineError::TimedOut(timeout));
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(source) => {
                return Err(EngineError::Io {
                    what: "cannot wait for the engine started in",
                    path: PathBuf::new(),
                    source,
                });
            }
        }
    }
}

fn describe(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exited with status {code}"),
        None => "was ended by a signal".to_owned(),
    }
}

/// The last non-empty line of `text`, within [`MESSAGE_LIMIT`].
fn last_line(text: &str) -> Option<String> {
    let line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())?
        .trim();
    Some(line.chars().take(MESSAGE_LIMIT).collect())
}

/// Removes whatever is at `path` — a link itself, never what it points to.
fn remove_entry(path: &Path) -> Result<(), EngineError> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(EngineError::Io {
                what: "cannot inspect",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let removed = if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    removed.map_err(|source| EngineError::Io {
        what: "cannot remove",
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_graphify_selects_the_shipped_adapter_anything_else_a_command() {
        assert_eq!(Engine::from_setting(None), Engine::Shipped);
        assert_eq!(Engine::from_setting(Some(SHIPPED_ENGINE)), Engine::Shipped);
        assert_eq!(
            Engine::from_setting(Some("./map --json")),
            Engine::Command("./map --json".into())
        );
    }

    #[test]
    fn only_the_shipped_adapter_offers_query_and_explain() {
        for question in [Question::Query, Question::Explain] {
            assert!(Engine::Shipped.offers(question));
            assert!(!Engine::Command("map".into()).offers(question));
        }
    }

    #[test]
    fn a_command_engine_receives_the_checkout_and_its_stdout_is_the_graph() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("tree");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(
            tree.join("graph.json"),
            br#"{"nodes": [{"id": "a", "source_file": "a.rs"}], "links": []}"#,
        )
        .unwrap();
        let graph = Engine::Command("cat \"$1/graph.json\" #".into())
            .generate(&tree, dir.path(), Duration::from_secs(30))
            .unwrap();
        assert_eq!(graph.nodes()[0].id, "a");
    }

    #[test]
    fn a_failing_or_contract_breaking_command_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let failed = Engine::Command("exit 3 #".into()).generate(
            dir.path(),
            dir.path(),
            Duration::from_secs(30),
        );
        assert!(
            matches!(failed, Err(EngineError::Failed { .. })),
            "{failed:?}"
        );
        let refused = Engine::Command("echo '[]' #".into()).generate(
            dir.path(),
            dir.path(),
            Duration::from_secs(30),
        );
        assert!(
            matches!(refused, Err(EngineError::Refused(ContractError::NotObject))),
            "{refused:?}"
        );
    }
}
