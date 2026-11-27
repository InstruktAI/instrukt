//! Durable OAuth client registrations (`mcp_registrations`):
//! - Host-level shared OAuth clients (`mcp_clients.<name>`, `company = NULL, server = NULL`)
//! - Company-local declared OAuth clients (`sign_in.client`)
//! - Dynamic Client Registration (RFC 7591) via `rmcp::transport::auth::AuthorizationManager`
//!
//! Secrets for declared clients are never persisted into SQLite (`client_secret`
//! stays NULL; only `secret_digest = SHA-256(secret)` is recorded to detect rotation).

use std::path::Path;

use base64::Engine as _;
use instrukt_core::config::{
    Configuration, LayerKind, McpClient, McpServer, SecretSource, SignInClient, resolve_secret_from,
};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension, Transaction, params};
use instrukt_core::state_db::{StateDb, StateDbError, StateRead};
use jiff::Timestamp;
use ring::rand::{SecureRandom, SystemRandom};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::_discover::{self, DiscoveredAuth};
use super::_http;
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

struct StoredDeclaredRow {
    return_id: String,
    issuer: String,
    client_id: String,
    secret_digest: Option<String>,
    return_address: String,
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

pub(crate) fn return_address_for(ingress_url: Option<&str>, return_id: &str) -> String {
    match ingress_url {
        Some(base) => format!(
            "{}/hooks/inbound/sign-in/{return_id}",
            base.trim_end_matches('/')
        ),
        None => format!("/hooks/inbound/sign-in/{return_id}"),
    }
}

fn query_host_row(
    conn: &Connection,
    path: &Path,
    name: &str,
) -> Result<Option<StoredDeclaredRow>, StateDbError> {
    conn.query_row(
        "SELECT return_id, issuer, client_id, secret_digest, return_address
         FROM mcp_registrations
         WHERE host_client = ?1 AND company IS NULL AND server IS NULL AND retired_at IS NULL
         LIMIT 1",
        [name],
        |r| {
            Ok(StoredDeclaredRow {
                return_id: r.get(0)?,
                issuer: r.get(1)?,
                client_id: r.get(2)?,
                secret_digest: r.get(3)?,
                return_address: r.get(4)?,
            })
        },
    )
    .optional()
    .map_err(StateDbError::sqlite(path))
}

fn query_company_row(
    conn: &Connection,
    path: &Path,
    company: &str,
    server: &str,
) -> Result<Option<StoredDeclaredRow>, StateDbError> {
    conn.query_row(
        "SELECT return_id, issuer, client_id, secret_digest, return_address
         FROM mcp_registrations
         WHERE company = ?1 AND server = ?2 AND host_client IS NULL AND declared = 1 AND retired_at IS NULL
         LIMIT 1",
        params![company, server],
        |r| {
            Ok(StoredDeclaredRow {
                return_id: r.get(0)?,
                issuer: r.get(1)?,
                client_id: r.get(2)?,
                secret_digest: r.get(3)?,
                return_address: r.get(4)?,
            })
        },
    )
    .optional()
    .map_err(StateDbError::sqlite(path))
}

fn declared_to_registration(row: StoredDeclaredRow, ingress_url: Option<&str>) -> Registration {
    let return_address = ingress_url
        .map(|base| return_address_for(Some(base), &row.return_id))
        .unwrap_or(row.return_address);
    Registration {
        return_id: row.return_id,
        issuer: row.issuer,
        client_id: row.client_id,
        client_secret: None,
        return_address,
    }
}

/// List all global `mcp_clients.<name>` entries with their pinned issuers,
/// non-secret client IDs, and stable shared return addresses. Never discloses any secret.
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
            StateRead::open(&inspection)?
                .map(|db| query_host_row(db.conn(), db.path(), name.as_str()))
                .transpose()?
                .flatten()
                .map(|row| declared_to_registration(row, ingress_url.as_deref()))
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
    let servers = company_cfg.mcp_servers(LayerKind::Company)?;
    let mut out = Vec::new();
    for (name, server) in servers {
        let Some(local_client) = server.sign_in.as_ref().and_then(|s| s.client.as_ref()) else {
            continue;
        };
        let reg = if read_only {
            StateRead::open(&inspection)?
                .map(|db| query_company_row(db.conn(), db.path(), company.as_str(), name.as_str()))
                .transpose()?
                .flatten()
                .map(|row| declared_to_registration(row, ingress_url.as_deref()))
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

/// Ensure a single stable host-scoped registration row exists for `mcp_clients.<name>`.
pub(crate) fn ensure_host_client_registration(
    home: &Home,
    name: &ProjectSlug,
    client: &McpClient,
    ingress_url: Option<&str>,
    now: Timestamp,
) -> Result<Registration, McpError> {
    let client_id = resolve_secret_from(home, &SecretSource::Machine, &client.id)?;
    let secret_digest = client
        .secret
        .as_deref()
        .map(|r| resolve_secret_from(home, &SecretSource::Machine, r).map(|v| sha256_hex(&v)))
        .transpose()?;

    if let Some(read) = StateRead::open(&home.for_inspection())?
        && let Some(row) = query_host_row(read.conn(), read.path(), name.as_str())?
    {
        let desired_addr = return_address_for(ingress_url, &row.return_id);
        let addr_ok = ingress_url.is_none() || row.return_address == desired_addr;
        if row.issuer == client.issuer
            && row.client_id == client_id
            && row.secret_digest == secret_digest
            && addr_ok
        {
            return Ok(declared_to_registration(row, None));
        }
    }

    let fresh_return_id = random_hex_128()?;
    let mut db = StateDb::open(home)?;
    let path = db.path().to_path_buf();
    db.write::<_, StateDbError>(|tx| {
        if let Some(row) = query_host_row(tx, &path, name.as_str())? {
            if row.issuer == client.issuer
                && row.client_id == client_id
                && row.secret_digest == secret_digest
            {
                let desired_addr = return_address_for(ingress_url, &row.return_id);
                if ingress_url.is_some() && row.return_address != desired_addr {
                    tx.execute(
                        "UPDATE mcp_registrations SET return_address = ?1 WHERE return_id = ?2",
                        params![desired_addr, row.return_id],
                    )
                    .map_err(StateDbError::sqlite(&path))?;
                }
                return Ok(declared_to_registration(row, ingress_url));
            }
            retire_registration(tx, &path, &row.return_id, now)?;
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
    let secret_digest = client
        .secret
        .as_deref()
        .map(|r| resolve_secret_from(home, &source, r).map(|v| sha256_hex(&v)))
        .transpose()?;

    if let Some(read) = StateRead::open(&home.for_inspection())?
        && let Some(row) =
            query_company_row(read.conn(), read.path(), company.as_str(), server.as_str())?
        && row.client_id == client_id
        && row.secret_digest == secret_digest
    {
        if !row.issuer.is_empty() && !issuer.is_empty() && row.issuer != issuer {
            return Err(McpError::Server {
                server: server.to_string(),
                cause: format!(
                    "discovered authorization server `{issuer}` does not match declared client's pinned issuer `{}`; restore `url` to the pinned authorization server or rotate `sign_in.client` credentials as the local operator to rebind",
                    row.issuer
                ),
            });
        }
        let desired_addr = return_address_for(ingress_url, &row.return_id);
        let addr_ok = ingress_url.is_none() || row.return_address == desired_addr;
        let issuer_ok = issuer.is_empty() || row.issuer == issuer;
        if addr_ok && issuer_ok {
            return Ok(declared_to_registration(row, None));
        }
    }

    let fresh_return_id = random_hex_128()?;
    let mut db = StateDb::open(home)?;
    let path = db.path().to_path_buf();
    let result: Result<Result<Registration, String>, StateDbError> = db.write(|tx| {
        if let Some(row) = query_company_row(tx, &path, company.as_str(), server.as_str())? {
            if row.client_id == client_id && row.secret_digest == secret_digest {
                if !row.issuer.is_empty() && !issuer.is_empty() && row.issuer != issuer {
                    return Ok(Err(format!(
                        "discovered authorization server `{issuer}` does not match declared client's pinned issuer `{}`; restore `url` to the pinned authorization server or rotate `sign_in.client` credentials as the local operator to rebind",
                        row.issuer
                    )));
                }
                let desired_addr = return_address_for(ingress_url, &row.return_id);
                let final_issuer = if issuer.is_empty() {
                    row.issuer.clone()
                } else {
                    issuer.to_owned()
                };
                let addr_changed = ingress_url.is_some() && row.return_address != desired_addr;
                let issuer_changed = row.issuer != final_issuer;
                if addr_changed || issuer_changed {
                    let stored_addr = if ingress_url.is_some() {
                        &desired_addr
                    } else {
                        &row.return_address
                    };
                    tx.execute(
                        "UPDATE mcp_registrations SET return_address = ?1, issuer = ?2 WHERE return_id = ?3",
                        params![stored_addr, final_issuer, row.return_id],
                    )
                    .map_err(StateDbError::sqlite(&path))?;
                }
                let mut reg = declared_to_registration(row, ingress_url);
                reg.issuer = final_issuer;
                return Ok(Ok(reg));
            }
            retire_registration(tx, &path, &row.return_id, now)?;
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

/// Ensure a dynamic RFC 7591 client registration for `(company, server, issuer)`
/// via `rmcp::transport::auth::AuthorizationManager::register_client`.
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

    let endpoint = auth.registration_endpoint().ok_or_else(|| McpError::Server {
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
    let mut manager = _discover::auth_manager(&auth.report.url, auth)?;
    let client_name = format!("instrukt {company}");
    let registered = _http::run_async(server.as_str(), async {
        manager
            .register_client(&client_name, &return_address, &[])
            .await
    })?
    .map_err(|error| McpError::Server {
        server: server.to_string(),
        cause: format!(
            "dynamic client registration failed ({}); declare `sign_in.client_ref` or `sign_in.client`",
            _http::sanitize_auth_error(&error)
        ),
    })?;
    let client_id = registered.client_id;
    let client_secret = registered.client_secret;

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
    tx: &Transaction<'_>,
    path: &Path,
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
/// when a server uses `sign_in.client_ref` or `sign_in.client`.
pub fn server_registration_hint(
    home: &Home,
    company: &ProjectSlug,
    server: &McpServer,
) -> Result<Option<ServerRegistrationHint>, McpError> {
    let Some(sign_in) = server.sign_in.as_ref() else {
        return Ok(None);
    };
    let inspection = home.for_inspection();
    let global = Configuration::load_global(&inspection)?;
    let ingress_url = global.ingress_url()?;
    let Some(db) = StateRead::open(&inspection)? else {
        let message = if let Some(client_ref) = &sign_in.client_ref {
            format!(
                "host client `{client_ref}` not initialized yet; run `instrukt mcp clients` as the local operator"
            )
        } else if sign_in.client.is_some() {
            "declared client registration not initialized yet".into()
        } else {
            return Ok(None);
        };
        return Ok(Some(ServerRegistrationHint::Uninitialized { message }));
    };

    if let Some(client_ref) = &sign_in.client_ref {
        let slug = ProjectSlug::parse(client_ref).map_err(|error| McpError::Server {
            server: server.name.to_string(),
            cause: error.to_string(),
        })?;
        return match query_host_row(db.conn(), db.path(), slug.as_str())? {
            Some(row) => Ok(Some(ServerRegistrationHint::Initialized {
                client_id: row.client_id,
                return_address: ingress_url
                    .as_deref()
                    .map(|base| return_address_for(Some(base), &row.return_id)),
            })),
            None => Ok(Some(ServerRegistrationHint::Uninitialized {
                message: format!(
                    "host client `{client_ref}` not initialized yet; run `instrukt mcp clients` as the local operator"
                ),
            })),
        };
    }
    if sign_in.client.is_some() {
        return match query_company_row(
            db.conn(),
            db.path(),
            company.as_str(),
            server.name.as_str(),
        )? {
            Some(row) => Ok(Some(ServerRegistrationHint::Initialized {
                client_id: row.client_id,
                return_address: ingress_url
                    .as_deref()
                    .map(|base| return_address_for(Some(base), &row.return_id)),
            })),
            None => Ok(Some(ServerRegistrationHint::Uninitialized {
                message: format!(
                    "declared client registration not initialized yet; run `instrukt mcp clients --company {company}` as the local operator"
                ),
            })),
        };
    }
    Ok(None)
}
