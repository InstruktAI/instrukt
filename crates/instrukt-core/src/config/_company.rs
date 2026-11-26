//! A company home's members and connector declarations
//! (`company-homes`, "Members"; `slack-connector`, "Declaration").

use std::collections::BTreeMap;

use serde_yaml_ng::Value;

use instrukt_connector::{Declaration, Rule};

use super::_agents::COMPANY_AGENT_ROLE;
use super::_connectors::{ConnectorAdmit, DeclaredConnector, ReplyMode, SessionScope};
use super::_error::{ConfigError, FindingKind};
use super::_interpolate::{join, key_text};
use super::_schema::KeyFinding;
use crate::ids::{ProjectSlug, RoleName};

/// Why an empty `profiles` list is a finding.
pub(super) const EMPTY_PROFILES: &str = "lists no profile; name the profiles its sessions start on, or remove profiles to use the machine-wide order";

/// The findings of a company home's `profiles` against the profiles the
/// machine's `agents.yaml` declares: each name it does not declare, and an
/// empty list. Each names the company.
pub(super) fn check_profiles(
    company: &ProjectSlug,
    value: &Value,
    declared: &Result<super::Agents, ConfigError>,
    out: &mut Vec<KeyFinding>,
) {
    let Some(names) = value.as_sequence() else {
        return;
    };
    if names.is_empty() {
        out.push(finding(
            "profiles",
            FindingKind::InvalidValue,
            format!("company {company} {EMPTY_PROFILES}"),
        ));
        return;
    }
    let declared = match declared {
        Ok(declared) => declared,
        Err(error) => {
            out.push(finding(
                "profiles",
                FindingKind::InvalidValue,
                format!(
                    "company {company} names profiles, and the machine's agents.yaml that declares them cannot be read: {error}"
                ),
            ));
            return;
        }
    };
    for (i, name) in names.iter().enumerate() {
        let Some(name) = name.as_str() else {
            continue;
        };
        let known = crate::ids::ProfileName::parse(name)
            .ok()
            .is_some_and(|name| declared.profiles.contains_key(&name));
        if !known {
            out.push(finding(
                &format!("profiles[{i}]"),
                FindingKind::UnresolvedReference,
                format!(
                    "company {company} names profile `{name}`, which the machine's agents.yaml does not declare"
                ),
            ));
        }
    }
}

/// One person as a company home lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The member's key in the company's list.
    pub key: ProjectSlug,
    /// The display name; the key when omitted.
    pub name: String,
    /// The person's email address, by which a speaker finds their entry.
    pub email: String,
    /// `${NAME}` references to the person's own tool logins.
    pub credentials: BTreeMap<String, String>,
}

/// A company's member list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Members(Vec<Member>);

impl Members {
    pub fn iter(&self) -> impl Iterator<Item = &Member> {
        self.0.iter()
    }

    /// The one member whose email address equals `email`, compared without
    /// regard to case, by lookup in this company's list alone.
    pub fn by_email(&self, email: &str) -> Option<&Member> {
        self.0
            .iter()
            .find(|member| member.email.to_lowercase() == email.to_lowercase())
    }
}

/// The editor schema of a company home's `members`, admitting what
/// [`check_members`] admits but for an address two members list, which no
/// JSON Schema can compare across entries.
pub(super) fn members_schema() -> serde_json::Value {
    use super::_schema::{non_blank_pattern, secret_reference};
    serde_json::json!({
        "type": "object",
        "description": "The company's people, keyed by member: what a provider cannot supply, \
            a person's own stored logins for the company's tools. An entry carries no role \
            and grants nothing.",
        "propertyNames": {"type": "string", "pattern": "^[a-z0-9][a-z0-9._-]*$"},
        "additionalProperties": {
            "type": "object",
            "description": "One person, under a member key unique within the company that \
                follows the grammar of a project slug.",
            "additionalProperties": false,
            "required": ["email"],
            "properties": {
                "name": {
                    "type": "string",
                    "pattern": non_blank_pattern(),
                    "description": "The person's display name; the member key when omitted.",
                },
                "email": {
                    "type": "string",
                    "pattern": "@",
                    "description": "The person's email address, by which a speaker's entry is \
                        found; two addresses differing only in case are one, and an address is \
                        listed by at most one member.",
                },
                "credentials": {
                    "type": "object",
                    "description": "The person's own logins for the company's tools, keyed by \
                        the name an `mcp.<name>.credential` gives.",
                    "additionalProperties": secret_reference(
                        "The person's own login for that tool, never a literal value",
                        super::LayerKind::Company,
                    ),
                },
            },
        },
    })
}

