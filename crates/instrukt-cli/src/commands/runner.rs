//! Runner commands: `run`, `tick`, `ticks status`, `sessions …`, `install`.

use std::process::ExitCode;

use crate::commands::not_built;
use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct RunArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

pub fn run_run(_args: RunArgs, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    not_built("run")
}

#[derive(Debug, clap::Args)]
pub struct TickArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

pub fn run_tick(_args: TickArgs, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    not_built("tick")
}

#[derive(Debug, clap::Args)]
pub struct TicksArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

pub fn run_ticks(_args: TicksArgs, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    not_built("ticks")
}

#[derive(Debug, clap::Args)]
pub struct SessionsArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

pub fn run_sessions(_args: SessionsArgs, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    not_built("sessions")
}

#[derive(Debug, clap::Args)]
pub struct InstallArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

pub fn run_install(_args: InstallArgs, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    not_built("install")
}
