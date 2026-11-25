//! One command/action catalog shared by admission and role-aware help.
use crate::{context::Ctx, wiring};
use clap::{ArgMatches, Command};
use instrukt_core::{
    capabilities::{Action, Assignment, Capabilities, RoleClass, Scope},
    config::{Agents, Configuration},
    ids::ProjectSlug,
};

#[derive(Debug)]
pub struct PermissionRefused(String);
impl std::fmt::Display for PermissionRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PermissionRefused {}

pub fn caller(ctx: &Ctx) -> anyhow::Result<Option<(Assignment, Capabilities)>> {
    let inspection = ctx.home.for_inspection();
    let config = Configuration::load_global(&inspection)?;
    let env = wiring::environment(ctx)?;
    let Some(session) = config.caller(&env)? else {
        return Ok(None);
    };
    let assignment = instrukt_runner::assignments::resolve(
        &ctx.home,
        &session,
        instrukt_todos::work::recorded_assignment,
        instrukt_company::conversations::conversation_assignment,
    )?;
    assignment
        .map(|assignment| {
            let grants = Agents::load(&ctx.home)?.capabilities(assignment.role.as_str())?;
            Ok((assignment, grants))
        })
        .transpose()
}

pub fn action(path: &[&str]) -> Option<Action> {
    use Action::*;
    Some(match path {
        ["status"] => ConfigRead,
        ["bugs", "suppressions" | "list"] => TodoRead,
        ["bugs", "report"] => BugReport,
        ["bugs", "suppress" | "unsuppress"] => TodoWrite,
        ["docs", "fold" | "render" | "plugin"] => DocsWrite,
        ["docs", "index" | "get" | "schema" | "validate"] => DocsRead,
        ["todo", "list" | "show" | "refresh"] => TodoRead,
        ["todo", "artifacts", "list" | "read" | "diff"] => ArtifactRead,
        ["todo", "artifacts", "write"] => ArtifactWrite,
        ["todo", "stamp"] => Stamp,
        ["todo", "work"] => TodoOrchestration,
        ["todo", "attempt" | "bind"] => AssignmentWrite,
        ["todo", "deliver"] => Integration,
        [
            "todo",
            "create" | "update" | "freeze" | "unfreeze" | "remove" | "reactivate",
        ] => TodoWrite,
        ["discuss", "open" | "close"] => ConversationLink,
        ["company", "reply"] => CompanyReply,
        ["sessions", "list"] => SessionObserve,
        ["history", "search" | "show"] => HistoryRead,
        ["history", "extract"] => HistoryRead,
        ["events", "bind"] => StandingBind,
        ["events", "list" | "schema" | "status" | "dead-letters"] => EventsRead,
        ["agents", "list" | "availability"] => AgentsRead,
        ["agents", "status"] => AgentsWrite,
        ["config", "get" | "show" | "validate" | "schema"] => ConfigRead,
        ["config", "init" | "patch"] => ConfigWrite,
        ["secrets", "set" | "generate"] => SecretsWrite,
        ["hooks", "status"] => RepositoryRead,
        ["hooks", "install" | "remove"] | ["init"] | ["dev-seed"] => RepositoryWrite,
        ["repo", "commit" | "reclaim"] | ["hooks", "run"] => CodeCommit,
        ["sync"] => Publication,
        ["specs", "coverage" | "reserve" | "trace"] => CodeCheck,
        [
            "blueprint",
            "affected" | "path" | "query" | "explain" | "status",
        ] => CodeCheck,
        ["eval", "report"] => EvalRead,
        ["review", "record" | "exempt"] => Stamp,
        ["review", "show"] => ArtifactRead,
        ["review", "export" | "import"] => InstallationWrite,
        [
            "mcp",
            "list" | "list-tools" | "call" | "clients" | "discover" | "check-ingress" | "sign-out",
        ]
        | ["mcp", "issuer", "init" | "publish" | "show"] => McpAccess,
        ["ticks", "status"] => InstallationRead,
        ["install"] => InstallationWrite,
        ["run"] | ["tick"] | ["hooks", "worker"] | ["blueprint", "worker"] | ["webhooks", ..] => {
            RunnerExecute
        }
        _ => return None,
    })
}

/// The command path `matches` selects, and its leaf's matches.
fn command_path(matches: &ArgMatches) -> (Vec<&str>, &ArgMatches) {
    let mut path = Vec::new();
    let mut leaf = matches;
    while let Some((name, sub)) = leaf.subcommand() {
        path.push(name);
        leaf = sub;
    }
    (path, leaf)
}

/// The commands that work on the repository the caller stands in and need
/// nothing else of the computer: `docs render [--check]`, `docs plugin
/// [--check | --validate]`, `hooks run`, the
/// detached `hooks worker` a run starts, and `hooks status`. They load no
/// project layer and skip the layer notices, so a root document that cannot
/// be read still has its runs recorded and shown; whether the caller is in
/// its project comes from what the working tree's root document is.
pub fn repository_local(matches: &ArgMatches) -> bool {
    matches!(
        command_path(matches).0.as_slice(),
        ["docs", "render" | "plugin"] | ["hooks", "run" | "worker" | "status"]
    )
}

