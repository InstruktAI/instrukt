//! Loading and validating a flavor's `workflow/` directory
//! (`docs/local/spec/domain/workflow.md`, "Flavor-facing contract").

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use instrukt_core::ids::{RoleName, SnippetId};
use serde_yaml_ng::{Mapping, Value};

use super::_condition::Condition;
use super::_finding::{Finding, Problem};
use super::_names::{ActionId, EffectName, FlowName, StateName};
use super::_render::placeholders;

/// The file holding actions, effects and roles.
const ACTIONS_FILE: &str = "actions.yaml";

/// A flavor could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum FlavorError {
    #[error("cannot read the flavor directory {dir}: {source}")]
    Unreadable {
        dir: PathBuf,
        source: std::io::Error,
    },
    #[error("the flavor at {dir} is invalid:\n{}", list(findings))]
    Invalid {
        dir: PathBuf,
        findings: Vec<Finding>,
    },
}

fn list(findings: &[Finding]) -> String {
    findings
        .iter()
        .map(|finding| format!("  {finding}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A configured action: its command, the procedure that command names (if
/// any), and its template's text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Action {
    pub command: String,
    pub procedure: Option<SnippetId>,
    pub template: String,
}

/// The procedure id a command names as written: the word after
/// `--procedure`, or after `docs get`; the first such naming wins.
fn named_procedure(command: &str) -> Option<&str> {
    let words: Vec<&str> = command.split_whitespace().collect();
    words.iter().enumerate().find_map(|(i, word)| match *word {
        "--procedure" => words.get(i + 1).copied(),
        "docs" if words.get(i + 1) == Some(&"get") => words.get(i + 2).copied(),
        _ => None,
    })
}

/// A configured effect.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Effect {
    pub command: String,
    pub done_when: Option<Condition>,
}

/// One ordered transition of a definition, its references resolved. Actions
/// and effects are shared by every transition naming them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Transition {
    pub from: StateName,
    pub to: StateName,
    pub when: Condition,
    pub action: (ActionId, Arc<Action>),
    pub effects: Vec<(EffectName, Arc<Effect>)>,
}

/// A flow definition. States are validated at load; evaluation needs only
/// the initial state and the ordered transitions.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Definition {
    /// The role of the flow's orchestrator: it owns every action whose
    /// command names no procedure.
    pub orchestrator: RoleName,
    pub initial: StateName,
    pub transitions: Vec<Transition>,
}

/// A loaded, valid flavor: every reference resolves.
#[derive(Debug, Clone, PartialEq)]
pub struct Flavor {
    pub(crate) roles: Vec<RoleName>,
    pub(crate) actions: BTreeMap<ActionId, Arc<Action>>,
    pub(crate) flows: BTreeMap<FlowName, Definition>,
}

impl Flavor {
    /// Loads and validates the flavor in `dir`, a published domain's
    /// `workflow/` directory. Every finding is reported, each naming its
    /// file and location.
    pub fn load(dir: &Path) -> Result<Flavor, FlavorError> {
        let entries = std::fs::read_dir(dir).map_err(|source| FlavorError::Unreadable {
            dir: dir.to_owned(),
            source,
        })?;
        let mut definition_files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| FlavorError::Unreadable {
                dir: dir.to_owned(),
                source,
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name != ACTIONS_FILE && name.ends_with(".yaml") && entry.path().is_file() {
                definition_files.push(name.to_owned());
            }
        }
        definition_files.sort();

        let mut findings = Vec::new();
        let catalog = read_yaml(dir, ACTIONS_FILE, &mut findings)
            .and_then(|doc| parse_actions(dir, &doc, &mut findings));
        let mut flows = BTreeMap::new();
        for file in &definition_files {
            let stem = file.trim_end_matches(".yaml");
            let Some(doc) = read_yaml(dir, file, &mut findings) else {
                continue;
            };
            let mut report = Report::new(file, &mut findings);
            if let Some(definition) = parse_definition(stem, &doc, catalog.as_ref(), &mut report) {
                flows.insert(FlowName::new(stem), definition);
            }
        }
        match catalog {
            Some(catalog) if findings.is_empty() => Ok(Flavor {
                roles: catalog.roles,
                // Without findings, every declared action is valid.
                actions: catalog
                    .actions
                    .into_iter()
                    .filter_map(|(id, action)| action.map(|action| (id, action)))
                    .collect(),
                flows,
            }),
            _ => Err(FlavorError::Invalid {
                dir: dir.to_owned(),
                findings,
            }),
        }
    }

