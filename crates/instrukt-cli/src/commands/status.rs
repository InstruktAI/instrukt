//! Publication-owned read-only state report for the cockpit.
use crate::context::Ctx;
use std::process::ExitCode;
#[derive(Debug, clap::Args)]
pub struct StatusArgs {
    /// Print the same eight sections as a JSON object.
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
    let instructions = match instrukt_core::publication::position_owners(&ctx.home) {
        Err(error) => Err(error.to_string()),
        Ok(owners) if owners.keys().any(|domain| domain.as_str() == "general") => {
            Ok(Some(match crate::wiring::configured_runtimes(ctx) {
                Ok(Some(runtimes)) => runtimes.configured(&ctx.home, &env),
                Ok(None) => instrukt_core::interfaces::Answer::Unreachable {
                    runtime: "selected".into(),
                    cause: "no compiled adapter".into(),
                },
                Err(error) => instrukt_core::interfaces::Answer::Unreachable {
                    runtime: "selected".into(),
                    cause: error.to_string(),
                },
            }))
        }
        Ok(_) => Ok(None),
    };
    let report =
        instrukt_core::publication::status::inspect(&instrukt_core::publication::status::Request {
            home: &ctx.home,
            cwd: &ctx.cwd,
            version: env!("CARGO_PKG_VERSION"),
            executable: std::env::current_exe(),
            caller_error,
            env: &env,
            instructions,
            backup: &instrukt_runner::backup::Standing,
            now: ctx.clock.now(),
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
