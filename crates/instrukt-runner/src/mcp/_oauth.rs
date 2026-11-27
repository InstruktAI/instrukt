//! OAuth 2.1 authorization-code sign-in (`sign_in.mode: authorization-code`),
//! local-boundary ingress reachability check (`instrukt mcp check-ingress`),
//! and `instrukt mcp sign-out`.
//!
//! OAuth PKCE authorization URL construction, authorization-code token
//! exchange, RFC 9207 `iss` verification, and refresh-token renewal delegate
//! to `rmcp::transport::auth::AuthorizationManager`.

use std::path::Path;
use std::time::{Duration, Instant};

use instrukt_core::config::{
    Configuration, LayerKind, McpServer, SecretSource, SignInIdentity, SignInMode,
    resolve_secret_from,
};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension, params};
use instrukt_core::state_db::{StateDb, StateDbError, StateRead};
use jiff::Timestamp;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use rmcp::transport::auth::{
    AuthError, CredentialStore, InMemoryCredentialStore, InMemoryStateStore, OAuthClientConfig,
    OAuthTokenResponse, StateStore, StoredAuthorizationState, StoredCredentials,
};
use serde::Serialize;
use serde_json::{Value, json};

use super::_discover::{self, DiscoveredAuth, discover_auth};
use super::_http;
use super::_issuer::{bearer_logins, cached_login_usable, is_token_fresh, obtain_issuer_login};
use super::_redis_mcp::{claim_sign_in_return, ping_cluster};
use super::_registrations::{
    Registration, ensure_company_client_registration, ensure_dynamic_registration,
    ensure_host_client_registration, random_base64url_256, random_hex_128, sha256_hex,
};
use super::{Login, McpError, Speaker};
use crate::state::RunnerState;
use crate::transport::ScopedTransport;