    /// The roles `actions.yaml` declares, in declaration order.
    pub fn roles(&self) -> &[RoleName] {
        &self.roles
    }

    /// Each action whose command names a procedure, with that procedure, in
    /// action-id order.
    pub fn action_procedures(&self) -> impl Iterator<Item = (&ActionId, &SnippetId)> {
        self.actions
            .iter()
            .filter_map(|(id, action)| action.procedure.as_ref().map(|p| (id, p)))
    }

    /// The orchestrator role a flow declares; `None` for an unknown flow.
    pub fn orchestrator(&self, flow: &str) -> Option<&RoleName> {
        self.flows
            .iter()
            .find(|(name, _)| name.as_str() == flow)
            .map(|(_, definition)| &definition.orchestrator)
    }

    /// The flows the flavor defines (`workflow/<flow>.yaml`), in name order.
    pub fn flows(&self) -> impl Iterator<Item = &FlowName> {
        self.flows.keys()
    }
}

/// Collects findings for one file.
struct Report<'f> {
    file: PathBuf,
    findings: &'f mut Vec<Finding>,
}

impl<'f> Report<'f> {
    fn new(file: &str, findings: &'f mut Vec<Finding>) -> Self {
        Self {
            file: PathBuf::from(file),
            findings,
        }
    }

    fn push(&mut self, location: &str, problem: Problem) {
        self.findings.push(Finding {
            file: self.file.clone(),
            location: location.to_owned(),
            problem,
        });
    }

    fn count(&self) -> usize {
        self.findings.len()
    }

    /// Parses a condition at `at`, recording its problems here.
    fn condition(&mut self, value: &Value, at: &str) -> Option<Condition> {
        let mut problems = Vec::new();
        let condition = Condition::parse(value, at, &mut problems);
        for (location, problem) in problems {
            self.push(&location, problem);
        }
        condition
    }

    fn string<'v>(&mut self, value: &'v Value, at: &str) -> Option<&'v str> {
        let text = value.as_str();
        if text.is_none() {
            self.push(at, Problem::Shape("a string"));
        }
        text
    }
}

fn read_yaml(dir: &Path, file: &str, findings: &mut Vec<Finding>) -> Option<Value> {
    let mut report = Report::new(file, findings);
    let text = match std::fs::read_to_string(dir.join(file)) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            report.push("", Problem::MissingFile);
            return None;
        }
        Err(err) => {
            report.push("", Problem::Unreadable(err.to_string()));
            return None;
        }
    };
    match serde_yaml_ng::from_str(&text) {
        Ok(value) => Some(value),
        Err(err) => {
            let location = err
                .location()
                .map(|at| format!("{}:{}", at.line(), at.column()))
                .unwrap_or_default();
            report.push(&location, Problem::Yaml(err.to_string()));
            None
        }
    }
}

/// What `actions.yaml` declares. An entry that is declared but invalid is
/// `None`: its own finding is reported once, and references to it are not
/// reported again as unknown.
struct Catalog {
    roles: Vec<RoleName>,
    actions: BTreeMap<ActionId, Option<Arc<Action>>>,
    effects: BTreeMap<EffectName, Option<Arc<Effect>>>,
}

fn parse_actions(dir: &Path, doc: &Value, findings: &mut Vec<Finding>) -> Option<Catalog> {
    let mut report = Report::new(ACTIONS_FILE, findings);
    let Value::Mapping(root) = doc else {
        report.push(
            "",
            Problem::Shape("a map of actions, `roles` and `effects`"),
        );
        return None;
    };
    let mut catalog = Catalog {
        roles: Vec::new(),
        actions: BTreeMap::new(),
        effects: BTreeMap::new(),
    };
    let mut saw_roles = false;
    for (key, value) in root {
        let Some(key) = key.as_str() else {
            report.push("", Problem::Shape("string keys at the file root"));
            continue;
        };
        match key {
            "roles" => {
                saw_roles = true;
                catalog.roles = parse_roles(value, &mut report);
            }
            "effects" => catalog.effects = parse_effects(value, &mut report),
            id => {
                let action = parse_action(dir, id, value, &mut report).map(Arc::new);
                catalog.actions.insert(ActionId::new(id), action);
            }
        }
    }
    if !saw_roles {
        report.push("", Problem::MissingKey("roles"));
    }
    Some(catalog)
}

