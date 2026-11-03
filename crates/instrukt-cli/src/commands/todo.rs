//! Todo commands: the work record and the workflow-driving `prepare`/`work`.

use std::process::ExitCode;

use crate::commands::not_built;
use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct TodoArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

pub fn run(_args: TodoArgs, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    not_built("todo")
}
