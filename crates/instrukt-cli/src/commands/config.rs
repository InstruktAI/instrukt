//! Configuration commands: `instrukt config get|patch|init|validate`.

use std::process::ExitCode;

use crate::commands::not_built;
use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct ConfigArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

pub fn run(_args: ConfigArgs, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    not_built("config")
}
