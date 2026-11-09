//! Publication-owned read-only state report for the cockpit.
use crate::context::Ctx;
use std::process::ExitCode;
#[derive(Debug, clap::Args)]
pub struct StatusArgs {
    /// Print the same seven sections as a JSON object.
    #[arg(long)]
    pub json: bool,
}
pub fn run(
    args: &StatusArgs,
    ctx: &mut Ctx,
    caller_error: Option<String>,
) -> anyhow::Result<ExitCode> {
    let env = crate::wiring::environment(ctx)
        .unwrap_or_else(|_| instrukt_core::config::Env::from_process());
    let report =
        instrukt_core::publication::status::inspect(&instrukt_core::publication::status::Request {
            home: &ctx.home,
            cwd: &ctx.cwd,
            version: env!("CARGO_PKG_VERSION"),
            executable: std::env::current_exe(),
            caller_error,
            env: &env,
        });
    if args.json {
        ctx.out.json(&report)?;
    } else {
        for (name, value) in serde_json::to_value(&report)?
            .as_object()
            .into_iter()
            .flatten()
        {
            ctx.out
                .line(&format!("{name}: {}", serde_json::to_string_pretty(value)?))?;
        }
    }
    ctx.out.flush()?;
    Ok(if report.healthy() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