fn parse_roles(value: &Value, report: &mut Report<'_>) -> Vec<RoleName> {
    let Value::Sequence(items) = value else {
        report.push("roles", Problem::Shape("a list of role names"));
        return Vec::new();
    };
    if items.is_empty() {
        report.push("roles", Problem::NoRoles);
    }
    let mut roles = Vec::with_capacity(items.len());
    let mut seen = BTreeSet::new();
    for (i, item) in items.iter().enumerate() {
        let at = format!("roles[{i}]");
        let Some(name) = report.string(item, &at) else {
            continue;
        };
        match RoleName::parse(name) {
            Ok(role) if seen.insert(role.clone()) => roles.push(role),
            Ok(_) => report.push(&at, Problem::DuplicateRole(name.to_owned())),
            Err(err) => report.push(&at, Problem::InvalidRole(err.to_string())),
        }
    }
    roles
}

fn parse_effects(
    value: &Value,
    report: &mut Report<'_>,
) -> BTreeMap<EffectName, Option<Arc<Effect>>> {
    let mut effects = BTreeMap::new();
    let Value::Mapping(map) = value else {
        report.push("effects", Problem::Shape("a map of effects"));
        return effects;
    };
    for (key, value) in map {
        let Some(name) = key.as_str() else {
            report.push("effects", Problem::Shape("effect names as keys"));
            continue;
        };
        let effect = parse_effect(value, &format!("effects.{name}"), report);
        effects.insert(EffectName::new(name), effect.map(Arc::new));
    }
    effects
}

fn parse_effect(value: &Value, at: &str, report: &mut Report<'_>) -> Option<Effect> {
    let before = report.count();
    let fields = fields(value, at, &["command", "done_when"], report)?;
    let command = required_string(fields, "command", at, report);
    if let Some(command) = command
        && placeholders(command).any(|p| p == "brief")
    {
        report.push(&format!("{at}.command"), Problem::BriefUnavailable);
    }
    let done_when = fields
        .get("done_when")
        .and_then(|cond| report.condition(cond, &format!("{at}.done_when")));
    if report.count() > before {
        return None;
    }
    Some(Effect {
        command: command?.to_owned(),
        done_when,
    })
}

fn parse_action(dir: &Path, id: &str, value: &Value, report: &mut Report<'_>) -> Option<Action> {
    let before = report.count();
    let fields = fields(value, id, &["command", "template"], report)?;
    let command = required_string(fields, "command", id, report);
    let template_path = required_string(fields, "template", id, report);
    let procedure =
        command
            .and_then(named_procedure)
            .and_then(|named| match SnippetId::parse(named) {
                Ok(procedure) => Some(procedure),
                Err(err) => {
                    report.push(
                        &format!("{id}.command"),
                        Problem::InvalidProcedure {
                            id: named.to_owned(),
                            reason: err.rule.to_owned(),
                        },
                    );
                    None
                }
            });
    let template = template_path.and_then(|path| read_template(dir, path, id, report));
    if report.count() > before {
        return None;
    }
    Some(Action {
        command: command?.to_owned(),
        procedure,
        template: template?,
    })
}

fn read_template(dir: &Path, path: &str, id: &str, report: &mut Report<'_>) -> Option<String> {
    let at = format!("{id}.template");
    let relative = Path::new(path);
    let contained = !path.is_empty()
        && relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if !contained {
        report.push(&at, Problem::TemplateOutside(path.to_owned()));
        return None;
    }
    let text = match std::fs::read_to_string(dir.join(relative)) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            report.push(&at, Problem::TemplateMissing(path.to_owned()));
            return None;
        }
        Err(err) => {
            report.push(&at, Problem::Unreadable(format!("{path}: {err}")));
            return None;
        }
    };
    if placeholders(&text).any(|p| p == "brief") {
        report.push(&at, Problem::BriefUnavailable);
        return None;
    }
    Some(text)
}