const SIGN_IN_WAIT_SECS: u64 = 90;
const SIGN_IN_LIFETIME_SECS: i64 = 600;
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Result of `instrukt mcp check-ingress [--json]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IngressCheckReport {
    pub ok: bool,
    pub ingress_url: String,
    pub redis_reachable: bool,
    pub callback_verified: bool,
}

fn current_generation(
    conn: &Connection,
    path: &Path,
    company: &str,
    person: &str,
    server: &str,
) -> Result<i64, StateDbError> {
    Ok(conn
        .query_row(
            "SELECT generation FROM mcp_generations WHERE company = ?1 AND person = ?2 AND server = ?3",
            params![company, person, server],
            |r| r.get(0),
        )
        .optional()
        .map_err(StateDbError::sqlite(path))?
        .unwrap_or(0))
}

struct StoredLogin {
    resource: String,
    subject: Option<String>,
    return_id: String,
    access_token: String,
    expires_at: Option<String>,
    checked: i32,
    refresh_token: Option<String>,
    renewing_until: Option<String>,
}

fn query_login(
    conn: &Connection,
    path: &Path,
    company: &str,
    person: &str,
    server: &str,
) -> Result<Option<StoredLogin>, StateDbError> {
    conn.query_row(
        "SELECT resource, subject, return_id, access_token, expires_at, checked, refresh_token, renewing_until
         FROM mcp_logins
         WHERE company = ?1 AND person = ?2 AND server = ?3",
        params![company, person, server],
        |r| {
            Ok(StoredLogin {
                resource: r.get(0)?,
                subject: r.get(1)?,
                return_id: r.get(2)?,
                access_token: r.get(3)?,
                expires_at: r.get(4)?,
                checked: r.get(5)?,
                refresh_token: r.get(6)?,
                renewing_until: r.get(7)?,
            })
        },
    )
    .optional()
    .map_err(StateDbError::sqlite(path))
}

/// Verify local `ingress.url`, local cluster Redis reachability, and one real
/// external GET probe through `<ingress.url>/hooks/inbound/sign-in/<probe_return_id>`.
pub fn check_ingress(home: &Home) -> Result<IngressCheckReport, McpError> {
    let global = Configuration::load_global(home)?;
    let ingress_url = global.ingress_url()?.ok_or_else(|| McpError::Server {
        server: "ingress".into(),
        cause: "global `ingress.url` is not configured locally; set it with `instrukt config patch --global`".into(),
    })?;

    let transport =
        ScopedTransport::cluster_only(&global, &RunnerState::new(home)).map_err(|error| {
            McpError::Server {
                server: "ingress".into(),
                cause: format!("local cluster Redis configuration invalid: {error}"),
            }
        })?;
    ping_cluster(&transport).map_err(|error| McpError::Server {
        server: "ingress".into(),
        cause: format!("cluster Redis is not reachable locally: {error}"),
    })?;

    let probe_return_id = random_hex_128()?;
    let probe_state = random_base64url_256()?;
    let state_digest = sha256_hex(&probe_state);
    let probe_url = format!(
        "{}/hooks/inbound/sign-in/{probe_return_id}?state={}&code=probe",
        ingress_url.trim_end_matches('/'),
        form_urlencoded::byte_serialize(probe_state.as_bytes()).collect::<String>()
    );

    let http_client = _http::client("ingress", PROBE_TIMEOUT, PROBE_TIMEOUT, false)?;
    let status = _http::run_async("ingress", async {
        http_client
            .get(&probe_url)
            .send()
            .await
            .map(|response| response.status().as_u16())
            .map_err(|error| McpError::Server {
                server: "ingress".into(),
                cause: format!("external callback probe to {ingress_url} failed: {error}"),
            })
    })??;
    if status != 200 {
        let _ = claim_sign_in_return(&transport, &state_digest);
        return Err(McpError::Server {
            server: "ingress".into(),
            cause: format!(
                "external callback probe to {ingress_url}/hooks/inbound/sign-in/... returned HTTP {status} (expected 200)"
            ),
        });
    }

    match claim_sign_in_return(&transport, &state_digest).map_err(|error| McpError::Server {
        server: "ingress".into(),
        cause: format!("failed to claim probe return from cluster Redis: {error}"),
    })? {
        Some(entry)
            if entry.return_id == probe_return_id && entry.code.as_deref() == Some("probe") =>
        {
            Ok(IngressCheckReport {
                ok: true,
                ingress_url,
                redis_reachable: true,
                callback_verified: true,
            })
        }
        Some(_) => Err(McpError::Server {
            server: "ingress".into(),
            cause: "probe return claimed from cluster Redis did not match the sent probe".into(),
        }),
        None => Err(McpError::Server {
            server: "ingress".into(),
            cause: "external callback probe returned HTTP 200, but no matching entry arrived in cluster Redis".into(),
        }),
    }
}

/// Remove a speaker's signed-in login(s) and cancel open/exchanging sign-ins
/// for one server (or all servers of `company`), raising `mcp_generations` in
/// one transaction, followed by best-effort OAuth token revocation.
pub fn sign_out(
    home: &Home,
    company: &ProjectSlug,
    email: &str,
    server: Option<&ProjectSlug>,
    now: Timestamp,
) -> Result<usize, McpError> {
    let person = email.trim().to_lowercase();
    if person.is_empty() {
        return Err(McpError::Server {
            server: company.to_string(),
            cause: "an email address is required to sign out".into(),
        });
    }
    let now_str = now.to_string();
    let (affected, to_revoke) = {
        let mut db = StateDb::open(home)?;
        let path = db.path().to_path_buf();
        db.write::<_, StateDbError>(|tx| {
            let servers: Vec<String> = match server {
                Some(name) => vec![name.to_string()],
                None => {
                    let mut set = std::collections::BTreeSet::new();
                    for sql in [
                        "SELECT server FROM mcp_logins WHERE company = ?1 AND person = ?2",
                        "SELECT server FROM mcp_sign_ins WHERE company = ?1 AND person = ?2 AND state IN ('open', 'exchanging')",
                    ] {
                        let mut stmt = tx.prepare(sql).map_err(StateDbError::sqlite(&path))?;
                        let rows = stmt
                            .query_map(params![company.as_str(), person], |r| r.get::<_, String>(0))
                            .map_err(StateDbError::sqlite(&path))?;
                        for row in rows {
                            set.insert(row.map_err(StateDbError::sqlite(&path))?);
                        }
                    }
                    set.into_iter().collect()
                }
            };

            let mut revoke_list: Vec<StoredLogin> = Vec::new();
            let mut affected = 0usize;
            for srv in &servers {
                if let Some(login) = query_login(tx, &path, company.as_str(), &person, srv)? {
                    revoke_list.push(login);
                }
                tx.execute(
                    "INSERT INTO mcp_generations (company, person, server, generation)
                     VALUES (?1, ?2, ?3, 1)
                     ON CONFLICT(company, person, server) DO UPDATE SET generation = generation + 1",
                    params![company.as_str(), person, srv],
                )
                .map_err(StateDbError::sqlite(&path))?;
                affected += tx
                    .execute(
                        "DELETE FROM mcp_logins WHERE company = ?1 AND person = ?2 AND server = ?3",
                        params![company.as_str(), person, srv],
                    )
                    .map_err(StateDbError::sqlite(&path))?;
                affected += tx
                    .execute(
                        "UPDATE mcp_sign_ins
                         SET state = 'cancelled', closed_at = ?4, code_verifier = NULL, nonce = NULL, link = NULL
                         WHERE company = ?1 AND person = ?2 AND server = ?3 AND state IN ('open', 'exchanging')",
                        params![company.as_str(), person, srv, now_str],
                    )
                    .map_err(StateDbError::sqlite(&path))?;
            }
            Ok((affected, revoke_list))
        })
        .map_err(McpError::State)?
    };

    for item in to_revoke {
        best_effort_revoke_token(
            home,
            company,
            &item.resource,
            &item.return_id,
            item.refresh_token.as_deref(),
            &item.access_token,
        );
    }
    Ok(affected)
}

fn best_effort_revoke_token(
    home: &Home,
    company: &ProjectSlug,
    resource: &str,
    return_id: &str,
    refresh_token: Option<&str>,
    access_token: &str,
) {
    let inspection = home.for_inspection();
    let Ok(Some(read)) = StateRead::open(&inspection) else {
        return;
    };
    struct RevokeReg {
        host_client: Option<String>,
        server: Option<String>,
        issuer: String,
        client_id: String,
        dcr_secret: Option<String>,
    }
    let reg_row = read
        .conn()
        .query_row(
            "SELECT host_client, server, issuer, client_id, client_secret
             FROM mcp_registrations WHERE return_id = ?1",
            [return_id],
            |r| {
                Ok(RevokeReg {
                    host_client: r.get(0)?,
                    server: r.get(1)?,
                    issuer: r.get(2)?,
                    client_id: r.get(3)?,
                    dcr_secret: r.get(4)?,
                })
            },
        )
        .optional()
        .ok()
        .flatten();
    drop(read);
    let Some(RevokeReg {
        host_client,
        server: reg_server,
        issuer: pinned_issuer,
        client_id,
        dcr_secret,
    }) = reg_row
    else {
        return;
    };
    let Ok(auth) = discover_auth(resource) else {
        return;
    };
    if !pinned_issuer.is_empty() && auth.issuer != pinned_issuer {
        return;
    }

    let client_secret = if let Some(ref host_name) = host_client {
        ProjectSlug::parse(host_name).ok().and_then(|slug| {
            Configuration::load_global(&inspection)
                .ok()?
                .mcp_clients()
                .ok()?
                .get(&slug)?
                .secret
                .as_deref()
                .and_then(|r| resolve_secret_from(&inspection, &SecretSource::Machine, r).ok())
        })
    } else if let Some(ref srv_name) = reg_server {
        ProjectSlug::parse(srv_name)
            .ok()
            .and_then(|slug| {
                Configuration::load_company(&inspection, company)
                    .ok()?
                    .mcp_servers(LayerKind::Company)
                    .ok()?
                    .get(&slug)?
                    .sign_in
                    .as_ref()?
                    .client
                    .as_ref()?
                    .secret
                    .as_deref()
                    .and_then(|r| {
                        resolve_secret_from(&inspection, &SecretSource::Company(company.clone()), r)
                            .ok()
                    })
            })
            .or(dcr_secret)
    } else {
        dcr_secret
    };

    revoke_direct(
        &auth,
        &client_id,
        client_secret.as_deref(),
        refresh_token,
        access_token,
    );
}

/// Obtain a speaker's Bearer login for a `speaker_login: sign-in` server,
/// dispatching by `sign_in.mode` (`authorization-code` or `issuer`).
pub(crate) fn resolve_sign_in_login(
    home: &Home,
    company: &ProjectSlug,
    server: &McpServer,
    speaker: &Speaker,
    rejected_access_token: Option<&str>,
    now: Timestamp,
) -> Result<(Login, Vec<Login>), McpError> {
    let mode = server
        .sign_in
        .as_ref()
        .map(|s| s.mode)
        .unwrap_or(SignInMode::AuthorizationCode);
    match mode {
        SignInMode::Issuer => {
            obtain_issuer_login(home, company, server, speaker, rejected_access_token, now)
        }
        SignInMode::AuthorizationCode => obtain_authorization_code_login(
            home,
            company,
            server,
            speaker,
            rejected_access_token,
            now,
        ),
    }
}

fn oauth_client_config(
    registration: &Registration,
    client_secret: Option<&str>,
) -> OAuthClientConfig {
    let mut cfg = OAuthClientConfig::new(
        registration.client_id.clone(),
        registration.return_address.clone(),
    );
    if let Some(secret) = client_secret {
        cfg = cfg.with_client_secret(secret.to_owned());
    }
    cfg
}

/// Start an OAuth 2.1 PKCE S256 authorization flow through `rmcp`'s
/// `AuthorizationManager`, returning `(auth_link, state, code_verifier)`.
fn start_oauth_authorization(
    server_url: &str,
    auth: &DiscoveredAuth,
    registration: &Registration,
    client_secret: Option<&str>,
    scopes: &[String],
    nonce: Option<&str>,
) -> Result<(String, String, String), McpError> {
    let state_store = InMemoryStateStore::new();
    let mut manager = _discover::auth_manager(server_url, auth)?;
    manager.set_state_store(state_store.clone());
    manager
        .configure_client(oauth_client_config(registration, client_secret))
        .map_err(|error| McpError::Server {
            server: server_url.to_owned(),
            cause: format!("failed to configure OAuth client: {error}"),
        })?;
    let scope_refs: Vec<&str> = scopes.iter().map(String::as_str).collect();
    let (mut auth_url, state, pkce_verifier) = _http::run_async(server_url, async {
        let raw_url = manager
            .get_authorization_url(&scope_refs)
            .await
            .map_err(|error| McpError::Server {
                server: server_url.to_owned(),
                cause: format!("failed to build OAuth authorization URL: {error}"),
            })?;
        let parsed = url::Url::parse(&raw_url).map_err(|error| McpError::Server {
            server: server_url.to_owned(),
            cause: format!("invalid authorization URL: {error}"),
        })?;
        let state = parsed
            .query_pairs()
            .find_map(|(k, v)| (k == "state").then(|| v.into_owned()))
            .ok_or_else(|| McpError::Server {
                server: server_url.to_owned(),
                cause: "authorization URL missing `state` parameter".into(),
            })?;
        let stored = state_store
            .load(&state)
            .await
            .map_err(|error| McpError::Server {
                server: server_url.to_owned(),
                cause: format!("failed to load PKCE state: {error}"),
            })?
            .ok_or_else(|| McpError::Server {
                server: server_url.to_owned(),
                cause: "PKCE state missing from OAuth manager".into(),
            })?;
        Ok::<_, McpError>((parsed, state, stored.pkce_verifier))
    })??;

    if let Some(nonce) = nonce {
        auth_url.query_pairs_mut().append_pair("nonce", nonce);
    }
    Ok((auth_url.into(), state, pkce_verifier))
}

fn obtain_authorization_code_login(
    home: &Home,
    company: &ProjectSlug,
    server: &McpServer,
    speaker: &Speaker,
    rejected_access_token: Option<&str>,
    now: Timestamp,
) -> Result<(Login, Vec<Login>), McpError> {
    let email = speaker
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| McpError::Server {
            server: server.name.to_string(),
            cause: format!(
                "speaker {speaker} has no recorded email address, which sign-in requires"
            ),
        })?;
    let person = email.to_lowercase();
    let sign_in_cfg = server.sign_in.clone().unwrap_or_default();
    let inspection = home.for_inspection();
    let global = Configuration::load_global(&inspection)?;
    let ingress_url_opt = global.ingress_url()?;

    // 1. Pre-check declared client registration (no network call) so rotated/switched
    //    credentials retire old logins before any cached token is honored.
    let prechecked_reg: Option<Registration> = if let Some(ref client_ref) = sign_in_cfg.client_ref
    {
        let slug = ProjectSlug::parse(client_ref).map_err(|e| McpError::Server {
            server: server.name.to_string(),
            cause: e.to_string(),
        })?;
        let clients = global.mcp_clients()?;
        let host_client = clients.get(&slug).ok_or_else(|| McpError::Server {
            server: server.name.to_string(),
            cause: format!("global `mcp_clients.{client_ref}` is not declared"),
        })?;
        Some(ensure_host_client_registration(
            home,
            &slug,
            host_client,
            ingress_url_opt.as_deref(),
            now,
        )?)
    } else if let Some(ref local_client) = sign_in_cfg.client {
        Some(ensure_company_client_registration(
            home,
            company,
            &server.name,
            local_client,
            "",
            ingress_url_opt.as_deref(),
            now,
        )?)
    } else {
        None
    };

    // 2. Check cached login in mcp_logins.
    let cached = if let Some(db) = StateRead::open(&inspection)? {
        query_login(
            db.conn(),
            db.path(),
            company.as_str(),
            &person,
            server.name.as_str(),
        )?
    } else {
        None
    };

    if let Some(ref row) = cached
        && row.resource == server.url
        && prechecked_reg
            .as_ref()
            .is_none_or(|reg| reg.return_id == row.return_id)
        && cached_login_usable(
            &row.access_token,
            row.checked,
            row.expires_at.as_deref(),
            rejected_access_token,
            now,
        )
    {
        return Ok(bearer_logins(&server.name, row.access_token.clone()));
    }

    // 3. Discover AS metadata and enforce security gates before resolving any client secret.
    let auth = discover_auth(&server.url)?;
    if !auth.report.pkce_s256_supported {
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: "authorization server does not advertise PKCE `S256`".into(),
        });
    }
    if !auth.report.iss_parameter_supported && !sign_in_cfg.accept_return_without_issuer {
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: "authorization server does not advertise RFC 9207 `iss` on returns; accept explicitly with `sign_in.accept_return_without_issuer: true`".into(),
        });
    }
    if sign_in_cfg.identity.is_none() && !auth.report.openid_provider {
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: "authorization server is not an OpenID Provider (missing UserInfo or JWKS); declare `sign_in.identity`".into(),
        });
    }
    let auth_endpoint = auth
        .authorization_endpoint()
        .ok_or_else(|| McpError::Server {
            server: server.name.to_string(),
            cause: "authorization server metadata omits `authorization_endpoint`".into(),
        })?;
    let token_endpoint = auth.token_endpoint().ok_or_else(|| McpError::Server {
        server: server.name.to_string(),
        cause: "authorization server metadata omits `token_endpoint`".into(),
    })?;
    if !instrukt_core::config::mcp_address_admitted(auth_endpoint)
        || !instrukt_core::config::mcp_address_admitted(token_endpoint)
    {
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: "authorization server endpoints must be https (or loopback http)".into(),
        });
    }

    // 4. Resolve ingress.url & cluster Redis transport.
    let ingress_url = ingress_url_opt.ok_or_else(|| McpError::Server {
        server: server.name.to_string(),
        cause: "global `ingress.url` must be configured for MCP sign-in".into(),
    })?;
    let transport = ScopedTransport::cluster_only(&global, &RunnerState::new(&inspection))
        .map_err(|error| McpError::Server {
            server: server.name.to_string(),
            cause: format!("cluster Redis is unavailable for MCP sign-in: {error}"),
        })?;

    // 5. Enforce issuer pinning BEFORE resolving client secret.
    let (registration, client_secret) = if let Some(ref client_ref) = sign_in_cfg.client_ref {
        let slug = ProjectSlug::parse(client_ref).map_err(|e| McpError::Server {
            server: server.name.to_string(),
            cause: e.to_string(),
        })?;
        let clients = global.mcp_clients()?;
        let host_client = clients.get(&slug).ok_or_else(|| McpError::Server {
            server: server.name.to_string(),
            cause: format!("global `mcp_clients.{client_ref}` is not declared"),
        })?;
        if auth.issuer != host_client.issuer {
            return Err(McpError::Server {
                server: server.name.to_string(),
                cause: format!(
                    "discovered authorization server `{}` does not match host client `mcp_clients.{client_ref}` pinned issuer `{}`",
                    auth.issuer, host_client.issuer
                ),
            });
        }
        let reg = match prechecked_reg {
            Some(reg) => reg,
            None => {
                ensure_host_client_registration(home, &slug, host_client, Some(&ingress_url), now)?
            }
        };
        let secret = host_client
            .secret
            .as_deref()
            .map(|r| resolve_secret_from(home, &SecretSource::Machine, r))
            .transpose()?;
        (reg, secret)
    } else if let Some(ref local_client) = sign_in_cfg.client {
        let reg = ensure_company_client_registration(
            home,
            company,
            &server.name,
            local_client,
            &auth.issuer,
            Some(&ingress_url),
            now,
        )?;
        let secret = local_client
            .secret
            .as_deref()
            .map(|r| resolve_secret_from(home, &SecretSource::Company(company.clone()), r))
            .transpose()?;
        (reg, secret)
    } else {
        let reg =
            ensure_dynamic_registration(home, company, &server.name, &auth, &ingress_url, now)?;
        let secret = reg.client_secret.clone();
        (reg, secret)
    };

    // 6. Handle existing cached login: remove+revoke if resource changed, or refresh.
    if let Some(ref row) = cached {
        if row.resource != server.url {
            let mut db = StateDb::open(home)?;
            let path = db.path().to_path_buf();
            db.write::<_, StateDbError>(|tx| {
                tx.execute(
                    "DELETE FROM mcp_logins WHERE company = ?1 AND person = ?2 AND server = ?3 AND resource = ?4",
                    params![company.as_str(), person, server.name.as_str(), row.resource],
                )
                .map_err(StateDbError::sqlite(&path))?;
                Ok(())
            })?;
            drop(db);
            best_effort_revoke_token(
                home,
                company,
                &row.resource,
                &row.return_id,
                row.refresh_token.as_deref(),
                &row.access_token,
            );
        } else if row.return_id == registration.return_id
            && let Some(renewed) = try_refresh_login(
                home,
                company,
                server,
                &person,
                &auth,
                &registration,
                client_secret.as_deref(),
                sign_in_cfg.identity.as_ref(),
                rejected_access_token,
            )?
        {
            return Ok(renewed);
        }
    }

    // 7. Build PKCE S256 authorization URL via `rmcp` and persist pending sign-in + waiter.
    let mut scopes = if !sign_in_cfg.scopes.is_empty() {
        sign_in_cfg.scopes.clone()
    } else {
        auth.report.scopes_supported.clone()
    };
    if sign_in_cfg.identity.is_none() {
        for req in ["openid", "email"] {
            if !scopes.iter().any(|s| s == req) {
                scopes.push(req.to_owned());
            }
        }
    }
    let scope_param = scopes.join(" ");
    let nonce = sign_in_cfg
        .identity
        .is_none()
        .then(random_base64url_256)
        .transpose()?;
    let (auth_link, state, code_verifier) = start_oauth_authorization(
        &server.url,
        &auth,
        &registration,
        client_secret.as_deref(),
        &scopes,
        nonce.as_deref(),
    )?;
    let state_digest = sha256_hex(&state);
    let waiter_id = random_hex_128()?;
    let expires_at = now
        .checked_add(jiff::SignedDuration::from_secs(SIGN_IN_LIFETIME_SECS))
        .unwrap_or(now);
    let waiting_until = now
        .checked_add(jiff::SignedDuration::from_secs(SIGN_IN_WAIT_SECS as i64))
        .unwrap_or(now);

    let (active_digest, active_generation, active_verifier, active_nonce) = {
        let mut db = StateDb::open(home)?;
        let path = db.path().to_path_buf();
        db.write::<_, StateDbError>(|tx| {
            let current_gen =
                current_generation(tx, &path, company.as_str(), &person, server.name.as_str())?;
            tx.execute(
                "UPDATE mcp_sign_ins
                 SET state = 'expired', closed_at = ?1, code_verifier = NULL, nonce = NULL, link = NULL
                 WHERE company = ?2 AND person = ?3 AND server = ?4 AND state = 'open' AND expires_at <= ?1",
                params![now.to_string(), company.as_str(), person, server.name.as_str()],
            )
            .map_err(StateDbError::sqlite(&path))?;

            let existing: Option<(String, i64, Option<String>, Option<String>)> = tx
                .query_row(
                    "SELECT state_digest, generation, code_verifier, nonce FROM mcp_sign_ins
                     WHERE company = ?1 AND person = ?2 AND server = ?3 AND state IN ('open', 'exchanging')
                     LIMIT 1",
                    params![company.as_str(), person, server.name.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()
                .map_err(StateDbError::sqlite(&path))?;

            let (digest, generation, verifier, existing_nonce) = match existing {
                Some(row) => row,
                None => {
                    tx.execute(
                        "INSERT INTO mcp_sign_ins
                         (state_digest, company, person, server, return_id, resource, scopes, generation,
                          connector, conversation, message, recipient, code_verifier, nonce, link,
                          created_at, expires_at, state)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, 'open')",
                        params![
                            state_digest,
                            company.as_str(),
                            person,
                            server.name.as_str(),
                            registration.return_id,
                            server.url,
                            scope_param,
                            current_gen,
                            speaker.connector.as_deref().unwrap_or_default(),
                            speaker.conversation.as_deref().unwrap_or_default(),
                            speaker.message.as_deref().unwrap_or_default(),
                            speaker.identity.as_deref().unwrap_or(&person),
                            code_verifier,
                            nonce,
                            auth_link,
                            now.to_string(),
                            expires_at.to_string(),
                        ],
                    )
                    .map_err(StateDbError::sqlite(&path))?;
                    (
                        state_digest.clone(),
                        current_gen,
                        Some(code_verifier.clone()),
                        nonce.clone(),
                    )
                }
            };

            tx.execute(
                "INSERT INTO mcp_sign_in_waiters (state_digest, waiter, waiting_until)
                 VALUES (?1, ?2, ?3)",
                params![digest, waiter_id, waiting_until.to_string()],
            )
            .map_err(StateDbError::sqlite(&path))?;

            Ok((digest, generation, verifier, existing_nonce))
        })?
    };

    // 8. Wait for the return entry in cluster Redis (up to 90s) without holding writer.lock.
    let deadline = Instant::now() + Duration::from_secs(SIGN_IN_WAIT_SECS);
    let result = wait_and_complete(
        home,
        &transport,
        company,
        server,
        &person,
        &auth,
        &registration,
        client_secret.as_deref(),
        sign_in_cfg.identity.as_ref(),
        &active_digest,
        active_generation,
        active_verifier.as_deref(),
        active_nonce.as_deref(),
        deadline,
    );

    if let Ok(mut db) = StateDb::open(home) {
        let path = db.path().to_path_buf();
        let _ = db.write::<_, StateDbError>(|tx| {
            tx.execute(
                "DELETE FROM mcp_sign_in_waiters WHERE state_digest = ?1 AND waiter = ?2",
                params![active_digest, waiter_id],
            )
            .map_err(StateDbError::sqlite(&path))?;
            Ok(())
        });
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn wait_and_complete(
    home: &Home,
    transport: &ScopedTransport,
    company: &ProjectSlug,
    server: &McpServer,
    person: &str,
    auth: &DiscoveredAuth,
    registration: &Registration,
    client_secret: Option<&str>,
    identity_cfg: Option<&SignInIdentity>,
    state_digest: &str,
    generation: i64,
    code_verifier: Option<&str>,
    nonce: Option<&str>,
    deadline: Instant,
) -> Result<(Login, Vec<Login>), McpError> {
    loop {
        if let Ok(Some(entry)) = claim_sign_in_return(transport, state_digest) {
            return complete_claimed_return(
                home,
                company,
                server,
                person,
                auth,
                registration,
                client_secret,
                identity_cfg,
                state_digest,
                generation,
                code_verifier,
                nonce,
                entry,
            );
        }

        let (status, completed_token) = {
            let read = StateRead::open(home)?.ok_or_else(|| McpError::Server {
                server: server.name.to_string(),
                cause: "pending sign-in record disappeared".into(),
            })?;
            let status: Option<(String, Option<String>, Option<String>)> = read
                .conn()
                .query_row(
                    "SELECT state, failure, link_sent_at FROM mcp_sign_ins WHERE state_digest = ?1",
                    [state_digest],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(StateDbError::sqlite(read.path()))?;
            let token = if status.as_ref().is_some_and(|(s, _, _)| s == "completed") {
                query_login(
                    read.conn(),
                    read.path(),
                    company.as_str(),
                    person,
                    server.name.as_str(),
                )?
                .map(|row| row.access_token)
            } else {
                None
            };
            (status, token)
        };
        let Some((state, failure, link_sent_at)) = status else {
            return Err(McpError::Server {
                server: server.name.to_string(),
                cause: "pending sign-in record disappeared".into(),
            });
        };
        match state.as_str() {
            "completed" => {
                if let Some(access_token) = completed_token {
                    return Ok(bearer_logins(&server.name, access_token));
                }
            }
            "cancelled" => {
                return Err(McpError::Server {
                    server: server.name.to_string(),
                    cause: "sign-in was cancelled by a sign-out".into(),
                });
            }
            "declined" | "refused" | "failed" | "undelivered" | "expired" => {
                return Err(McpError::Server {
                    server: server.name.to_string(),
                    cause: format!("sign-in did not complete: {}", failure.unwrap_or(state)),
                });
            }
            _ if Instant::now() >= deadline => {
                let msg = if state == "exchanging" {
                    "sign-in is being completed; please ask again shortly"
                } else if link_sent_at.is_some() {
                    "sign-in is pending; a sign-in link was sent by direct message"
                } else {
                    "sign-in is pending; its sign-in link has not been delivered yet"
                };
                return Err(McpError::Server {
                    server: server.name.to_string(),
                    cause: msg.into(),
                });
            }
            _ => {}
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

fn close_sign_in_failure(home: &Home, state_digest: &str, state_name: &str, cause: &str) {
    if let Ok(mut db) = StateDb::open(home) {
        let path = db.path().to_path_buf();
        let _ = db.write::<_, StateDbError>(|tx| {
            tx.execute(
                "UPDATE mcp_sign_ins
                 SET state = ?1, failure = ?2, closed_at = ?3, code_verifier = NULL, nonce = NULL, link = NULL
                 WHERE state_digest = ?4",
                params![state_name, cause, Timestamp::now().to_string(), state_digest],
            )
            .map_err(StateDbError::sqlite(&path))?;
            Ok(())
        });
    }
}

struct ParsedTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    granted_scopes: Option<String>,
    expires_at: Option<String>,
    id_token: Option<String>,
}

fn parse_token_response(
    response: &OAuthTokenResponse,
    now: Timestamp,
) -> Result<ParsedTokenResponse, String> {
    let body = serde_json::to_value(response)
        .map_err(|error| format!("failed to inspect OAuth token response: {error}"))?;
    let token_type = body
        .get("token_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !token_type.eq_ignore_ascii_case("Bearer") {
        return Err(format!("unsupported token_type `{token_type}`"));
    }
    let access_token = body
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "token response omitted access_token".to_owned())?;
    let refresh_token = body
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned);
    let granted_scopes = body.get("scope").and_then(Value::as_str).map(str::to_owned);
    let expires_at = body
        .get("expires_in")
        .and_then(Value::as_i64)
        .filter(|&secs| secs > 0)
        .and_then(|secs| now.checked_add(jiff::SignedDuration::from_secs(secs)).ok())
        .map(|ts| ts.to_string());
    let id_token = body
        .get("id_token")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned);
    Ok(ParsedTokenResponse {
        access_token,
        refresh_token,
        granted_scopes,
        expires_at,
        id_token,
    })
}

#[allow(clippy::too_many_arguments)]
fn complete_claimed_return(
    home: &Home,
    company: &ProjectSlug,
    server: &McpServer,
    person: &str,
    auth: &DiscoveredAuth,
    registration: &Registration,
    client_secret: Option<&str>,
    identity_cfg: Option<&SignInIdentity>,
    state_digest: &str,
    generation: i64,
    code_verifier: Option<&str>,
    nonce: Option<&str>,
    entry: super::_redis_mcp::SignInReturnEntry,
) -> Result<(Login, Vec<Login>), McpError> {
    let now = Timestamp::now();
    let claimed = {
        let mut db = StateDb::open(home)?;
        let path = db.path().to_path_buf();
        db.write::<_, StateDbError>(|tx| {
            let changed = tx
                .execute(
                    "UPDATE mcp_sign_ins SET state = 'exchanging'
                     WHERE state_digest = ?1 AND state = 'open' AND expires_at > ?2",
                    params![state_digest, now.to_string()],
                )
                .map_err(StateDbError::sqlite(&path))?;
            Ok(changed > 0)
        })?
    };
    if !claimed {
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: "sign-in expired or was no longer open when its return arrived".into(),
        });
    }

    let fail = |state_name: &str, msg: String| {
        close_sign_in_failure(home, state_digest, state_name, &msg);
        McpError::Server {
            server: server.name.to_string(),
            cause: msg,
        }
    };

    if entry.return_id != registration.return_id {
        return Err(fail(
            "failed",
            "return_id did not match the sign-in's registration".into(),
        ));
    }
    if let Some(err_code) = entry.error {
        close_sign_in_failure(home, state_digest, "declined", &err_code);
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: format!("authorization server declined sign-in: {err_code}"),
        });
    }
    let Some(code) = entry.code else {
        return Err(fail(
            "failed",
            "return carried neither code nor error".into(),
        ));
    };
    let Some(verifier) = code_verifier else {
        return Err(fail("failed", "missing PKCE code_verifier".into()));
    };

    let token_response = exchange_code_via_rmcp(
        &server.url,
        auth,
        registration,
        client_secret,
        state_digest,
        verifier,
        &code,
        entry.iss.as_deref(),
    )
    .map_err(|cause| fail("failed", cause))?;

    let now = Timestamp::now();
    let parsed =
        parse_token_response(&token_response, now).map_err(|cause| fail("failed", cause))?;
    let access_token = parsed.access_token;
    let refresh_token = parsed.refresh_token;

    let revoke_and_refuse = |cause: String| {
        revoke_direct(
            auth,
            &registration.client_id,
            client_secret,
            refresh_token.as_deref(),
            &access_token,
        );
        fail("refused", cause)
    };

    let (verified_email, subject) = match identity_cfg {
        Some(identity) => {
            let email =
                verify_declared_identity(identity, &access_token).map_err(|c| fail("failed", c))?;
            (email, None)
        }
        None => {
            let id_token = parsed.id_token.as_deref().ok_or_else(|| {
                revoke_and_refuse("token response omitted OpenID Connect id_token".into())
            })?;
            verify_oidc_identity(
                auth,
                &registration.client_id,
                nonce,
                id_token,
                &access_token,
            )
            .map_err(revoke_and_refuse)?
        }
    };

    if verified_email.trim().to_lowercase() != person {
        return Err(revoke_and_refuse(
            "sign-in account email did not match the speaker's email address".into(),
        ));
    }

    let now_str = now.to_string();
    let saved = {
        let mut db = StateDb::open(home)?;
        let path = db.path().to_path_buf();
        db.write::<_, StateDbError>(|tx| {
            if current_generation(tx, &path, company.as_str(), person, server.name.as_str())?
                != generation
            {
                return Ok(false);
            }
            tx.execute(
                "INSERT INTO mcp_logins
                 (company, person, server, resource, subject, return_id, access_token, expires_at, checked, refresh_token, scopes, signed_in_at, renewed_at, renewing_until)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?10, ?11, NULL, NULL)
                 ON CONFLICT(company, person, server) DO UPDATE SET
                   resource = excluded.resource,
                   subject = excluded.subject,
                   return_id = excluded.return_id,
                   access_token = excluded.access_token,
                   expires_at = excluded.expires_at,
                   checked = 1,
                   refresh_token = excluded.refresh_token,
                   scopes = excluded.scopes,
                   signed_in_at = excluded.signed_in_at,
                   renewed_at = NULL,
                   renewing_until = NULL",
                params![
                    company.as_str(),
                    person,
                    server.name.as_str(),
                    server.url,
                    subject,
                    registration.return_id,
                    access_token,
                    parsed.expires_at,
                    refresh_token,
                    parsed.granted_scopes,
                    now_str,
                ],
            )
            .map_err(StateDbError::sqlite(&path))?;
            let active_waiters: i64 = tx
                .query_row(
                    "SELECT count(*) FROM mcp_sign_in_waiters WHERE state_digest = ?1 AND waiting_until > ?2",
                    params![state_digest, now_str],
                    |r| r.get(0),
                )
                .map_err(StateDbError::sqlite(&path))?;
            let told_at = (active_waiters > 0).then_some(now_str.as_str());
            tx.execute(
                "UPDATE mcp_sign_ins
                 SET state = 'completed', closed_at = ?1, told_at = COALESCE(told_at, ?2), code_verifier = NULL, nonce = NULL, link = NULL, failure = NULL
                 WHERE state_digest = ?3",
                params![now_str, told_at, state_digest],
            )
            .map_err(StateDbError::sqlite(&path))?;
            Ok(true)
        })?
    };
    if !saved {
        revoke_direct(
            auth,
            &registration.client_id,
            client_secret,
            refresh_token.as_deref(),
            &access_token,
        );
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: "sign-in was cancelled by a sign-out".into(),
        });
    }

    Ok(bearer_logins(&server.name, access_token))
}

#[allow(clippy::too_many_arguments)]
fn exchange_code_via_rmcp(
    server_url: &str,
    auth: &DiscoveredAuth,
    registration: &Registration,
    client_secret: Option<&str>,
    csrf_key: &str,
    code_verifier: &str,
    code: &str,
    received_issuer: Option<&str>,
) -> Result<OAuthTokenResponse, String> {
    let mut manager = _discover::auth_manager(server_url, auth).map_err(|e| e.to_string())?;
    let state_store = InMemoryStateStore::new();
    let stored_state: StoredAuthorizationState = serde_json::from_value(json!({
        "pkce_verifier": code_verifier,
        "csrf_token": csrf_key,
        "expected_issuer": auth.issuer,
        "require_issuer": auth.report.iss_parameter_supported,
        "created_at": u64::try_from(Timestamp::now().as_second()).unwrap_or(0),
        "requested_scopes": [],
    }))
    .map_err(|error| format!("failed to prepare PKCE authorization state: {error}"))?;
    manager.set_state_store(state_store.clone());
    manager
        .configure_client(oauth_client_config(registration, client_secret))
        .map_err(|error| {
            format!(
                "failed to configure OAuth client: {}",
                _http::sanitize_auth_error(&error)
            )
        })?;
    _http::run_async(server_url, async {
        state_store
            .save(csrf_key, stored_state)
            .await
            .map_err(|error| {
                format!(
                    "failed to store PKCE state: {}",
                    _http::sanitize_auth_error(&error)
                )
            })?;
        manager
            .exchange_code_for_token_with_issuer(code, csrf_key, received_issuer)
            .await
            .map_err(|error| {
                format!(
                    "token exchange failed: {}",
                    _http::sanitize_auth_error(&error)
                )
            })
    })
    .map_err(|e| e.to_string())?
}

fn verify_declared_identity(
    identity: &SignInIdentity,
    access_token: &str,
) -> Result<String, String> {
    let http_client = _http::client(
        &identity.url,
        Duration::from_secs(10),
        Duration::from_secs(20),
        false,
    )
    .map_err(|e| e.to_string())?;
    let (status, body_text) = _http::run_async(&identity.url, async {
        _http::get_json_text(&http_client, &identity.url, Some(access_token)).await
    })
    .map_err(|e| e.to_string())??;
    if status != 200 {
        return Err(format!("identity request returned HTTP {status}"));
    }
    let doc: Value =
        serde_json::from_str(&body_text).map_err(|_| "identity response is not JSON".to_owned())?;
    let mut current = &doc;
    for segment in identity.email_field.split('.') {
        current = current
            .get(segment)
            .ok_or_else(|| format!("identity response missing `{}`", identity.email_field))?;
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "identity field `{}` is not a nonempty string",
                identity.email_field
            )
        })
}

fn fetch_userinfo_json(userinfo_endpoint: &str, access_token: &str) -> Result<Value, String> {
    let http_client = _http::client(
        userinfo_endpoint,
        Duration::from_secs(10),
        Duration::from_secs(20),
        false,
    )
    .map_err(|e| e.to_string())?;
    let (status, text) = _http::run_async(userinfo_endpoint, async {
        _http::get_json_text(&http_client, userinfo_endpoint, Some(access_token)).await
    })
    .map_err(|e| e.to_string())??;
    if status != 200 {
        return Err(format!("UserInfo endpoint returned HTTP {status}"));
    }
    serde_json::from_str(&text).map_err(|_| "UserInfo response is not JSON".to_owned())
}

fn verify_userinfo_claims(ui: &Value, expected_sub: Option<&str>) -> Result<String, String> {
    if let Some(sub) = expected_sub
        && ui.get("sub").and_then(Value::as_str) != Some(sub)
    {
        return Err("UserInfo `sub` does not match the recorded sign-in subject".into());
    }
    if ui.get("email_verified").and_then(Value::as_bool) != Some(true) {
        return Err("UserInfo `email_verified` is not true".into());
    }
    ui.get("email")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "UserInfo omitted `email`".to_owned())
}

fn verify_oidc_identity(
    auth: &DiscoveredAuth,
    client_id: &str,
    expected_nonce: Option<&str>,
    id_token: &str,
    access_token: &str,
) -> Result<(String, Option<String>), String> {
    let jwks_uri = auth
        .jwks_uri()
        .ok_or_else(|| "OpenID Provider metadata omits jwks_uri".to_owned())?;
    let userinfo_endpoint = auth
        .userinfo_endpoint()
        .ok_or_else(|| "OpenID Provider metadata omits userinfo_endpoint".to_owned())?;
    if !instrukt_core::config::mcp_address_admitted(jwks_uri)
        || !instrukt_core::config::mcp_address_admitted(userinfo_endpoint)
    {
        return Err("OIDC endpoint address is not admitted".into());
    }

    let header = decode_header(id_token).map_err(|e| format!("invalid id_token header: {e}"))?;
    let http_client = _http::client(
        jwks_uri,
        Duration::from_secs(10),
        Duration::from_secs(20),
        false,
    )
    .map_err(|e| e.to_string())?;
    let (jwks_status, jwks_text) = _http::run_async(jwks_uri, async {
        _http::get_json_text(&http_client, jwks_uri, None).await
    })
    .map_err(|e| e.to_string())??;
    if jwks_status != 200 {
        return Err(format!("JWKS endpoint returned HTTP {jwks_status}"));
    }
    let jwks: jsonwebtoken::jwk::JwkSet =
        serde_json::from_str(&jwks_text).map_err(|_| "invalid JWKS JSON".to_owned())?;
    let jwk = match header.kid.as_deref() {
        Some(kid) => jwks.find(kid),
        None => jwks.keys.first(),
    }
    .ok_or_else(|| "matching signing key not found in JWKS".to_owned())?;
    let decoding_key =
        DecodingKey::from_jwk(jwk).map_err(|e| format!("unsupported JWK in JWKS: {e}"))?;

    let alg = match header.alg {
        Algorithm::ES256
        | Algorithm::ES384
        | Algorithm::RS256
        | Algorithm::RS384
        | Algorithm::RS512 => header.alg,
        other => return Err(format!("unsupported id_token algorithm `{other:?}`")),
    };
    let mut validation = Validation::new(alg);
    validation.set_issuer(&[auth.issuer.as_str()]);
    validation.set_audience(&[client_id]);
    let claims = decode::<Value>(id_token, &decoding_key, &validation)
        .map_err(|e| format!("id_token verification failed: {e}"))?
        .claims;
    if let Some(expected) = expected_nonce
        && claims.get("nonce").and_then(Value::as_str) != Some(expected)
    {
        return Err("id_token nonce mismatch".into());
    }
    let sub = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "id_token missing `sub`".to_owned())?
        .to_owned();

    let ui = fetch_userinfo_json(userinfo_endpoint, access_token)?;
    let email = verify_userinfo_claims(&ui, Some(&sub))?;
    Ok((email, Some(sub)))
}

/// Best-effort RFC 7009 revocation using already-discovered AS metadata and resolved
/// client credentials (revokes refresh_token when present, else access_token).
fn revoke_direct(
    auth: &DiscoveredAuth,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: Option<&str>,
    access_token: &str,
) {
    let Some(revocation_endpoint) = auth.revocation_endpoint() else {
        return;
    };
    if !instrukt_core::config::mcp_address_admitted(revocation_endpoint) {
        return;
    }
    let Ok(http_client) = _http::client(
        revocation_endpoint,
        Duration::from_secs(5),
        Duration::from_secs(10),
        false,
    ) else {
        return;
    };
    let (token, hint) = match refresh_token {
        Some(rt) => (rt, "refresh_token"),
        None => (access_token, "access_token"),
    };
    let methods = auth.token_endpoint_auth_methods();
    let use_post_secret = client_secret.is_some()
        && methods.iter().any(|m| m == "client_secret_post")
        && !methods.iter().any(|m| m == "client_secret_basic");
    let mut form: Vec<(&str, &str)> = vec![("token", token), ("token_type_hint", hint)];
    let basic = match (client_secret, use_post_secret) {
        (Some(secret), false) => Some(_http::form_basic_auth(client_id, secret)),
        (Some(secret), true) => {
            form.push(("client_id", client_id));
            form.push(("client_secret", secret));
            None
        }
        (None, _) => {
            form.push(("client_id", client_id));
            None
        }
    };
    let _ = _http::run_async(revocation_endpoint, async {
        let mut req = http_client.post(revocation_endpoint);
        if let Some(auth_val) = basic {
            req = req.header(http::header::AUTHORIZATION, auth_val);
        }
        let _ = req.form(&form).send().await;
    });
}

enum IdentityRecheckOutcome {
    Matched,
    Mismatch(String),
    RequestFailed(String),
}

fn recheck_renewed_identity(
    auth: &DiscoveredAuth,
    identity_cfg: Option<&SignInIdentity>,
    access_token: &str,
    expected_person: &str,
    expected_subject: Option<&str>,
) -> IdentityRecheckOutcome {
    if let Some(identity) = identity_cfg {
        return match verify_declared_identity(identity, access_token) {
            Ok(email) if email.trim().to_lowercase() == expected_person => {
                IdentityRecheckOutcome::Matched
            }
            Ok(_) => IdentityRecheckOutcome::Mismatch(
                "renewed login email no longer matches the speaker's address".into(),
            ),
            Err(cause) => IdentityRecheckOutcome::RequestFailed(cause),
        };
    }

    let Some(userinfo_endpoint) = auth.userinfo_endpoint() else {
        return IdentityRecheckOutcome::RequestFailed(
            "authorization server omits `userinfo_endpoint`".into(),
        );
    };
    let ui = match fetch_userinfo_json(userinfo_endpoint, access_token) {
        Ok(ui) => ui,
        Err(cause) => return IdentityRecheckOutcome::RequestFailed(cause),
    };
    match verify_userinfo_claims(&ui, expected_subject) {
        Ok(email) if email.to_lowercase() == expected_person => IdentityRecheckOutcome::Matched,
        Ok(_) => IdentityRecheckOutcome::Mismatch(
            "UserInfo `email` no longer matches the speaker's address".into(),
        ),
        Err(cause) => IdentityRecheckOutcome::Mismatch(cause),
    }
}

fn clear_renewal_lease(home: &Home, company: &ProjectSlug, person: &str, server: &str) {
    if let Ok(mut db) = StateDb::open(home) {
        let path = db.path().to_path_buf();
        if let Err(error) = db.write::<_, StateDbError>(|tx| {
            tx.execute(
                "UPDATE mcp_logins SET renewing_until = NULL
                 WHERE company = ?1 AND person = ?2 AND server = ?3",
                params![company.as_str(), person, server],
            )
            .map_err(StateDbError::sqlite(&path))?;
            Ok(())
        }) {
            tracing::warn!(event = "mcp_renewal_lease_clear_failed", company = %company, server, %error);
        }
    }
}

enum RefreshOutcome {
    Refreshed(OAuthTokenResponse),
    Rejected,
    Failed(String),
}

fn refresh_via_rmcp(
    server_url: &str,
    auth: &DiscoveredAuth,
    registration: &Registration,
    client_secret: Option<&str>,
    existing_access: &str,
    refresh_token: &str,
) -> RefreshOutcome {
    let mut manager = match _discover::auth_manager(server_url, auth) {
        Ok(m) => m,
        Err(error) => return RefreshOutcome::Failed(error.to_string()),
    };
    let cred_store = InMemoryCredentialStore::new();
    let seed_token: OAuthTokenResponse = match serde_json::from_value(json!({
        "access_token": existing_access,
        "token_type": "Bearer",
        "refresh_token": refresh_token,
    })) {
        Ok(token) => token,
        Err(error) => {
            return RefreshOutcome::Failed(format!(
                "failed to prepare refresh credentials: {error}"
            ));
        }
    };
    let stored = StoredCredentials::new(
        registration.client_id.clone(),
        Some(seed_token),
        Vec::new(),
        None,
    );
    manager.set_credential_store(cred_store.clone());
    if let Err(error) = manager.configure_client(oauth_client_config(registration, client_secret)) {
        return RefreshOutcome::Failed(format!(
            "failed to configure OAuth client: {}",
            _http::sanitize_auth_error(&error)
        ));
    }
    match _http::run_async(server_url, async {
        cred_store.save(stored).await?;
        manager.refresh_token().await
    }) {
        Ok(Ok(token)) => RefreshOutcome::Refreshed(token),
        Ok(Err(AuthError::TokenRefreshRejected(_))) => RefreshOutcome::Rejected,
        Ok(Err(error)) => RefreshOutcome::Failed(_http::sanitize_auth_error(&error)),
        Err(error) => RefreshOutcome::Failed(error.to_string()),
    }
}

/// Single-flight OAuth 2.1 refresh-token renewal and identity re-check:
/// delegates the refresh-token grant to `rmcp::transport::auth::AuthorizationManager`
/// while coordinating cross-process callers through `mcp_logins.renewing_until`.
#[allow(clippy::too_many_arguments)]
fn try_refresh_login(
    home: &Home,
    company: &ProjectSlug,
    server: &McpServer,
    person: &str,
    auth: &DiscoveredAuth,
    registration: &Registration,
    client_secret: Option<&str>,
    identity_cfg: Option<&SignInIdentity>,
    failed_access_token: Option<&str>,
) -> Result<Option<(Login, Vec<Login>)>, McpError> {
    let token_endpoint = auth.token_endpoint().ok_or_else(|| McpError::Server {
        server: server.name.to_string(),
        cause: "authorization server metadata omits `token_endpoint`".into(),
    })?;
    if !instrukt_core::config::mcp_address_admitted(token_endpoint) {
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: "token_endpoint must be https (or loopback http)".into(),
        });
    }

    enum LeaseStep {
        AlreadyFresh(String),
        StartSignIn,
        Wait,
        Acquired {
            generation: i64,
            subject: Option<String>,
            access_token: String,
            refresh_token: Option<String>,
            needs_refresh: bool,
        },
    }

    const RENEWAL_LEASE_SECS: i64 = 90;
    let deadline = Instant::now() + Duration::from_secs(SIGN_IN_WAIT_SECS);
    let inspection = home.for_inspection();
    let (generation, subject, existing_access, existing_refresh, needs_refresh) = loop {
        let now = Timestamp::now();
        let now_str = now.to_string();

        if let Some(read) = StateRead::open(&inspection)? {
            let peek = query_login(
                read.conn(),
                read.path(),
                company.as_str(),
                person,
                server.name.as_str(),
            )?;
            drop(read);
            let Some(peek) = peek else {
                return Ok(None);
            };
            if peek.resource != server.url || peek.return_id != registration.return_id {
                return Ok(None);
            }
            let still_bad = failed_access_token.is_some_and(|bad| bad == peek.access_token);
            if peek.checked != 0 && !still_bad && is_token_fresh(peek.expires_at.as_deref(), now) {
                return Ok(Some(bearer_logins(&server.name, peek.access_token)));
            }
            let lease_held = peek
                .renewing_until
                .as_deref()
                .and_then(|s| s.parse::<Timestamp>().ok())
                .is_some_and(|until| until > now);
            if lease_held {
                if Instant::now() >= deadline {
                    return Err(McpError::Server {
                        server: server.name.to_string(),
                        cause: "timed out waiting for an in-flight token renewal".into(),
                    });
                }
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
        }

        let lease_until = now
            .checked_add(jiff::SignedDuration::from_secs(RENEWAL_LEASE_SECS))
            .unwrap_or(now)
            .to_string();

        let step = {
            let mut db = StateDb::open(home)?;
            let path = db.path().to_path_buf();
            db.write::<_, StateDbError>(|tx| {
                let Some(row) =
                    query_login(tx, &path, company.as_str(), person, server.name.as_str())?
                else {
                    return Ok(LeaseStep::StartSignIn);
                };
                if row.resource != server.url || row.return_id != registration.return_id {
                    return Ok(LeaseStep::StartSignIn);
                }
                let fresh = is_token_fresh(row.expires_at.as_deref(), now);
                let still_bad = failed_access_token.is_some_and(|bad| bad == row.access_token);
                if row.checked != 0 && !still_bad && fresh {
                    return Ok(LeaseStep::AlreadyFresh(row.access_token));
                }
                let lease_held = row
                    .renewing_until
                    .as_deref()
                    .and_then(|s| s.parse::<Timestamp>().ok())
                    .is_some_and(|until| until > now);
                if lease_held {
                    return Ok(LeaseStep::Wait);
                }
                let needs_refresh = row.checked != 0 || !fresh || still_bad;
                if needs_refresh && row.refresh_token.is_none() {
                    tx.execute(
                        "DELETE FROM mcp_logins
                         WHERE company = ?1 AND person = ?2 AND server = ?3 AND return_id = ?4 AND access_token = ?5",
                        params![
                            company.as_str(),
                            person,
                            server.name.as_str(),
                            registration.return_id,
                            row.access_token,
                        ],
                    )
                    .map_err(StateDbError::sqlite(&path))?;
                    return Ok(LeaseStep::StartSignIn);
                }

                let gen_val =
                    current_generation(tx, &path, company.as_str(), person, server.name.as_str())?;
                let acquired = tx
                    .execute(
                        "UPDATE mcp_logins SET renewing_until = ?4
                         WHERE company = ?1 AND person = ?2 AND server = ?3 AND return_id = ?5
                           AND (renewing_until IS NULL OR renewing_until <= ?6)",
                        params![
                            company.as_str(),
                            person,
                            server.name.as_str(),
                            lease_until,
                            registration.return_id,
                            now_str,
                        ],
                    )
                    .map_err(StateDbError::sqlite(&path))?;
                if acquired == 0 {
                    return Ok(LeaseStep::Wait);
                }
                Ok(LeaseStep::Acquired {
                    generation: gen_val,
                    subject: row.subject,
                    access_token: row.access_token,
                    refresh_token: row.refresh_token,
                    needs_refresh,
                })
            })?
        };

        match step {
            LeaseStep::AlreadyFresh(tok) => return Ok(Some(bearer_logins(&server.name, tok))),
            LeaseStep::StartSignIn => return Ok(None),
            LeaseStep::Wait => {
                if Instant::now() >= deadline {
                    return Err(McpError::Server {
                        server: server.name.to_string(),
                        cause: "timed out waiting for an in-flight token renewal".into(),
                    });
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            LeaseStep::Acquired {
                generation,
                subject,
                access_token,
                refresh_token,
                needs_refresh,
            } => {
                break (
                    generation,
                    subject,
                    access_token,
                    refresh_token,
                    needs_refresh,
                );
            }
        }
    };

    let (candidate_access, candidate_refresh) = if !needs_refresh {
        (existing_access, existing_refresh)
    } else {
        let Some(refresh_token) = existing_refresh else {
            clear_renewal_lease(home, company, person, server.name.as_str());
            return Ok(None);
        };
        let token_response = match refresh_via_rmcp(
            &server.url,
            auth,
            registration,
            client_secret,
            &existing_access,
            &refresh_token,
        ) {
            RefreshOutcome::Refreshed(resp) => resp,
            RefreshOutcome::Rejected => {
                let mut db = StateDb::open(home)?;
                let path = db.path().to_path_buf();
                db.write::<_, StateDbError>(|tx| {
                    let deleted = tx
                        .execute(
                            "DELETE FROM mcp_logins
                             WHERE company = ?1 AND person = ?2 AND server = ?3 AND return_id = ?4 AND refresh_token = ?5",
                            params![
                                company.as_str(),
                                person,
                                server.name.as_str(),
                                registration.return_id,
                                refresh_token,
                            ],
                        )
                        .map_err(StateDbError::sqlite(&path))?;
                    if deleted == 0 {
                        tx.execute(
                            "UPDATE mcp_logins SET renewing_until = NULL
                             WHERE company = ?1 AND person = ?2 AND server = ?3",
                            params![company.as_str(), person, server.name.as_str()],
                        )
                        .map_err(StateDbError::sqlite(&path))?;
                    }
                    Ok(())
                })?;
                return Ok(None);
            }
            RefreshOutcome::Failed(cause) => {
                clear_renewal_lease(home, company, person, server.name.as_str());
                return Err(McpError::Server {
                    server: server.name.to_string(),
                    cause: format!("token renewal failed: {cause}"),
                });
            }
        };

        let now_done = Timestamp::now();
        let parsed = match parse_token_response(&token_response, now_done) {
            Ok(parsed) => parsed,
            Err(cause) => {
                clear_renewal_lease(home, company, person, server.name.as_str());
                return Err(McpError::Server {
                    server: server.name.to_string(),
                    cause: format!("token renewal failed: {cause}"),
                });
            }
        };
        let access_token = parsed.access_token;
        let new_refresh = parsed.refresh_token.unwrap_or(refresh_token);

        let saved = {
            let mut db = StateDb::open(home)?;
            let path = db.path().to_path_buf();
            db.write::<_, StateDbError>(|tx| {
                if current_generation(tx, &path, company.as_str(), person, server.name.as_str())?
                    != generation
                {
                    return Ok(false);
                }
                let updated = tx
                    .execute(
                        "UPDATE mcp_logins
                         SET access_token = ?4, expires_at = ?5, checked = 0, refresh_token = ?6,
                             scopes = COALESCE(?7, scopes), renewed_at = ?8
                         WHERE company = ?1 AND person = ?2 AND server = ?3 AND return_id = ?9",
                        params![
                            company.as_str(),
                            person,
                            server.name.as_str(),
                            access_token,
                            parsed.expires_at,
                            new_refresh,
                            parsed.granted_scopes,
                            now_done.to_string(),
                            registration.return_id,
                        ],
                    )
                    .map_err(StateDbError::sqlite(&path))?;
                Ok(updated == 1)
            })?
        };
        if !saved {
            clear_renewal_lease(home, company, person, server.name.as_str());
            revoke_direct(
                auth,
                &registration.client_id,
                client_secret,
                Some(&new_refresh),
                &access_token,
            );
            return Err(McpError::Server {
                server: server.name.to_string(),
                cause: "login was signed out while renewing".into(),
            });
        }
        (access_token, Some(new_refresh))
    };

    match recheck_renewed_identity(
        auth,
        identity_cfg,
        &candidate_access,
        person,
        subject.as_deref(),
    ) {
        IdentityRecheckOutcome::Matched => {
            let promoted = {
                let mut db = StateDb::open(home)?;
                let path = db.path().to_path_buf();
                db.write::<_, StateDbError>(|tx| {
                    if current_generation(
                        tx,
                        &path,
                        company.as_str(),
                        person,
                        server.name.as_str(),
                    )? != generation
                    {
                        return Ok(false);
                    }
                    let updated = tx
                        .execute(
                            "UPDATE mcp_logins
                             SET checked = 1, renewing_until = NULL
                             WHERE company = ?1 AND person = ?2 AND server = ?3 AND return_id = ?4 AND access_token = ?5",
                            params![
                                company.as_str(),
                                person,
                                server.name.as_str(),
                                registration.return_id,
                                candidate_access,
                            ],
                        )
                        .map_err(StateDbError::sqlite(&path))?;
                    Ok(updated == 1)
                })?
            };
            if !promoted {
                clear_renewal_lease(home, company, person, server.name.as_str());
                revoke_direct(
                    auth,
                    &registration.client_id,
                    client_secret,
                    candidate_refresh.as_deref(),
                    &candidate_access,
                );
                return Err(McpError::Server {
                    server: server.name.to_string(),
                    cause: "login was signed out while verifying renewed identity".into(),
                });
            }
            Ok(Some(bearer_logins(&server.name, candidate_access)))
        }
        IdentityRecheckOutcome::Mismatch(cause) => {
            tracing::warn!(event = "mcp_renewal_identity_mismatch", company = %company, server = %server.name, %cause);
            let mut db = StateDb::open(home)?;
            let path = db.path().to_path_buf();
            db.write::<_, StateDbError>(|tx| {
                if current_generation(tx, &path, company.as_str(), person, server.name.as_str())?
                    == generation
                {
                    tx.execute(
                        "DELETE FROM mcp_logins
                         WHERE company = ?1 AND person = ?2 AND server = ?3 AND return_id = ?4",
                        params![
                            company.as_str(),
                            person,
                            server.name.as_str(),
                            registration.return_id,
                        ],
                    )
                    .map_err(StateDbError::sqlite(&path))?;
                }
                Ok(())
            })?;
            drop(db);
            revoke_direct(
                auth,
                &registration.client_id,
                client_secret,
                candidate_refresh.as_deref(),
                &candidate_access,
            );
            Ok(None)
        }
        IdentityRecheckOutcome::RequestFailed(cause) => {
            clear_renewal_lease(home, company, person, server.name.as_str());
            Err(McpError::Server {
                server: server.name.to_string(),
                cause: format!("renewed login identity check failed: {cause}"),
            })
        }
    }
}