/// The editor schema of a company home's `connectors` map: one named entry per
/// connector, defaulting `kind` to the map key when omitted, plus common
/// conversation settings (`role`, `admit`, `session_scope`, `reply_mode`) and
/// the provider fields declared by its kind. Each kind's own entry carries
/// the kind's setup, its manifest in full, as its description; a token field
/// says where its value comes from and how that value is written.
pub(super) fn connectors_schema() -> serde_json::Value {
    let reference = |source: &str| {
        serde_json::json!({
            "type": "string",
            "pattern": "^\\$\\{[A-Za-z_][A-Za-z0-9_]*\\}$",
            "description": format!(
                "{source}; an exact ${{NAME}} reference into the company home's secrets file, its value written with `instrukt secrets set <NAME> --company <name>`"
            ),
        })
    };
    let entry_schema = |declaration: &Declaration, kind_required: bool| {
        let mut properties = serde_json::Map::new();
        let mut required: Vec<&str> = Vec::new();
        properties.insert(
            "kind".to_owned(),
            serde_json::json!({
                "enum": [declaration.key],
                "default": declaration.key,
                "description": "registered connector kind; defaults to the map key when omitted",
            }),
        );
        if kind_required {
            required.push("kind");
        }
        properties.insert(
            "role".to_owned(),
            serde_json::json!({
                "type": "string",
                "default": COMPANY_AGENT_ROLE,
                "description": "role assigned to this connector's session(s); defaults to company-agent",
            }),
        );
        properties.insert(
            "admit".to_owned(),
            serde_json::json!({
                "enum": ["members", "admins_only"],
                "default": "members",
                "description": "who is admitted to address this connector",
            }),
        );
        properties.insert(
            "session_scope".to_owned(),
            serde_json::json!({
                "enum": ["conversation", "company"],
                "default": "conversation",
                "description": "whether each conversation gets its own session or all admitted conversations share one standing session per (company, connector)",
            }),
        );
        properties.insert(
            "reply_mode".to_owned(),
            serde_json::json!({
                "enum": ["turn_output", "explicit"],
                "default": "turn_output",
                "description": "whether finished turn output posts automatically or only explicit `instrukt company reply` calls post",
            }),
        );
        for field in declaration.fields {
            let schema = match field.rule {
                Rule::SecretReference {
                    required: needed,
                    source,
                } => {
                    if needed {
                        required.push(field.name);
                    }
                    reference(source)
                }
                Rule::PositiveCount { default, meaning } => serde_json::json!({
                    "type": "integer",
                    "minimum": 1,
                    "default": default,
                    "description": meaning,
                }),
            };
            properties.insert(field.name.to_owned(), schema);
        }
        let description = if kind_required {
            format!(
                "a further `{key}` connector under its own name; set it up as `connectors.{key}` shows",
                key = declaration.key
            )
        } else {
            setup_text(declaration)
        };
        serde_json::json!({
            "type": "object",
            "description": description,
            "additionalProperties": false,
            "required": required,
            "properties": properties,
        })
    };
    let mut kinds = serde_json::Map::new();
    let mut named_variants = Vec::new();
    for declaration in super::_connectors::declarations() {
        kinds.insert(declaration.key.to_owned(), entry_schema(declaration, false));
        named_variants.push(entry_schema(declaration, true));
    }
    let additional = match named_variants.len() {
        0 => serde_json::json!(false),
        1 => named_variants
            .pop()
            .unwrap_or_else(|| serde_json::json!(false)),
        _ => serde_json::json!({ "anyOf": named_variants }),
    };
    serde_json::json!({
        "type": "object",
        "description": "The connectors the company's people reach its agent through, one named \
            entry each; an entry under a kind's own key, such as `slack`, is of that kind.",
        "additionalProperties": additional,
        "properties": kinds,
    })
}

/// A kind's setup as its schema entry describes it: the setup, then the
/// manifest in full when the kind has one.
fn setup_text(declaration: &Declaration) -> String {
    let setup = declaration.setup.trim_end();
    match declaration.manifest {
        Some(manifest) => format!("{setup}\nManifest:\n{}", manifest.trim_end()),
        None => setup.to_owned(),
    }
}

fn finding(path: &str, kind: FindingKind, message: impl Into<String>) -> KeyFinding {
    KeyFinding::new(path, kind, message)
}

