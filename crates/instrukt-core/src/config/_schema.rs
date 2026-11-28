//! The `instrukt.yaml` schema: which key may live in which layer, each
//! value's shape, and the defaults reads answer when a key is unset.

use serde_yaml_ng::{Mapping, Value};

use super::_error::FindingKind;
use super::_interpolate::{join, key_text, mask_references};
use super::_module::Module;
use crate::ids::{DomainName, ProjectSlug};

mod _json;

pub use _json::{describe_schema, editor_schema, layer_json_schema};
pub(super) use _json::{non_blank_pattern, secret_reference};

/// The schema version of the global layer (`~/.instrukt/instrukt.yaml`)
/// this instrukt writes. The first is 1.
pub const GLOBAL_SCHEMA_VERSION: u64 = 8;

/// The schema version of a project layer (`instrukt.yaml` at a repository
/// root) this instrukt writes. The first is 1.
pub const PROJECT_SCHEMA_VERSION: u64 = 6;

/// The schema version of a company home's document
/// (`~/.instrukt/companies/<name>/company.yaml`) this instrukt writes. The
/// first is 1.
pub const COMPANY_SCHEMA_VERSION: u64 = 6;

pub(super) const DEFAULT_DEPENDENCY_BRANCH_PREFIX: &str = "renovate/";

/// The key below `modules` that switched conversations on in schema
/// versions before 6; conversion removes it.
pub(super) const CONVERSATIONS: &str = "conversations";

/// The `blueprint.engine` values that name a shipped adapter: the only
/// values a project layer may give it. Any other value is a command, which
/// only the global layer may configure — a cloned repository does not
/// decide what runs on a machine.
pub const SHIPPED_BLUEPRINT_ENGINES: [&str; 1] = ["graphify"];

/// The default `github.api_url`.
pub(crate) const DEFAULT_GITHUB_API_URL: &str = "https://api.github.com";

/// One kind of configuration document: the two layers, and a company
/// home's own document beside them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    /// `~/.instrukt/instrukt.yaml`: the machine.
    Global,
    /// `instrukt.yaml` at a repository root: the project.
    Project,
    /// `~/.instrukt/companies/<name>/company.yaml`: a company home's own
    /// document, read on its own and combined with no layer.
    Company,
}

impl LayerKind {
    /// The schema version of this kind of document this instrukt writes:
    /// each kind versions its own shape, so a change to one raises only its
    /// own (`project/instrukt/spec/feature/configuration/layered-configuration`).
    pub fn current_schema_version(self) -> u64 {
        match self {
            LayerKind::Global => GLOBAL_SCHEMA_VERSION,
            LayerKind::Project => PROJECT_SCHEMA_VERSION,
            LayerKind::Company => COMPANY_SCHEMA_VERSION,
        }
    }

