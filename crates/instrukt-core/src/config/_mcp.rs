//! MCP server declarations in the global layer and in a company home
//! (`mcp-access`, "Declaration").

use std::collections::BTreeMap;

use serde_yaml_ng::Value;

use super::_error::{ConfigError, FindingKind};
use super::_interpolate::{join, key_text};
use super::_schema::{KeyFinding, LayerKind};
use crate::ids::ProjectSlug;

/// One declared MCP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    /// The server's name.
    pub name: ProjectSlug,
    /// The server's network address.
    pub url: String,
    /// One line saying what the server is for.
    pub description: String,
    /// Company home only: the name under `members.<member>.credentials`
    /// whose value a speaker's call carries.
    pub credential: Option<String>,
    /// A `${NAME}` reference: the company's own login in a company home, the
    /// machine's own login in the global layer.
    pub login: Option<String>,
}

impl McpServer {
    /// Whether its calls carry no login.
    pub fn is_open(&self) -> bool {
        self.credential.is_none() && self.login.is_none()
    }
}

/// Whether a login may be sent to `url`: an `https` address, or an `http`
/// address on this computer's loopback, so a login never crosses the network
/// unprotected.
pub fn admitted_address(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    match parsed.scheme() {
        "https" => parsed.host().is_some(),
        "http" => match parsed.host() {
            Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            None => false,
        },
        _ => false,
    }
}

fn finding(path: &str, kind: FindingKind, message: impl Into<String>) -> KeyFinding {
    KeyFinding::new(path, kind, message)
}

/// The findings of an `mcp` map declared in `layer`.
pub(super) fn check(value: &Value, path: &str, layer: LayerKind, out: &mut Vec<KeyFinding>) {
    let Some(servers) = value.as_mapping() else {
        out.push(finding(
            path,
            FindingKind::InvalidValue,
            "expected a mapping of server names to declarations",
        ));
        return;
    };
    for (name, declaration) in servers {
        let name = key_text(name);
        let at = join(path, &name);
        if ProjectSlug::parse(&name).is_err() {
            out.push(finding(
                &at,
                FindingKind::InvalidValue,
                format!("server `{name}`: a server name follows the grammar of a project slug"),
            ));
        }
        let Some(fields) = declaration.as_mapping() else {
            out.push(finding(
                &at,
                FindingKind::InvalidValue,
                format!("server `{name}`: expected a mapping"),
            ));
            continue;
        };
        for required in ["url", "description"] {
            if !fields.contains_key(required) {
                out.push(finding(
                    &join(&at, required),
                    FindingKind::Missing,
                    format!("server `{name}`: missing `{required}`"),
                ));
            }
        }
        let has_credential = fields.contains_key("credential");
        for (field, value) in fields {
            let field = key_text(field);
            let field_at = join(&at, &field);
            let text = value
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty());
            match field.as_str() {
                "url" if text.is_some_and(|url| !admitted_address(url)) => out.push(finding(
                    &field_at,
                    FindingKind::InvalidValue,
                    format!(
                        "server `{name}`: `url` is an https address, or an http address on this computer's loopback; a login would cross the network unprotected"
                    ),
                )),
                "description" if text.is_some_and(|text| text.contains(['\n', '\r'])) => {
                    out.push(finding(
                        &field_at,
                        FindingKind::InvalidValue,
                        format!("server `{name}`: `description` is one line"),
                    ));
                }
                "url" | "description" if text.is_some() => {}
                "url" | "description" => out.push(finding(
                    &field_at,
                    FindingKind::InvalidValue,
                    format!("server `{name}`: `{field}` expects nonempty text"),
                )),
                "credential" if layer != LayerKind::Company => out.push(finding(
                    &field_at,
                    FindingKind::NotPermittedInLayer,
                    format!(
                        "server `{name}`: `credential` names a member's own login and is declared in a company home only"
                    ),
                )),
                "credential" if text.is_some() => {}
                "credential" => out.push(finding(
                    &field_at,
                    FindingKind::InvalidValue,
                    format!(
                        "server `{name}`: `credential` expects the name under members.<member>.credentials"
                    ),
                )),
                "login" => {
                    if !value.as_str().is_some_and(super::is_secret_reference) {
                        out.push(finding(
                            &field_at,
                            FindingKind::InvalidValue,
                            format!(
                                "server `{name}`: `login` expects an exact ${{NAME}} reference into the secrets file; a literal login is not stored"
                            ),
                        ));
                    } else if layer == LayerKind::Company && !has_credential {
                        out.push(finding(
                            &field_at,
                            FindingKind::Missing,
                            format!(
                                "server `{name}`: a company's `login` requires `credential`, the members' own login a speaker's call carries"
                            ),
                        ));
                    }
                }
                _ => out.push(finding(
                    &field_at,
                    FindingKind::UnknownKey,
                    format!(
                        "server `{name}`: unknown field; a server takes url, description, credential and login"
                    ),
                )),
            }
        }
    }
}