/// The findings of a `members` map.
pub(super) fn check_members(value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let Some(members) = value.as_mapping() else {
        out.push(finding(
            path,
            FindingKind::InvalidValue,
            "expected a mapping of member keys to member entries",
        ));
        return;
    };
    let mut listed: BTreeMap<String, String> = BTreeMap::new();
    for (key, entry) in members {
        let key = key_text(key);
        let at = join(path, &key);
        if ProjectSlug::parse(&key).is_err() {
            out.push(finding(
                &at,
                FindingKind::InvalidValue,
                "a member key follows the grammar of a project slug",
            ));
        }
        let Some(fields) = entry.as_mapping() else {
            out.push(finding(
                &at,
                FindingKind::InvalidValue,
                "expected a mapping",
            ));
            continue;
        };
        if !fields.contains_key("email") {
            out.push(finding(
                &join(&at, "email"),
                FindingKind::Missing,
                "missing; the person's email address, by which their own logins are found",
            ));
        }
        for (field, value) in fields {
            let field = key_text(field);
            let field_at = join(&at, &field);
            match field.as_str() {
                "name" if value.as_str().is_some_and(|name| !name.trim().is_empty()) => {}
                "name" => out.push(finding(
                    &field_at,
                    FindingKind::InvalidValue,
                    "expected a nonempty display name",
                )),
                "email" => match value
                    .as_str()
                    .map(str::trim)
                    .filter(|email| email.contains('@'))
                {
                    Some(email) => {
                        // Two addresses that differ only in case are one.
                        let address = email.to_lowercase();
                        if let Some(other) = listed.get(&address) {
                            out.push(finding(
                                &field_at,
                                FindingKind::InvalidValue,
                                format!(
                                    "this email address is also listed by member `{other}`; an address is listed by at most one member"
                                ),
                            ));
                        } else {
                            listed.insert(address, key.clone());
                        }
                    }
                    None => out.push(finding(
                        &field_at,
                        FindingKind::InvalidValue,
                        "expected the person's email address",
                    )),
                },
                "credentials" => check_credentials(value, &field_at, out),
                _ => out.push(finding(
                    &field_at,
                    FindingKind::UnknownKey,
                    "unknown member field; a member has name, email and credentials",
                )),
            }
        }
    }
}

fn check_credentials(value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let Some(credentials) = value.as_mapping() else {
        out.push(finding(
            path,
            FindingKind::InvalidValue,
            "expected a mapping of tools to ${NAME} references",
        ));
        return;
    };
    for (tool, reference) in credentials {
        if !reference.as_str().is_some_and(super::is_secret_reference) {
            out.push(finding(
                &join(path, &key_text(tool)),
                FindingKind::InvalidValue,
                "expected an exact ${NAME} reference into the home's secrets file; a literal credential is not stored",
            ));
        }
    }
}

