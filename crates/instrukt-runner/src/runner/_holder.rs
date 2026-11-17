//! Who holds the machine's runner lock. The operating system lock is the
//! only exclusion; its holder publishes `<holder> <pid> <start>` in the lock
//! file itself while it holds the lock and clears it when it lets go. A
//! contender reads that record again on every attempt and believes it only
//! when the process it names is that same live incarnation: the same id and
//! the same start as the operating system records it, to its finest unit. A
//! missing, partial or mismatching record, one left by a holder that died,
//! one whose incarnation cannot be read, or one not yet replaced by a new
//! holder, names no holder; the contender keeps waiting.
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

/// The start identity of the live process `pid`: when it started, as the
/// operating system's process information records it numerically, to its
/// finest unit. `None` when no such live process can be read: it is gone,
/// it ended and has not been reaped, or its information is unavailable.
#[cfg(target_os = "linux")]
pub(crate) fn incarnation(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The fields after the command name: state is the first, the start time
    // in clock ticks since boot the twentieth.
    let fields = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    if matches!(fields.first(), None | Some(&"Z") | Some(&"X")) {
        return None;
    }
    let ticks = fields.get(19)?.parse::<u64>().ok()?;
    Some(format!("ticks:{ticks}"))
}

/// The start identity of the live process `pid`: when it started, as the
/// operating system's process information records it numerically, to its
/// finest unit. `None` when no such live process can be read: it is gone,
/// it ended and has not been reaped, or its information is unavailable.
#[cfg(target_os = "macos")]
pub(crate) fn incarnation(pid: u32) -> Option<String> {
    /// `p_stat` of a process that ended and has not been reaped.
    const SZOMB: u32 = 5;
    let info = darwin_libproc::task_all_info(i32::try_from(pid).ok()?).ok()?;
    let bsd = info.pbsd;
    (bsd.pbi_pid == pid && bsd.pbi_status != SZOMB)
        .then(|| format!("start:{}.{:06}", bsd.pbi_start_tvsec, bsd.pbi_start_tvusec))
}

/// No process information is read on other systems: no holder is known.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn incarnation(_pid: u32) -> Option<String> {
    None
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
pub(super) fn holder(path: &Path) -> Option<Holder> {
    live_holder(path).map(|(holder, _, _)| holder)
}

/// The holder the lock file names with its process id and start, when it is
/// that live incarnation.
pub(super) fn live_holder(path: &Path) -> Option<(Holder, u32, String)> {
    let record = std::fs::read_to_string(path).ok()?;
    let mut fields = record.trim_end_matches('\n').splitn(3, ' ');
    let holder = Holder::named(fields.next()?)?;
    let pid = fields.next()?.parse::<u32>().ok()?;
    let start = fields.next()?;
    (incarnation(pid).as_deref() == Some(start)).then(|| (holder, pid, start.to_owned()))
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
