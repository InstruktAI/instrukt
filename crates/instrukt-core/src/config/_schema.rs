//! The `instrukt.yaml` schema: which key may live in which layer, each
//! value's shape, and the defaults reads answer when a key is unset.

use serde_yaml_ng::{Mapping, Value};

use super::_error::FindingKind;
use super::_interpolate::{join, key_text, mask_references};
use super::_module::Module;
use super::_runtime::{OPTIONAL_FIELDS, REQUIRED_FIELDS};
use crate::ids::{DomainName, ProjectSlug};

/// The schema version this instrukt writes. The first is 1.
pub const CURRENT_SCHEMA_VERSION: u64 = 1;

/// The `blueprint.engine` values that name a shipped adapter: the only
/// values a project layer may give it. Any other value is a command, which
/// only the global layer may configure — a cloned repository does not
/// decide what runs on a machine.
pub const SHIPPED_BLUEPRINT_ENGINES: [&str; 1] = ["graphify"];

/// The default `github.api_url`.
pub(crate) const DEFAULT_GITHUB_API_URL: &str = "https://api.github.com";

/// One of the two layers of configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    /// `~/.instrukt/instrukt.yaml`: the machine.
    Global,
    /// `instrukt.yaml` at a repository root: the project.
    Project,
}

impl LayerKind {
    fn name(self) -> &'static str {
        match self {
            LayerKind::Global => "global",
            LayerKind::Project => "project",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Rule {
    GlobalOnly,
    ProjectOnly,
    Both,
    /// Both layers may name a shipped adapter; the command form is global
    /// only (`blueprint.engine`).
    ShippedAdapterInProject,
}

impl Rule {
    fn permits(self, layer: LayerKind, value: &Value) -> bool {
        match (self, layer) {
            (Rule::Both, _)
            | (Rule::GlobalOnly, LayerKind::Global)
            | (Rule::ProjectOnly, LayerKind::Project)
            | (Rule::ShippedAdapterInProject, LayerKind::Global) => true,
            (Rule::ShippedAdapterInProject, LayerKind::Project) => value
                .as_str()
                .is_some_and(|name| SHIPPED_BLUEPRINT_ENGINES.contains(&name.trim())),
            (Rule::GlobalOnly, LayerKind::Project) | (Rule::ProjectOnly, LayerKind::Global) => {
                false
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Shape {
    Version,
    Slug,
    Domain,
    Bool,
    Str,
    OnOff,
    Approver,
    Autostart,
    Runtimes,
    WatchList,
    StrList,
    ModuleList,
    Actions,
    AgentOrder,
    ThinkingMode,
    Identity,
    Publishes,
}

enum Node {
    Leaf(Rule, Shape),
    Map {
        children: Vec<(&'static str, Node)>,
        /// The schema of keys not named in `children` (`agents.<name>`).
        open: Option<Box<Node>>,
    },
}

fn map(children: Vec<(&'static str, Node)>) -> Node {
    Node::Map {
        children,
        open: None,
    }
}

fn schema() -> Node {
    use Node::Leaf;
    use Rule::{Both, GlobalOnly, ProjectOnly, ShippedAdapterInProject};
    map(vec![
        ("schema_version", Leaf(Both, Shape::Version)),
        ("project", Leaf(ProjectOnly, Shape::Slug)),
        ("publishes", Leaf(ProjectOnly, Shape::Publishes)),
        ("requires", Leaf(ProjectOnly, Shape::ModuleList)),
        ("actions", Leaf(ProjectOnly, Shape::Actions)),
        ("board", Leaf(ProjectOnly, Shape::Bool)),
        (
            "github",
            map(vec![
                ("approvers", Leaf(ProjectOnly, Shape::StrList)),
                ("repository", Leaf(ProjectOnly, Shape::Str)),
                ("api_url", Leaf(GlobalOnly, Shape::Str)),
            ]),
        ),
        (
            "approvals",
            map(vec![
                ("prd", Leaf(ProjectOnly, Shape::Approver)),
                ("task", Leaf(ProjectOnly, Shape::Approver)),
            ]),
        ),
        (
            "workflow",
            map(vec![("domain", Leaf(ProjectOnly, Shape::Domain))]),
        ),
        (
            "modules",
            map(Module::ALL
                .iter()
                .map(|module| (module.key(), Leaf(Both, Shape::Bool)))
                .collect()),
        ),
        (
            "events",
            map(vec![("watch", Leaf(ProjectOnly, Shape::WatchList))]),
        ),
        ("ticks", Leaf(GlobalOnly, Shape::OnOff)),
        (
            "blueprint",
            map(vec![("engine", Leaf(ShippedAdapterInProject, Shape::Str))]),
        ),
        (
            "runner",
            map(vec![
                ("notify", Leaf(GlobalOnly, Shape::Str)),
                ("runtime", Leaf(GlobalOnly, Shape::Str)),
                ("autostart", Leaf(Both, Shape::Autostart)),
            ]),
        ),
        ("runtimes", Leaf(GlobalOnly, Shape::Runtimes)),
        (
            "agents",
            Node::Map {
                children: vec![
                    ("order", Leaf(GlobalOnly, Shape::AgentOrder)),
                    ("thinking_mode", Leaf(GlobalOnly, Shape::ThinkingMode)),
                ],
                open: Some(Box::new(map(vec![(
                    "git_author",
                    Leaf(GlobalOnly, Shape::Identity),
                )]))),
            },
        ),
    ])
}

/// A finding before it is attached to its document: the dotted key, the
/// rule it breaks and a message for the reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyFinding {
    pub(crate) key: String,
    pub(crate) kind: FindingKind,
    pub(crate) message: String,
}

impl KeyFinding {
    pub(crate) fn new(key: &str, kind: FindingKind, message: impl Into<String>) -> Self {
        Self {
            key: key.to_owned(),
            kind,
            message: message.into(),
        }
    }
}

/// The structural findings of a layer document: keys in a layer that does
/// not permit them, unknown keys, values of the wrong shape and a missing
/// `schema_version`.
pub(crate) fn check(doc: &Value, layer: LayerKind) -> Vec<KeyFinding> {
    let mut findings = Vec::new();
    let Some(mapping) = doc.as_mapping() else {
        findings.push(KeyFinding::new(
            "",
            FindingKind::InvalidValue,
            "the document is not a mapping",
        ));
        return findings;
    };
    if !mapping.contains_key("schema_version") {
        findings.push(KeyFinding::new(
            "schema_version",
            FindingKind::Missing,
            "missing",
        ));
    }
    check_node(&schema(), doc, "", layer, &mut findings);
    findings
}

fn check_node(node: &Node, value: &Value, path: &str, layer: LayerKind, out: &mut Vec<KeyFinding>) {
    match node {
        Node::Leaf(Rule::ShippedAdapterInProject, shape) => {
            if Rule::ShippedAdapterInProject.permits(layer, value) || !value.is_string() {
                check_shape(*shape, value, path, out);
            } else {
                out.push(KeyFinding::new(
                    path,
                    FindingKind::NotPermittedInLayer,
                    format!(
                        "a command is not permitted in the project layer (global layer only); \
                         a project may only name a shipped adapter ({})",
                        SHIPPED_BLUEPRINT_ENGINES.join(", ")
                    ),
                ));
            }
        }
        Node::Leaf(rule, shape) => {
            if rule.permits(layer, value) {
                check_shape(*shape, value, path, out);
            } else {
                let other = match layer {
                    LayerKind::Global => LayerKind::Project,
                    LayerKind::Project => LayerKind::Global,
                };
                out.push(KeyFinding::new(
                    path,
                    FindingKind::NotPermittedInLayer,
                    format!(
                        "not permitted in the {} layer ({} layer only)",
                        layer.name(),
                        other.name()
                    ),
                ));
            }
        }
        Node::Map { children, open } => {
            let Some(mapping) = value.as_mapping() else {
                out.push(KeyFinding::new(
                    path,
                    FindingKind::InvalidValue,
                    "expected a mapping",
                ));
                return;
            };
            for (key, child) in mapping {
                let Some(name) = key.as_str() else {
                    out.push(KeyFinding::new(
                        &join(path, &key_text(key)),
                        FindingKind::InvalidValue,
                        "keys must be strings",
                    ));
                    continue;
                };
                let child_path = join(path, name);
                match child_schema(children, open.as_deref(), name) {
                    Some(schema) => check_node(schema, child, &child_path, layer, out),
                    None => out.push(KeyFinding::new(
                        &child_path,
                        FindingKind::UnknownKey,
                        "unknown key",
                    )),
                }
            }
        }
    }
}

fn child_schema<'a>(
    children: &'a [(&'static str, Node)],
    open: Option<&'a Node>,
    name: &str,
) -> Option<&'a Node> {
    children
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, node)| node)
        .or(open)
}

/// The shape of one field of a record (a runtime entry, a watched ref, an
/// action declaration, a published domain) or of a simple leaf.
#[derive(Debug, Clone, Copy)]
enum Field {
    Str,
    NonEmptyStr,
    Bool,
    Domain,
}

fn check_shape(shape: Shape, value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let invalid = |message: &str| KeyFinding::new(path, FindingKind::InvalidValue, message);
    match shape {
        Shape::Version => {
            if value.as_u64().is_none_or(|v| v < 1) {
                out.push(invalid("expected a positive integer"));
            }
        }
        Shape::Slug => match value.as_str() {
            Some(s) => {
                if let Err(err) = ProjectSlug::parse(s) {
                    out.push(invalid(&err.to_string()));
                }
            }
            None => out.push(invalid("expected a project slug string")),
        },
        Shape::Domain => check_field(Field::Domain, value, path, out),
        Shape::Bool => check_field(Field::Bool, value, path, out),
        Shape::Str => check_field(Field::Str, value, path, out),
        Shape::OnOff => {
            if !matches!(value.as_str(), Some("on" | "off")) {
                out.push(invalid("expected `on` or `off`"));
            }
        }
        Shape::Approver => {
            if !matches!(value.as_str(), Some("human" | "peer")) {
                out.push(invalid("expected `human` or `peer`"));
            }
        }
        Shape::ThinkingMode => {
            if !matches!(value.as_str(), Some("fast" | "med" | "slow")) {
                out.push(invalid("expected `fast`, `med` or `slow`"));
            }
        }
        Shape::Identity => match value.as_str() {
            Some(text) if is_identity_template(text) => {}
            Some(_) => out.push(invalid("expected a git author identity `Name <email>`")),
            None => out.push(invalid("expected a string")),
        },
        Shape::Autostart => match value {
            Value::Bool(_) => {}
            Value::Mapping(map) => {
                for (kind, on) in map {
                    if !kind.is_string() || !on.is_bool() {
                        out.push(KeyFinding::new(
                            &join(path, &key_text(kind)),
                            FindingKind::InvalidValue,
                            "expected a kind mapped to true or false",
                        ));
                    }
                }
            }
            _ => out.push(invalid(
                "expected true, false or a mapping of kind to true or false",
            )),
        },
        Shape::StrList => {
            if !value
                .as_sequence()
                .is_some_and(|seq| seq.iter().all(Value::is_string))
            {
                out.push(invalid("expected a list of strings"));
            }
        }
        Shape::ModuleList => match value.as_sequence() {
            Some(seq) => {
                let names: Vec<&str> = Module::ALL.iter().map(|m| m.key()).collect();
                for (i, item) in seq.iter().enumerate() {
                    if item.as_str().and_then(Module::from_key).is_none() {
                        out.push(KeyFinding::new(
                            &join(path, &i.to_string()),
                            FindingKind::InvalidValue,
                            format!("expected one of {}", names.join(", ")),
                        ));
                    }
                }
            }
            None => out.push(invalid("expected a list of modules")),
        },
        Shape::AgentOrder => match value.as_sequence() {
            Some(seq) if !seq.is_empty() => {
                for (i, item) in seq.iter().enumerate() {
                    check_field(Field::NonEmptyStr, item, &join(path, &i.to_string()), out);
                }
            }
            _ => out.push(invalid("expected a non-empty list of agent names")),
        },
        Shape::Actions => match value.as_mapping() {
            Some(map) => {
                for (name, action) in map {
                    let action_path = join(path, &key_text(name));
                    match action {
                        Value::String(_) => {}
                        Value::Mapping(_) => check_record(
                            action,
                            &action_path,
                            &[("command", Field::Str)],
                            &[("takes_files", Field::Bool)],
                            out,
                        ),
                        _ => out.push(KeyFinding::new(
                            &action_path,
                            FindingKind::InvalidValue,
                            "expected a command string or `{command, takes_files}`",
                        )),
                    }
                }
            }
            None => out.push(invalid("expected a mapping of action name to command")),
        },
        Shape::Publishes => match value.as_sequence() {
            Some(seq) => {
                let mut seen: Vec<&str> = Vec::new();
                for (i, item) in seq.iter().enumerate() {
                    let item_path = join(path, &i.to_string());
                    check_record(
                        item,
                        &item_path,
                        &[("slug", Field::Domain), ("trigger", Field::NonEmptyStr)],
                        &[],
                        out,
                    );
                    if let Some(slug) = item.get("slug").and_then(Value::as_str) {
                        if seen.contains(&slug) {
                            out.push(KeyFinding::new(
                                &join(&item_path, "slug"),
                                FindingKind::DuplicateDomain,
                                format!("domain `{slug}` is published twice"),
                            ));
                        }
                        seen.push(slug);
                    }
                }
            }
            None => out.push(invalid("expected a list of `{slug, trigger}`")),
        },
        Shape::WatchList => match value.as_sequence() {
            Some(seq) => {
                for (i, item) in seq.iter().enumerate() {
                    check_record(
                        item,
                        &join(path, &i.to_string()),
                        &[("remote", Field::Str), ("ref", Field::Str)],
                        &[],
                        out,
                    );
                }
            }
            None => out.push(invalid("expected a list of `{remote, ref}`")),
        },
        Shape::Runtimes => match value.as_sequence() {
            Some(seq) => {
                let required: Vec<(&str, Field)> =
                    REQUIRED_FIELDS.iter().map(|f| (*f, Field::Str)).collect();
                let optional: Vec<(&str, Field)> =
                    OPTIONAL_FIELDS.iter().map(|f| (*f, Field::Str)).collect();
                for (i, item) in seq.iter().enumerate() {
                    check_record(item, &join(path, &i.to_string()), &required, &optional, out);
                }
            }
            None => out.push(invalid("expected a list of runtime entries")),
        },
    }
}

fn check_field(field: Field, value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let message = match (field, value) {
        (Field::Str, Value::String(_)) | (Field::Bool, Value::Bool(_)) => None,
        (Field::Str, _) => Some("expected a string".to_owned()),
        (Field::Bool, _) => Some("expected true or false".to_owned()),
        (Field::NonEmptyStr, Value::String(s)) if !s.trim().is_empty() => None,
        (Field::NonEmptyStr, _) => Some("expected a non-empty string".to_owned()),
        (Field::Domain, Value::String(s)) => DomainName::parse(s).err().map(|e| e.to_string()),
        (Field::Domain, _) => Some("expected a domain name string".to_owned()),
    };
    if let Some(message) = message {
        out.push(KeyFinding::new(path, FindingKind::InvalidValue, message));
    }
}

/// A mapping of fields: every required field present, no field outside
/// required and optional, each of its shape.
fn check_record(
    value: &Value,
    path: &str,
    required: &[(&str, Field)],
    optional: &[(&str, Field)],
    out: &mut Vec<KeyFinding>,
) {
    let Some(map) = value.as_mapping() else {
        out.push(KeyFinding::new(
            path,
            FindingKind::InvalidValue,
            "expected a mapping",
        ));
        return;
    };
    for (field, _) in required {
        if !map.contains_key(*field) {
            out.push(KeyFinding::new(
                &join(path, field),
                FindingKind::Missing,
                "missing",
            ));
        }
    }
    for (key, field_value) in map {
        let name = key_text(key);
        let child = join(path, &name);
        let shape = required
            .iter()
            .chain(optional)
            .find(|(n, _)| *n == name)
            .map(|(_, shape)| *shape);
        match shape {
            Some(shape) => check_field(shape, field_value, &child, out),
            None => out.push(KeyFinding::new(
                &child,
                FindingKind::UnknownKey,
                "unknown key",
            )),
        }
    }
}

/// Whether `text` is a git author identity `Name <email>` once its
/// references are expanded. References are judged when read; here each
/// stands for a value that fits, and a value made only of references is
/// accepted.
fn is_identity_template(text: &str) -> bool {
    let (masked, only_references) = mask_references(text, "r@r");
    only_references || is_identity(&masked)
}

fn is_identity(text: &str) -> bool {
    let Some(inner) = text.strip_suffix('>') else {
        return false;
    };
    let Some((name, email)) = inner.split_once(" <") else {
        return false;
    };
    let name_ok = !name.trim().is_empty() && !name.contains(['<', '>']);
    let email_ok = !email.contains(['<', '>']) && !email.contains(char::is_whitespace);
    let at_ok = email
        .split_once('@')
        .is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty());
    name_ok && email_ok && at_ok
}

