//! Running a configured command: the boundary to the runtime, the
//! notification command and the OS scheduler's registrar.
//!
//! A command line from the runtime mapping runs through `sh -c`. Operands
//! (a message text, a session id, a prompt) are never interpolated into
//! that text: they are appended as positional parameters (`sh -c '<command>
//! "$@"' instrukt <operands…>`), the argument-vector form
//! `session-continuity` requires. Every wait is bounded.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How a command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// A command could not be run to its end.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("cannot start `{program}`: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },
    #[error("`{program}` did not finish within {timeout:?} and was stopped")]
    Timeout { program: String, timeout: Duration },
    #[error("cannot talk to `{program}`: {source}")]
    Io {
        program: String,
        source: std::io::Error,
    },
}

/// One command to run.
#[derive(Debug, Clone)]
pub struct Invocation<'a> {
    /// The program, or `sh` for a shell command line.
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    input: Option<&'a [u8]>,
    cwd: Option<&'a Path>,
}

impl<'a> Invocation<'a> {
    /// The shell command line `command`, with `operands` passed as its
    /// positional parameters.
    pub fn shell(command: &str, operands: &[&str]) -> Self {
        let mut args = vec![
            "-c".to_owned(),
            format!("{command} \"$@\""),
            "instrukt".to_owned(),
        ];
        args.extend(operands.iter().map(|op| (*op).to_owned()));
        Self {
            program: "sh".to_owned(),
            args,
            env: Vec::new(),
            input: None,
            cwd: None,
        }
    }

    /// `program` with `args`, found on `PATH`.
    pub fn program(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            env: Vec::new(),
            input: None,
            cwd: None,
        }
    }

    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_owned(), value.to_owned()));
        self
    }

    pub fn input(mut self, input: &'a [u8]) -> Self {
        self.input = Some(input);
        self
    }

    pub fn cwd(mut self, cwd: &'a Path) -> Self {
        self.cwd = Some(cwd);
        self
    }

    /// Runs the command, waiting at most `timeout`; a command still running
    /// then is stopped and reported.
    pub fn run(&self, timeout: Duration) -> Result<Finished, ProcessError> {
        let started = Instant::now();
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .stdin(if self.input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = self.cwd {
            command.current_dir(cwd);
        }
        let mut child = command.spawn().map_err(|source| ProcessError::Spawn {
            program: self.program.clone(),
            source,
        })?;
        if let (Some(input), Some(mut stdin)) = (self.input, child.stdin.take()) {
            stdin.write_all(input).map_err(|source| self.io(source))?;
        }
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let status = self.wait(&mut child, timeout)?;
        let finished = Finished {
            success: status.success(),
            stdout: collect(stdout),
            stderr: collect(stderr),
        };
        tracing::debug!(
            event = "process_finished",
            program = %self.program,
            success = finished.success,
            duration_ms = started.elapsed().as_millis() as u64
        );
        Ok(finished)
    }

    fn wait(
        &self,
        child: &mut Child,
        timeout: Duration,
    ) -> Result<std::process::ExitStatus, ProcessError> {
        let started = Instant::now();
        let mut pause = Duration::from_millis(2);
        loop {
            if let Some(status) = child.try_wait().map_err(|source| self.io(source))? {
                return Ok(status);
            }
            if started.elapsed() >= timeout {
                if let Err(err) = child.kill() {
                    tracing::warn!(event = "process_kill_failed", program = %self.program, error = %err);
                }
                if let Err(err) = child.wait() {
                    tracing::warn!(event = "process_reap_failed", program = %self.program, error = %err);
                }
                return Err(ProcessError::Timeout {
                    program: self.program.clone(),
                    timeout,
                });
            }
            std::thread::sleep(pause.min(timeout.saturating_sub(started.elapsed())));
            pause = (pause * 2).min(Duration::from_millis(50));
        }
    }

    fn io(&self, source: std::io::Error) -> ProcessError {
        ProcessError::Io {
            program: self.program.clone(),
            source,
        }
    }
}

type Drain = Option<std::thread::JoinHandle<Vec<u8>>>;

/// Reads a pipe to its end on its own thread, so a chatty child cannot
/// block on a full pipe while it is waited for.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Drain {
    pipe.map(|mut pipe| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Err(err) = pipe.read_to_end(&mut bytes) {
                tracing::debug!(event = "process_pipe_read_failed", error = %err);
            }
            bytes
        })
    })
}

fn collect(drain: Drain) -> String {
    let bytes = drain
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operands_reach_the_command_as_arguments_never_as_shell_text() {
        let hostile = "a b; echo pwned $(echo x) \"q\"";
        let done = Invocation::shell("printf '%s|'", &[hostile, "two"])
            .run(Duration::from_secs(10))
            .unwrap();
        assert!(done.success);
        let parts: Vec<&str> = done.stdout.split('|').collect();
        assert_eq!(parts, [hostile, "two", ""]);
    }

    #[test]
    fn a_command_running_past_its_bound_is_stopped() {
        let err = Invocation::shell("sleep 5", &[])
            .run(Duration::from_millis(100))
            .unwrap_err();
        assert!(matches!(err, ProcessError::Timeout { .. }));
    }
}
