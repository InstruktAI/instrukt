//! Running a configured command: the boundary to the runtime, the
//! notification command and the OS scheduler's registrar.
//!
//! An explicit notification or scheduler command can use `sh -c`. Runtime
//! primitives use direct program arguments and stdin. Operands
//! (a message text, a session id, a prompt) are never interpolated into
//! that text: they are appended as positional parameters (`sh -c '<command>
//! "$@"' instrukt <operands…>`), the argument-vector form
//! `session-continuity` requires. Every wait is bounded.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, OnceLock};
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
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .envs(self.env.iter().map(|(k, v)| (k, v)));
        if let Some(cwd) = self.cwd {
            command.current_dir(cwd);
        }
        let output = self.execute(&mut command, timeout)?;
        Ok(Finished {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn execute(
        &self,
        command: &mut Command,
        timeout: Duration,
    ) -> Result<std::process::Output, ProcessError> {
        let started = Instant::now();
        let deadline = started + timeout;
        command
            .stdin(if self.input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let (mut child, group) = spawn(command).map_err(|source| ProcessError::Spawn {
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
                group.stop().map_err(|source| self.io(source))?;
                let _ = child.wait();
                return Err(err);
            }
        };
        // Descendants end with their command, including ones that closed pipes.
        group.stop().map_err(|source| self.io(source))?;
        let outcome = (|| {
            if let Some(writer) = writer {
                receive(&writer, deadline)
                    .ok_or_else(|| self.timeout(timeout))?
                    .map_err(|source| self.io(source))?;
            }
            let out = receive(&stdout, deadline)
                .ok_or_else(|| self.timeout(timeout))?
                .map_err(|source| self.io(source))?;
            let err = receive(&stderr, deadline)
                .ok_or_else(|| self.timeout(timeout))?
                .map_err(|source| self.io(source))?;
            Ok((out, err))
        })();
        let (out, err) = match outcome {
            Ok(pair) => pair,
            Err(error) => {
                group.stop().map_err(|source| self.io(source))?;
                let _ = child.wait();
                return Err(error);
            }
        };
        tracing::debug!(event="process_finished",program=%self.program,success=status.success(),duration_ms=started.elapsed().as_millis() as u64);
        Ok(std::process::Output {
            status,
            stdout: out,
            stderr: err,
        })
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
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Receiver<std::io::Result<Vec<u8>>> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let read = (|| {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                pipe.read_to_end(&mut bytes)?;
            }
            Ok(bytes)
        })();
        if send.send(read).is_err() {
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
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

// OS signal delivery and child process groups are process-wide resources.
// This registry contains only live owned groups, never domain/configuration state.
static GROUPS: Mutex<Vec<rustix::process::Pid>> = Mutex::new(Vec::new());
static SIGNALS: OnceLock<Result<(), String>> = OnceLock::new();

fn groups() -> std::sync::MutexGuard<'static, Vec<rustix::process::Pid>> {
    match GROUPS.lock() {
        Ok(groups) => groups,
        Err(poisoned) => poisoned.into_inner(),
    }
}
fn install_signals() -> std::io::Result<()> {
    SIGNALS.get_or_init(|| {
        let mut signals=signal_hook::iterator::Signals::new([signal_hook::consts::SIGINT,signal_hook::consts::SIGTERM]).map_err(|e|e.to_string())?;
        std::thread::Builder::new().name("instrukt-child-signals".into()).spawn(move|| {
            for signal in signals.forever() {
                for pid in groups().iter().copied() {
                    if let Err(error)=kill_group(pid){tracing::error!(event="interrupted_group_cleanup_failed",pid=pid.as_raw_nonzero().get(),error=%error);}
                }
                // Preserve the signal's normal exit semantics after cleanup,
                // including when it arrives between child commands.
                if let Err(error)=signal_hook::low_level::emulate_default_handler(signal) {
                    tracing::error!(event="signal_exit_failed",error=%error);
                    std::process::exit(1);
                }
            }
        }).map_err(|e|e.to_string())?;
        Ok(())
    }).as_ref().map(|_|()).map_err(|e|std::io::Error::other(e.clone()))
}
fn kill_group(pid: rustix::process::Pid) -> std::io::Result<()> {
    match rustix::process::kill_process_group(pid, rustix::process::Signal::KILL) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Ownership of one command's process group, retained until its descendants stop.
pub struct OwnedProcessGroup {
    pid: rustix::process::Pid,
    stopped: std::cell::Cell<bool>,
}
impl OwnedProcessGroup {
    pub fn stop(&self) -> std::io::Result<()> {
        if self.stopped.get() {
            return Ok(());
        }
        let mut registry = groups();
        kill_group(self.pid)?;
        registry.retain(|pid| *pid != self.pid);
        self.stopped.set(true);
        Ok(())
    }
}
impl Drop for OwnedProcessGroup {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            tracing::error!(event="process_group_cleanup_failed",error=%error);
        }
    }
}

/// Start and register a foreground child atomically with respect to signal cleanup.
pub fn spawn(command: &mut Command) -> std::io::Result<(Child, OwnedProcessGroup)> {
    install_signals()?;
    let mut registry = groups();
    command.process_group(0);
    let child = command.spawn()?;
    let pid = rustix::process::Pid::from_raw(child.id() as i32)
        .ok_or_else(|| std::io::Error::other("invalid child pid"))?;
    registry.push(pid);
    Ok((
        child,
        OwnedProcessGroup {
            pid,
            stopped: std::cell::Cell::new(false),
        },
    ))
}

/// Execute an existing native command without losing byte output or its
/// configured environment; retain ownership of its process group to completion.
pub fn output(
    command: &mut Command,
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<std::process::Output, ProcessError> {
    let name = command.get_program().to_string_lossy().into_owned();
    let mut invocation = Invocation::program(&name, &[]);
    invocation.input = input;
    invocation.execute(command, timeout)
}