/// `doc` without the keys its layer does not permit; unknown keys stay. A
/// mapping emptied by the removal is removed too, and so is a malformed
/// container (anything but a mapping where the schema has one): it is a
/// finding of its own and must not hide the other layer's keys below it.
pub(crate) fn permitted(doc: &Value, layer: LayerKind) -> Value {
    filter_node(&schema(), doc, layer).unwrap_or(Value::Mapping(Mapping::new()))
}

fn filter_node(node: &Node, value: &Value, layer: LayerKind) -> Option<Value> {
    match node {
        Node::Leaf(rule, _) => rule.permits(layer, value).then(|| value.clone()),
        Node::Map { children, open } => {
            let mapping = value.as_mapping()?;
            let mut out = Mapping::new();
            for (key, child) in mapping {
                let schema = key
                    .as_str()
                    .and_then(|name| child_schema(children, open.as_deref(), name));
                let kept = match schema {
                    Some(schema) => filter_node(schema, child, layer),
                    None => Some(child.clone()),
                };
                if let Some(kept) = kept {
                    out.insert(key.clone(), kept);
                }
            }
            (out.is_empty() == mapping.is_empty()).then_some(Value::Mapping(out))
        }
    }
}

/// The values reads answer for unset keys of `layer`'s own keys.
pub(crate) fn defaults(layer: LayerKind) -> Value {
    let mut root = Mapping::new();
    let text = |s: &str| Value::String(s.to_owned());
    match layer {
        LayerKind::Global => {
            for module in Module::ALL {
                insert_at(&mut root, &["modules", module.key()], Value::Bool(false));
            }
            insert_at(&mut root, &["ticks"], text("on"));
            insert_at(
                &mut root,
                &["github", "api_url"],
                text(DEFAULT_GITHUB_API_URL),
            );
            insert_at(&mut root, &["runner", "autostart"], Value::Bool(false));
        }
        LayerKind::Project => {
            insert_at(&mut root, &["board"], Value::Bool(false));
            insert_at(&mut root, &["approvals", "prd"], text("human"));
            insert_at(&mut root, &["approvals", "task"], text("human"));
            insert_at(&mut root, &["workflow", "domain"], text("sdlc"));
        }
    }
    Value::Mapping(root)
}