/// The documentation reads, as the catalog classifies them: they inspect the
/// state database, so an older one is read through its migrated in-memory
/// image and the file stays as found
/// (`project/instrukt/spec/feature/work/state-database`).
pub fn state_inspection(matches: &ArgMatches) -> bool {
    action(&command_path(matches).0) == Some(Action::DocsRead)
}

/// The commands that write no log: `docs render [--check]` and `docs plugin
/// [--check | --validate]`, so a check writes nothing anywhere and errors go
/// to stderr alone.
pub fn untraced(matches: &ArgMatches) -> bool {
    matches!(
        command_path(matches).0.as_slice(),
        ["docs", "render" | "plugin"]
    )
}

/// Whether the caller's working tree's root document is a project layer,
/// and which project it declares; an unreadable or absent document, or a
/// company's, is no project.
fn root_project(ctx: &Ctx) -> anyhow::Result<Option<instrukt_core::ids::ProjectSlug>> {
    let root = instrukt_core::git::worktree_root(&ctx.cwd)?.unwrap_or_else(|| ctx.cwd.clone());
    let path = root.join(instrukt_core::config::CONFIG_FILE_NAME);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(anyhow::Error::new(error).context(path.display().to_string())),
    };
    Ok(
        match instrukt_core::config::classify_root(bytes.as_deref()) {
            instrukt_core::config::RootDocument::ProjectLayer(project) => project,
            _ => None,
        },
    )
}

/// The commands that know only once they run whether they write: `mcp
/// clients` lists read-only in an admin's direct message and materializes
/// registrations for the local operator
/// (`project/instrukt/spec/feature/runner/mcp-access`). They enter reading
/// and turn writing themselves ([`crate::context::Ctx::enter_writing`]).
pub fn deferred_writing(matches: &ArgMatches) -> bool {
    command_path(matches).0 == ["mcp", "clients"]
}

pub fn strict_inspection(matches: &ArgMatches) -> bool {
    let (path, leaf) = command_path(matches);
    matches!(
        path.as_slice(),
        ["events", "status" | "dead-letters"] | ["ticks", "status"] | ["status"]
    ) || (path == ["install"] && leaf.get_flag("check"))
}
/// The owning project a work read names with `--project`: the project whose
/// layer it reads, wherever the caller stands.
pub fn named_project(matches: &ArgMatches) -> Option<ProjectSlug> {
    let (path, leaf) = command_path(matches);
    matches!(
        path.as_slice(),
        ["todo", "list"] | ["bugs", "list" | "suppressions" | "report"]
    )
    .then(|| leaf.get_one::<ProjectSlug>("project").cloned())
    .flatten()
}
pub fn read_only(matches: &ArgMatches) -> bool {
    let (path, leaf) = command_path(matches);
    if path == ["webhooks", "serve"] {
        return true;
    }
    if path == ["install"] {
        return leaf.get_flag("check");
    }
    // A report that is not approved files nothing: it reads, as a listing
    // does.
    if path == ["bugs", "report"] {
        return !leaf.get_flag("approved");
    }
    // A coverage measurement reads; updating its baseline writes.
    if path == ["specs", "coverage"] {
        return !leaf.get_flag("update_baseline");
    }
    matches!(
        path.as_slice(),
        ["status"]
            | ["bugs", "suppressions" | "list"]
            | ["config", "get" | "show" | "validate" | "schema"]
            | ["docs", "get" | "index" | "schema" | "validate"]
            | ["events", "list" | "schema" | "status" | "dead-letters"]
            | ["agents", "list" | "availability"]
            | ["hooks", "status"]
            | ["ticks", "status"]
            | ["sessions", "list"]
            | ["history", "search" | "show"]
            | ["todo", "list" | "show"]
            | ["review", "show"]
            | ["todo", "artifacts", "list" | "read" | "diff"]
            | ["specs", "trace" | "reserve"]
            | [
                "mcp",
                "list" | "list-tools" | "call" | "discover" | "check-ingress"
            ]
            | ["mcp", "issuer", "show"]
            | [
                "blueprint",
                "status" | "query" | "path" | "explain" | "affected"
            ]
    )
}

