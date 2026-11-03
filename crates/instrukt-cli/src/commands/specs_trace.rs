//! Blueprint's `instrukt specs trace <spec_id> <UC-id>` and
//! `instrukt specs trace --file <path>`
//! (`docs/local/spec/feature/blueprint/use-case-trace.md`).
//!
//! Owned by the blueprint builder in wave 3. The subcommand's one-line
//! about stays on `SpecsCommand::Trace` in `specs.rs`, so this file is the
//! only one the builder edits.

use std::process::ExitCode;

use crate::commands::not_built;
use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct TraceArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

pub fn run(_args: TraceArgs, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    not_built("specs trace")
}