/// The findings of a `connectors` map, against the installed kinds and common
/// connector settings.
pub(super) fn check_connectors(value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let Some(connectors) = value.as_mapping() else {
        out.push(finding(
            path,
            FindingKind::InvalidValue,
            "expected a mapping of connectors",
        ));
        return;
    };
    let supported_list = || {
        super::_connectors::declarations()
            .iter()
            .map(|declaration| declaration.key)
            .collect::<Vec<_>>()
            .join(", ")
    };
    for (name, declaration) in connectors {
        let name = key_text(name);
        let at = join(path, &name);
        if ProjectSlug::parse(&name).is_err() {
            out.push(finding(
                &at,
                FindingKind::InvalidValue,
                "a connector name follows the grammar of a project slug",
            ));
        }
        let Some(fields) = declaration.as_mapping() else {
            out.push(finding(
                &at,
                FindingKind::InvalidValue,
                "expected a mapping",
            ));
            continue;
        };
        let kind = match fields.get("kind") {
            Some(raw_kind) => {
                let Some(kind_str) = raw_kind.as_str() else {
                    out.push(finding(
                        &join(&at, "kind"),
                        FindingKind::InvalidValue,
                        format!(
                            "expected a connector kind name; supported: {}",
                            supported_list()
                        ),
                    ));
                    continue;
                };
                match super::_connectors::declaration(kind_str) {
                    Some(kind) => kind,
                    None => {
                        out.push(finding(
                            &join(&at, "kind"),
                            FindingKind::InvalidValue,
                            format!(
                                "unsupported connector kind `{kind_str}`; supported: {}",
                                supported_list()
                            ),
                        ));
                        continue;
                    }
                }
            }
            None => match super::_connectors::declaration(&name) {
                Some(kind) => kind,
                None => {
                    out.push(finding(
                        &at,
                        FindingKind::UnknownKey,
                        format!(
                            "unsupported connector; supported kinds: {} (set `kind` for a named connector)",
                            supported_list()
                        ),
                    ));
                    continue;
                }
            },
        };
        for field in kind.fields {
            if matches!(field.rule, Rule::SecretReference { required: true, .. })
                && !fields.contains_key(field.name)
            {
                out.push(finding(
                    &join(&at, field.name),
                    FindingKind::Missing,
                    "missing; a ${NAME} reference into the home's secrets file",
                ));
            }
        }
        for (field, value) in fields {
            let field = key_text(field);
            let field_at = join(&at, &field);
            match field.as_str() {
                "kind" => {}
                "role" => {
                    if !value
                        .as_str()
                        .is_some_and(|role| RoleName::parse(role).is_ok())
                    {
                        out.push(finding(
                            &field_at,
                            FindingKind::InvalidValue,
                            "expected a valid role name",
                        ));
                    }
                }
                "admit" => {
                    if !value
                        .as_str()
                        .is_some_and(|v| ConnectorAdmit::parse(v).is_some())
                    {
                        out.push(finding(
                            &field_at,
                            FindingKind::InvalidValue,
                            "expected `members` or `admins_only`",
                        ));
                    }
                }
                "session_scope" => {
                    if !value
                        .as_str()
                        .is_some_and(|v| SessionScope::parse(v).is_some())
                    {
                        out.push(finding(
                            &field_at,
                            FindingKind::InvalidValue,
                            "expected `conversation` or `company`",
                        ));
                    }
                }
                "reply_mode" => {
                    if !value
                        .as_str()
                        .is_some_and(|v| ReplyMode::parse(v).is_some())
                    {
                        out.push(finding(
                            &field_at,
                            FindingKind::InvalidValue,
                            "expected `turn_output` or `explicit`",
                        ));
                    }
                }
                _ => match kind.fields.iter().find(|known| known.name == field) {
                    Some(known) => match known.rule {
                        Rule::SecretReference { .. } => {
                            if !value.as_str().is_some_and(super::is_secret_reference) {
                                out.push(finding(
                                    &field_at,
                                    FindingKind::InvalidValue,
                                    "expected an exact ${NAME} reference; a literal token is not stored",
                                ));
                            }
                        }
                        Rule::PositiveCount { meaning, .. } => {
                            if !value.as_u64().is_some_and(|count| count > 0) {
                                out.push(finding(&field_at, FindingKind::InvalidValue, meaning));
                            }
                        }
                    },
                    None => out.push(finding(
                        &field_at,
                        FindingKind::UnknownKey,
                        format!(
                            "unknown field; {} takes {}, kind, role, admit, session_scope and reply_mode",
                            kind.key,
                            listed(kind)
                        ),
                    )),
                },
            }
        }
    }
}