    /// The document as a finding names it.
    fn described(self) -> &'static str {
        match self {
            LayerKind::Global => "the global layer",
            LayerKind::Project => "the project layer",
            LayerKind::Company => "a company home",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Rule {
    /// Every document (`schema_version`).
    Every,
    GlobalOnly,
    ProjectOnly,
    /// The global and the project layer, the project overriding.
    Both,
    /// Both layers may name a shipped adapter; the command form is global
    /// only (`blueprint.engine`).
    ShippedAdapterInProject,
    /// A project layer or a company home (`webhooks`).
    ProjectOrCompany,
    /// A company home only (`company`).
    CompanyOnly,
    /// The global layer or a company home (`mcp`).
    GlobalOrCompany,
}

impl Rule {
    fn permits(self, layer: LayerKind, value: &Value) -> bool {
        match (self, layer) {
            (Rule::ShippedAdapterInProject, LayerKind::Project) => value
                .as_str()
                .is_some_and(|name| SHIPPED_BLUEPRINT_ENGINES.contains(&name.trim())),
            _ => self.admits(layer),
        }
    }

    /// Whether `layer` may hold the key at all, whatever its value.
    fn admits(self, layer: LayerKind) -> bool {
        matches!(
            (self, layer),
            (Rule::Every, _)
                | (
                    Rule::Both
                        | Rule::GlobalOnly
                        | Rule::ShippedAdapterInProject
                        | Rule::GlobalOrCompany,
                    LayerKind::Global
                )
                | (
                    Rule::Both
                        | Rule::ProjectOnly
                        | Rule::ProjectOrCompany
                        | Rule::ShippedAdapterInProject,
                    LayerKind::Project
                )
                | (
                    Rule::ProjectOrCompany | Rule::CompanyOnly | Rule::GlobalOrCompany,
                    LayerKind::Company
                )
        )
    }

    /// Where the key may live, as a finding names it.
    fn places(self) -> &'static str {
        match self {
            Rule::Every => "every document",
            Rule::GlobalOnly | Rule::ShippedAdapterInProject => "global layer only",
            Rule::ProjectOnly => "project layer only",
            Rule::Both => "global or project layer only",
            Rule::ProjectOrCompany => "project layer or company home only",
            Rule::CompanyOnly => "company home only",
            Rule::GlobalOrCompany => "global layer or company home only",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Shape {
    /// A positive integer.
    Version,
    /// `schema_version`: a [`Shape::Version`] with that key's contract.
    SchemaVersion,
    RedisTimeout,
    Webhooks,
    Members,
    Connectors,
    Mcp,
    McpClients,
    Issuer,
    IngressUrl,
    Runtimes,
    MachineId,
    Port,
    Credential,
    LocalRedisUrl,
    Slug,
    /// `company`: a [`Shape::Slug`] naming the company home's folder.
    CompanyName,
    Domain,
    Bool,
    /// `github.org`: a [`Shape::Str`] with that key's contract.
    OwningOrganisation,
    /// `company_repository`: a [`Shape::Bool`] with that key's contract.
    CompanyRepository,
    /// `company_name`: the one line people write the company by.
    CompanyDisplayName,
    /// `false`, or a GitHub project `<owner>/<number>` (`board`).
    Board,
    Apps,
    Str,
    OnOff,
    Approver,
    Autostart,
    WatchList,
    StrList,
    /// `profiles`: a [`Shape::StrList`] of the machine's profile names.
    Profiles,
    BranchPrefixes,
    ModuleList,
    Identity,
    /// Paths inside a checkout (`regenerable`).
    Regenerable,
    /// `off` or a retention window (`runner.archive_finished_after`).
    RetentionWindow,
    /// Machine-wide encrypted home backup (`backup`).
    Backup,
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
        ("schema_version", Leaf(Rule::Every, Shape::SchemaVersion)),
        ("project", Leaf(ProjectOnly, Shape::Slug)),
        ("company", Leaf(Rule::CompanyOnly, Shape::CompanyName)),
        // Later company-home keys: refused until this instrukt supports them.
        ("members", Leaf(Rule::CompanyOnly, Shape::Members)),
        ("connectors", Leaf(Rule::CompanyOnly, Shape::Connectors)),
        ("profiles", Leaf(Rule::CompanyOnly, Shape::Profiles)),
        ("mcp", Leaf(Rule::GlobalOrCompany, Shape::Mcp)),
        ("mcp_clients", Leaf(GlobalOnly, Shape::McpClients)),
        ("issuer", Leaf(Rule::CompanyOnly, Shape::Issuer)),
        ("requires", Leaf(ProjectOnly, Shape::ModuleList)),
        ("board", Leaf(ProjectOnly, Shape::Board)),
        (
            "github",
            map(vec![
                ("approvers", Leaf(ProjectOnly, Shape::StrList)),
                ("repository", Leaf(ProjectOnly, Shape::Str)),
                ("api_url", Leaf(GlobalOnly, Shape::Str)),
                ("apps", Leaf(GlobalOnly, Shape::Apps)),
                ("project_app", Leaf(GlobalOnly, Shape::Str)),
                ("org", Leaf(GlobalOnly, Shape::OwningOrganisation)),
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
        (
            "redis",
            map(vec![
                ("cluster", redis_scope(false)),
                ("local", redis_scope(true)),
            ]),
        ),
        ("webhooks", Leaf(Rule::ProjectOrCompany, Shape::Webhooks)),
        ("runtimes", Leaf(GlobalOnly, Shape::Runtimes)),
        (
            "ingress",
            map(vec![
                ("computer", Leaf(GlobalOnly, Shape::MachineId)),
                ("port", Leaf(GlobalOnly, Shape::Port)),
                ("reactions", Leaf(GlobalOnly, Shape::MachineId)),
                ("url", Leaf(GlobalOnly, Shape::IngressUrl)),
            ]),
        ),
        (
            "ci",
            map(vec![(
                "dependency_branch_prefixes",
                Leaf(ProjectOnly, Shape::BranchPrefixes),
            )]),
        ),
        (
            "review",
            map(vec![("automatic", Leaf(ProjectOnly, Shape::Bool))]),
        ),
        (
            "company_repository",
            Leaf(ProjectOnly, Shape::CompanyRepository),
        ),
        (
            super::_identity::COMPANY_NAME_KEY,
            Leaf(ProjectOnly, Shape::CompanyDisplayName),
        ),
        (
            super::_regenerable::REGENERABLE_KEY,
            Leaf(ProjectOnly, Shape::Regenerable),
        ),
        ("ticks", Leaf(GlobalOnly, Shape::OnOff)),
        (super::_backup::BACKUP_KEY, Leaf(GlobalOnly, Shape::Backup)),
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
                (
                    "archive_finished_after",
                    Leaf(GlobalOnly, Shape::RetentionWindow),
                ),
            ]),
        ),
        (
            "agents",
            Node::Map {
                children: vec![],
                open: Some(Box::new(map(vec![(
                    "git_author",
                    Leaf(GlobalOnly, Shape::Identity),
                )]))),
            },
        ),
    ])
}

fn redis_scope(local: bool) -> Node {
    use Node::Leaf;
    use Rule::GlobalOnly;
    map(vec![
        (
            "url",
            Leaf(
                GlobalOnly,
                if local {
                    Shape::LocalRedisUrl
                } else {
                    Shape::Str
                },
            ),
        ),
        ("username", Leaf(GlobalOnly, Shape::Credential)),
        ("password", Leaf(GlobalOnly, Shape::Credential)),
        ("max_connections", Leaf(GlobalOnly, Shape::Version)),
        ("socket_timeout", Leaf(GlobalOnly, Shape::RedisTimeout)),
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
                check_shape(*shape, value, path, layer, out);
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
                check_shape(*shape, value, path, layer, out);
            } else {
                out.push(KeyFinding::new(
                    path,
                    FindingKind::NotPermittedInLayer,
                    format!("not permitted in {} ({})", layer.described(), rule.places()),
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

fn check_shape(
    shape: Shape,
    value: &Value,
    path: &str,
    layer: LayerKind,
    out: &mut Vec<KeyFinding>,
) {
    let invalid = |message: &str| KeyFinding::new(path, FindingKind::InvalidValue, message);
    match shape {
        Shape::Runtimes => match value.as_mapping() {
            Some(entries) => {
                let mut fields: Vec<_> = super::_runtime::ACTIONS
                    .iter()
                    .map(|(name, _)| (*name, Field::NonEmptyStr))
                    .collect();
                // Retain compatibility for existing machine overrides that still
                // carry the retired `discuss` runtime action key.
                fields.push(("discuss", Field::NonEmptyStr));
                for (name, entry) in entries {
                    check_record(entry, &join(path, &key_text(name)), &[], &fields, out);
                }
            }
            None => out.push(invalid(
                "expected a mapping of runtime names to agent-action values",
            )),
        },
        Shape::Webhooks => {
            if let Err(error) = super::_webhooks::parse(Some(value)) {
                out.push(invalid(&error.to_string()));
            }
        }
        Shape::Members => super::_company::check_members(value, path, out),
        Shape::Connectors => super::_company::check_connectors(value, path, out),
        Shape::Mcp => super::_mcp::check(value, path, layer, out),
        Shape::McpClients => super::_mcp::check_clients(value, path, out),
        Shape::Issuer => super::_mcp::check_issuer(value, path, out),
        Shape::IngressUrl => {
            if !value
                .as_str()
                .is_some_and(super::_webhooks::ingress_url_admitted)
            {
                out.push(invalid(
                    "expected an absolute https address (or loopback http address) without credentials, query or fragment",
                ));
            }
        }
        Shape::MachineId => {
            if value
                .as_str()
                .and_then(crate::machine::MachineId::parse)
                .is_none()
            {
                out.push(invalid("expected an existing stable machine ID"));
            }
        }
        Shape::Port => {
            if value
                .as_u64()
                .is_none_or(|port| !(1..=65535).contains(&port))
            {
                out.push(invalid("expected a TCP port from 1 to 65535"));
            }
        }
        Shape::Credential => {
            if !value
                .as_str()
                .is_some_and(|s| mask_references(s, "reference").1)
            {
                out.push(invalid("credentials must be references"));
            }
        }
        Shape::LocalRedisUrl => {
            if !value
                .as_str()
                .is_some_and(|s| s.starts_with("unix://") || mask_references(s, "reference").1)
            {
                out.push(invalid("local Redis requires a unix:// socket URL"));
            }
        }
        Shape::RedisTimeout => {
            if value
                .as_u64()
                .is_none_or(|v| v <= super::_redis::REDIS_BLOCK_SECONDS)
            {
                out.push(invalid("must outlast the 10 second blocking read"));
            }
        }
        Shape::Version | Shape::SchemaVersion => {
            if value.as_u64().is_none_or(|v| v < 1) {
                out.push(invalid("expected a positive integer"));
            }
        }
        Shape::Slug | Shape::CompanyName => match value.as_str() {
            Some(s) => {
                if let Err(err) = ProjectSlug::parse(s) {
                    out.push(invalid(&err.to_string()));
                }
            }
            None => out.push(invalid("expected a project slug string")),
        },
        Shape::CompanyDisplayName => match value.as_str() {
            Some(text) => {
                if let Err(error) = super::_identity::CompanyName::parse(text) {
                    out.push(invalid(&error.to_string()));
                }
            }
            None => out.push(invalid("expected the company's name as one line of text")),
        },
        Shape::Domain => check_field(Field::Domain, value, path, out),
        Shape::Bool | Shape::CompanyRepository => check_field(Field::Bool, value, path, out),
        Shape::Apps => {
            if let Err(error) = crate::github::inventory(value) {
                out.push(invalid(&error.to_string()));
            }
        }
        Shape::Board => match value {
            Value::Bool(false) => {}
            Value::Bool(true) => out.push(invalid(
                "name the GitHub project todos are reflected to: `<owner>/<number>` (or `false`)",
            )),
            Value::String(text) if super::_effective::board_ref(text).is_some() => {}
            _ => out.push(invalid(
                "expected `false` or a GitHub project `<owner>/<number>`",
            )),
        },
        Shape::Str | Shape::OwningOrganisation => check_field(Field::Str, value, path, out),
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
        Shape::BranchPrefixes => {
            if !value.as_sequence().is_some_and(|seq| {
                seq.iter()
                    .all(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
            }) {
                out.push(invalid("expected a list of nonempty branch prefixes"));
            }
        }
        Shape::StrList | Shape::Profiles => {
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

        Shape::Regenerable => super::_regenerable::check(value, path, out),
        Shape::RetentionWindow => super::_thread_retention::check(value, path, out),
        Shape::Backup => super::_backup::check(value, path, out),
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
                insert_at(
                    &mut root,
                    &["modules", module.key()],
                    Value::Bool(module == Module::History),
                );
            }
            insert_at(&mut root, &["ticks"], text("on"));
            insert_at(
                &mut root,
                &["github", "api_url"],
                text(DEFAULT_GITHUB_API_URL),
            );
            insert_at(&mut root, &["runner", "autostart"], Value::Bool(false));
            insert_at(
                &mut root,
                &["runner", "archive_finished_after"],
                text(super::_thread_retention::OFF),
            );
        }
        LayerKind::Project => {
            insert_at(&mut root, &["board"], Value::Bool(false));
            insert_at(&mut root, &["approvals", "prd"], text("human"));
            insert_at(&mut root, &["approvals", "task"], text("human"));
            insert_at(&mut root, &["workflow", "domain"], text("sdlc"));
            insert_at(
                &mut root,
                &["ci", "dependency_branch_prefixes"],
                Value::Sequence(vec![text(DEFAULT_DEPENDENCY_BRANCH_PREFIX)]),
            );
            insert_at(&mut root, &["review", "automatic"], Value::Bool(false));
        }
        // A company home's document is read alone, without defaults.
        LayerKind::Company => {}
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

    const GLOBAL_ONLY: &str = "agents: {claude: {git_author: 'A <a@x>'}}\n\
        ticks: 'off'\nrunner: {notify: 'off', runtime: host, autostart: true, archive_finished_after: 7d}\n\
        github: {api_url: 'http://x'}\n";
    const PROJECT_ONLY: &str = "project: demo\n\
        requires: [todos]\n\
        board: acme/7\ngithub: {approvers: [me]}\napprovals: {prd: peer, task: human}\n\
        workflow: {domain: sdlc}\nevents: {watch: [{remote: origin, ref: main}]}\n\
        runner: {autostart: {build: true}}\n";
    const BOTH: &str = "schema_version: 1\nmodules: {blueprint: true, todos: false, events: true}\n\
        blueprint: {engine: graphify}\n";

    /// A JSON Schema `pattern`, compiled as Draft 2020-12 reads it: an
    /// unanchored ECMA-262 regular expression, here in Unicode mode as
    /// editors' validators compile it.
    fn ecma_pattern(pattern: &str) -> regress::Regex {
        regress::Regex::with_flags(pattern, "u").unwrap()
    }

    /// Whether `value` fits `schema`, for the JSON Schema keywords
    /// [`editor_schema`] writes.
    fn fits(schema: &serde_json::Value, value: &serde_json::Value) -> bool {
        use serde_json::Value as J;
        let object = schema.as_object().unwrap();
        if let Some(options) = object.get("anyOf").and_then(J::as_array) {
            return options.iter().any(|option| fits(option, value));
        }
        if let Some(wanted) = object.get("const") {
            return wanted == value;
        }
        if let Some(names) = object.get("enum").and_then(J::as_array) {
            return names.contains(value);
        }
        match object.get("type").and_then(J::as_str) {
            Some("string") => {
                let pattern = object.get("pattern").and_then(J::as_str);
                value.as_str().is_some_and(|text| {
                    pattern.is_none_or(|p| ecma_pattern(p).find(text).is_some())
                })
            }
            Some("boolean") => value.is_boolean(),
            Some("integer") => value
                .as_u64()
                .is_some_and(|n| object["minimum"].as_u64().is_none_or(|min| n >= min)),
            Some("array") => value.as_array().is_some_and(|items| {
                object
                    .get("minItems")
                    .and_then(J::as_u64)
                    .is_none_or(|n| items.len() as u64 >= n)
                    && items.iter().all(|item| fits(&object["items"], item))
            }),
            Some("object") => value.as_object().is_some_and(|map| {
                let required = object.get("required").and_then(J::as_array);
                let names = object.get("propertyNames");
                names.is_none_or(|names| map.keys().all(|key| fits(names, &serde_json::json!(key))))
                    && required.is_none_or(|names| {
                        names
                            .iter()
                            .all(|name| map.contains_key(name.as_str().unwrap()))
                    })
                    && map.iter().all(|(key, child)| {
                        match object.get("properties").and_then(|p| p.get(key)) {
                            Some(property) => fits(property, child),
                            // An object without `additionalProperties` is open.
                            None => match object.get("additionalProperties") {
                                None => true,
                                Some(J::Bool(open)) => *open,
                                Some(other) => fits(other, child),
                            },
                        }
                    })
            }),
            other => panic!("a type the editor schema does not write: {other:?}"),
        }
    }

    fn as_json(text: &str) -> serde_json::Value {
        serde_json::to_value(yaml(text)).unwrap()
    }

    #[test]
    fn the_editor_schema_admits_every_project_layer_key_validation_admits() {
        let doc = format!("{BOTH}{PROJECT_ONLY}");
        assert!(check(&yaml(&doc), LayerKind::Project).is_empty());
        assert!(fits(&editor_schema(), &as_json(&doc)));
    }

    #[test]
    fn the_editor_schema_marks_each_global_only_key_as_validation_does() {
        for line in GLOBAL_ONLY.lines() {
            let doc = format!("{BOTH}{line}\n");
            assert!(!check(&yaml(&doc), LayerKind::Project).is_empty(), "{line}");
            assert!(!fits(&editor_schema(), &as_json(&doc)), "{line}");
        }
    }

    #[test]
    fn the_editor_schema_marks_an_unknown_key_and_a_wrong_shape() {
        for extra in [
            "publishes: [sdlc]\n",
            "board: true\n",
            "approvals: {prd: robot}\n",
        ] {
            let doc = format!("{BOTH}{extra}");
            assert!(
                !check(&yaml(&doc), LayerKind::Project).is_empty(),
                "{extra}"
            );
            assert!(!fits(&editor_schema(), &as_json(&doc)), "{extra}");
        }
    }

    #[test]
    fn the_global_schema_reads_runtime_actions_as_validation_does() {
        let schema = layer_json_schema(LayerKind::Global);
        let entry = |actions: &str| format!("schema_version: 1\nruntimes:\n  t3:\n{actions}");
        let every: String = super::super::_runtime::ACTIONS
            .iter()
            .map(|(name, _)| format!("    {name}: Follow the native {name}.\n"))
            .collect();
        for (doc, valid) in [
            (entry(&every), true),
            (entry("    finish_thread: Settle the thread.\n"), true),
            (entry("    finish_thread: '  '\n"), false),
            // The retired action is an unknown key, not an override.
            (
                entry("    retire_thread: Archive the finished thread.\n"),
                false,
            ),
        ] {
            assert_eq!(
                check(&yaml(&doc), LayerKind::Global).is_empty(),
                valid,
                "{doc}"
            );
            assert_eq!(fits(&schema, &as_json(&doc)), valid, "{doc}");
        }
    }

    #[test]
    fn the_global_schema_reads_blank_runtime_actions_as_validation_does() {
        let schema = layer_json_schema(LayerKind::Global);
        for (action, valid) in [
            ("\u{85}", false),
            ("\u{FEFF}", true),
            ("", false),
            (" \t\n\r\u{B}\u{C}", false),
            (
                "\u{A0}\u{1680}\u{2000}\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}",
                false,
            ),
            ("settle", true),
            ("\u{85} settle\u{3000}", true),
            ("\u{1F5C4}", true),
        ] {
            let mut doc = yaml("schema_version: 1\nruntimes: {t3: {}}\n");
            doc["runtimes"]["t3"]
                .as_mapping_mut()
                .unwrap()
                .insert(Value::from("finish_thread"), Value::from(action));
            let json = serde_json::to_value(&doc).unwrap();
            assert_eq!(
                check(&doc, LayerKind::Global).is_empty(),
                valid,
                "{action:?}"
            );
            assert_eq!(fits(&schema, &json), valid, "{action:?}");
        }
    }

    /// Whether validation and the published global schema each admit
    /// `window` as the retention window.
    fn window_admitted(window: &str) -> (bool, bool) {
        let doc = format!("schema_version: 1\nrunner: {{archive_finished_after: {window}}}\n");
        (
            check(&yaml(&doc), LayerKind::Global).is_empty(),
            fits(&layer_json_schema(LayerKind::Global), &as_json(&doc)),
        )
    }

    #[test]
    fn validation_and_the_global_schema_admit_a_counted_window() {
        assert_eq!(window_admitted("7d"), (true, true));
    }

    #[test]
    fn validation_and_the_global_schema_admit_an_off_window() {
        assert_eq!(window_admitted("off"), (true, true));
    }

    #[test]
    fn validation_and_the_global_schema_refuse_a_zero_count() {
        assert_eq!(window_admitted("0d"), (false, false));
    }

    #[test]
    fn validation_and_the_global_schema_refuse_a_leading_zero() {
        assert_eq!(window_admitted("07d"), (false, false));
    }

    #[test]
    fn validation_and_the_global_schema_refuse_a_count_without_a_unit() {
        assert_eq!(window_admitted("7"), (false, false));
    }

    #[test]
    fn validation_and_the_global_schema_refuse_an_unknown_unit() {
        assert_eq!(window_admitted("7y"), (false, false));
    }

    #[test]
    fn validation_and_the_global_schema_refuse_a_unit_without_a_count() {
        assert_eq!(window_admitted("d"), (false, false));
    }

    #[test]
    fn validation_and_the_global_schema_refuse_a_window_that_is_not_text() {
        assert_eq!(window_admitted("true"), (false, false));
    }

    /// Read as ECMA-262, each published action pattern admits a value exactly
    /// when `str::trim` leaves something of it, on every Unicode scalar,
    /// including those whose whitespace status ECMA-262 and Rust disagree on.
    #[test]
    fn the_runtime_action_pattern_admits_exactly_what_trim_keeps() {
        let schema = layer_json_schema(LayerKind::Global);
        let actions = &schema["properties"]["runtimes"]["additionalProperties"]["properties"];
        let patterns: std::collections::BTreeSet<&str> = super::super::_runtime::ACTIONS
            .iter()
            .map(|(name, _)| actions[*name]["pattern"].as_str().unwrap())
            .collect();
        let whitespace: String = (0..=u32::from(char::MAX))
            .filter_map(char::from_u32)
            .filter(|c| c.is_whitespace())
            .collect();
        for pattern in patterns {
            let compiled = ecma_pattern(pattern);
            let admits = |text: &str| compiled.find(text).is_some();
            let mut one = [0; 4];
            let wrong: Vec<char> = (0..=u32::from(char::MAX))
                .filter_map(char::from_u32)
                .filter(|&c| {
                    let text = c.encode_utf8(&mut one);
                    admits(text) == text.trim().is_empty()
                })
                .collect();
            assert!(wrong.is_empty(), "{pattern} misreads {wrong:?}");
            for text in [
                String::new(),
                whitespace.clone(),
                format!("{whitespace}\u{FEFF}{whitespace}"),
                format!("{whitespace}archive{whitespace}"),
            ] {
                assert_eq!(admits(&text), !text.trim().is_empty(), "{text:?}");
            }
        }
    }

    #[test]
    fn the_editor_schema_requires_the_schema_version() {
        assert!(!fits(&editor_schema(), &as_json("project: demo\n")));
    }

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

    /// Whether validation and the company home's JSON Schema each admit a
    /// company home holding `members`, in that order.
    fn members_admitted(members: &str) -> (bool, bool) {
        let doc = format!("schema_version: 1\ncompany: acme\nmembers: {members}\n");
        (
            check(&yaml(&doc), LayerKind::Company).is_empty(),
            fits(&layer_json_schema(LayerKind::Company), &as_json(&doc)),
        )
    }

    #[test]
    fn the_company_schema_admits_a_complete_member_entry_as_validation_does() {
        let member = "{alex: {name: Alex Example, email: alex@example.com, \
                      credentials: {erpnext: '${ALEX_ERPNEXT_LOGIN}'}}}";
        assert_eq!(members_admitted(member), (true, true));
    }

    #[test]
    fn the_company_schema_refuses_a_member_key_outside_the_slug_grammar_as_validation_does() {
        assert_eq!(
            members_admitted("{Alex: {email: alex@example.com}}"),
            (false, false)
        );
    }

    #[test]
    fn the_company_schema_refuses_a_member_without_an_email_as_validation_does() {
        assert_eq!(members_admitted("{alex: {name: Alex}}"), (false, false));
    }

    #[test]
    fn the_company_schema_refuses_an_email_without_an_address_as_validation_does() {
        assert_eq!(members_admitted("{alex: {email: alex}}"), (false, false));
    }

    #[test]
    fn the_company_schema_refuses_a_blank_display_name_as_validation_does() {
        assert_eq!(
            members_admitted("{alex: {name: ' ', email: alex@example.com}}"),
            (false, false)
        );
    }

    #[test]
    fn the_company_schema_refuses_an_unknown_member_field_as_validation_does() {
        assert_eq!(
            members_admitted("{alex: {email: alex@example.com, role: admin}}"),
            (false, false)
        );
    }

    #[test]
    fn the_company_schema_refuses_a_literal_credential_as_validation_does() {
        assert_eq!(
            members_admitted("{alex: {email: alex@example.com, credentials: {erpnext: hunter2}}}"),
            (false, false)
        );
    }

    #[test]
    fn a_company_home_holds_only_its_identity_and_sources() {
        use FindingKind::NotPermittedInLayer as N;
        let company = "schema_version: 1\ncompany: acme\n\
            webhooks: {erp: {secret: '${ERP_HOOK}'}}\n";
        assert_eq!(check(&yaml(company), LayerKind::Company), []);
        assert_eq!(permitted(&yaml(company), LayerKind::Company), yaml(company));
        let layered = yaml(&format!("{BOTH}{PROJECT_ONLY}ticks: 'off'\n"));
        let refused = check(&layered, LayerKind::Company);
        assert!(!refused.is_empty());
        assert!(refused.iter().all(|f| f.kind == N), "{refused:?}");
        assert_eq!(
            permitted(&layered, LayerKind::Company),
            yaml("schema_version: 1\n")
        );
        for layer in [LayerKind::Global, LayerKind::Project] {
            assert_eq!(
                judged(&check(&yaml("schema_version: 1\ncompany: acme\n"), layer)),
                [("company", N)]
            );
        }
        assert_eq!(
            judged(&check(
                &yaml("schema_version: 1\nwebhooks: {}\n"),
                LayerKind::Global
            )),
            [("webhooks", N)]
        );
        // MCP servers are declared machine-wide or in a company home.
        let servers = yaml("schema_version: 1\nmcp: {}\n");
        for layer in [LayerKind::Global, LayerKind::Company] {
            assert_eq!(check(&servers, layer), []);
        }
        assert_eq!(judged(&check(&servers, LayerKind::Project)), [("mcp", N)]);
        let people = yaml("schema_version: 1\nmembers: {}\nconnectors: {}\nprofiles: [a]\n");
        assert_eq!(check(&people, LayerKind::Company), []);
        for layer in [LayerKind::Global, LayerKind::Project] {
            assert_eq!(
                judged(&check(&people, layer)),
                [("members", N), ("connectors", N), ("profiles", N)]
            );
        }
        // A declared connector gives conversations; no layer holds a switch.
        let switch = yaml("schema_version: 1\nmodules: {conversations: true}\n");
        for layer in [LayerKind::Global, LayerKind::Project] {
            assert_eq!(
                judged(&check(&switch, layer)),
                [("modules.conversations", FindingKind::UnknownKey)]
            );
        }
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
                ("agents.claude.git_author", N),
                ("ticks", N),
                ("runner.notify", N),
                ("runner.runtime", N),
                ("runner.archive_finished_after", N),
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
        assert_eq!(findings.len(), 8, "{findings:?}");
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
                 agents: {tier: deep, order: [claude, true, {a: b}]}\nnope: 1\n",
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
                ("runtimes", I),
                ("agents.tier", I),
                ("agents.order", I),
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
    fn actions_is_an_unknown_key_in_validation_and_the_editor_schema() {
        let text = "schema_version: 3\nproject: demo\nactions: {test: make test}\n";
        assert_eq!(
            judged(&check(&yaml(text), LayerKind::Project)),
            [("actions", FindingKind::UnknownKey)]
        );
        assert!(!fits(&editor_schema(), &as_json(text)));
    }

    #[test]
    fn publishes_is_an_unknown_key() {
        let findings = check(
            &yaml("schema_version: 1\npublishes: [{slug: sdlc, trigger: code}]\n"),
            LayerKind::Project,
        );
        assert_eq!(judged(&findings), [("publishes", FindingKind::UnknownKey)]);
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