/// The servers an `mcp` value declared in `layer` names, by name.
pub(super) fn servers(
    raw: Option<&Value>,
    layer: LayerKind,
) -> Result<BTreeMap<ProjectSlug, McpServer>, ConfigError> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let mut found = Vec::new();
    check(raw, "mcp", layer, &mut found);
    if let Some(first) = found.first() {
        return Err(ConfigError::InvalidValue {
            key: first.key.clone(),
            message: first.message.clone(),
        });
    }
    let mut servers = BTreeMap::new();
    for (name, declaration) in raw.as_mapping().into_iter().flatten() {
        let name = ProjectSlug::parse(&key_text(name)).map_err(|e| ConfigError::InvalidValue {
            key: "mcp".into(),
            message: e.to_string(),
        })?;
        let text = |field: &str| {
            declaration
                .get(field)
                .and_then(Value::as_str)
                .map(|text| text.trim().to_owned())
        };
        servers.insert(
            name.clone(),
            McpServer {
                name,
                url: text("url").unwrap_or_default(),
                description: text("description").unwrap_or_default(),
                credential: text("credential"),
                login: text("login"),
            },
        );
    }
    Ok(servers)
}

#[cfg(test)]
mod tests {
    use super::{FindingKind, LayerKind, ProjectSlug, admitted_address, check, servers};

    fn judged(text: &str, layer: LayerKind) -> Vec<(String, FindingKind)> {
        let mut out = Vec::new();
        check(
            &serde_yaml_ng::from_str(text).unwrap(),
            "mcp",
            layer,
            &mut out,
        );
        out.into_iter().map(|f| (f.key, f.kind)).collect()
    }

    #[test]
    fn a_complete_declaration_has_no_findings_in_either_place() {
        let company = "erp: {url: 'https://erp.example/mcp', description: ERP, credential: erp, login: '${ERP_LOGIN}'}\nwiki: {url: 'https://wiki.example/mcp', description: Wiki}\n";
        assert_eq!(judged(company, LayerKind::Company), []);
        let machine = "search: {url: 'https://search.example/mcp', description: Search, login: '${SEARCH_LOGIN}'}\n";
        assert_eq!(judged(machine, LayerKind::Global), []);
    }

    #[test]
    fn misdeclared_servers_are_findings_naming_the_server_and_the_field() {
        use FindingKind::{InvalidValue, Missing, NotPermittedInLayer, UnknownKey};
        assert_eq!(
            judged(
                "a: {url: 'https://a.example', description: A, login: plain, extra: 1}\nb: {description: B, login: '${B}'}\n",
                LayerKind::Company
            ),
            [
                ("mcp.a.login".to_owned(), InvalidValue),
                ("mcp.a.extra".to_owned(), UnknownKey),
                ("mcp.b.url".to_owned(), Missing),
                ("mcp.b.login".to_owned(), Missing),
            ]
        );
        assert_eq!(
            judged(
                "c: {url: 'https://c.example', description: C, credential: c}\n",
                LayerKind::Global
            ),
            [("mcp.c.credential".to_owned(), NotPermittedInLayer)]
        );
    }

    #[test]
    fn a_login_goes_only_to_https_or_this_computers_loopback() {
        for admitted in [
            "https://erp.example/mcp",
            "http://127.0.0.1:8080/mcp",
            "http://localhost/mcp",
            "http://[::1]:9000/mcp",
        ] {
            assert!(admitted_address(admitted));
        }
        for refused in [
            "http://erp.example/mcp",
            "http://10.0.0.5/mcp",
            "ftp://erp.example/mcp",
            "erp.example/mcp",
        ] {
            assert!(!admitted_address(refused));
        }
        assert_eq!(
            judged(
                "a: {url: 'http://erp.example/mcp', description: A}\nb: {url: 'https://b.example', description: \"two\\nlines\"}\n",
                LayerKind::Company
            ),
            [
                ("mcp.a.url".to_owned(), FindingKind::InvalidValue),
                ("mcp.b.description".to_owned(), FindingKind::InvalidValue),
            ]
        );
    }

    #[test]
    fn declared_servers_are_read_with_their_logins_as_references() {
        let servers = servers(
            Some(&serde_yaml_ng::from_str(
                "erp: {url: 'https://erp.example/mcp', description: ERP, credential: erp, login: '${ERP_LOGIN}'}\nwiki: {url: 'https://wiki.example/mcp', description: Wiki}\n",
            )
            .unwrap()),
            LayerKind::Company,
        )
        .unwrap();
        let erp = &servers[&ProjectSlug::parse("erp").unwrap()];
        assert_eq!(erp.credential.as_deref(), Some("erp"));
        assert!(
            erp.login
                .as_deref()
                .is_some_and(crate::config::is_secret_reference)
        );
        assert!(servers[&ProjectSlug::parse("wiki").unwrap()].is_open());
    }
}