fn parse_definition(
    stem: &str,
    doc: &Value,
    catalog: Option<&Catalog>,
    report: &mut Report<'_>,
) -> Option<Definition> {
    let fields = fields(
        doc,
        "",
        &["flow", "orchestrator", "initial", "states", "transitions"],
        report,
    )?;
    let before = report.count();
    if let Some(flow) = required_string(fields, "flow", "", report)
        && flow != stem
    {
        report.push(
            "flow",
            Problem::FlowMismatch {
                declared: flow.to_owned(),
                expected: stem.to_owned(),
            },
        );
    }
    let orchestrator = required_string(fields, "orchestrator", "", report).and_then(|raw| {
        match RoleName::parse(raw) {
            Ok(role) => match catalog {
                Some(catalog) if !catalog.roles.is_empty() && !catalog.roles.contains(&role) => {
                    report.push(
                        "orchestrator",
                        Problem::UndeclaredOrchestrator(raw.to_owned()),
                    );
                    None
                }
                _ => Some(role),
            },
            Err(err) => {
                report.push("orchestrator", Problem::InvalidRole(err.to_string()));
                None
            }
        }
    });
    let states = parse_states(fields.get("states"), report);
    let initial = required_string(fields, "initial", "", report);
    if let Some(initial) = initial
        && !states.contains(initial)
    {
        report.push("initial", Problem::UndeclaredState(initial.to_owned()));
    }
    let transitions = match fields.get("transitions") {
        None => {
            report.push("", Problem::MissingKey("transitions"));
            Vec::new()
        }
        Some(Value::Sequence(items)) => items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                parse_transition(item, &format!("transitions[{i}]"), &states, catalog, report)
            })
            .collect(),
        Some(_) => {
            report.push("transitions", Problem::Shape("a list of transitions"));
            Vec::new()
        }
    };
    if report.count() > before {
        return None;
    }
    Some(Definition {
        orchestrator: orchestrator?,
        initial: StateName::new(initial?),
        transitions,
    })
}

fn parse_states(value: Option<&Value>, report: &mut Report<'_>) -> BTreeSet<String> {
    let mut states = BTreeSet::new();
    match value {
        None => report.push("", Problem::MissingKey("states")),
        Some(Value::Sequence(items)) => {
            for (i, item) in items.iter().enumerate() {
                let at = format!("states[{i}]");
                if let Some(state) = report.string(item, &at)
                    && !states.insert(state.to_owned())
                {
                    report.push(&at, Problem::DuplicateState(state.to_owned()));
                }
            }
        }
        Some(_) => report.push("states", Problem::Shape("a list of state names")),
    }
    states
}

fn parse_transition(
    value: &Value,
    at: &str,
    states: &BTreeSet<String>,
    catalog: Option<&Catalog>,
    report: &mut Report<'_>,
) -> Option<Transition> {
    let fields = fields(
        value,
        at,
        &["from", "to", "when", "action", "effects"],
        report,
    )?;
    let state = |key: &'static str, report: &mut Report<'_>| {
        let name = required_string(fields, key, at, report)?;
        if states.contains(name) {
            Some(StateName::new(name))
        } else {
            report.push(
                &format!("{at}.{key}"),
                Problem::UndeclaredState(name.to_owned()),
            );
            None
        }
    };
    let from = state("from", report);
    let to = state("to", report);
    let when = match fields.get("when") {
        Some(cond) => report.condition(cond, &format!("{at}.when")),
        None => {
            report.push(at, Problem::MissingKey("when"));
            None
        }
    };
    // Without a readable actions.yaml, or for a declared but invalid entry,
    // references cannot resolve; actions.yaml already carries the finding.
    let action = required_string(fields, "action", at, report).and_then(|id| {
        match catalog?.actions.get_key_value(id) {
            None => {
                report.push(
                    &format!("{at}.action"),
                    Problem::UnknownAction(id.to_owned()),
                );
                None
            }
            Some((id, action)) => action.as_ref().map(|a| (id.clone(), Arc::clone(a))),
        }
    });
    let effects = match fields.get("effects") {
        None => Some(Vec::new()),
        Some(Value::Sequence(items)) => {
            let mut effects = Some(Vec::with_capacity(items.len()));
            for (i, item) in items.iter().enumerate() {
                let item_at = format!("{at}.effects[{i}]");
                let resolved = report.string(item, &item_at).and_then(|name| {
                    match catalog?.effects.get_key_value(name) {
                        None => {
                            report.push(&item_at, Problem::UnknownEffect(name.to_owned()));
                            None
                        }
                        Some((name, effect)) => {
                            effect.as_ref().map(|e| (name.clone(), Arc::clone(e)))
                        }
                    }
                });
                match (resolved, effects.as_mut()) {
                    (Some(effect), Some(list)) => list.push(effect),
                    _ => effects = None,
                }
            }
            effects
        }
        Some(_) => {
            report.push(
                &format!("{at}.effects"),
                Problem::Shape("a list of effect names"),
            );
            None
        }
    };
    Some(Transition {
        from: from?,
        to: to?,
        when: when?,
        action: action?,
        effects: effects?,
    })
}

