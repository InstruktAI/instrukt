//! A company home's members and connector declarations
//! (`company-homes`, "Members"; `slack-connector`, "Declaration").

use std::collections::BTreeMap;

use serde_yaml_ng::Value;

use super::_error::{ConfigError, FindingKind};
use super::_interpolate::{join, key_text};
use super::_schema::KeyFinding;
use crate::ids::ProjectSlug;

/// The connectors this instrukt supports, by their key under `connectors`.
/// Older member entries named a person's identity under the same keys.
pub const CONNECTORS: [&str; 1] = ["slack"];

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

/// The largest number of earlier messages that accompany an addressed
/// message when a Slack declaration does not say.
pub const DEFAULT_CONTEXT_MESSAGES: u64 = 30;

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

/// A company home's Slack connector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackConnector {
    /// Reference to the app-level connection token.
    pub app_token: String,
    /// Reference to the bot token used to read and post.
    pub bot_token: String,
    /// The largest number of earlier messages that accompany an addressed
    /// message.
    pub context_messages: u64,
}

/// What `context_messages` is, as its finding and editor schema say it.
const CONTEXT_MESSAGES: &str = "the largest number of earlier messages that accompany an addressed message: a positive whole number; 30 when omitted";

/// The editor schema of a company home's `connectors` map.
pub(super) fn connectors_schema() -> serde_json::Value {
    let reference =
        serde_json::json!({"type": "string", "pattern": "^\\$\\{[A-Za-z_][A-Za-z0-9_]*\\}$"});
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "slack": {
                "type": "object",
                "additionalProperties": false,
                "required": ["app_token", "bot_token"],
                "properties": {
                    "app_token": reference,
                    "bot_token": reference,
                    "context_messages": {
                        "type": "integer",
                        "minimum": 1,
                        "default": DEFAULT_CONTEXT_MESSAGES,
                        "description": CONTEXT_MESSAGES,
                    },
                },
            },
        },
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

/// The findings of a `connectors` map.
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
        if name != "slack" {
            out.push(finding(
                &at,
                FindingKind::UnknownKey,
                format!(
                    "unsupported connector; supported: {}",
                    CONNECTORS.join(", ")
                ),
            ));
            continue;
        }
        let Some(fields) = declaration.as_mapping() else {
            out.push(finding(
                &at,
                FindingKind::InvalidValue,
                "expected a mapping",
            ));
            continue;
        };
        for required in ["app_token", "bot_token"] {
            if !fields.contains_key(required) {
                out.push(finding(
                    &join(&at, required),
                    FindingKind::Missing,
                    "missing; a ${NAME} reference into the home's secrets file",
                ));
            }
        }
        for (field, value) in fields {
            let field = key_text(field);
            let field_at = join(&at, &field);
            match field.as_str() {
                "app_token" | "bot_token" => {
                    if !value.as_str().is_some_and(super::is_secret_reference) {
                        out.push(finding(
                            &field_at,
                            FindingKind::InvalidValue,
                            "expected an exact ${NAME} reference; a literal token is not stored",
                        ));
                    }
                }
                "context_messages" => {
                    if !value.as_u64().is_some_and(|count| count > 0) {
                        out.push(finding(
                            &field_at,
                            FindingKind::InvalidValue,
                            CONTEXT_MESSAGES,
                        ));
                    }
                }
                _ => out.push(finding(
                    &field_at,
                    FindingKind::UnknownKey,
                    "unknown field; slack takes app_token, bot_token and context_messages",
                )),
            }
        }
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

/// The Slack connector a company document declares, if any.
pub(super) fn slack(raw: Option<&Value>) -> Result<Option<SlackConnector>, ConfigError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut found = Vec::new();
    check_connectors(raw, "connectors", &mut found);
    if let Some(first) = found.first() {
        return Err(invalid(&first.key, &first.message));
    }
    let Some(slack) = raw.get("slack") else {
        return Ok(None);
    };
    let text = |field: &str| {
        slack
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Ok(Some(SlackConnector {
        app_token: text("app_token"),
        bot_token: text("bot_token"),
        context_messages: slack
            .get("context_messages")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_CONTEXT_MESSAGES),
    }))
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

    #[test]
    fn a_slack_declaration_needs_both_references_and_a_positive_context() {
        use FindingKind::{InvalidValue, Missing, UnknownKey};
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
        let declared = slack(Some(&yaml(
            "slack: {app_token: '${A}', bot_token: '${B}'}\n",
        )))
        .unwrap()
        .unwrap();
        assert_eq!(declared.context_messages, DEFAULT_CONTEXT_MESSAGES);
    }
}
