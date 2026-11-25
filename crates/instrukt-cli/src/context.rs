//! The invocation context every command receives.

use std::path::PathBuf;

use anyhow::Context as _;
use instrukt_core::home::Home;

use crate::clock::Clock;
use crate::output::Output;

pub struct Ctx {
    pub github: std::sync::Arc<instrukt_todos::work::GitHubAccess>,
    /// The home (`$HOME/.instrukt`) of this invocation.
    pub home: Home,
    /// The caller's working directory.
    pub cwd: PathBuf,
    /// The command output writer.
    pub out: Output,
    /// The clock every command takes its time from.
    pub clock: Clock,
    pub receiver: Option<instrukt_runner::webhooks::PreparedReceiver>,
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
    /// Turns a context built for reading into a writing one, for a command
    /// that learns only after its context exists that it writes: the home
    /// writes, and the owned configuration is brought current as on any
    /// writing entry. Its GitHub access keeps the home it was built with.
    pub fn enter_writing(&mut self) -> anyhow::Result<()> {
        self.home = self.home.for_writing();
        enter_writing(&self.home, &self.cwd)
    }
    fn load(inspection: bool) -> anyhow::Result<Self> {
        let home = Home::from_env()?;
        let home = if inspection {
            home.for_inspection()
        } else {
            home
        };
        let cwd = std::env::current_dir().context("cannot read the working directory")?;
        if !inspection {
            enter_writing(&home, &cwd)?;
        }
        let clock = Clock::from_process()?;
        let github = std::sync::Arc::new(instrukt_todos::work::GitHubAccess::new(home.clone()));
        Ok(Self {
            github,
            home,
            cwd,
            out: Output::stdout(),
            clock,
            receiver: None,
        })
    }
}

/// Every writing command enters with current owned configuration, before its
/// module's refusal or unchanged-work decision.
fn enter_writing(home: &Home, cwd: &std::path::Path) -> anyhow::Result<()> {
    instrukt_core::config::Agents::converge_file(home)?;
    instrukt_core::config::Configuration::load_at(home, cwd)?;
    Ok(())
}