pub fn check(ctx: &Ctx, matches: &ArgMatches) -> anyhow::Result<()> {
    let (path, leaf) = command_path(matches);
    let action = if path == ["install"] && leaf.get_flag("check") {
        Some(Action::InstallationRead)
    } else if path == ["docs", "render"] && leaf.get_flag("check")
        || path == ["docs", "plugin"] && (leaf.get_flag("check") || leaf.get_flag("validate"))
    {
        Some(Action::DocsRead)
    } else {
        action(&path)
    }
    .ok_or_else(|| {
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
    if action == Action::ConfigRead {
        if !grants.admits(action, false) {
            return Err(PermissionRefused(grants.refusal(action, &path.join(" "))).into());
        }
        return Ok(());
    }
    // Reclaiming acts on the worktree the assignment records, wherever the
    // caller stands: nothing of the caller's directory is read for it.
    let reclaim = path == ["repo", "reclaim"];
    let config = if repository_local(matches) || reclaim {
        None
    } else {
        Some(Configuration::load_at(&ctx.home, &ctx.cwd)?)
    };
    let project = match &config {
        Some(config) => config.project()?,
        None if reclaim => None,
        None => root_project(ctx)?,
    };
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
    let own_todo = match (target.filter(|_| todo_action), project.as_ref()) {
        (Some(target), Some(project)) => {
            instrukt_todos::work::target_identity(&ctx.home, project.as_str(), target)?.as_ref()
                == assignment.todo.as_ref()
                && assignment.todo.is_some()
        }
        _ => false,
    };
    let workspace = if reclaim {
        assignment.workspace.is_some()
    } else {
        recorded_workspace_is_cwd(ctx, &assignment)
    };
    use Action::*;
    let owned = match action {
        // Review evidence addresses the project the caller stands in.
        ArtifactRead | Stamp if path.first() == Some(&"review") => in_project,
        ArtifactRead | ArtifactWrite | Stamp | AssignmentWrite | Integration
        | TodoOrchestration => own_todo,
        TodoRead | TodoWrite => {
            if path == ["todo", "create"] {
                in_project
            } else {
                own_todo
            }
        }
        // A report addresses the project it names, or the one the caller
        // stands in; at `own` that is the assignment's project.
        BugReport => {
            let named = raw("project")
                .map(instrukt_core::ids::ProjectSlug::parse)
                .transpose()?;
            assignment.project.is_some() && assignment.project == named.or_else(|| project.clone())
        }
        CodeCommit | DocsWrite | RepositoryWrite => workspace,
        Publication => in_project,
        SessionObserve | ConversationLink => true,
        CompanyReply => assignment.company.is_some(),
        // `own` reaches the servers of the assignment's own company, which
        // are all that company's sessions see; a machine-wide server that
        // carries a login is a machine-wide object no assignment owns.
        McpAccess => {
            assignment.company.is_some()
                && raw("company").is_none_or(|asked| {
                    assignment.company.as_ref().map(|own| own.as_str()) == Some(asked)
                })
        }
        StandingBind => {
            if let Some(procedure) = assignment
                .procedure
                .as_ref()
                .filter(|id| Some(id.as_str()) == raw("procedure"))
                && let Some(config) = &config
            {
                match instrukt_runner::standing::binding_home(
                    &ctx.home,
                    config,
                    procedure,
                    raw("reservation"),
                    raw("company")
                        .map(instrukt_core::ids::ProjectSlug::parse)
                        .transpose()?
                        .as_ref(),
                )? {
                    instrukt_runner::standing::BindingHome::Project(project) => {
                        assignment.project.as_ref() == Some(&project)
                    }
                    instrukt_runner::standing::BindingHome::Platform => assignment.platform,
                    instrukt_runner::standing::BindingHome::Company(company) => {
                        assignment.company.as_ref() == Some(&company)
                    }
                }
            } else {
                false
            }
        }
        _ => false,
    };
    if !grants.admits(action, owned) {
        let mut refusal = grants.refusal(
            action,
            &format!(
                "{}{}",
                path.join(" "),
                target.map(|s| format!(" ({s})")).unwrap_or_default()
            ),
        );
        if reclaim && (assignment.todo.is_none() || assignment.workspace.is_none()) {
            refusal.push_str(&format!(
                "; reclaiming acts on the todo worktree an assignment records, and this session's assignment records none: {}",
                crate::commands::repository::reclaim_door(Some(&assignment))
            ));
        }
        return Err(PermissionRefused(refusal).into());
    }
    Ok(())
}

/// Whether the caller stands in the workspace its assignment records: the
/// repository root of its working directory is that workspace.
fn recorded_workspace_is_cwd(ctx: &Ctx, assignment: &Assignment) -> bool {
    assignment.workspace.as_ref().is_some_and(|workspace| {
        workspace
            .canonicalize()
            .ok()
            .zip(
                instrukt_core::git::run(&ctx.cwd, ["rev-parse", "--show-toplevel"])
                    .ok()
                    .and_then(|root| std::path::Path::new(root.trim()).canonicalize().ok()),
            )
            .is_some_and(|(workspace, root)| root == workspace)
    })
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
        if caps.class == RoleClass::Focused
            && (scope.is_none()
                || (action == Action::ConversationLink
                    && path.last().is_some_and(|name| name == "open")))
        {
            command = command
                .hide(true)
                .override_help(caps.refusal(action, &path.join(" ")));
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
