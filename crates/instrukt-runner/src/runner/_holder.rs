//! Who holds the machine's runner lock. The operating system lock is the
//! only exclusion; its holder publishes `<holder> <pid>` in the lock file
//! itself while it holds the lock and clears it when it lets go. A
//! contender reads that record again on every attempt and believes it only
//! while the process it names is alive, so a record left by a crashed
//! holder, or not yet replaced by a new one, is never taken for the holder.
use std::os::unix::fs::FileExt as _;
use std::path::Path;
use std::time::{Duration, Instant};

use instrukt_core::fsx::{FsError, LockGuard};
use instrukt_core::home::Home;

use super::{RUN_LOCK_WAIT, RunnerError};

/// How often a contender tries the lock again and rereads its holder.
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

/// The held runner lock with its published holder; the record is cleared
/// when it is dropped.
pub(super) struct RunnerLock(LockGuard);
impl RunnerLock {
    /// Publishes `holder` under the lock `guard` holds. A holder that
    /// cannot publish does not run: contenders could not tell who holds
    /// the lock.
    fn publish(guard: LockGuard, holder: Holder) -> Result<Self, RunnerError> {
        let record = format!("{} {}\n", holder.name(), std::process::id());
        let file = guard.file();
        file.set_len(0)
            .and_then(|()| file.write_all_at(record.as_bytes(), 0))
            .map_err(|source| RunnerError::HolderUnpublished {
                path: guard.path().to_path_buf(),
                source,
            })?;
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

/// The live holder the lock file names, if any.
fn holder(path: &Path) -> Option<Holder> {
    let record = std::fs::read_to_string(path).ok()?;
    let (name, pid) = record.trim().split_once(' ')?;
    let pid = rustix::process::Pid::from_raw(pid.parse().ok()?)?;
    let alive = match rustix::process::test_kill_process(pid) {
        Ok(()) | Err(rustix::io::Errno::PERM) => true,
        Err(_) => false,
    };
    alive.then(|| Holder::named(name)).flatten()
}

/// The lock for a tick, which does nothing while another invocation holds it.
pub(super) fn tick(home: &Home) -> Result<Option<RunnerLock>, RunnerError> {
    LockGuard::try_acquire(&home.runner_lock_file())?
        .map(|guard| RunnerLock::publish(guard, Holder::Tick))
        .transpose()
}

/// The lock for `instrukt run`. The service waits for whoever holds it. A
/// run started by hand is refused as soon as the service is the live
/// holder, also when the service takes the lock while it waits, and
/// otherwise waits a bounded time.
pub(super) fn run(home: &Home, service: bool) -> Result<RunnerLock, RunnerError> {
    let path = home.runner_lock_file();
    let started = Instant::now();
    loop {
        if let Some(guard) = LockGuard::try_acquire(&path)? {
            let holder = if service {
                Holder::Service
            } else {
                Holder::Run
            };
            return RunnerLock::publish(guard, holder);
        }
        if !service {
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
        }
        std::thread::sleep(RETRY);
    }
}
