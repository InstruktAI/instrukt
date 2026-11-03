//! Running a configured command: the boundary to the runtime, the
//! notification command and the OS scheduler's registrar.
//!
//! A command line from the runtime mapping runs through `sh -c`. Operands
//! (a message text, a session id, a prompt) are never interpolated into
//! that text: they are appended as positional parameters (`sh -c '<command>
//! "$@"' instrukt <operands…>`), the argument-vector form
//! `session-continuity` requires. Every wait is bounded.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
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

    /// Runs the command in a process group of its own. The whole
    /// invocation — writing its input, reading its output and waiting for
    /// it — finishes within `timeout`; otherwise the process group is
    /// killed (descendants still holding its pipes included) and the
    /// timeout reported.
    pub fn run(&self, timeout: Duration) -> Result<Finished, ProcessError> {
        let started = Instant::now();
        let deadline = started + timeout;
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
            .stderr(Stdio::piped())
            .process_group(0);
        if let Some(cwd) = self.cwd {
            command.current_dir(cwd);
        }
        let mut child = command.spawn().map_err(|source| ProcessError::Spawn {
            program: self.program.clone(),
            source,
        })?;
        let writer = match (self.input, child.stdin.take()) {
            (Some(input), Some(stdin)) => Some(feed(stdin, input.to_vec())),
            _ => None,
        };
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let status = match self.wait(&mut child, deadline) {
            Ok(status) => status,
            Err(err) => {
                self.kill_group(&mut child);
                return Err(err);
            }
        };
        let outcome = (|| {
            if let Some(writer) = writer {
                receive(&writer, deadline)
                    .ok_or_else(|| self.timeout(timeout))?
                    .map_err(|source| self.io(source))?;
            }
            let out = receive(&stdout, deadline).ok_or_else(|| self.timeout(timeout))?;
            let err = receive(&stderr, deadline).ok_or_else(|| self.timeout(timeout))?;
            Ok((out, err))
        })();
        let (out, err) = match outcome {
            Ok(pair) => pair,
            Err(error) => {
                self.kill_group(&mut child);
                return Err(error);
            }
        };
        let finished = Finished {
            success: status.success(),
            stdout: String::from_utf8_lossy(&out).into_owned(),
            stderr: String::from_utf8_lossy(&err).into_owned(),
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
        deadline: Instant,
    ) -> Result<std::process::ExitStatus, ProcessError> {
        let mut pause = Duration::from_millis(2);
        loop {
            if let Some(status) = child.try_wait().map_err(|source| self.io(source))? {
                return Ok(status);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(ProcessError::Timeout {
                    program: self.program.clone(),
                    timeout: deadline.saturating_duration_since(now),
                });
            }
            std::thread::sleep(pause.min(deadline - now));
            pause = (pause * 2).min(Duration::from_millis(50));
        }
    }

    /// Kills the command's process group, then reaps the command.
    fn kill_group(&self, child: &mut Child) {
        let group = format!("-{}", child.id());
        match Command::new("kill")
            .args(["-KILL", "--", &group])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(status) if status.success() => {}
            Ok(status) => {
                tracing::debug!(event = "process_group_kill_refused", program = %self.program, status = %status);
            }
            Err(err) => {
                tracing::warn!(event = "process_group_kill_failed", program = %self.program, error = %err);
            }
        }
        if let Err(err) = child.kill() {
            tracing::debug!(event = "process_kill_failed", program = %self.program, error = %err);
        }
        if let Err(err) = child.wait() {
            tracing::warn!(event = "process_reap_failed", program = %self.program, error = %err);
        }
    }

    fn timeout(&self, timeout: Duration) -> ProcessError {
        ProcessError::Timeout {
            program: self.program.clone(),
            timeout,
        }
    }

    fn io(&self, source: std::io::Error) -> ProcessError {
        ProcessError::Io {
            program: self.program.clone(),
            source,
        }
    }
}

/// Writes `input` to the command on a thread of its own; a command that
/// stops reading (a closed pipe) is not a failure of the write.
fn feed(mut stdin: ChildStdin, input: Vec<u8>) -> Receiver<std::io::Result<()>> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let written = match stdin.write_all(&input) {
            Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
            other => other,
        };
        drop(stdin);
        if send.send(written).is_err() {
            tracing::debug!(event = "process_input_unclaimed");
        }
    });
    receive
}

/// Reads a pipe to its end on a thread of its own.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Receiver<Vec<u8>> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe
            && let Err(err) = pipe.read_to_end(&mut bytes)
        {
            tracing::debug!(event = "process_pipe_read_failed", error = %err);
        }
        if send.send(bytes).is_err() {
            tracing::debug!(event = "process_output_unclaimed");
        }
    });
    receive
}

/// What `receiver` delivers before `deadline`; `None` once it has passed.
fn receive<T>(receiver: &Receiver<T>, deadline: Instant) -> Option<T> {
    receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .ok()
}
