//! Specs commands: `instrukt specs coverage|reserve` and blueprint's
//! `instrukt specs trace`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use instrukt_core::git;
use instrukt_core::ids::TodoSlug;
use instrukt_core::specs::{
    Answer, Coverage, Debt, Ratchet, RequestedBinding, Reservation, Verdict,
};

use crate::commands::not_built;
use crate::context::Ctx;

/// `reserve`: a requested binding is held by another todo.
const RESERVED: u8 = 3;
/// `reserve`: a requested binding names no use case.
const UNKNOWN: u8 = 4;
/// `reserve`: a requested binding lies in a spec pending for several todos
/// with no in-body markers.
const AMBIGUOUS: u8 = 5;

#[derive(Debug, clap::Args)]
pub struct SpecsArgs {
    #[command(subcommand)]
    command: SpecsCommand,
}

#[derive(Debug, clap::Subcommand)]
enum SpecsCommand {
    /// How the project's tests stand against its feature specs' use cases.
    ///
    /// Reads the working tree: every use case and anonymous scenario of the
    /// feature specs, every `spec` binding of the tests. Prints the counts,
    /// then one line per orphan, anonymous scenario, unbound use case,
    /// multi-bound use case and ambiguous claim. Exits zero without
    /// `--check`.
    Coverage(CoverageArgs),
    /// Check a todo's planned bindings against the use cases other todos
    /// hold.
    ///
    /// Prints one answer per binding. Exits 0 when every binding is clear,
    /// 3 when another todo holds one, 4 when one names no use case (no claim
    /// is checked then) and 5 when one lies in a spec pending for several
    /// todos with no in-body markers.
    Reserve(ReserveArgs),
    /// Use cases joined with the structural graph (blueprint module).
    Trace {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Debug, clap::Args)]
struct CoverageArgs {
    /// Fail on any ambiguous claim, and on orphan or anonymous counts above
    /// the committed baseline (`instrukt-coverage-baseline.json`).
    #[arg(long)]
    check: bool,
    /// Record the current orphan and anonymous counts as the baseline.
    #[arg(long, conflicts_with = "check")]
    update_baseline: bool,
}

#[derive(Debug, clap::Args)]
struct ReserveArgs {
    /// The requesting todo's slug.
    #[arg(long)]
    slug: TodoSlug,
    /// A binding the todo's plan declares, `<spec_id>:<UC-id>`.
    #[arg(long = "binding", value_name = "SPEC_ID:UC-ID", required = true, num_args = 1..)]
    bindings: Vec<RequestedBinding>,
}

pub fn run(args: SpecsArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        SpecsCommand::Coverage(args) => coverage(args, ctx),
        SpecsCommand::Reserve(args) => reserve(args, ctx),
        SpecsCommand::Trace { .. } => not_built("specs trace"),
    }
}

fn repo_root(cwd: &Path) -> anyhow::Result<PathBuf> {
    git::worktree_root(cwd)?
        .with_context(|| format!("{} is not inside a git repository", cwd.display()))
}

fn coverage(args: CoverageArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = repo_root(&ctx.cwd)?;
    let coverage = Coverage::measure(&root, &ctx.home)?;
    report(&coverage, ctx)?;
    if args.update_baseline {
        let baseline = coverage.update_baseline()?;
        ctx.out.line(&format!(
            "baseline updated: orphans {}, anonymous {}",
            baseline.orphans, baseline.anonymous
        ))?;
        return Ok(ExitCode::SUCCESS);
    }
    if !args.check {
        return Ok(ExitCode::SUCCESS);
    }
    let check = coverage.check()?;
    for claim in check.ambiguous() {
        ctx.out.line(&format!(
            "check: ambiguous claim in {} (delivered by {})",
            claim.spec,
            join(&claim.delivered_by)
        ))?;
    }
    for ratchet in check.regressions() {
        ctx.out.line(&format!(
            "check: {} {} above baseline {}",
            debt(ratchet),
            ratchet.count,
            ratchet.baseline
        ))?;
    }
    for ratchet in check.tightenable() {
        ctx.out.line(&format!(
            "check: {} {} below baseline {}; the baseline can be tightened with --update-baseline",
            debt(ratchet),
            ratchet.count,
            ratchet.baseline
        ))?;
    }
    if check.passes() {
        ctx.out.line("check: passed")?;
        Ok(ExitCode::SUCCESS)
    } else {
        ctx.out.line("check: failed")?;
        Ok(ExitCode::FAILURE)
    }
}

fn report(coverage: &Coverage, ctx: &mut Ctx) -> anyhow::Result<()> {
    let counts = coverage.counts();
    let out = &mut ctx.out;
    for (label, value) in [
        ("spec files with scenarios", counts.spec_files),
        ("scenarios", counts.scenarios),
        ("bindable use cases", counts.bindable),
        ("anonymous scenarios", counts.anonymous),
        ("bound", counts.bound),
        ("unbound", counts.unbound),
        ("multi-bound", counts.multi_bound),
        ("orphans", counts.orphans),
    ] {
        out.line(&format!("{label}: {value}"))?;
    }
    for orphan in coverage.orphans() {
        let detached = if orphan.is_attached() {
            ""
        } else {
            " (no test function follows it)"
        };
        out.line(&format!(
            "orphan: {} cites {}{detached}",
            orphan.test, orphan.cites
        ))?;
    }
    for scenario in coverage.anonymous() {
        out.line(&format!(
            "anonymous: {} {}",
            scenario.spec(),
            scenario.definition()
        ))?;
    }
    for use_case in coverage.unbound() {
        out.line(&format!("unbound: {use_case}"))?;
    }
    for (use_case, tests) in coverage.multi_bound() {
        out.line(&format!("multi-bound: {use_case} ({tests} tests)"))?;
    }
    for claim in coverage.ambiguous_claims() {
        out.line(&format!(
            "ambiguous claim: {} (delivered by {})",
            claim.spec,
            join(&claim.delivered_by)
        ))?;
    }
    Ok(())
}

fn reserve(args: ReserveArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = repo_root(&ctx.cwd)?;
    let reservation = Reservation::check(&root, &ctx.home, &args.slug, &args.bindings)?;
    match &reservation {
        Reservation::Unknown(unknown) => {
            for request in unknown {
                ctx.out
                    .line(&format!("unknown: {request} names no use case"))?;
            }
        }
        Reservation::Checked(answers) => {
            for (use_case, answer) in answers {
                let line = match answer {
                    Answer::Clear => format!("clear: {use_case}"),
                    Answer::Reserved { holder } => format!(
                        "reserved: {use_case} is held by {holder}, requested by {}",
                        args.slug
                    ),
                    Answer::Ambiguous { delivered_by } => format!(
                        "ambiguous claim: {use_case} (delivered by {})",
                        join(delivered_by)
                    ),
                };
                ctx.out.line(&line)?;
            }
        }
    }
    Ok(match reservation.verdict() {
        Verdict::Clear => ExitCode::SUCCESS,
        Verdict::Reserved => ExitCode::from(RESERVED),
        Verdict::Unknown => ExitCode::from(UNKNOWN),
        Verdict::Ambiguous => ExitCode::from(AMBIGUOUS),
    })
}

fn debt(ratchet: Ratchet) -> &'static str {
    match ratchet.debt {
        Debt::Orphans => "orphans",
        Debt::Anonymous => "anonymous scenarios",
    }
}

fn join(slugs: &[TodoSlug]) -> String {
    slugs
        .iter()
        .map(TodoSlug::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}
