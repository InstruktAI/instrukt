//! The clock of one invocation: the system clock, or the operator's
//! override `INSTRUKT_NOW=<RFC 3339>` (layered-configuration § Environment).
//!
//! The override is read once, here, and every command takes its time from
//! this clock. It is never silent: while it is set, every command says so on
//! stderr. An unparseable value refuses the command rather than falling back
//! to the system clock.

use std::sync::Arc;

use anyhow::Context as _;
use jiff::Timestamp;

/// The environment variable that overrides the clock.
pub const OVERRIDE_VAR: &str = "INSTRUKT_NOW";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clock {
    System,
    Fixed(Timestamp),
}

impl Clock {
    /// The clock the process environment selects.
    pub fn from_process() -> anyhow::Result<Self> {
        match std::env::var_os(OVERRIDE_VAR) {
            None => Ok(Clock::System),
            Some(raw) => {
                let text = raw
                    .to_str()
                    .with_context(|| format!("{OVERRIDE_VAR} is not valid UTF-8"))?;
                let at: Timestamp = text.parse().with_context(|| {
                    format!("{OVERRIDE_VAR}={text:?} is not an RFC 3339 time; unset it or give one")
                })?;
                eprintln!("instrukt: clock overridden to {at}");
                Ok(Clock::Fixed(at))
            }
        }
    }

    pub fn now(self) -> Timestamp {
        match self {
            Clock::System => Timestamp::now(),
            Clock::Fixed(at) => at,
        }
    }

    /// This clock as the shared function the work record takes.
    pub fn shared(self) -> Arc<dyn Fn() -> Timestamp + Send + Sync> {
        Arc::new(move || self.now())
    }
}
