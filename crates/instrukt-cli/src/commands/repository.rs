//! Repository commands: `instrukt init`, `instrukt hooks …`, `instrukt repo commit|stamp`.

use std::process::ExitCode;

use crate::commands::not_built;
use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct InitArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

pub fn run_init(_args: InitArgs, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    not_built("init")
}

#[derive(Debug, clap::Args)]
pub struct HooksArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

pub fn run_hooks(_args: HooksArgs, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    not_built("hooks")
}

#[derive(Debug, clap::Args)]
pub struct RepoArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

pub fn run_repo(_args: RepoArgs, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    not_built("repo")
}
