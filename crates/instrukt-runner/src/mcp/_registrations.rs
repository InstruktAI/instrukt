//! Durable OAuth client registrations (`mcp_registrations`):
//! - Host-level shared OAuth clients (`mcp_clients.<name>`, `company = NULL, server = NULL`)
//! - Company-local declared OAuth clients (`sign_in.client`)
//! - Dynamic Client Registration (RFC 7591)
//!
//! Secrets for declared clients are never persisted into SQLite (`client_secret`
//! stays NULL; only `secret_digest = SHA-256(secret)` is recorded to detect rotation).

use base64::Engine as _;
use instrukt_core::config::{
    Configuration, McpClient, SecretSource, SignInClient, resolve_secret_from,
};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use instrukt_core::state_db::{StateDb, StateDbError, StateRead};
use jiff::Timestamp;
use ring::rand::{SecureRandom, SystemRandom};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::_discover::DiscoveredAuth;
use super::McpError;

/// Public view of one reusable host-level OAuth client (`instrukt mcp clients`).
/// Never carries secret material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostClientView {
    pub name: String,
    pub issuer: String,
    pub initialized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_address: Option<String>,
    pub has_secret: bool,
}

/// Read-only registration hint for `instrukt mcp list` (never reads secrets or writes DB).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerRegistrationHint {
    Initialized {
        client_id: String,
        return_address: Option<String>,
    },
    Uninitialized {
        message: String,
    },
}

/// Active registration row loaded from `mcp_registrations`.
#[derive(Debug, Clone)]
pub(crate) struct Registration {
    pub return_id: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub return_address: String,
}

/// 32 lowercase hex characters from 128 cryptographically random bits.
pub(crate) fn random_hex_128() -> Result<String, McpError> {
    let mut bytes = [0u8; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| McpError::Server {
            server: "random".into(),
            cause: "secure random source unavailable".into(),
        })?;
    Ok(hex::encode(bytes))
}

/// Base64url (unpadded) encoding of 256 cryptographically random bits.
pub(crate) fn random_base64url_256() -> Result<String, McpError> {
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| McpError::Server {
            server: "random".into(),
            cause: "secure random source unavailable".into(),
        })?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

pub(crate) fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

pub(crate) fn sha256_base64url(input: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(input.as_bytes()))
}

pub(crate) fn return_address_for(ingress_url: Option<&str>, return_id: &str) -> String {
    match ingress_url {
        Some(base) => format!(
            "{}/hooks/inbound/sign-in/{return_id}",
            base.trim_end_matches('/')
        ),
        None => format!("/hooks/inbound/sign-in/{return_id}"),
    }
}

/// List all global `mcp_clients.<name>` entries with their pinned issuers,
/// non-secret client IDs, and stable shared return addresses. Never discloses any secret.
/// When `read_only` is true (admin DM session), reads only `mcp_registrations` and
/// never touches `~/.instrukt/secrets.env` or the state writer lock.
pub fn list_host_clients(
    home: &Home,
    read_only: bool,
    now: Timestamp,
) -> Result<Vec<HostClientView>, McpError> {
    let inspection = home.for_inspection();
    let global = Configuration::load_global(&inspection)?;
    let declared = global.mcp_clients()?;
    let ingress_url = global.ingress_url()?;
    let mut out = Vec::new();
    for (name, client) in declared {
        let reg = if read_only {
            read_host_client_registration(&inspection, &name, ingress_url.as_deref())?
        } else {
            Some(ensure_host_client_registration(
                home,
                &name,
                &client,
                ingress_url.as_deref(),
                now,
            )?)
        };
        let (initialized, client_id, return_id, return_address) = match reg {
            Some(reg) => {
                let addr = ingress_url
                    .as_deref()
                    .map(|base| return_address_for(Some(base), &reg.return_id));
                (true, Some(reg.client_id), Some(reg.return_id), addr)
            }
            None => (false, None, None, None),
        };
        out.push(HostClientView {
            name: name.to_string(),
            issuer: client.issuer,
            initialized,
            client_id,
            description: client.description,
            return_id,
            return_address,
            has_secret: client.secret.is_some(),
        });
    }
    Ok(out)
}