fn insert_at(root: &mut Mapping, path: &[&str], value: Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut node = root;
    for key in parents {
        let entry = node
            .entry(Value::String((*key).to_owned()))
            .or_insert_with(|| Value::Mapping(Mapping::new()));
        let Value::Mapping(next) = entry else { return };
        node = next;
    }
    node.insert(Value::String((*last).to_owned()), value);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(text: &str) -> Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    fn judged(findings: &[KeyFinding]) -> Vec<(&str, FindingKind)> {
        findings.iter().map(|f| (f.key.as_str(), f.kind)).collect()
    }

    const GLOBAL_ONLY: &str = "runtimes: []\n\
        agents: {order: [a], thinking_mode: slow, claude: {git_author: 'A <a@x>'}}\n\
        ticks: 'off'\nrunner: {notify: 'off', runtime: host, autostart: true}\n\
        github: {api_url: 'http://x'}\n";
    const PROJECT_ONLY: &str = "project: demo\npublishes: [{slug: sdlc, trigger: code}]\n\
        requires: [todos]\nactions: {test: make test, lint: {command: lint, takes_files: true}}\n\
        board: true\ngithub: {approvers: [me]}\napprovals: {prd: peer, task: human}\n\
        workflow: {domain: sdlc}\nevents: {watch: [{remote: origin, ref: main}]}\n\
        runner: {autostart: {build: true}}\n";
    const BOTH: &str = "schema_version: 1\nmodules: {blueprint: true, todos: false, events: true}\n\
        blueprint: {engine: graphify}\n";

    #[test]
    fn every_key_of_the_layer_table_is_permitted_in_its_layer() {
        assert_eq!(
            check(&yaml(&format!("{BOTH}{GLOBAL_ONLY}")), LayerKind::Global),
            []
        );
        assert_eq!(
            check(&yaml(&format!("{BOTH}{PROJECT_ONLY}")), LayerKind::Project),
            []
        );
    }

    #[test]
    fn every_global_only_key_is_refused_in_a_project() {
        let findings = check(
            &yaml(&format!("schema_version: 1\n{GLOBAL_ONLY}")),
            LayerKind::Project,
        );
        use FindingKind::NotPermittedInLayer as N;
        assert_eq!(
            judged(&findings),
            [
                ("runtimes", N),
                ("agents.order", N),
                ("agents.thinking_mode", N),
                ("agents.claude.git_author", N),
                ("ticks", N),
                ("runner.notify", N),
                ("runner.runtime", N),
                ("github.api_url", N),
            ]
        );
    }

    #[test]
    fn an_engine_command_is_global_only_and_a_project_may_name_a_shipped_adapter() {
        let command = yaml("schema_version: 1\nblueprint: {engine: ./map --json}\n");
        assert_eq!(check(&command, LayerKind::Global), []);
        assert_eq!(
            judged(&check(&command, LayerKind::Project)),
            [("blueprint.engine", FindingKind::NotPermittedInLayer)]
        );
        assert_eq!(
            permitted(&command, LayerKind::Project),
            yaml("schema_version: 1\n")
        );
        let shipped = yaml("schema_version: 1\nblueprint: {engine: graphify}\n");
        assert_eq!(check(&shipped, LayerKind::Project), []);
        assert_eq!(permitted(&shipped, LayerKind::Project), shipped);
    }

    #[test]
    fn every_project_only_key_is_refused_in_the_global_layer() {
        let findings = check(
            &yaml(&format!("schema_version: 1\n{PROJECT_ONLY}")),
            LayerKind::Global,
        );
        assert_eq!(findings.len(), 10, "{findings:?}");
        assert!(
            findings
                .iter()
                .all(|f| f.kind == FindingKind::NotPermittedInLayer)
        );
    }

    #[test]
    fn shapes_unknown_keys_and_a_missing_version_are_findings() {
        let findings = check(
            &yaml(
                "modules: {core: true, todos: 'yes'}\nticks: true\nrunner: {autostart: 3}\n\
                 runtimes: [{name: a, session_id_env: A, start: s, send: s, alive: a, bogus: x}]\n\
                 agents: {thinking_mode: deep, order: [claude, true, {a: b}]}\nnope: 1\n",
            ),
            LayerKind::Global,
        );
        use FindingKind::{InvalidValue as I, Missing as M, UnknownKey as U};
        assert_eq!(
            judged(&findings),
            [
                ("schema_version", M),
                ("modules.core", U),
                ("modules.todos", I),
                ("ticks", I),
                ("runner.autostart", I),
                ("runtimes.0.conversation", M),
                ("runtimes.0.bogus", U),
                ("agents.thinking_mode", I),
                ("agents.order.1", I),
                ("agents.order.2", I),
                ("nope", U),
            ]
        );
        let empty_order = check(
            &yaml("schema_version: 1\nagents: {order: []}\n"),
            LayerKind::Global,
        );
        assert_eq!(judged(&empty_order), [("agents.order", I)]);
    }

    #[test]
    fn every_thinking_mode_is_accepted() {
        for mode in ["fast", "med", "slow"] {
            let doc = yaml(&format!(
                "schema_version: 1\nagents: {{thinking_mode: {mode}}}\n"
            ));
            assert_eq!(check(&doc, LayerKind::Global), []);
        }
    }

    #[test]
    fn an_action_is_a_string_or_a_declaration_of_command_and_takes_files() {
        let findings = check(
            &yaml(
                "schema_version: 1\nactions:\n  plain: make\n  files: {command: lint}\n  \
                 bad_flag: {command: echo, takes_files: not-a-bool}\n  \
                 no_command: {takes_files: true}\n  \
                 extra: {command: x, shell: bash}\n  number: 3\n",
            ),
            LayerKind::Project,
        );
        use FindingKind::{InvalidValue as I, Missing as M, UnknownKey as U};
        assert_eq!(
            judged(&findings),
            [
                ("actions.bad_flag.takes_files", I),
                ("actions.no_command.command", M),
                ("actions.extra.shell", U),
                ("actions.number", I),
            ]
        );
    }

    #[test]
    fn publishes_is_a_list_of_distinct_domains_with_their_trigger() {
        let findings = check(
            &yaml(
                "schema_version: 1\npublishes:\n  - {slug: sdlc, trigger: code}\n  \
                 - {slug: sdlc, trigger: again}\n  - {slug: local, trigger: x}\n  \
                 - {slug: ok, trigger: '  '}\n  - {slug: other}\n  \
                 - {slug: more, trigger: t, extra: 1}\n",
            ),
            LayerKind::Project,
        );
        use FindingKind::{DuplicateDomain as D, InvalidValue as I, Missing as M, UnknownKey as U};
        assert_eq!(
            judged(&findings),
            [
                ("publishes.1.slug", D),
                ("publishes.2.slug", I),
                ("publishes.3.trigger", I),
                ("publishes.4.trigger", M),
                ("publishes.5.extra", U),
            ]
        );
        let mapping = check(
            &yaml("schema_version: 1\npublishes: {sdlc: code}\n"),
            LayerKind::Project,
        );
        assert_eq!(judged(&mapping), [("publishes", I)]);
    }

    #[test]
    fn a_git_author_is_an_identity_with_references_standing_for_their_values() {
        for good in [
            "Claude <claude@example.test>",
            "${AUTHOR_NAME} <${AUTHOR_EMAIL}>",
            "Claude <{agents.domain}>",
            "{agents.identity}",
        ] {
            assert!(is_identity_template(good), "{good}");
        }
        for bad in [
            "anonymous",
            "<a@b>",
            "Claude <claude>",
            "Claude <@b>",
            "Claude <a b@c>",
            "Claude a@b",
            "{agents.name} anonymous",
        ] {
            assert!(!is_identity_template(bad), "{bad}");
        }
    }

    #[test]
    fn reads_drop_keys_of_the_other_layer_and_malformed_containers() {
        let doc = yaml(
            "schema_version: 1\nproject: demo\nruntimes: [x]\nrunner: text\ngithub: ~\n\
             agents: ~\nmodules: {todos: true}\nunknown: 1\n",
        );
        assert_eq!(
            permitted(&doc, LayerKind::Project),
            yaml("schema_version: 1\nproject: demo\nmodules: {todos: true}\nunknown: 1\n")
        );
        let findings = check(&doc, LayerKind::Project);
        for key in ["runner", "github", "agents"] {
            assert!(
                findings
                    .iter()
                    .any(|f| f.key == key && f.kind == FindingKind::InvalidValue),
                "{key}: {findings:?}"
            );
        }
    }
}
