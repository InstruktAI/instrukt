//! One command/action catalog shared by admission and role-aware help.
use crate::{context::Ctx, wiring};
use clap::{ArgMatches, Command};
use instrukt_core::{
    capabilities::{Action, Assignment, Capabilities, RoleClass, Scope},
    config::{Agents, Configuration},
};

pub fn caller(ctx: &Ctx) -> anyhow::Result<Option<(Assignment, Capabilities)>> {
    let config = Configuration::load_global(&ctx.home)?;
    let env = wiring::environment(ctx)?;
    let Some(session) = config.caller(&env)? else {
        return Ok(None);
    };
    let assignment = instrukt_runner::assignments::resolve(
        &ctx.home,
        &session,
        instrukt_todos::work::recorded_assignment,
    )?;
    assignment
        .map(|assignment| {
            let grants = Agents::load(&ctx.home)?.capabilities(&assignment.role)?;
            Ok((assignment, grants))
        })
        .transpose()
}

pub fn action(path: &[&str]) -> Option<Action> {
    use Action::*;
    Some(match path {
        ["docs", "fold"] => DocsWrite,
        ["docs", "index" | "get" | "schema" | "validate"] => DocsRead,
        ["todo", "list" | "show" | "refresh"] => TodoRead,
        ["todo", "artifacts", "list" | "read" | "diff"] => ArtifactRead,
        ["todo", "artifacts", "write"] => ArtifactWrite,
        ["todo", "stamp"] => Stamp,
        ["todo", "prepare" | "work"] => TodoOrchestration,
        ["todo", "attempt" | "bind"] => AssignmentWrite,
        ["todo", "deliver"] => Integration,
        [
            "todo",
            "create" | "update" | "freeze" | "unfreeze" | "remove" | "reactivate",
        ] => TodoWrite,
        ["sessions", "list"] => SessionObserve,
        ["sessions", "resolve-start"] => SessionRecover,
        ["history", "search" | "show"] => HistoryRead,
        ["history", "import-teleclaude" | "extract"] => HistoryRead,
        ["events", "bind"] => StandingBind,
        ["events", "list" | "schema" | "status"] => EventsRead,
        ["agents", "list" | "availability"] => AgentsRead,
        ["agents", "status"] => AgentsWrite,
        ["config", "get" | "show" | "validate"] => ConfigRead,
        ["config", "init" | "patch"] => ConfigWrite,
        ["secrets", "set" | "generate"] => SecretsWrite,
        ["hooks", "status"] => RepositoryRead,
        ["hooks", "install" | "remove"] | ["init"] | ["dev-seed"] => RepositoryWrite,
        ["repo", "commit"] => CodeCommit,
        ["sync"] => Publication,
        ["specs", "coverage" | "reserve" | "trace"] => CodeCheck,
        [
            "blueprint",
            "affected" | "path" | "query" | "explain" | "status",
        ] => CodeCheck,
        ["eval", "report"] => EvalRead,
        ["ticks", "status"] => InstallationRead,
        ["install"] => InstallationWrite,
        ["run"]
        | ["tick"]
        | ["hooks", "run" | "worker"]
        | ["blueprint", "worker"]
        | ["webhooks", ..] => RunnerExecute,
        _ => return None,
    })
}

pub fn check(ctx: &Ctx, matches: &ArgMatches) -> anyhow::Result<()> {
    let mut path = Vec::new();
    let mut leaf = matches;
    while let Some((name, sub)) = leaf.subcommand() {
        path.push(name);
        leaf = sub;
    }
    let action = action(&path).ok_or_else(|| {
        anyhow::anyhow!(
            "command `{}` has no action mapping; report this catalog defect",
            path.join(" ")
        )
    })?;
    let Some((assignment, grants)) = caller(ctx)? else {
        return Ok(());
    };
    if grants.class != RoleClass::Focused {
        return Ok(());
    }
    let config = Configuration::load_at(&ctx.home, &ctx.cwd)?;
    let project = config.project()?.map(|p| p.to_string());
    let in_project = assignment.project.is_some() && assignment.project == project;
    // Clap's parsed identifiers retain their types; raw values are used only as target addresses.
    let raw = |name: &str| {
        leaf.try_get_raw(name)
            .ok()
            .flatten()
            .and_then(|mut v| v.next())
            .and_then(|v| v.to_str())
    };
    let target = raw("todo").or_else(|| raw("slug"));
    let todo_action = matches!(
        action,
        Action::TodoRead
            | Action::TodoWrite
            | Action::ArtifactRead
            | Action::ArtifactWrite
            | Action::Stamp
            | Action::AssignmentWrite
            | Action::Integration
            | Action::TodoOrchestration
    );
    let own_todo = match (target.filter(|_| todo_action), project.as_deref()) {
        (Some(target), Some(project)) => {
            instrukt_todos::work::target_identity(&ctx.home, project, target)?.as_ref()
                == assignment.todo.as_ref()
                && assignment.todo.is_some()
        }
        _ => false,
    };
    let workspace = assignment.workspace.as_ref().is_some_and(|workspace| {
        workspace
            .canonicalize()
            .ok()
            .zip(
                instrukt_core::git::run(&ctx.cwd, ["rev-parse", "--show-toplevel"])
                    .ok()
                    .and_then(|root| std::path::Path::new(root.trim()).canonicalize().ok()),
            )
            .is_some_and(|(workspace, root)| root == workspace)
    });
    use Action::*;
    let owned = match action {
        ArtifactRead | ArtifactWrite | Stamp | AssignmentWrite | Integration
        | TodoOrchestration => own_todo,
        TodoRead | TodoWrite => {
            if path == ["todo", "create"] {
                in_project
            } else {
                own_todo
            }
        }
        CodeCommit | DocsWrite | RepositoryWrite => workspace,
        Publication => in_project,
        SessionObserve => true,
        SessionRecover => raw("key") == Some(assignment.key.as_str()),
        StandingBind => {
            raw("procedure") == assignment.procedure.as_deref() && assignment.procedure.is_some()
        }
        _ => false,
    };
    if !grants.admits(action, owned) {
        anyhow::bail!(
            "{}",
            grants.refusal(
                action,
                &format!(
                    "{}{}",
                    path.join(" "),
                    target.map(|s| format!(" ({s})")).unwrap_or_default()
                )
            )
        );
    }
    Ok(())
}

pub fn help(mut command: Command, path: &mut Vec<String>, caps: &Capabilities) -> Command {
    let children = command.get_subcommands().cloned().collect::<Vec<_>>();
    for child in children {
        let name = child.get_name().to_owned();
        path.push(name.clone());
        let rendered = help(child, path, caps);
        path.pop();
        command = command.mut_subcommand(name, |_| rendered);
    }
    if let Some(action) = action(&path.iter().map(String::as_str).collect::<Vec<_>>()) {
        let scope = caps.scope(action);
        if scope.is_none() && caps.class == RoleClass::Focused {
            command = command.hide(true);
        } else {
            command = command.after_help(format!(
                "Action: {} · scope: {}. Object claims, phase and writer constraints apply.",
                action.name(),
                match scope {
                    Some(Scope::Own) => "own",
                    Some(Scope::Any) => "any",
                    None => "outside usual role surface (awareness only)",
                }
            ));
        }
    } else if !path.is_empty() && command.has_subcommands() {
        let hidden = command.get_subcommands().all(|sub| sub.is_hide_set());
        command = command.hide(hidden);
    }
    command
}
