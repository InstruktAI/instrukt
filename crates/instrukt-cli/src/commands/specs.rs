//! Specs commands: `instrukt specs coverage|reserve` and blueprint's `instrukt specs trace`.

use std::process::ExitCode;

use crate::commands::not_built;
use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct SpecsArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

pub fn run(_args: SpecsArgs, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    not_built("specs")
}
