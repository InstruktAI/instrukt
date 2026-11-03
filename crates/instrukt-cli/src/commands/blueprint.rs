//! Blueprint commands: `instrukt blueprint affected|path|query|explain|status`.

use std::process::ExitCode;

use crate::commands::not_built;
use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct BlueprintArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

pub fn run(_args: BlueprintArgs, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    not_built("blueprint")
}
