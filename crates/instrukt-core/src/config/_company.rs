//! A company home's members and connector declarations
//! (`company-homes`, "Members"; `slack-connector`, "Declaration").

use std::collections::BTreeMap;

use serde_yaml_ng::Value;

use instrukt_connector::{Declaration, Rule};

use super::_connectors::DeclaredConnector;
use super::_error::{ConfigError, FindingKind};
use super::_interpolate::{join, key_text};
use super::_schema::KeyFinding;
use crate::ids::ProjectSlug;

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

/// The editor schema of a company home's `connectors` map: one property
/// per installed kind, its fields as its declaration states them.
pub(super) fn connectors_schema() -> serde_json::Value {
    let reference =
        serde_json::json!({"type": "string", "pattern": "^\\$\\{[A-Za-z_][A-Za-z0-9_]*\\}$"});
    let mut kinds = serde_json::Map::new();
    for declaration in super::_connectors::declarations() {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for field in declaration.fields {
            let schema = match field.rule {
                Rule::SecretReference { required: needed } => {
                    if needed {
                        required.push(field.name);
                    }
                    reference.clone()
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
        kinds.insert(
            declaration.key.to_owned(),
            serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": required,
                "properties": properties,
            }),
        );
    }
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": kinds,
    })
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

/// The findings of a `connectors` map, against the installed kinds.
pub(super) fn check_connectors(value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let Some(connectors) = value.as_mapping() else {
        out.push(finding(
            path,
            FindingKind::InvalidValue,
            "expected a mapping of connectors",
        ));
        return;
    };
    for (name, declaration) in connectors {
        let name = key_text(name);
        let at = join(path, &name);
        let Some(kind) = super::_connectors::declaration(&name) else {
            let supported: Vec<&str> = super::_connectors::declarations()
                .iter()
                .map(|declaration| declaration.key)
                .collect();
            out.push(finding(
                &at,
                FindingKind::UnknownKey,
                format!("unsupported connector; supported: {}", supported.join(", ")),
            ));
            continue;
        };
        let Some(fields) = declaration.as_mapping() else {
            out.push(finding(
                &at,
                FindingKind::InvalidValue,
                "expected a mapping",
            ));
            continue;
        };
        for field in kind.fields {
            if matches!(field.rule, Rule::SecretReference { required: true })
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
            match kind.fields.iter().find(|known| known.name == field) {
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
                    format!("unknown field; {} takes {}", kind.key, listed(kind)),
                )),
            }
        }
    }
}

/// The findings of a `connectors` value at `path`, by key and kind, against
/// the installed kinds.
pub fn connector_findings(value: &Value, path: &str) -> Vec<(String, FindingKind)> {
    let mut out = Vec::new();
    check_connectors(value, path, &mut out);
    out.into_iter()
        .map(|found| (found.key, found.kind))
        .collect()
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
    Ok(raw
        .as_mapping()
        .into_iter()
        .flatten()
        .map(|(key, entry)| DeclaredConnector {
            key: key_text(key),
            entry: entry.clone(),
        })
        .collect())
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
