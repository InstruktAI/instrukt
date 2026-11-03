//! The `instrukt.yaml` schema: which key may live in which layer, each
//! value's shape, and the defaults reads answer when a key is unset.

use serde_yaml_ng::{Mapping, Value};

use super::_interpolate::{join, key_text};
use super::_runtime::{OPTIONAL_FIELDS, REQUIRED_FIELDS};
use crate::ids::{DomainName, ProjectSlug};

/// The schema version this instrukt writes. The first is 1.
pub const CURRENT_SCHEMA_VERSION: u64 = 1;

/// The default `github.api_url`.
pub(crate) const DEFAULT_GITHUB_API_URL: &str = "https://api.github.com";

/// The modules a switch or `requires` may name. Core has no switch.
pub(crate) const MODULE_KEYS: [&str; 3] = ["blueprint", "todos", "events"];

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
}

impl Rule {
    fn permits(self, layer: LayerKind) -> bool {
        matches!(
            (self, layer),
            (Rule::Both, _)
                | (Rule::GlobalOnly, LayerKind::Global)
                | (Rule::ProjectOnly, LayerKind::Project)
        )
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
    NonEmptyList,
    /// A key whose value shape the specs do not settle yet.
    Unchecked,
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
    use Rule::{Both, GlobalOnly, ProjectOnly};
    map(vec![
        ("schema_version", Leaf(Both, Shape::Version)),
        ("project", Leaf(ProjectOnly, Shape::Slug)),
        ("publishes", Leaf(ProjectOnly, Shape::Unchecked)),
        ("requires", Leaf(ProjectOnly, Shape::ModuleList)),
        ("actions", Leaf(ProjectOnly, Shape::Actions)),
        ("board", Leaf(ProjectOnly, Shape::Bool)),
        (
            "github",
            map(vec![
                ("approvers", Leaf(ProjectOnly, Shape::StrList)),
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
            map(MODULE_KEYS
                .iter()
                .map(|key| (*key, Leaf(Both, Shape::Bool)))
                .collect()),
        ),
        (
            "events",
            map(vec![("watch", Leaf(ProjectOnly, Shape::WatchList))]),
        ),
        ("ticks", Leaf(GlobalOnly, Shape::OnOff)),
        ("blueprint", map(vec![("engine", Leaf(Both, Shape::Str))])),
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
                children: vec![("order", Leaf(GlobalOnly, Shape::NonEmptyList))],
                open: Some(Box::new(map(vec![(
                    "git_author",
                    Leaf(GlobalOnly, Shape::Str),
                )]))),
            },
        ),
    ])
}

/// A structural finding: the dotted key and what is wrong with it.
pub(crate) type KeyFinding = (String, String);

/// The structural findings of a layer document: keys in a layer that does
/// not permit them, unknown keys, values of the wrong shape and a missing
/// `schema_version`.
pub(crate) fn check(doc: &Value, layer: LayerKind) -> Vec<KeyFinding> {
    let mut findings = Vec::new();
    let Some(mapping) = doc.as_mapping() else {
        findings.push((String::new(), "the document is not a mapping".to_owned()));
        return findings;
    };
    if !mapping.contains_key("schema_version") {
        findings.push(("schema_version".to_owned(), "missing".to_owned()));
    }
    check_node(&schema(), doc, "", layer, &mut findings);
    findings
}

fn check_node(node: &Node, value: &Value, path: &str, layer: LayerKind, out: &mut Vec<KeyFinding>) {
    match node {
        Node::Leaf(rule, shape) => {
            if rule.permits(layer) {
                check_shape(*shape, value, path, out);
            } else {
                let other = match layer {
                    LayerKind::Global => LayerKind::Project,
                    LayerKind::Project => LayerKind::Global,
                };
                out.push((
                    path.to_owned(),
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
                out.push((path.to_owned(), "expected a mapping".to_owned()));
                return;
            };
            for (key, child) in mapping {
                let Some(name) = key.as_str() else {
                    out.push((
                        join(path, &key_text(key)),
                        "keys must be strings".to_owned(),
                    ));
                    continue;
                };
                let child_path = join(path, name);
                let schema = children
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, node)| node)
                    .or(open.as_deref());
                match schema {
                    Some(schema) => check_node(schema, child, &child_path, layer, out),
                    None => out.push((child_path, "unknown key".to_owned())),
                }
            }
        }
    }
}

fn check_shape(shape: Shape, value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let mut fail = |message: &str| out.push((path.to_owned(), message.to_owned()));
    match shape {
        Shape::Version => {
            if value.as_u64().is_none_or(|v| v < 1) {
                fail("expected a positive integer");
            }
        }
        Shape::Slug => match value.as_str() {
            Some(s) => {
                if let Err(err) = ProjectSlug::parse(s) {
                    fail(&err.to_string());
                }
            }
            None => fail("expected a project slug string"),
        },
        Shape::Domain => match value.as_str() {
            Some(s) => {
                if let Err(err) = DomainName::parse(s) {
                    fail(&err.to_string());
                }
            }
            None => fail("expected a domain name string"),
        },
        Shape::Bool => {
            if !value.is_bool() {
                fail("expected true or false");
            }
        }
        Shape::Str => {
            if !value.is_string() {
                fail("expected a string");
            }
        }
        Shape::OnOff => {
            if !matches!(value.as_str(), Some("on" | "off")) {
                fail("expected `on` or `off`");
            }
        }
        Shape::Approver => {
            if !matches!(value.as_str(), Some("human" | "peer")) {
                fail("expected `human` or `peer`");
            }
        }
        Shape::Autostart => match value {
            Value::Bool(_) => {}
            Value::Mapping(map) => {
                for (kind, on) in map {
                    if !kind.is_string() || !on.is_bool() {
                        out.push((
                            join(path, &key_text(kind)),
                            "expected a kind mapped to true or false".to_owned(),
                        ));
                    }
                }
            }
            _ => fail("expected true, false or a mapping of kind to true or false"),
        },
        Shape::StrList => {
            if !value
                .as_sequence()
                .is_some_and(|seq| seq.iter().all(Value::is_string))
            {
                fail("expected a list of strings");
            }
        }
        Shape::ModuleList => match value.as_sequence() {
            Some(seq) => {
                for (i, item) in seq.iter().enumerate() {
                    if !item.as_str().is_some_and(|m| MODULE_KEYS.contains(&m)) {
                        out.push((
                            join(path, &i.to_string()),
                            "expected one of `blueprint`, `todos`, `events`".to_owned(),
                        ));
                    }
                }
            }
            None => fail("expected a list of modules"),
        },
        Shape::NonEmptyList => {
            if value.as_sequence().is_none_or(|seq| seq.is_empty()) {
                fail("expected a non-empty list");
            }
        }
        Shape::Actions => match value.as_mapping() {
            Some(map) => {
                for (name, action) in map {
                    let ok = action.is_string()
                        || action
                            .as_mapping()
                            .is_some_and(|decl| decl.get("command").is_some_and(Value::is_string));
                    if !ok {
                        out.push((
                            join(path, &key_text(name)),
                            "expected a command string or a declaration with a `command` string"
                                .to_owned(),
                        ));
                    }
                }
            }
            None => fail("expected a mapping of action name to command"),
        },
        Shape::WatchList => match value.as_sequence() {
            Some(seq) => {
                for (i, item) in seq.iter().enumerate() {
                    check_record(
                        item,
                        &join(path, &i.to_string()),
                        &["remote", "ref"],
                        &[],
                        out,
                    );
                }
            }
            None => fail("expected a list of `{remote, ref}`"),
        },
        Shape::Runtimes => match value.as_sequence() {
            Some(seq) => {
                for (i, item) in seq.iter().enumerate() {
                    check_record(
                        item,
                        &join(path, &i.to_string()),
                        &REQUIRED_FIELDS,
                        &OPTIONAL_FIELDS,
                        out,
                    );
                }
            }
            None => fail("expected a list of runtime entries"),
        },
        Shape::Unchecked => {}
    }
}

/// A mapping of string fields: every required field present, no field
/// outside required and optional.
fn check_record(
    value: &Value,
    path: &str,
    required: &[&str],
    optional: &[&str],
    out: &mut Vec<KeyFinding>,
) {
    let Some(map) = value.as_mapping() else {
        out.push((path.to_owned(), "expected a mapping".to_owned()));
        return;
    };
    for field in required {
        if !map.contains_key(*field) {
            out.push((join(path, field), "missing".to_owned()));
        }
    }
    for (key, field) in map {
        let name = key_text(key);
        let child = join(path, &name);
        if !required.contains(&name.as_str()) && !optional.contains(&name.as_str()) {
            out.push((child, "unknown key".to_owned()));
        } else if !field.is_string() {
            out.push((child, "expected a string".to_owned()));
        }
    }
}

/// `doc` without the keys its layer does not permit; unknown keys stay.
/// A mapping emptied by the removal is removed too.
pub(crate) fn permitted(doc: &Value, layer: LayerKind) -> Value {
    filter_node(&schema(), doc, layer).unwrap_or(Value::Mapping(Mapping::new()))
}

fn filter_node(node: &Node, value: &Value, layer: LayerKind) -> Option<Value> {
    match node {
        Node::Leaf(rule, _) => rule.permits(layer).then(|| value.clone()),
        Node::Map { children, open } => {
            let Some(mapping) = value.as_mapping() else {
                return Some(value.clone());
            };
            let mut out = Mapping::new();
            for (key, child) in mapping {
                let schema = key.as_str().and_then(|name| {
                    children
                        .iter()
                        .find(|(n, _)| *n == name)
                        .map(|(_, node)| node)
                        .or(open.as_deref())
                });
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
    let text = match layer {
        LayerKind::Global => format!(
            "modules: {{blueprint: false, todos: false, events: false}}\n\
             ticks: 'on'\n\
             github: {{api_url: '{DEFAULT_GITHUB_API_URL}'}}\n\
             runner: {{autostart: false}}\n"
        ),
        LayerKind::Project => "board: false\n\
             approvals: {prd: human, task: human}\n\
             workflow: {domain: sdlc}\n"
            .to_owned(),
    };
    serde_yaml_ng::from_str(&text).unwrap_or(Value::Mapping(Mapping::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(text: &str) -> Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    fn keys(findings: &[KeyFinding]) -> Vec<&str> {
        findings.iter().map(|(k, _)| k.as_str()).collect()
    }

    #[test]
    fn every_key_of_the_layer_table_is_judged_by_its_layer() {
        let global_only = "runtimes: []\nagents: {order: [a], claude: {git_author: 'A <a@x>'}}\n\
            ticks: 'off'\nrunner: {notify: 'off', runtime: host, autostart: true}\ngithub: {api_url: 'http://x'}\n";
        let project_only = "project: demo\npublishes: {}\nrequires: [todos]\nactions: {test: make test}\n\
            board: true\ngithub: {approvers: [me]}\napprovals: {prd: peer, task: human}\n\
            workflow: {domain: sdlc}\nevents: {watch: [{remote: origin, ref: main}]}\nrunner: {autostart: {build: true}}\n";
        let both = "schema_version: 1\nmodules: {blueprint: true, todos: false, events: true}\n\
            blueprint: {engine: shipped}\n";
        assert!(check(&yaml(&format!("{both}{global_only}")), LayerKind::Global).is_empty());
        assert!(check(&yaml(&format!("{both}{project_only}")), LayerKind::Project).is_empty());

        let in_project = check(
            &yaml(&format!("schema_version: 1\n{global_only}")),
            LayerKind::Project,
        );
        assert_eq!(
            keys(&in_project),
            [
                "runtimes",
                "agents.order",
                "agents.claude.git_author",
                "ticks",
                "runner.notify",
                "runner.runtime",
                "github.api_url"
            ]
        );
        assert!(
            in_project[0]
                .1
                .contains("not permitted in the project layer")
        );
        let in_global = check(
            &yaml(&format!("schema_version: 1\n{project_only}")),
            LayerKind::Global,
        );
        assert_eq!(in_global.len(), 10, "{in_global:?}");
    }

    #[test]
    fn shapes_unknown_keys_and_a_missing_version_are_findings() {
        let findings = check(
            &yaml(
                "modules: {core: true, todos: 'yes'}\nticks: true\nrunner: {autostart: 3}\n\
                 runtimes: [{name: a, session_id_env: A, start: s, send: s, alive: a, bogus: x}]\n\
                 nope: 1\n",
            ),
            LayerKind::Global,
        );
        assert_eq!(
            keys(&findings),
            [
                "schema_version",
                "modules.core",
                "modules.todos",
                "ticks",
                "runner.autostart",
                "runtimes.0.conversation",
                "runtimes.0.bogus",
                "nope"
            ]
        );
    }

    #[test]
    fn reads_drop_keys_of_the_other_layer_and_keep_the_rest() {
        let doc = yaml(
            "schema_version: 1\nproject: demo\nruntimes: [x]\nrunner: {notify: n}\nmodules: {todos: true}\n",
        );
        assert_eq!(
            permitted(&doc, LayerKind::Project),
            yaml("schema_version: 1\nproject: demo\nmodules: {todos: true}\n")
        );
    }

    #[test]
    fn on_and_off_stay_strings() {
        assert_eq!(
            yaml("ticks: on").get("ticks").and_then(Value::as_str),
            Some("on")
        );
        assert_eq!(
            defaults(LayerKind::Global)
                .get("ticks")
                .and_then(Value::as_str),
            Some("on")
        );
    }
}
