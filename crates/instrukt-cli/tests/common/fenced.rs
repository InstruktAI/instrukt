//! The instrukt processes a test starts. An instrukt process ends with the
//! write fence's own status when it refused a write, or when it could not
//! record a refusal; any process the harness started that ends so fails the
//! test, before an assertion that expects a failure can accept it. A test of
//! the fence itself says that it expects the refusal.

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{Command as StdCommand, ExitStatus, Output, Stdio};
use std::time::Duration;

use assert_cmd::assert::Assert;
use instrukt_core::fsx::FENCE_EXIT;

/// Fails the test when `status` is the fence's own and the test did not
/// expect a refusal.
fn checked(status: &ExitStatus, refusal_expected: bool, stderr: &[u8]) {
    if status.code() == Some(i32::from(FENCE_EXIT))
        && !refusal_expected
        && !std::thread::panicking()
    {
        panic!(
            "instrukt ended with the write fence's status {FENCE_EXIT}: it refused a write of its own outside the temporary HOME, or could not record that refusal\n{}",
            String::from_utf8_lossy(stderr)
        );
    }
}

/// `instrukt` to run to completion, as `assert_cmd` runs it.
pub struct Instrukt {
    command: assert_cmd::Command,
    refusal_expected: bool,
}

impl Instrukt {
    pub(super) fn new(command: assert_cmd::Command) -> Self {
        Self {
            command,
            refusal_expected: false,
        }
    }
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.command.arg(arg);
        self
    }
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.args(args);
        self
    }
    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.command.env(key, value);
        self
    }
    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.command.env_remove(key);
        self
    }
    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.command.current_dir(dir);
        self
    }
    pub fn write_stdin(&mut self, buffer: impl Into<Vec<u8>>) -> &mut Self {
        self.command.write_stdin(buffer);
        self
    }
    pub fn timeout(&mut self, timeout: Duration) -> &mut Self {
        self.command.timeout(timeout);
        self
    }
    /// The test is about the fence: the process may end with its status.
    pub fn refusal_expected(&mut self) -> &mut Self {
        self.refusal_expected = true;
        self
    }
    pub fn assert(&mut self) -> Assert {
        let assert = self.command.assert();
        let output = assert.get_output();
        checked(&output.status, self.refusal_expected, &output.stderr);
        assert
    }
    pub fn output(&mut self) -> io::Result<Output> {
        let output = self.command.output()?;
        checked(&output.status, self.refusal_expected, &output.stderr);
        Ok(output)
    }
}

/// `instrukt` as a plain process, for a test that runs it in the background
/// and stops it part-way.
pub struct InstruktProcess {
    command: StdCommand,
    refusal_expected: bool,
}

impl InstruktProcess {
    pub(super) fn new(command: StdCommand) -> Self {
        Self {
            command,
            refusal_expected: false,
        }
    }
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.command.arg(arg);
        self
    }
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.args(args);
        self
    }
    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.command.env(key, value);
        self
    }
    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.command.env_remove(key);
        self
    }
    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.command.current_dir(dir);
        self
    }
    pub fn stdin(&mut self, stdin: impl Into<Stdio>) -> &mut Self {
        self.command.stdin(stdin);
        self
    }
    pub fn stdout(&mut self, stdout: impl Into<Stdio>) -> &mut Self {
        self.command.stdout(stdout);
        self
    }
    pub fn stderr(&mut self, stderr: impl Into<Stdio>) -> &mut Self {
        self.command.stderr(stderr);
        self
    }
    /// The test is about the fence: the process may end with its status.
    pub fn refusal_expected(&mut self) -> &mut Self {
        self.refusal_expected = true;
        self
    }
    pub fn spawn(&mut self) -> io::Result<Child> {
        let mut child = Child::from(self.command.spawn()?);
        child.refusal_expected = self.refusal_expected;
        Ok(child)
    }
    pub fn output(&mut self) -> io::Result<Output> {
        let output = self.command.output()?;
        checked(&output.status, self.refusal_expected, &output.stderr);
        Ok(output)
    }
}

/// A process the harness started in the background. However its owner
/// stops and reaps it, when it is dropped an end with the fence's status
/// fails the test.
pub struct Child {
    inner: std::process::Child,
    reaped: Option<ExitStatus>,
    refusal_expected: bool,
}

impl Child {
    /// Tells the harness how the process ended, when its owner reaped it
    /// by a native wait the standard library does not see.
    pub fn reaped(&mut self, status: ExitStatus) {
        self.reaped = Some(status);
    }
}

impl From<std::process::Child> for Child {
    fn from(inner: std::process::Child) -> Self {
        Self {
            inner,
            reaped: None,
            refusal_expected: false,
        }
    }
}

impl std::ops::Deref for Child {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for Child {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        let ended = self.reaped.or_else(|| self.inner.try_wait().ok().flatten());
        if let Some(status) = ended {
            checked(&status, self.refusal_expected, b"");
        }
    }
}

/// What a background process is started from: the harness's launcher, or
/// a command for a copy of the binary.
pub trait SpawnChild {
    fn spawn_child(&mut self) -> io::Result<Child>;
}

impl SpawnChild for InstruktProcess {
    fn spawn_child(&mut self) -> io::Result<Child> {
        self.spawn()
    }
}

impl SpawnChild for StdCommand {
    fn spawn_child(&mut self) -> io::Result<Child> {
        self.spawn().map(Child::from)
    }
}