/// List company-declared `mcp.<server>.sign_in.client` registrations for `company`.
/// When `read_only` is false (local operator), materializes or refreshes each server's
/// declared registration so the operator can register the return address at the vendor
/// before the first speaker sign-in. When `read_only` is true (admin DM), reads only
/// `mcp_registrations` and never touches secrets or the state writer lock.
pub fn list_company_clients(
    home: &Home,
    company: &ProjectSlug,
    read_only: bool,
    now: Timestamp,
) -> Result<Vec<HostClientView>, McpError> {
    let inspection = home.for_inspection();
    let global = Configuration::load_global(&inspection)?;
    let ingress_url = global.ingress_url()?;
    let company_cfg = Configuration::load_company(&inspection, company)?;
    let servers = company_cfg.mcp_servers(instrukt_core::config::LayerKind::Company)?;
    let mut out = Vec::new();
    for (name, server) in servers {
        let Some(local_client) = server.sign_in.as_ref().and_then(|s| s.client.as_ref()) else {
            continue;
        };
        let reg = if read_only {
            read_company_client_registration(&inspection, company, &name, ingress_url.as_deref())?
        } else {
            Some(ensure_company_client_registration(
                home,
                company,
                &name,
                local_client,
                "",
                ingress_url.as_deref(),
                now,
            )?)
        };
        let (initialized, issuer, client_id, return_id, return_address) = match reg {
            Some(reg) => {
                let addr = ingress_url
                    .as_deref()
                    .map(|base| return_address_for(Some(base), &reg.return_id));
                (
                    true,
                    reg.issuer,
                    Some(reg.client_id),
                    Some(reg.return_id),
                    addr,
                )
            }
            None => (false, String::new(), None, None, None),
        };
        out.push(HostClientView {
            name: name.to_string(),
            issuer,
            initialized,
            client_id,
            description: Some(server.description),
            return_id,
            return_address,
            has_secret: local_client.secret.is_some(),
        });
    }
    Ok(out)
}

fn read_company_client_registration(
    home: &Home,
    company: &ProjectSlug,
    server: &ProjectSlug,
    ingress_url: Option<&str>,
) -> Result<Option<Registration>, McpError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(None);
    };
    let row = db
        .conn()
        .query_row(
            "SELECT return_id, issuer, client_id, client_secret, return_address
             FROM mcp_registrations
             WHERE company = ?1 AND server = ?2 AND host_client IS NULL AND declared = 1 AND retired_at IS NULL
             LIMIT 1",
            params![company.as_str(), server.as_str()],
            |r| {
                Ok(Registration {
                    return_id: r.get(0)?,
                    issuer: r.get(1)?,
                    client_id: r.get(2)?,
                    client_secret: r.get(3)?,
                    return_address: r.get(4)?,
                })
            },
        )
        .optional()
        .map_err(StateDbError::sqlite(db.path()))?;
    let Some(mut reg) = row else {
        return Ok(None);
    };
    if let Some(base) = ingress_url {
        reg.return_address = return_address_for(Some(base), &reg.return_id);
    }
    Ok(Some(reg))
}

