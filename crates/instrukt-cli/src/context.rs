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
        Self::load(false)
    }
    pub fn inspection() -> anyhow::Result<Self> {
        Self::load(true)
    }
    pub fn configuration_read() -> anyhow::Result<Self> {
        let mut ctx = Self::load(true)?;
        ctx.home = ctx.home.for_configuration_read();
        Ok(ctx)
    }
    fn load(inspection: bool) -> anyhow::Result<Self> {
        let home = Home::from_env()?;
        let home = if inspection {
            home.for_inspection()
        } else {
            home
        };
        let cwd = std::env::current_dir().context("cannot read the working directory")?;
        // Every command enters with current owned configuration, before its
        // module's refusal or unchanged-work decision.
        if !inspection {
            instrukt_core::config::Configuration::load_at(&home, &cwd)?;
        }
        let clock = Clock::from_process()?;
        Ok(Self {
            home,
            cwd,
            out: Output::stdout(),
            clock,
        })
    }
}
