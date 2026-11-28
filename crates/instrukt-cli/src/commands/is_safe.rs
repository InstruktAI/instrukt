//! Stateless untrusted-text safety check (`instrukt is-safe [<file>] [--json]`)
//! (`project/instrukt/spec/domain/screening`,
//! `project/instrukt/spec/feature/screening/untrusted-text-screening`).

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Context as _;
use clap::Args;
use instrukt_outside::{CallerWait, Gate, Report};
use instrukt_screening::{SafeOutcome, is_safe_async};
use tokio::signal::unix::{SignalKind, signal};

use crate::context::Ctx;

#[derive(Debug, Args)]
pub struct IsSafeArgs {
    /// File containing untrusted text to screen (reads stdin when omitted or `-`).
    #[arg(value_name = "FILE")]
    pub file: Option<PathBuf>,
    /// Output the verdict as JSON (`{"safe":true}` or `{"safe":false,"reason":"..."}`).
    #[arg(long)]
    pub json: bool,
}

enum Execution {
    Completed(Result<SafeOutcome, instrukt_screening::CallDefect>),
    Signaled(i32),
}

pub fn run(args: IsSafeArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let text = read_input(args.file.as_deref())?;
    let admission = instrukt_core::config::inspect_global_screening(&ctx.home)?;
    let gate = Gate::new(
        ctx.clock.shared(),
        Arc::new(Instant::now),
        Report::foreground(Arc::new(Mutex::new(std::io::stderr()))),
    );

    let execution = instrukt_outside::runtime::block_on(async {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_check = {
            let flag = Arc::clone(&stop);
            move || flag.load(Ordering::SeqCst)
        };
        let mut sigint = signal(SignalKind::interrupt())
            .context("cannot install SIGINT handler for `instrukt is-safe`")?;
        let mut sigterm = signal(SignalKind::terminate())
            .context("cannot install SIGTERM handler for `instrukt is-safe`")?;

        let outcome = is_safe_async(
            &admission,
            &text,
            &gate,
            CallerWait::UntilReturns,
            &stop_check,
        );
        tokio::pin!(outcome);

        tokio::select! {
            result = &mut outcome => Ok::<_, anyhow::Error>(Execution::Completed(result)),
            _ = sigint.recv() => {
                stop.store(true, Ordering::SeqCst);
                Ok(Execution::Signaled(signal_hook::consts::SIGINT))
            }
            _ = sigterm.recv() => {
                stop.store(true, Ordering::SeqCst);
                Ok(Execution::Signaled(signal_hook::consts::SIGTERM))
            }
        }
    })
    .map_err(|error| anyhow::anyhow!("async runtime unavailable: {error}"))??;

    match execution {
        Execution::Signaled(sig) => {
            let _ = signal_hook::low_level::emulate_default_handler(sig);
            let fallback = u8::try_from(128 + sig).unwrap_or(1);
            Ok(ExitCode::from(fallback))
        }
        Execution::Completed(Ok(SafeOutcome::Safe)) => {
            if args.json {
                ctx.out
                    .line(&serde_json::json!({ "safe": true }).to_string())?;
            } else {
                ctx.out.line("true")?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Execution::Completed(Ok(SafeOutcome::Unsafe { reason, .. })) => {
            if args.json {
                ctx.out
                    .line(&serde_json::json!({ "safe": false, "reason": reason }).to_string())?;
            } else {
                ctx.out.line("false")?;
            }
            Ok(ExitCode::FAILURE)
        }
        Execution::Completed(Ok(SafeOutcome::Refused(refusal))) => {
            Err(anyhow::anyhow!("{refusal}"))
        }
        Execution::Completed(Ok(SafeOutcome::Waiting(wait))) => Err(anyhow::anyhow!("{wait}")),
        Execution::Completed(Ok(SafeOutcome::Stopped)) => Ok(ExitCode::FAILURE),
        Execution::Completed(Err(defect)) => Err(anyhow::Error::new(defect)),
    }
}

fn read_input(file: Option<&Path>) -> anyhow::Result<String> {
    let mut bytes = Vec::new();
    match file {
        None => {
            std::io::stdin()
                .read_to_end(&mut bytes)
                .context("cannot read standard input")?;
        }
        Some(path) if path == Path::new("-") => {
            std::io::stdin()
                .read_to_end(&mut bytes)
                .context("cannot read standard input")?;
        }
        Some(path) => {
            bytes =
                std::fs::read(path).with_context(|| format!("cannot read `{}`", path.display()))?;
        }
    }
    String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("input is not valid UTF-8"))
}