/// The map at `at`, reporting keys outside `allowed`.
fn fields<'v>(
    value: &'v Value,
    at: &str,
    allowed: &[&str],
    report: &mut Report<'_>,
) -> Option<&'v Mapping> {
    let Value::Mapping(map) = value else {
        report.push(at, Problem::Shape("a map"));
        return None;
    };
    for key in map.keys() {
        match key.as_str() {
            Some(key) if allowed.contains(&key) => {}
            Some(key) => report.push(at, Problem::UnknownKey(key.to_owned())),
            None => report.push(at, Problem::Shape("string keys")),
        }
    }
    Some(map)
}

fn required_string<'v>(
    fields: &'v Mapping,
    key: &'static str,
    at: &str,
    report: &mut Report<'_>,
) -> Option<&'v str> {
    match fields.get(key) {
        None => {
            report.push(at, Problem::MissingKey(key));
            None
        }
        Some(value) => {
            let here = if at.is_empty() {
                key.to_owned()
            } else {
                format!("{at}.{key}")
            };
            report.string(value, &here)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::_testing::{flavor_dir, sdlc_dir};
    use super::*;

    const ACTIONS: &str = "\
roles: [builder, reviewer]
build:
  command: run build {slug}
  template: templates/build.md
effects:
  refresh:
    command: show {slug}
";

    const FLOW: &str = "\
flow: work
orchestrator: builder
initial: start
states: [start, building]
transitions:
- from: start
  to: building
  when: {exists: prd.digest}
  action: build
  effects: [refresh]
";

    fn findings(files: &[(&str, &str)]) -> Vec<String> {
        let dir = flavor_dir(files);
        match Flavor::load(dir.path()) {
            Ok(_) => Vec::new(),
            Err(FlavorError::Invalid { findings, .. }) => {
                findings.iter().map(ToString::to_string).collect()
            }
            Err(err) => vec![err.to_string()],
        }
    }

    #[test]
    fn a_valid_flavor_loads_with_its_roles_and_flows() {
        let dir = flavor_dir(&[
            ("actions.yaml", ACTIONS),
            ("work.yaml", FLOW),
            ("templates/build.md", "Build {slug}"),
        ]);
        let flavor = Flavor::load(dir.path()).unwrap();
        let roles: Vec<_> = flavor.roles().iter().map(RoleName::as_str).collect();
        assert_eq!(roles, ["builder", "reviewer"]);
        let flows: Vec<_> = flavor.flows().map(FlowName::as_str).collect();
        assert_eq!(flows, ["work"]);
    }

    #[test]
    fn the_sdlc_flavor_loads_without_findings() {
        let flavor = Flavor::load(&sdlc_dir()).unwrap_or_else(|err| panic!("{err}"));
        let flows: Vec<_> = flavor.flows().map(FlowName::as_str).collect();
        assert_eq!(flows, ["prepare", "work"]);
        assert_eq!(flavor.roles().len(), 25);
        assert!(flavor.roles().iter().any(|r| r.as_str() == "integrator"));
    }

    #[test]
    fn a_missing_directory_is_unreadable() {
        let dir = flavor_dir(&[]);
        let err = Flavor::load(&dir.path().join("absent")).unwrap_err();
        assert!(matches!(err, FlavorError::Unreadable { .. }));
    }

    #[test]
    fn every_flow_declares_an_orchestrator_role_from_roles() {
        let template = ("templates/build.md", "x");
        let actions = ("actions.yaml", ACTIONS);
        let without = FLOW.replace("orchestrator: builder\n", "");
        assert_eq!(
            findings(&[actions, ("work.yaml", &without), template]),
            ["work.yaml: required key `orchestrator` is missing"]
        );
        let undeclared = FLOW.replace("orchestrator: builder", "orchestrator: integrator");
        assert_eq!(
            findings(&[actions, ("work.yaml", &undeclared), template]),
            [
                "work.yaml: orchestrator: orchestrator role `integrator` is not declared under `roles` in actions.yaml"
            ]
        );
        let dir = flavor_dir(&[actions, ("work.yaml", FLOW), template]);
        let flavor = Flavor::load(dir.path()).unwrap();
        assert_eq!(
            flavor.orchestrator("work").map(RoleName::as_str),
            Some("builder")
        );
        assert_eq!(flavor.orchestrator("prepare"), None);
    }

    #[test]
    fn roles_must_be_present_non_empty_unique_and_well_formed() {
        let template = ("templates/build.md", "x");
        let flow = ("work.yaml", FLOW);
        let without = ACTIONS.replace("roles: [builder, reviewer]\n", "");
        assert_eq!(
            findings(&[("actions.yaml", &without), flow, template]),
            ["actions.yaml: required key `roles` is missing"]
        );
        let empty = ACTIONS.replace("[builder, reviewer]", "[]");
        assert_eq!(
            findings(&[("actions.yaml", &empty), flow, template]),
            ["actions.yaml: roles: `roles` must list at least one role"]
        );
        let duplicate = ACTIONS.replace("[builder, reviewer]", "[builder, ' padded', builder]");
        let got = findings(&[("actions.yaml", &duplicate), flow, template]);
        assert_eq!(got.len(), 2);
        assert!(got[0].starts_with("actions.yaml: roles[1]: invalid role ` padded`"));
        assert_eq!(
            got[1],
            "actions.yaml: roles[2]: role `builder` is declared more than once"
        );
    }

    #[test]
    fn actions_need_a_command_and_a_template_inside_the_workflow_directory() {
        let actions = "\
roles: [builder]
build:
  template: ../outside.md
  extra: 1
other:
  command: x
  template: templates/missing.md
";
        assert_eq!(
            findings(&[("actions.yaml", actions)]),
            [
                "actions.yaml: build: unknown key `extra`",
                "actions.yaml: build: required key `command` is missing",
                "actions.yaml: build.template: template path `../outside.md` must stay within workflow/",
                "actions.yaml: other.template: template `templates/missing.md` does not exist",
            ]
        );
    }

    #[test]
    fn a_reference_to_a_declared_but_invalid_action_is_reported_once() {
        assert_eq!(
            findings(&[("actions.yaml", ACTIONS), ("work.yaml", FLOW)]),
            ["actions.yaml: build.template: template `templates/build.md` does not exist"]
        );
    }

    #[test]
    fn brief_is_refused_where_it_cannot_be_rendered() {
        let actions = "\
roles: [builder]
build:
  command: run --brief '{brief}'
  template: templates/build.md
effects:
  notify:
    command: send {brief}
";
        assert_eq!(
            findings(&[
                ("actions.yaml", actions),
                ("templates/build.md", "see {brief}")
            ]),
            [
                "actions.yaml: build.template: `{brief}` is the rendered template and is not available here",
                "actions.yaml: effects.notify.command: `{brief}` is the rendered template and is not available here",
            ]
        );
    }

    #[test]
    fn every_definition_reference_must_resolve() {
        let flow = "\
flow: prepare
orchestrator: builder
initial: begin
states: [start, building, start]
transitions:
- from: start
  to: nowhere
  when: {exists: prd.digst}
  action: deploy
  effects: [refresh, notify]
- from: building
  to: start
  action: build
";
        assert_eq!(
            findings(&[
                ("actions.yaml", ACTIONS),
                ("templates/build.md", "x"),
                ("work.yaml", flow),
            ]),
            [
                "work.yaml: flow: declared `flow: prepare` does not match the file name `work`",
                "work.yaml: states[2]: state `start` is declared more than once",
                "work.yaml: initial: state `begin` is not declared in `states`",
                "work.yaml: transitions[0].to: state `nowhere` is not declared in `states`",
                "work.yaml: transitions[0].when.exists: `prd.digst` is not a fact the workflow declares",
                "work.yaml: transitions[0].action: action `deploy` is not configured in actions.yaml",
                "work.yaml: transitions[0].effects[1]: effect `notify` is not configured under `effects` in actions.yaml",
                "work.yaml: transitions[1]: required key `when` is missing",
            ]
        );
    }

    #[test]
    fn unparsable_yaml_names_its_line_and_column() {
        let got = findings(&[
            ("actions.yaml", ACTIONS),
            ("templates/build.md", "x"),
            ("work.yaml", "flow: [unclosed\n"),
        ]);
        assert_eq!(got.len(), 1);
        assert!(
            got[0].starts_with("work.yaml: 2:1: not valid YAML"),
            "{}",
            got[0]
        );
    }

    #[test]
    fn a_missing_actions_file_is_a_finding() {
        assert_eq!(
            findings(&[("work.yaml", FLOW)]),
            ["actions.yaml: the file is missing"]
        );
    }
}
