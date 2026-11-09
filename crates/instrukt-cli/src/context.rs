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
}

impl Ctx {
    pub fn from_process() -> anyhow::Result<Self> {
        let home = Home::from_env()?;
        let cwd = std::env::current_dir().context("cannot read the working directory")?;
        // Every command enters with current owned configuration, before its
        // module's refusal or unchanged-work decision.
        instrukt_core::config::Configuration::load_at(&home, &cwd)?;
        let clock = Clock::from_process()?;
        let github = std::sync::Arc::new(instrukt_todos::work::GitHubAccess::new(home.clone()));
        Ok(Self {
            github,
            home,
            cwd,
            out: Output::stdout(),
            clock,
        })
    }
}
