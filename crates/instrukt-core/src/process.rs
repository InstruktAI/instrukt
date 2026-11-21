//! Running a configured command: the boundary to the runtime, the
//! notification command and the OS scheduler's registrar.
//!
//! An explicit notification or scheduler command can use `sh -c`. Runtime
//! primitives use direct program arguments and stdin. Operands
//! (a message text, a session id, a prompt) are never interpolated into
//! that text: they are appended as positional parameters (`sh -c '<command>
//! "$@"' instrukt <operands…>`), the argument-vector form
//! `session-continuity` requires. Every wait is bounded, except for a command
//! that runs another party's code for as long as that code takes — git
//! running the repository's hooks — which is waited for to its end, as the
//! plain command is ([`output_to_end`]). Limits use monotonic real time.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How a command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// `duration` in whole seconds, a started second counted.
pub fn whole_seconds(duration: Duration) -> u64 {
    duration.as_secs() + u64::from(duration.subsec_nanos() > 0)
}

/// A command could not be run to its end.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("cannot start `{program}`: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },
    #[error("`{program}` did not finish within {} s and was stopped", whole_seconds(*timeout))]
    Timeout { program: String, timeout: Duration },
    #[error("cannot talk to `{program}`: {source}")]
    Io {
        program: String,
        source: std::io::Error,
    },
}

/// The longest pause between two looks at a running command.
const LONGEST_PAUSE: Duration = Duration::from_millis(50);

/// A program's limit, measured from its start in monotonic real time.
#[derive(Clone, Copy)]
pub struct Limit {
    limit: Duration,
    deadline: Instant,
}

/// What a look at a running program finds against its limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look<T> {
    /// The program ended.
    Ended(T),
    Running,
    /// The limit passed.
    Passed,
}

impl Limit {
    /// `limit`, measured from now.
    pub fn start(limit: Duration) -> Self {
        Self::from(limit, Instant::now())
    }

    /// `limit`, measured from `started`.
    fn from(limit: Duration, started: Instant) -> Self {
        Self {
            limit,
            deadline: started + limit,
        }
    }

    /// The limit the program was given.
    pub fn limit(&self) -> Duration {
        self.limit
    }