fn read_host_client_registration(
    home: &Home,
    name: &ProjectSlug,
    ingress_url: Option<&str>,
) -> Result<Option<Registration>, McpError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(None);
    };
    let row = db
        .conn()
        .query_row(
            "SELECT return_id, issuer, client_id, client_secret, return_address
             FROM mcp_registrations
             WHERE host_client = ?1 AND company IS NULL AND server IS NULL AND retired_at IS NULL
             LIMIT 1",
            [name.as_str()],
            |r| {
                Ok(Registration {
                    return_id: r.get(0)?,
                    issuer: r.get(1)?,
                    client_id: r.get(2)?,
                    client_secret: r.get(3)?,
                    return_address: r.get(4)?,
                })
            },
        )
        .optional()
        .map_err(StateDbError::sqlite(db.path()))?;
    let Some(mut reg) = row else {
        return Ok(None);
    };
    if let Some(base) = ingress_url {
        reg.return_address = return_address_for(Some(base), &reg.return_id);
    }
    Ok(Some(reg))
}

/// Ensure a single stable host-scoped registration row exists for `mcp_clients.<name>`.
/// Pins the registration to `client.issuer` and reuses the same `return_id` across every
/// company referencing `name`. Retires the previous registration and purges its logins
/// and open sign-ins if `issuer`, `id`, or `secret` changed.
pub(crate) fn ensure_host_client_registration(
    home: &Home,
    name: &ProjectSlug,
    client: &McpClient,
    ingress_url: Option<&str>,
    now: Timestamp,
) -> Result<Registration, McpError> {
    let client_id = resolve_secret_from(home, &SecretSource::Machine, &client.id)?;
    let secret_digest = match &client.secret {
        Some(reference) => {
            let value = resolve_secret_from(home, &SecretSource::Machine, reference)?;
            Some(sha256_hex(&value))
        }
        None => None,
    };
    // Read-only fast path: avoid opening writer.lock when nothing changed (N-b).
    if let Some(read) = StateRead::open(&home.for_inspection())? {
        let existing: Option<(String, String, String, Option<String>, String)> = read
            .conn()
            .query_row(
                "SELECT return_id, issuer, client_id, secret_digest, return_address
                 FROM mcp_registrations
                 WHERE host_client = ?1 AND company IS NULL AND server IS NULL AND retired_at IS NULL
                 LIMIT 1",
                [name.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(StateDbError::sqlite(read.path()))?;
        if let Some((return_id, existing_issuer, existing_id, existing_digest, existing_addr)) =
            existing
        {
            let desired_addr = return_address_for(ingress_url, &return_id);
            let addr_ok = ingress_url.is_none() || existing_addr == desired_addr;
            if existing_issuer == client.issuer
                && existing_id == client_id
                && existing_digest == secret_digest
                && addr_ok
            {
                return Ok(Registration {
                    return_id,
                    issuer: existing_issuer,
                    client_id,
                    client_secret: None,
                    return_address: existing_addr,
                });
            }
        }
    }
    let fresh_return_id = random_hex_128()?;
    let mut db = StateDb::open(home)?;
    let path = db.path().to_path_buf();
    db.write::<_, StateDbError>(|tx| {
        let existing: Option<(String, String, String, Option<String>, String)> = tx
            .query_row(
                "SELECT return_id, issuer, client_id, secret_digest, return_address
                 FROM mcp_registrations
                 WHERE host_client = ?1 AND company IS NULL AND server IS NULL AND retired_at IS NULL
                 LIMIT 1",
                [name.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(StateDbError::sqlite(&path))?;

        if let Some((return_id, existing_issuer, existing_id, existing_digest, existing_addr)) =
            existing
        {
            if existing_issuer == client.issuer
                && existing_id == client_id
                && existing_digest == secret_digest
            {
                let desired_addr = return_address_for(ingress_url, &return_id);
                if ingress_url.is_some() && existing_addr != desired_addr {
                    tx.execute(
                        "UPDATE mcp_registrations SET return_address = ?1 WHERE return_id = ?2",
                        params![desired_addr, return_id],
                    )
                    .map_err(StateDbError::sqlite(&path))?;
                }
                let final_addr = if ingress_url.is_some() {
                    desired_addr
                } else {
                    existing_addr
                };
                return Ok(Registration {
                    return_id,
                    issuer: existing_issuer,
                    client_id,
                    client_secret: None,
                    return_address: final_addr,
                });
            }
            retire_registration(tx, &path, &return_id, now)?;
        }

        let return_address = return_address_for(ingress_url, &fresh_return_id);
        tx.execute(
            "INSERT INTO mcp_registrations
             (return_id, company, server, host_client, issuer, declared, client_id, client_secret, secret_digest, return_address, registered_at, retired_at)
             VALUES (?1, NULL, NULL, ?2, ?3, 1, ?4, NULL, ?5, ?6, ?7, NULL)",
            params![
                fresh_return_id,
                name.as_str(),
                client.issuer,
                client_id,
                secret_digest,
                return_address,
                now.to_string(),
            ],
        )
        .map_err(StateDbError::sqlite(&path))?;

        Ok(Registration {
            return_id: fresh_return_id,
            issuer: client.issuer.clone(),
            client_id,
            client_secret: None,
            return_address,
        })
    })
    .map_err(McpError::State)
}

/// Ensure a company-scoped declared client registration (`sign_in.client`), pinned to
/// the authorization server `issuer` once established.
pub(crate) fn ensure_company_client_registration(
    home: &Home,
    company: &ProjectSlug,
    server: &ProjectSlug,
    client: &SignInClient,
    issuer: &str,
    ingress_url: Option<&str>,
    now: Timestamp,
) -> Result<Registration, McpError> {
    let source = SecretSource::Company(company.clone());
    let client_id = resolve_secret_from(home, &source, &client.id)?;
    let secret_digest = match &client.secret {
        Some(reference) => {
            let value = resolve_secret_from(home, &source, reference)?;
            Some(sha256_hex(&value))
        }
        None => None,
    };
    // Read-only fast path: avoid opening writer.lock when registration matches unchanged (N-b).
    if let Some(read) = StateRead::open(&home.for_inspection())? {
        let existing: Option<(String, String, Option<String>, String, String)> = read
            .conn()
            .query_row(
                "SELECT return_id, client_id, secret_digest, return_address, issuer
                 FROM mcp_registrations
                 WHERE company = ?1 AND server = ?2 AND host_client IS NULL AND declared = 1 AND retired_at IS NULL
                 LIMIT 1",
                params![company.as_str(), server.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(StateDbError::sqlite(read.path()))?;
        if let Some((return_id, existing_id, existing_digest, existing_addr, existing_issuer)) =
            existing
            && existing_id == client_id
            && existing_digest == secret_digest
        {
            if !existing_issuer.is_empty() && !issuer.is_empty() && existing_issuer != issuer {
                return Err(McpError::Server {
                    server: server.to_string(),
                    cause: format!(
                        "discovered authorization server `{issuer}` does not match declared client's pinned issuer `{existing_issuer}`; restore `url` to the pinned authorization server or rotate `sign_in.client` credentials as the local operator to rebind"
                    ),
                });
            }
            let desired_addr = return_address_for(ingress_url, &return_id);
            let addr_ok = ingress_url.is_none() || existing_addr == desired_addr;
            let issuer_ok = issuer.is_empty() || existing_issuer == issuer;
            if addr_ok && issuer_ok {
                return Ok(Registration {
                    return_id,
                    issuer: existing_issuer,
                    client_id,
                    client_secret: None,
                    return_address: existing_addr,
                });
            }
        }
    }
    let fresh_return_id = random_hex_128()?;
    let mut db = StateDb::open(home)?;
    let path = db.path().to_path_buf();
    let result: Result<Result<Registration, String>, StateDbError> = db.write(|tx| {
        let existing: Option<(String, String, Option<String>, String, String)> = tx
            .query_row(
                "SELECT return_id, client_id, secret_digest, return_address, issuer
                 FROM mcp_registrations
                 WHERE company = ?1 AND server = ?2 AND host_client IS NULL AND declared = 1 AND retired_at IS NULL
                 LIMIT 1",
                params![company.as_str(), server.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(StateDbError::sqlite(&path))?;

        if let Some((return_id, existing_id, existing_digest, existing_addr, existing_issuer)) =
            existing
        {
            if existing_id == client_id && existing_digest == secret_digest {
                if !existing_issuer.is_empty() && !issuer.is_empty() && existing_issuer != issuer {
                    return Ok(Err(format!(
                        "discovered authorization server `{issuer}` does not match declared client's pinned issuer `{existing_issuer}`; restore `url` to the pinned authorization server or rotate `sign_in.client` credentials as the local operator to rebind"
                    )));
                }
                let desired_addr = return_address_for(ingress_url, &return_id);
                let final_issuer = if issuer.is_empty() {
                    existing_issuer.clone()
                } else {
                    issuer.to_owned()
                };
                let addr_changed = ingress_url.is_some() && existing_addr != desired_addr;
                let issuer_changed = existing_issuer != final_issuer;
                if addr_changed || issuer_changed {
                    let stored_addr = if ingress_url.is_some() {
                        &desired_addr
                    } else {
                        &existing_addr
                    };
                    tx.execute(
                        "UPDATE mcp_registrations SET return_address = ?1, issuer = ?2 WHERE return_id = ?3",
                        params![stored_addr, final_issuer, return_id],
                    )
                    .map_err(StateDbError::sqlite(&path))?;
                }
                let final_addr = if ingress_url.is_some() {
                    desired_addr
                } else {
                    existing_addr
                };
                return Ok(Ok(Registration {
                    return_id,
                    issuer: final_issuer,
                    client_id,
                    client_secret: None,
                    return_address: final_addr,
                }));
            }
            retire_registration(tx, &path, &return_id, now)?;
        }

        let return_address = return_address_for(ingress_url, &fresh_return_id);
        tx.execute(
            "INSERT INTO mcp_registrations
             (return_id, company, server, host_client, issuer, declared, client_id, client_secret, secret_digest, return_address, registered_at, retired_at)
             VALUES (?1, ?2, ?3, NULL, ?4, 1, ?5, NULL, ?6, ?7, ?8, NULL)",
            params![
                fresh_return_id,
                company.as_str(),
                server.as_str(),
                issuer,
                client_id,
                secret_digest,
                return_address,
                now.to_string(),
            ],
        )
        .map_err(StateDbError::sqlite(&path))?;

        Ok(Ok(Registration {
            return_id: fresh_return_id,
            issuer: issuer.to_owned(),
            client_id,
            client_secret: None,
            return_address,
        }))
    });
    match result.map_err(McpError::State)? {
        Ok(reg) => Ok(reg),
        Err(cause) => Err(McpError::Server {
            server: server.to_string(),
            cause,
        }),
    }
}

/// Ensure a dynamic RFC 7591 client registration for `(company, server, issuer)`.
pub(crate) fn ensure_dynamic_registration(
    home: &Home,
    company: &ProjectSlug,
    server: &ProjectSlug,
    auth: &DiscoveredAuth,
    ingress_url: &str,
    now: Timestamp,
) -> Result<Registration, McpError> {
    let existing = if let Some(read) = StateRead::open(home)? {
        read.conn()
            .query_row(
                "SELECT return_id, issuer, client_id, client_secret, return_address
                 FROM mcp_registrations
                 WHERE company = ?1 AND server = ?2 AND host_client IS NULL AND declared = 0 AND retired_at IS NULL
                 LIMIT 1",
                params![company.as_str(), server.as_str()],
                |r| {
                    Ok(Registration {
                        return_id: r.get(0)?,
                        issuer: r.get(1)?,
                        client_id: r.get(2)?,
                        client_secret: r.get(3)?,
                        return_address: r.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(StateDbError::sqlite(read.path()))?
    } else {
        None
    };

    if let Some(reg) = existing {
        let expected_addr = return_address_for(Some(ingress_url), &reg.return_id);
        if reg.issuer == auth.issuer && reg.return_address == expected_addr {
            return Ok(reg);
        }
    }

    let endpoint = auth
        .registration_endpoint
        .as_deref()
        .ok_or_else(|| McpError::Server {
            server: server.to_string(),
            cause: "authorization server offers no dynamic registration; declare `sign_in.client_ref` or `sign_in.client`".into(),
        })?;
    if !instrukt_core::config::mcp_address_admitted(endpoint) {
        return Err(McpError::Server {
            server: server.to_string(),
            cause: "registration endpoint must be https (or loopback http)".into(),
        });
    }

    let return_id = random_hex_128()?;
    let return_address = return_address_for(Some(ingress_url), &return_id);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(std::time::Duration::from_secs(10)))
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into();
    let payload = json!({
        "client_name": format!("instrukt {company}"),
        "redirect_uris": [return_address],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    let mut response = agent
        .post(endpoint)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .send(payload.to_string())
        .map_err(|error| McpError::Server {
            server: server.to_string(),
            cause: format!("dynamic client registration failed: {error}"),
        })?;
    let status = response.status().as_u16();
    let body_text = response
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_string()
        .unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(McpError::Server {
            server: server.to_string(),
            cause: format!(
                "dynamic client registration refused (HTTP {status}); declare `sign_in.client_ref` or `sign_in.client`"
            ),
        });
    }
    let body: Value = serde_json::from_str(&body_text).map_err(|_| McpError::Server {
        server: server.to_string(),
        cause: "dynamic client registration returned invalid JSON".into(),
    })?;
    let client_id = body
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| McpError::Server {
            server: server.to_string(),
            cause: "dynamic client registration omitted `client_id`".into(),
        })?
        .to_owned();
    let client_secret = body
        .get("client_secret")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if let Some(redirects) = body.get("redirect_uris").and_then(Value::as_array)
        && !redirects
            .iter()
            .any(|u| u.as_str() == Some(return_address.as_str()))
    {
        return Err(McpError::Server {
            server: server.to_string(),
            cause: "dynamic registration returned a different redirect URI; declare `sign_in.client_ref` or `sign_in.client`".into(),
        });
    }
    if let Some(grants) = body.get("grant_types").and_then(Value::as_array)
        && !grants
            .iter()
            .any(|g| g.as_str() == Some("authorization_code"))
    {
        return Err(McpError::Server {
            server: server.to_string(),
            cause: "dynamic registration omitted authorization_code grant".into(),
        });
    }
    if let Some(method) = body
        .get("token_endpoint_auth_method")
        .and_then(Value::as_str)
        && method != "none"
        && client_secret.is_none()
    {
        return Err(McpError::Server {
            server: server.to_string(),
            cause: format!(
                "dynamic registration required auth method `{method}` without issuing a client_secret"
            ),
        });
    }

    let mut db = StateDb::open(home)?;
    let path = db.path().to_path_buf();
    db.write::<_, StateDbError>(|tx| {
        if let Some(old_id) = tx
            .query_row(
                "SELECT return_id FROM mcp_registrations
                 WHERE company = ?1 AND server = ?2 AND host_client IS NULL AND declared = 0 AND retired_at IS NULL
                 LIMIT 1",
                params![company.as_str(), server.as_str()],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(StateDbError::sqlite(&path))?
        {
            retire_registration(tx, &path, &old_id, now)?;
        }
        tx.execute(
            "INSERT INTO mcp_registrations
             (return_id, company, server, host_client, issuer, declared, client_id, client_secret, secret_digest, return_address, registered_at, retired_at)
             VALUES (?1, ?2, ?3, NULL, ?4, 0, ?5, ?6, NULL, ?7, ?8, NULL)",
            params![
                return_id,
                company.as_str(),
                server.as_str(),
                auth.issuer,
                client_id,
                client_secret,
                return_address,
                now.to_string(),
            ],
        )
        .map_err(StateDbError::sqlite(&path))?;
        Ok(Registration {
            return_id,
            issuer: auth.issuer.clone(),
            client_id,
            client_secret,
            return_address,
        })
    })
    .map_err(McpError::State)
}

fn retire_registration(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    path: &std::path::Path,
    return_id: &str,
    now: Timestamp,
) -> Result<(), StateDbError> {
    let now_str = now.to_string();
    tx.execute(
        "UPDATE mcp_registrations SET retired_at = ?1 WHERE return_id = ?2",
        params![now_str, return_id],
    )
    .map_err(StateDbError::sqlite(path))?;
    tx.execute(
        "DELETE FROM mcp_logins WHERE return_id = ?1",
        params![return_id],
    )
    .map_err(StateDbError::sqlite(path))?;
    tx.execute(
        "UPDATE mcp_sign_ins SET state = 'cancelled', closed_at = ?1, code_verifier = NULL, nonce = NULL, link = NULL
         WHERE return_id = ?2 AND state IN ('open', 'exchanging')",
        params![now_str, return_id],
    )
    .map_err(StateDbError::sqlite(path))?;
    Ok(())
}

/// Read-only non-secret registration info attached to `instrukt mcp list` output
/// when a server uses `sign_in.client_ref` or `sign_in.client`. Never reads secrets
/// and never writes to the state database.
pub fn server_registration_hint(
    home: &Home,
    company: &ProjectSlug,
    server: &instrukt_core::config::McpServer,
) -> Result<Option<ServerRegistrationHint>, McpError> {
    let Some(sign_in) = server.sign_in.as_ref() else {
        return Ok(None);
    };
    let inspection = home.for_inspection();
    let global = Configuration::load_global(&inspection)?;
    let ingress_url = global.ingress_url()?;
    if let Some(client_ref) = &sign_in.client_ref {
        let slug = ProjectSlug::parse(client_ref).map_err(|error| McpError::Server {
            server: server.name.to_string(),
            cause: error.to_string(),
        })?;
        return match read_host_client_registration(&inspection, &slug, ingress_url.as_deref())? {
            Some(reg) => {
                let addr = ingress_url
                    .as_deref()
                    .map(|base| return_address_for(Some(base), &reg.return_id));
                Ok(Some(ServerRegistrationHint::Initialized {
                    client_id: reg.client_id,
                    return_address: addr,
                }))
            }
            None => Ok(Some(ServerRegistrationHint::Uninitialized {
                message: format!(
                    "host client `{client_ref}` not initialized yet; run `instrukt mcp clients` as the local operator"
                ),
            })),
        };
    }
    if sign_in.client.is_some() {
        let Some(db) = StateRead::open(&inspection)? else {
            return Ok(Some(ServerRegistrationHint::Uninitialized {
                message: "declared client registration not initialized yet".into(),
            }));
        };
        let row: Option<(String, String)> = db
            .conn()
            .query_row(
                "SELECT client_id, return_id FROM mcp_registrations
                 WHERE company = ?1 AND server = ?2 AND host_client IS NULL AND declared = 1 AND retired_at IS NULL
                 LIMIT 1",
                params![company.as_str(), server.name.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(StateDbError::sqlite(db.path()))?;
        return match row {
            Some((client_id, return_id)) => {
                let addr = ingress_url
                    .as_deref()
                    .map(|base| return_address_for(Some(base), &return_id));
                Ok(Some(ServerRegistrationHint::Initialized {
                    client_id,
                    return_address: addr,
                }))
            }
            None => Ok(Some(ServerRegistrationHint::Uninitialized {
                message: format!(
                    "declared client registration not initialized yet; run `instrukt mcp clients --company {company}` as the local operator"
                ),
            })),
        };
    }
    Ok(None)
}
