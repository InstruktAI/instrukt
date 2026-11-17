//! Who holds the machine's runner lock. The operating system lock is the
//! only exclusion; its holder publishes `<holder> <pid> <start>` in the lock
//! file itself while it holds the lock and clears it when it lets go. A
//! contender reads that record again on every attempt and believes it only
//! when the process it names is that same live incarnation: the same id and
//! the same start time. A missing, partial or mismatching record, one left
//! by a holder that died, or one not yet replaced by a new holder, names no
//! holder; the contender keeps waiting.
use std::os::unix::fs::FileExt as _;
use std::path::Path;
use std::time::{Duration, Instant};

use instrukt_core::fsx::{FsError, LockGuard};
use instrukt_core::home::Home;

use super::{RUN_LOCK_WAIT, RunnerError};

/// How often a run started by hand tries the lock again and rereads its
/// holder.
const RETRY: Duration = Duration::from_millis(250);

/// The invocation holding the runner lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Holder {
    Service,
    Run,
    Tick,
}
impl Holder {
    fn name(self) -> &'static str {
        match self {
            Self::Service => "service",
            Self::Run => "run",
            Self::Tick => "tick",
        }
    }
    fn named(name: &str) -> Option<Self> {
        [Self::Service, Self::Run, Self::Tick]
            .into_iter()
            .find(|holder| holder.name() == name)
    }
}

/// The start identity of the live process `pid`, as the operating system
/// records when it started. `None` when no such process runs: it is gone,
/// or it ended and has not been reaped.
pub(crate) fn incarnation(pid: u32) -> Option<String> {
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // The fields after the command name: state is the first, the start
        // time the twentieth.
        let fields = stat
            .rsplit_once(')')?
            .1
            .split_whitespace()
            .collect::<Vec<_>>();
        if matches!(fields.first(), None | Some(&"Z") | Some(&"X")) {
            return None;
        }
        return fields.get(19).map(|start| (*start).to_owned());
    }
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "stat=,lstart=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let (state, start) = text.trim().split_once(char::is_whitespace)?;
    (!state.starts_with('Z')).then(|| start.trim().to_owned())
}

/// This process's start identity.
pub(crate) fn own_incarnation() -> std::io::Result<String> {
    incarnation(std::process::id()).ok_or_else(|| {
        std::io::Error::other("cannot read this process's start time from the operating system")
    })
}

/// The held runner lock with its published holder; the record is cleared
/// when it is dropped.
pub(super) struct RunnerLock(LockGuard);
impl RunnerLock {
    /// Publishes `holder` under the lock `guard` holds. A holder that
    /// cannot publish does not run: contenders could not tell who holds
    /// the lock.
    fn publish(guard: LockGuard, holder: Holder) -> Result<Self, RunnerError> {
        let unpublished = |source| RunnerError::HolderUnpublished {
            path: guard.path().to_path_buf(),
            source,
        };
        let start = own_incarnation().map_err(unpublished)?;
        let record = format!("{} {} {start}\n", holder.name(), std::process::id());
        let file = guard.file();
        file.set_len(0)
            .and_then(|()| file.write_all_at(record.as_bytes(), 0))
            .map_err(unpublished)?;
        Ok(Self(guard))
    }
}
impl Drop for RunnerLock {
    fn drop(&mut self) {
        if let Err(error) = self.0.file().set_len(0) {
            tracing::warn!(event = "runner_holder_uncleared", error = %error);
        }
    }
}

/// The holder the lock file names, when it is that live incarnation.
fn holder(path: &Path) -> Option<Holder> {
    let record = std::fs::read_to_string(path).ok()?;
    let mut fields = record.trim_end_matches('\n').splitn(3, ' ');
    let holder = Holder::named(fields.next()?)?;
    let pid = fields.next()?.parse::<u32>().ok()?;
    let start = fields.next()?;
    (incarnation(pid).as_deref() == Some(start)).then_some(holder)
}

/// The lock for a tick, which does nothing while another invocation holds it.
pub(super) fn tick(home: &Home) -> Result<Option<RunnerLock>, RunnerError> {
    LockGuard::try_acquire(&home.runner_lock_file())?
        .map(|guard| RunnerLock::publish(guard, Holder::Tick))
        .transpose()
}

/// The lock for `instrukt run`. The service waits for whoever holds it. A
/// run started by hand is refused as soon as the service is the holder,
/// also when the service takes the lock while it waits, and otherwise
/// waits a bounded time, saying so once.
pub(super) fn run(home: &Home, service: bool) -> Result<RunnerLock, RunnerError> {
    let path = home.runner_lock_file();
    if service {
        return RunnerLock::publish(LockGuard::wait(&path)?, Holder::Service);
    }
    let started = Instant::now();
    let mut announced = false;
    loop {
        if let Some(guard) = LockGuard::try_acquire(&path)? {
            return RunnerLock::publish(guard, Holder::Run);
        }
        if holder(&path) == Some(Holder::Service) {
            return Err(RunnerError::ServiceRunning);
        }
        if started.elapsed() >= RUN_LOCK_WAIT {
            return Err(FsError::LockTimeout {
                path,
                waited: RUN_LOCK_WAIT,
            }
            .into());
        }
        if !announced {
            eprintln!(
                "instrukt: waiting for the runner lock, which another runner invocation holds"
            );
            announced = true;
        }
        std::thread::sleep(RETRY);
    }
}
