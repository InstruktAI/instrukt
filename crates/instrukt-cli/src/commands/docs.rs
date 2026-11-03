//! Docs commands: `instrukt docs validate|fold|index|get`.

use std::process::ExitCode;

use crate::commands::not_built;
use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct DocsArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

pub fn run(_args: DocsArgs, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    not_built("docs")
}