    /// Judges the latest look at the program, giving completion precedence.
    pub fn judge<T>(&self, ended: Option<T>) -> Look<T> {
        match ended {
            Some(ended) => Look::Ended(ended),
            None if Instant::now() >= self.deadline => Look::Passed,
            None => Look::Running,
        }
    }
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
        let limit = Limit::from(timeout, started);
        self.finish(limit, started)
    }

    fn finish(&self, limit: Limit, started: Instant) -> Result<Finished, ProcessError> {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .envs(self.env.iter().map(|(k, v)| (k, v)));
        if let Some(cwd) = self.cwd {
            command.current_dir(cwd);
        }
        let output = self.execute(&mut command, Some(limit), started)?;
        Ok(Finished {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    /// Runs `command` to its end, or until `timeout` when there is one.
    fn execute(
        &self,
        command: &mut Command,
        limit: Option<Limit>,
        started: Instant,
    ) -> Result<std::process::Output, ProcessError> {
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
        self.finish_child(&mut child, &group, limit, started)
    }

    fn finish_child(
        &self,
        child: &mut Child,
        group: &OwnedProcessGroup,
        limit: Option<Limit>,
        started: Instant,
    ) -> Result<std::process::Output, ProcessError> {
        // A helper thread the system cannot start is a handled failure:
        // the command's group is stopped and its direct child reaped.
        let helpers = (|| {
            let writer = match (self.input, child.stdin.take()) {
                (Some(input), Some(stdin)) => Some(feed(stdin, input.to_vec())?),
                _ => None,
            };
            let stdout = drain(child.stdout.take())?;
            let stderr = drain(child.stderr.take())?;
            Ok((writer, stdout, stderr))
        })();
        let (writer, stdout, stderr) = match helpers {
            Ok(helpers) => helpers,
            Err(source) => {
                group.stop().map_err(|source| self.io(source))?;
                let _ = group.wait(child);
                return Err(self.io(source));
            }
        };
        let status = match self.wait(group, child, limit) {
            Ok(status) => status,
            Err(err) => {
                group.stop().map_err(|source| self.io(source))?;
                let _ = group.wait(child);
                return Err(err);
            }
        };
        // Descendants end with their command, including ones that closed pipes.
        group.stop().map_err(|source| self.io(source))?;
        let outcome = (|| {
            if let Some(writer) = writer {
                self.receive(&writer, limit)?;
            }
            let out = self.receive(&stdout, limit)?;
            let err = self.receive(&stderr, limit)?;
            Ok((out, err))
        })();
        let (out, err) = match outcome {
            Ok(pair) => pair,
            Err(error) => {
                group.stop().map_err(|source| self.io(source))?;
                let _ = group.wait(child);
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

    /// Waits for `child` to end, or until its limit passes; the timeout
    /// reported is the limit the command was given.
    fn wait(
        &self,
        group: &OwnedProcessGroup,
        child: &mut Child,
        limit: Option<Limit>,
    ) -> Result<std::process::ExitStatus, ProcessError> {
        let Some(limit) = limit else {
            return group.wait(child).map_err(|source| self.io(source));
        };
        let mut pause = Duration::from_millis(2);
        loop {
            if let Some(status) = group.try_wait(child).map_err(|source| self.io(source))? {
                return Ok(status);
            }
            let now = Instant::now();
            if now >= limit.deadline {
                return Err(self.timeout(limit.limit));
            }
            std::thread::sleep(pause.min(limit.deadline - now));
            pause = (pause * 2).min(LONGEST_PAUSE);
        }
    }

    /// What a reader or writer of the command delivers within its limit.
    fn receive<T>(
        &self,
        receiver: &Receiver<std::io::Result<T>>,
        limit: Option<Limit>,
    ) -> Result<T, ProcessError> {
        let delivered = match limit {
            None => receiver.recv().map_err(|_| self.ended()),
            Some(Limit { limit, deadline }) => receiver
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| self.timeout(limit)),
        };
        delivered?.map_err(|source| self.io(source))
    }

    /// A reader or writer of the command that ended without its result.
    fn ended(&self) -> ProcessError {
        self.io(std::io::Error::other(
            "its input or output ended without a result",
        ))
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
fn feed(mut stdin: ChildStdin, input: Vec<u8>) -> std::io::Result<Receiver<std::io::Result<()>>> {
    let (send, receive) = mpsc::channel();
    std::thread::Builder::new()
        .name("instrukt-command-input".into())
        .spawn(move || {
            let written = match stdin.write_all(&input) {
                Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
                other => other,
            };
            drop(stdin);
            if send.send(written).is_err() {
                tracing::debug!(event = "process_input_unclaimed");
            }
        })?;
    Ok(receive)
}

/// Reads a pipe to its end on a thread of its own.
fn drain<R: Read + Send + 'static>(
    pipe: Option<R>,
) -> std::io::Result<Receiver<std::io::Result<Vec<u8>>>> {
    let (send, receive) = mpsc::channel();
    std::thread::Builder::new()
        .name("instrukt-command-output".into())
        .spawn(move || {
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
        })?;
    Ok(receive)
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

// OS signal delivery and child process groups are process-wide resources.
// A group stays registered until cleanup, including after its leader is reaped.
static GROUPS: Mutex<Vec<Arc<ProcessGroup>>> = Mutex::new(Vec::new());
static SIGNALS: OnceLock<Result<(), String>> = OnceLock::new();
/// What a catchable stop runs after the owned command groups are stopped and
/// before the process ends, in the order registered.
type StopHook = Box<dyn Fn() + Send + Sync>;
static STOP_HOOKS: Mutex<Vec<StopHook>> = Mutex::new(Vec::new());

/// Runs each stop hook in turn. A hook that panics is logged and the next
/// one runs: no hook can end the thread that handles the signal, which goes
/// on to end the process.
fn run_stop_hooks(hooks: Vec<StopHook>) {
    for hook in hooks {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(hook)).is_err() {
            tracing::error!(event = "stop_hook_panicked");
        }
    }
}

/// Runs `hook` when this process is stopped by SIGINT or SIGTERM: after
/// its commands' groups are stopped, before it ends. A hook bounds its own
/// work; the process ends once every hook has returned.
pub fn on_stop(hook: StopHook) -> std::io::Result<()> {
    install_signals()?;
    match STOP_HOOKS.lock() {
        Ok(mut hooks) => hooks.push(hook),
        Err(poisoned) => poisoned.into_inner().push(hook),
    }
    Ok(())
}

fn groups() -> std::sync::MutexGuard<'static, Vec<Arc<ProcessGroup>>> {
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
                for group in groups().iter() {
                    if let Err(error)=group.stop() {tracing::error!(event="interrupted_group_cleanup_failed",pid=group.pid.as_raw_nonzero().get(),error=%error);}
                }
                let hooks = match STOP_HOOKS.lock() {
                    Ok(mut hooks) => std::mem::take(&mut *hooks),
                    Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
                };
                run_stop_hooks(hooks);
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
/// Ends this process with `code` after stopping every command group it
/// owns, as an interruption does: a refusal that ends the whole process
/// leaves none of its commands running, whichever thread started them.
pub fn exit_stopping_owned(code: i32) -> ! {
    // Held to the end: no command starts after the groups were stopped.
    let registry = groups();
    for group in registry.iter() {
        if let Err(error) = group.stop() {
            tracing::error!(event="exit_group_cleanup_failed",pid=group.pid.as_raw_nonzero().get(),error=%error);
        }
    }
    std::process::exit(code)
}

struct ProcessGroup {
    pid: rustix::process::Pid,
    #[cfg(target_os = "macos")]
    reaped: AtomicBool,
}

impl ProcessGroup {
    fn stop(&self) -> std::io::Result<()> {
        use rustix::process::{Signal, kill_process_group};

        self.stop_with(|pid| kill_process_group(pid, Signal::KILL))
    }

    fn stop_with(
        &self,
        mut signal: impl FnMut(rustix::process::Pid) -> rustix::io::Result<()>,
    ) -> std::io::Result<()> {
        use rustix::io::Errno;

        // Classify the signal using the reap evidence available when it starts.
        #[cfg(target_os = "macos")]
        let reaped = self.reaped.load(Ordering::Acquire);
        let result = signal(self.pid);
        // The new snapshot is already true, so the retry cannot need another.
        #[cfg(target_os = "macos")]
        let (result, reaped) =
            if result == Err(Errno::PERM) && !reaped && self.reaped.load(Ordering::Acquire) {
                (signal(self.pid), true)
            } else {
                (result, reaped)
            };
        match result {
            Ok(()) | Err(Errno::SRCH) => Ok(()),
            // Darwin's POSIX group signal found no member we can signal.
            #[cfg(target_os = "macos")]
            Err(Errno::PERM) if reaped => {
                tracing::warn!(
                    event = "reaped_group_cannot_be_signalled",
                    pgid = self.pid.as_raw_nonzero().get(),
                    "no member of the reaped child's process group could be signalled"
                );
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }
}

/// Ownership of one command's process group, retained until its descendants stop.
pub struct OwnedProcessGroup {
    group: Arc<ProcessGroup>,
    stopped: std::cell::Cell<bool>,
}
impl OwnedProcessGroup {
    /// Waits for the direct child of this group and records the reap.
    pub fn wait(&self, child: &mut Child) -> std::io::Result<std::process::ExitStatus> {
        let status = child.wait()?;
        #[cfg(target_os = "macos")]
        self.record_reap(child);
        Ok(status)
    }

    /// Checks the direct child of this group without blocking, recording a reap.
    pub fn try_wait(&self, child: &mut Child) -> std::io::Result<Option<std::process::ExitStatus>> {
        let status = child.try_wait()?;
        #[cfg(target_os = "macos")]
        if status.is_some() {
            self.record_reap(child);
        }
        Ok(status)
    }

    #[cfg(target_os = "macos")]
    fn record_reap(&self, child: &Child) {
        assert_eq!(child.id(), self.group.pid.as_raw_nonzero().get() as u32);
        self.group.reaped.store(true, Ordering::Release);
    }

    pub fn stop(&self) -> std::io::Result<()> {
        if self.stopped.get() {
            return Ok(());
        }
        let mut registry = groups();
        self.group.stop()?;
        registry.retain(|group| !Arc::ptr_eq(group, &self.group));
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
    let group = Arc::new(ProcessGroup {
        pid,
        #[cfg(target_os = "macos")]
        reaped: AtomicBool::new(false),
    });
    registry.push(Arc::clone(&group));
    Ok((
        child,
        OwnedProcessGroup {
            group,
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
    let started = Instant::now();
    let limit = Limit::from(timeout, started);
    invocation.execute(command, Some(limit), started)
}

/// Executes a native command as [`output`] does, waiting for it to its end
/// however long that takes: a command that runs another party's code, such
/// as git running the repository's hooks, takes as long as the plain command
/// does.
pub fn output_to_end(
    command: &mut Command,
    input: Option<&[u8]>,
) -> Result<std::process::Output, ProcessError> {
    let name = command.get_program().to_string_lossy().into_owned();
    let mut invocation = Invocation::program(&name, &[]);
    invocation.input = input;
    invocation.execute(command, None, Instant::now())
}

#[cfg(test)]
mod stop_hook_tests {
    use super::run_stop_hooks;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn a_panicking_stop_hook_leaves_the_rest_of_the_stop_to_run() {
        let later = Arc::new(AtomicBool::new(false));
        let ran = Arc::clone(&later);
        // The signal thread re-raises the signal once this returns.
        run_stop_hooks(vec![
            Box::new(|| panic!("a stop hook that fails")),
            Box::new(move || ran.store(true, Ordering::SeqCst)),
        ]);
        assert!(later.load(Ordering::SeqCst));
    }
}

#[cfg(all(test, target_os = "macos"))]
#[allow(clippy::unwrap_used, reason = "test child and signal setup")]
mod darwin_group_tests {
    use super::*;
    use rustix::io::Errno;
    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};

    fn wait_without_reaping(child: &Child) {
        let pid = Pid::from_raw(child.id() as i32).unwrap();
        assert!(
            waitid(
                WaitId::Pid(pid),
                WaitIdOptions::EXITED | WaitIdOptions::NOWAIT
            )
            .unwrap()
            .is_some()
        );
    }

    #[test]
    fn a_reap_during_signalling_requires_one_new_attempt() {
        use std::sync::Barrier;

        for (reap_before_signal, retry_result) in [
            (true, Err(Errno::PERM)),
            (false, Err(Errno::IO)),
            (false, Err(Errno::SRCH)),
            (false, Ok(())),
        ] {
            let group = ProcessGroup {
                pid: rustix::process::getpid(),
                reaped: AtomicBool::new(false),
            };
            let start_reap = Barrier::new(2);
            let reap_recorded = Barrier::new(2);
            let reap = || {
                start_reap.wait();
                reap_recorded.wait();
            };

            let mut calls = 0;
            let mut first_signal_saw_reap = false;
            let result = std::thread::scope(|scope| {
                scope.spawn(|| {
                    start_reap.wait();
                    group.reaped.store(true, Ordering::Release);
                    reap_recorded.wait();
                });
                group.stop_with(|_| {
                    calls += 1;
                    if calls > 1 {
                        assert!(group.reaped.load(Ordering::Acquire));
                        return retry_result;
                    }
                    if reap_before_signal {
                        reap();
                    }
                    first_signal_saw_reap = group.reaped.load(Ordering::Acquire);
                    let refusal = Err(Errno::PERM);
                    if !reap_before_signal {
                        // The signal was already refused when wait recorded the reap.
                        reap();
                    }
                    refusal
                })
            });

            assert_eq!(first_signal_saw_reap, reap_before_signal);
            assert_eq!(calls, 2, "the first refusal needs exactly one retry");
            if retry_result == Err(Errno::IO) {
                assert_eq!(
                    result.unwrap_err().raw_os_error(),
                    Some(Errno::IO.raw_os_error())
                );
            } else {
                result.unwrap();
            }
        }
    }

    #[test]
    fn an_unsignalable_group_preserves_the_reaped_childs_exit() {
        let duration = Duration::from_secs(5);
        let limits = [
            None,
            Some(Limit {
                limit: duration,
                deadline: Instant::now() + duration,
            }),
        ];
        for limit in limits {
            let invocation = Invocation::program("/bin/sh", &[]);
            let mut command = Command::new("/bin/sh");
            command
                .args(["-c", "read line; printf captured; exit 17"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let (mut child, group) = spawn(&mut command).unwrap();
            let mut member = Command::new("/bin/cat")
                .process_group(group.group.pid.as_raw_nonzero().get())
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            drop(child.stdin.take());
            drop(member.stdin.take());
            wait_without_reaping(&child);
            // Keep this zombie in the group through both cleanup attempts.
            wait_without_reaping(&member);
            assert_eq!(
                group.stop().unwrap_err().raw_os_error(),
                Some(Errno::PERM.raw_os_error())
            );

            let result = invocation.finish_child(&mut child, &group, limit, Instant::now());
            assert!(member.wait().unwrap().success());
            let result = result.unwrap();
            assert_eq!(result.status.code(), Some(17));
            assert_eq!(result.stdout, b"captured");
        }
    }
}