/// A kind's field names, as a sentence lists them: `a, b and c`.
fn listed(kind: &Declaration) -> String {
    let names: Vec<&str> = kind.fields.iter().map(|field| field.name).collect();
    match names.split_last() {
        None => String::new(),
        Some((last, [])) => (*last).to_owned(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

fn invalid(key: &str, message: &str) -> ConfigError {
    ConfigError::InvalidValue {
        key: key.into(),
        message: message.into(),
    }
}

/// The member list of a company document's `members` value.
pub(super) fn members(raw: Option<&Value>) -> Result<Members, ConfigError> {
    let Some(raw) = raw else {
        return Ok(Members::default());
    };
    let mut found = Vec::new();
    check_members(raw, "members", &mut found);
    if let Some(first) = found.first() {
        return Err(invalid(&first.key, &first.message));
    }
    let mut members = Vec::new();
    for (key, entry) in raw.as_mapping().into_iter().flatten() {
        let key =
            ProjectSlug::parse(&key_text(key)).map_err(|e| invalid("members", &e.to_string()))?;
        let mut member = Member {
            name: key.to_string(),
            key,
            email: String::new(),
            credentials: BTreeMap::new(),
        };
        for (field, value) in entry.as_mapping().into_iter().flatten() {
            match key_text(field).as_str() {
                "name" => member.name = value.as_str().unwrap_or_default().to_owned(),
                "credentials" => {
                    for (tool, reference) in value.as_mapping().into_iter().flatten() {
                        member.credentials.insert(
                            key_text(tool),
                            reference.as_str().unwrap_or_default().to_owned(),
                        );
                    }
                }
                "email" => member.email = value.as_str().unwrap_or_default().trim().to_owned(),
                _ => {}
            }
        }
        members.push(member);
    }
    Ok(Members(members))
}

/// The connectors a company document's `connectors` value declares, each
/// as declared, references unresolved.
pub(super) fn connectors(raw: Option<&Value>) -> Result<Vec<DeclaredConnector>, ConfigError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    check_connectors(raw, "connectors", &mut found);
    if let Some(first) = found.first() {
        return Err(invalid(&first.key, &first.message));
    }
    raw.as_mapping()
        .into_iter()
        .flatten()
        .map(|(key, entry)| {
            let name = key_text(key);
            let kind = entry
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or(&name)
                .to_owned();
            let role_str = entry
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or(COMPANY_AGENT_ROLE);
            let role = RoleName::parse(role_str)
                .map_err(|e| invalid(&join("connectors", &join(&name, "role")), &e.to_string()))?;
            let admit = entry
                .get("admit")
                .and_then(Value::as_str)
                .and_then(ConnectorAdmit::parse)
                .unwrap_or_default();
            let session_scope = entry
                .get("session_scope")
                .and_then(Value::as_str)
                .and_then(SessionScope::parse)
                .unwrap_or_default();
            let reply_mode = entry
                .get("reply_mode")
                .and_then(Value::as_str)
                .and_then(ReplyMode::parse)
                .unwrap_or_default();
            Ok(DeclaredConnector {
                key: name,
                kind,
                role,
                admit,
                session_scope,
                reply_mode,
                entry: entry.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(text: &str) -> Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    fn judged(
        value: &str,
        check: fn(&Value, &str, &mut Vec<KeyFinding>),
        path: &str,
    ) -> Vec<(String, FindingKind)> {
        let mut out = Vec::new();
        check(&yaml(value), path, &mut out);
        out.into_iter().map(|f| (f.key, f.kind)).collect()
    }

    #[test]
    fn an_email_address_finds_the_one_member_listing_it_regardless_of_case() {
        let members = members(Some(&yaml(
            "alex: {name: Alex, email: Alex@Example.com, credentials: {erp: '${ALEX_ERP}'}}\nsam: {email: sam@example.com}\n",
        )))
        .unwrap();
        let alex = members.by_email("alex@example.com").unwrap();
        assert_eq!(alex.key, ProjectSlug::parse("alex").unwrap());
        // Without a display name, the member is named by their key.
        let sam = members.by_email("SAM@example.com").unwrap();
        assert_eq!(ProjectSlug::parse(&sam.name).as_ref(), Ok(&sam.key));
        assert_eq!(members.by_email("other@example.com"), None);
    }

    #[test]
    fn a_slack_declaration_needs_both_references_and_a_positive_context() {
        use FindingKind::{InvalidValue, Missing, UnknownKey};
        crate::config::_connectors::install_fixture();
        assert_eq!(
            judged(
                "slack: {app_token: literal, context_messages: 0, extra: 1}\nteams: {}\n",
                check_connectors,
                "connectors"
            ),
            [
                ("connectors.slack.bot_token".to_owned(), Missing),
                ("connectors.slack.app_token".to_owned(), InvalidValue),
                ("connectors.slack.context_messages".to_owned(), InvalidValue),
                ("connectors.slack.extra".to_owned(), UnknownKey),
                ("connectors.teams".to_owned(), UnknownKey),
            ]
        );
        // Both references suffice.
        assert_eq!(
            judged(
                "slack: {app_token: '${A}', bot_token: '${B}'}\n",
                check_connectors,
                "connectors"
            ),
            []
        );
    }

    #[test]
    fn missing_or_shared_addresses_literal_credentials_and_unknown_fields_are_findings() {
        use FindingKind::{InvalidValue, Missing, UnknownKey};
        assert_eq!(
            judged(
                "a: {email: a@example.com, credentials: {erp: plain}, slack: U1}\nb: {email: A@EXAMPLE.COM}\nc: {name: C}\n",
                check_members,
                "members"
            ),
            [
                ("members.a.credentials.erp".to_owned(), InvalidValue),
                ("members.a.slack".to_owned(), UnknownKey),
                ("members.b.email".to_owned(), InvalidValue),
                ("members.c.email".to_owned(), Missing),
            ]
        );
    }
}
