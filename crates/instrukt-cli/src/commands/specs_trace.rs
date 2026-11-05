//! Blueprint's `instrukt specs trace <spec_id> <UC-id>` and
//! `instrukt specs trace --file <path>`
//! (`docs/project/spec/feature/blueprint/use-case-trace.md`).
//!
//! The subcommand's one-line about stays on `SpecsCommand::Trace` in
//! `specs.rs`; this file owns its arguments and answers.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use instrukt_core::git;
use instrukt_core::ids::{SnippetId, UseCaseId, UseCaseRef};
use instrukt_core::specs::Coverage;

use crate::commands::blueprint::{UNAVAILABLE, dependent, latest_graph, require_module};
use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct TraceArgs {
    /// The spec id of the use case to trace.
    #[arg(required_unless_present = "file", conflicts_with = "file")]
    spec_id: Option<String>,
    /// The use-case id (`UC-…`) within that spec.
    #[arg(required_unless_present = "file")]
    use_case: Option<String>,
    /// Trace a file instead: the tests that reach it and the use cases they
    /// bind. Relative to the repository root.
    #[arg(long)]
    file: Option<String>,
}

pub fn run(args: TraceArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = git::worktree_root(&ctx.cwd)?;
    require_module(ctx, root.as_deref())?;
    let Some(root) = root else {
        bail!("not inside a git repository; `specs trace` traces a project's use cases");
    };
    let coverage = Coverage::measure(&root, &ctx.home)
        .context("cannot read the project's use cases and bindings")?;
    match (args.file, args.spec_id, args.use_case) {
        (Some(file), _, _) => trace_file(ctx, &root, &coverage, &file),
        (None, Some(spec), Some(use_case)) => {
            trace_use_case(ctx, &root, &coverage, &spec, &use_case)
        }
        _ => bail!("give a spec id and a use-case id, or --file <path>"),
    }
}

fn trace_use_case(
    ctx: &mut Ctx,
    root: &Path,
    coverage: &Coverage,
    spec: &str,
    use_case: &str,
) -> anyhow::Result<ExitCode> {
    let reference = UseCaseRef {
        spec: SnippetId::parse(spec)
            .map_err(|err| anyhow::anyhow!("unknown spec id `{spec}`: {err}"))?,
        use_case: UseCaseId::parse(use_case)
            .map_err(|err| anyhow::anyhow!("unknown use-case id `{use_case}`: {err}"))?,
    };
    if coverage.inventory().use_case(&reference).is_none() {
        if !coverage
            .inventory()
            .use_cases()
            .iter()
            .any(|known| known.reference().spec == reference.spec)
        {
            bail!(
                "unknown spec id `{spec}`: no feature spec of this project defines use cases under it"
            );
        }
        bail!("unknown use case `{reference}`: the spec defines no {use_case}");
    }
    let latest = latest_graph(ctx, Some(root))?;
    let trace = instrukt_blueprint::trace_use_case(
        latest.as_ref().map(|(_, latest)| &latest.graph),
        coverage.bindings().bindings(),
        &reference,
    );
    match &latest {
        Some((_, latest)) => ctx.out.line(&format!(
            "trace of {reference} from the blueprint graph of {}",
            latest.sha
        ))?,
        None => ctx.out.line(&format!(
            "trace of {reference}: no blueprint graph of this project yet; the binding tests only"
        ))?,
    }
    if trace.tests.is_empty() {
        ctx.out.line(&format!("no test binds {reference}"))?;
        return Ok(ExitCode::SUCCESS);
    }
    ctx.out.line("tests binding it:")?;
    for test in &trace.tests {
        ctx.out.line(&format!("  {test}"))?;
    }
    if let Some(reached) = &trace.reached {
        if reached.is_empty() {
            ctx.out.line("the tests reach no code outside test files")?;
        } else {
            ctx.out.line("code they reach:")?;
            for node in reached {
                ctx.out.line(&format!("  {}", dependent(node)))?;
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn trace_file(
    ctx: &mut Ctx,
    root: &Path,
    coverage: &Coverage,
    file: &str,
) -> anyhow::Result<ExitCode> {
    let Some((root, latest)) = latest_graph(ctx, Some(root))? else {
        ctx.out.line(UNAVAILABLE)?;
        return Ok(ExitCode::SUCCESS);
    };
    let file = instrukt_blueprint::repo_relative(&root, file);
    let reaching =
        instrukt_blueprint::trace_file(&latest.graph, coverage.bindings().bindings(), &file);
    ctx.out.line(&format!(
        "trace of {file} from the blueprint graph of {}",
        latest.sha
    ))?;
    if reaching.is_empty() {
        ctx.out.line(&format!("no tests reach {file}"))?;
        return Ok(ExitCode::SUCCESS);
    }
    ctx.out
        .line("tests reaching it, with the use cases they bind:")?;
    for test in &reaching {
        let use_cases: Vec<String> = test.use_cases.iter().map(ToString::to_string).collect();
        ctx.out
            .line(&format!("  {}: {}", test.test, use_cases.join(", ")))?;
    }
    Ok(ExitCode::SUCCESS)
}
