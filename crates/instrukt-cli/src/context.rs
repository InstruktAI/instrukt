//! The invocation context every command receives.

use std::path::PathBuf;

use anyhow::Context as _;
use instrukt_core::home::Home;

use crate::clock::Clock;
use crate::output::Output;

pub struct Ctx {
    /// The home (`$HOME/.instrukt`) of this invocation.
    pub home: Home,
    /// The caller's working directory.
    pub cwd: PathBuf,
    /// The command output writer.
    pub out: Output,
    /// The clock every command takes its time from.
    pub clock: Clock,
}

impl Ctx {
    pub fn from_process() -> anyhow::Result<Self> {
        let home = Home::from_env()?;
        let cwd = std::env::current_dir().context("cannot read the working directory")?;
        let clock = Clock::from_process()?;
        Ok(Self {
            home,
            cwd,
            out: Output::stdout(),
            clock,
        })
    }
}
