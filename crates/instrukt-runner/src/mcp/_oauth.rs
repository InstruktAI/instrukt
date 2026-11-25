//! OAuth 2.1 authorization-code sign-in (`sign_in.mode: authorization-code`),
//! local-boundary ingress reachability check (`instrukt mcp check-ingress`),
//! and `instrukt mcp sign-out`.

use std::time::{Duration, Instant};

use base64::Engine as _;
use instrukt_core::config::{
    Configuration, McpServer, SecretSource, SignInIdentity, SignInMode, resolve_secret_from,
};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use instrukt_core::state_db::{StateDb, StateDbError, StateRead};
use jiff::Timestamp;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Serialize;
use serde_json::Value;

use super::_discover::{DiscoveredAuth, discover_auth};
use super::_issuer::{bearer_logins, is_token_fresh, obtain_issuer_login};
use super::_redis_mcp::{claim_sign_in_return, ping_cluster};
use super::_registrations::{
    Registration, ensure_company_client_registration, ensure_dynamic_registration,
    ensure_host_client_registration, random_base64url_256, random_hex_128, sha256_base64url,
    sha256_hex,
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

/// Verify local `ingress.url`, local cluster Redis reachability, and one real
/// external GET probe through `<ingress.url>/hooks/inbound/sign-in/<probe_return_id>`,
/// confirming the matching `SHA-256(probe_state)` entry arrives in cluster Redis
/// and immediately claiming and deleting it. Never inspects any other computer's config.
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

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(PROBE_TIMEOUT))
        .timeout_global(Some(PROBE_TIMEOUT))
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into();
    let response = agent
        .get(&probe_url)
        .call()
        .map_err(|error| McpError::Server {
            server: "ingress".into(),
            cause: format!("external callback probe to {ingress_url} failed: {error}"),
        })?;
    let status = response.status().as_u16();
    if status != 200 {
        let _ = claim_sign_in_return(&transport, &state_digest);
        return Err(McpError::Server {
            server: "ingress".into(),
            cause: format!(
                "external callback probe to {ingress_url}/hooks/inbound/sign-in/... returned HTTP {status} (expected 200)"
            ),
        });
    }

    let claimed =
        claim_sign_in_return(&transport, &state_digest).map_err(|error| McpError::Server {
            server: "ingress".into(),
            cause: format!("failed to claim probe return from cluster Redis: {error}"),
        })?;
    match claimed {
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
/// one transaction, followed by best-effort OAuth token revocation for any
/// removed refresh tokens.
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

            struct RevokeItem {
                resource: String,
                return_id: String,
                token: String,
                token_type_hint: &'static str,
            }
            let mut revoke_list: Vec<RevokeItem> = Vec::new();
            let mut affected = 0usize;
            for srv in &servers {
                if let Some((resource, return_id, access_token, refresh_token)) = tx
                    .query_row(
                        "SELECT resource, return_id, access_token, refresh_token FROM mcp_logins
                         WHERE company = ?1 AND person = ?2 AND server = ?3",
                        params![company.as_str(), person, srv],
                        |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, String>(2)?,
                                r.get::<_, Option<String>>(3)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(StateDbError::sqlite(&path))?
                {
                    let (token, hint) = match refresh_token {
                        Some(rt) => (rt, "refresh_token"),
                        None => (access_token, "access_token"),
                    };
                    revoke_list.push(RevokeItem {
                        resource,
                        return_id,
                        token,
                        token_type_hint: hint,
                    });
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

    // Best-effort RFC 7009 revocation after writer.lock has been released.
    for item in to_revoke {
        best_effort_revoke_token(
            home,
            company,
            &item.resource,
            &item.return_id,
            &item.token,
            item.token_type_hint,
        );
    }

    Ok(affected)
}

fn best_effort_revoke_token(
    home: &Home,
    company: &ProjectSlug,
    resource: &str,
    return_id: &str,
    token: &str,
    token_type_hint: &str,
) {
    let Ok(Some(read)) = StateRead::open(&home.for_inspection()) else {
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
    let Some(revocation_endpoint) = auth.revocation_endpoint.as_deref() else {
        return;
    };
    if !instrukt_core::config::mcp_address_admitted(revocation_endpoint) {
        return;
    }

    let inspection = home.for_inspection();
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
                    .mcp_servers(instrukt_core::config::LayerKind::Company)
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

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into();
    let use_post_secret = client_secret.is_some()
        && auth
            .token_endpoint_auth_methods_supported
            .iter()
            .any(|m| m == "client_secret_post")
        && !auth
            .token_endpoint_auth_methods_supported
            .iter()
            .any(|m| m == "client_secret_basic");
    let mut form: Vec<(&str, &str)> = vec![("token", token), ("token_type_hint", token_type_hint)];
    let mut req = agent.post(revocation_endpoint);
    match (client_secret.as_deref(), use_post_secret) {
        (Some(secret), false) => {
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(format!("{client_id}:{secret}"));
            req = req.header("Authorization", format!("Basic {encoded}"));
        }
        (Some(secret), true) => {
            form.push(("client_id", &client_id));
            form.push(("client_secret", secret));
        }
        (None, _) => {
            form.push(("client_id", &client_id));
        }
    }
    let _ = req.send_form(form);
}

/// Obtain a speaker's Bearer login for a `speaker_login: sign-in` server,
/// dispatching by `sign_in.mode` (`authorization-code` or `issuer`).
/// `rejected_access_token` is the access token the server just refused, on a retry.
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

    // 1. Check declared client registration FIRST (no network call) so any rotation or
    //    switch (`client_ref` <-> `client`, or rotated client_id/secret/issuer) retires
    //    its old logins BEFORE we ever honor a cached token!
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

    // 2. Check cached login in mcp_logins, verifying it belongs to the active registration.
    type LoginRow = (
        String,
        Option<String>,
        String,
        String,
        Option<String>,
        i32,
        Option<String>,
    );
    let cached: Option<LoginRow> = if let Some(db) = StateRead::open(&inspection)? {
        db.conn()
            .query_row(
                "SELECT resource, subject, return_id, access_token, expires_at, checked, refresh_token
                 FROM mcp_logins
                 WHERE company = ?1 AND person = ?2 AND server = ?3",
                params![company.as_str(), person, server.name.as_str()],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .optional()
            .map_err(StateDbError::sqlite(db.path()))?
    } else {
        None
    };

    let cached_matches_reg = |ret_id: &str| {
        prechecked_reg
            .as_ref()
            .is_none_or(|reg| reg.return_id == ret_id)
    };

    if let Some((ref resource, _, ref ret_id, ref access_token, ref expires_at, checked, _)) =
        cached
        && rejected_access_token.is_none_or(|bad| bad != access_token.as_str())
        && resource == &server.url
        && cached_matches_reg(ret_id)
        && checked != 0
        && is_token_fresh(expires_at.as_deref(), now)
    {
        return Ok(bearer_logins(&server.name, access_token.clone()));
    }

    // 3. Discover AS metadata and enforce security gates BEFORE resolving any client secret.
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
        .authorization_endpoint
        .as_deref()
        .ok_or_else(|| McpError::Server {
            server: server.name.to_string(),
            cause: "authorization server metadata omits `authorization_endpoint`".into(),
        })?;
    let token_endpoint = auth
        .token_endpoint
        .as_deref()
        .ok_or_else(|| McpError::Server {
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

    // 4. Resolve local ingress.url & cluster Redis transport.
    let ingress_url = ingress_url_opt.ok_or_else(|| McpError::Server {
        server: server.name.to_string(),
        cause: "global `ingress.url` must be configured for MCP sign-in".into(),
    })?;
    let transport = ScopedTransport::cluster_only(&global, &RunnerState::new(&inspection))
        .map_err(|error| McpError::Server {
            server: server.name.to_string(),
            cause: format!("cluster Redis is unavailable for MCP sign-in: {error}"),
        })?;

    // 5. Enforce issuer pinning BEFORE resolving client secret (prevents cross-tenant
    //    secret exfiltration to a company-controlled rogue authorization server).
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

    // 6. Handle existing cached login:
    //    - If the server's declared URL changed from the login's resource, remove + revoke it.
    //    - If it belongs to the active registration, attempt single-flight refresh-token renewal
    //      (or repeat identity re-check for an unchecked token) before falling back to sign-in.
    if let Some((ref resource, _, ref ret_id, ref access_token, _, _, ref refresh_token)) = cached {
        if resource != &server.url {
            let mut db = StateDb::open(home)?;
            let path = db.path().to_path_buf();
            db.write::<_, StateDbError>(|tx| {
                tx.execute(
                    "DELETE FROM mcp_logins WHERE company = ?1 AND person = ?2 AND server = ?3 AND resource = ?4",
                    params![company.as_str(), person, server.name.as_str(), resource],
                )
                .map_err(StateDbError::sqlite(&path))?;
                Ok(())
            })?;
            drop(db);
            let (tok, hint) = match refresh_token.as_deref() {
                Some(rt) => (rt, "refresh_token"),
                None => (access_token.as_str(), "access_token"),
            };
            best_effort_revoke_token(home, company, resource, ret_id, tok, hint);
        } else if ret_id == &registration.return_id
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

    // 7. Capture generation and record (or join) the pending sign-in + waiter,
    //    then release the writer lock immediately before waiting or calling network endpoints.
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

    let state = random_base64url_256()?;
    let state_digest = sha256_hex(&state);
    let code_verifier = random_base64url_256()?;
    let code_challenge = sha256_base64url(&code_verifier);
    let nonce = sign_in_cfg
        .identity
        .is_none()
        .then(random_base64url_256)
        .transpose()?;
    let waiter_id = random_hex_128()?;
    let expires_at = now
        .checked_add(jiff::SignedDuration::from_secs(SIGN_IN_LIFETIME_SECS))
        .unwrap_or(now);
    let waiting_until = now
        .checked_add(jiff::SignedDuration::from_secs(SIGN_IN_WAIT_SECS as i64))
        .unwrap_or(now);

    let auth_link = build_authorization_link(
        auth_endpoint,
        &registration.client_id,
        &registration.return_address,
        &scope_param,
        &state,
        &code_challenge,
        &server.url,
        nonce.as_deref(),
    )?;

    let connector_name = speaker.connector.as_deref().unwrap_or_default();
    let conversation_id = speaker.conversation.as_deref().unwrap_or_default();
    let message_id = speaker.message.as_deref().unwrap_or_default();
    let recipient_id = speaker.identity.as_deref().unwrap_or(&person);

    let (active_digest, active_generation, active_verifier, active_nonce) = {
        let mut db = StateDb::open(home)?;
        let path = db.path().to_path_buf();
        db.write::<_, StateDbError>(|tx| {
            let current_gen: i64 = tx
                .query_row(
                    "SELECT generation FROM mcp_generations WHERE company = ?1 AND person = ?2 AND server = ?3",
                    params![company.as_str(), person, server.name.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StateDbError::sqlite(&path))?
                .unwrap_or(0);

            // Expire any stale open sign-ins first.
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
                            connector_name,
                            conversation_id,
                            message_id,
                            recipient_id,
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

    // 6. Wait for the return entry in cluster Redis (up to 90s) without holding writer.lock.
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

    // Always remove our own waiter row when stopping.
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

        // Check if another waiter or sign-out already closed this sign-in (read-only, no lock).
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
                read.conn()
                    .query_row(
                        "SELECT access_token FROM mcp_logins WHERE company = ?1 AND person = ?2 AND server = ?3",
                        params![company.as_str(), person, server.name.as_str()],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(StateDbError::sqlite(read.path()))?
            } else {
                None
            };
            (status, token)
        };
        if let Some((state, failure, link_sent_at)) = status {
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
                    let detail = failure.unwrap_or(state);
                    return Err(McpError::Server {
                        server: server.name.to_string(),
                        cause: format!("sign-in did not complete: {detail}"),
                    });
                }
                _ => {
                    if Instant::now() >= deadline {
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
                }
            }
        } else {
            return Err(McpError::Server {
                server: server.name.to_string(),
                cause: "pending sign-in record disappeared".into(),
            });
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
    // Move open -> exchanging atomically before calling the authorization server,
    // releasing the writer lock before any HTTP call.
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

    if entry.return_id != registration.return_id {
        let msg = "return_id did not match the sign-in's registration";
        close_sign_in_failure(home, state_digest, "failed", msg);
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: msg.into(),
        });
    }

    if auth.report.iss_parameter_supported {
        if entry.iss.as_deref() != Some(auth.issuer.as_str()) {
            let msg = "return omitted or mismatched RFC 9207 `iss` parameter";
            close_sign_in_failure(home, state_digest, "failed", msg);
            return Err(McpError::Server {
                server: server.name.to_string(),
                cause: msg.into(),
            });
        }
    } else if let Some(ref got_iss) = entry.iss
        && got_iss != &auth.issuer
    {
        let msg = "return `iss` parameter did not match the authorization server";
        close_sign_in_failure(home, state_digest, "failed", msg);
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: msg.into(),
        });
    }

    if let Some(err_code) = entry.error {
        close_sign_in_failure(home, state_digest, "declined", &err_code);
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: format!("authorization server declined sign-in: {err_code}"),
        });
    }

    let Some(code) = entry.code else {
        close_sign_in_failure(
            home,
            state_digest,
            "failed",
            "return carried neither code nor error",
        );
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: "return carried neither code nor error".into(),
        });
    };
    let Some(verifier) = code_verifier else {
        close_sign_in_failure(home, state_digest, "failed", "missing PKCE code_verifier");
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: "missing PKCE code_verifier".into(),
        });
    };

    // Exchange authorization code at token endpoint.
    let token_endpoint = auth.token_endpoint.as_deref().unwrap_or_default();
    let token_body = exchange_authorization_code(
        token_endpoint,
        &code,
        verifier,
        &registration.return_address,
        &server.url,
        &registration.client_id,
        client_secret,
        &auth.token_endpoint_auth_methods_supported,
    )
    .map_err(|cause| {
        close_sign_in_failure(home, state_digest, "failed", &cause);
        McpError::Server {
            server: server.name.to_string(),
            cause,
        }
    })?;

    let token_type = token_body
        .get("token_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !token_type.eq_ignore_ascii_case("Bearer") {
        let msg = format!("unsupported token_type `{token_type}`");
        close_sign_in_failure(home, state_digest, "failed", &msg);
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: msg,
        });
    }
    let Some(access_token) = token_body
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
    else {
        close_sign_in_failure(
            home,
            state_digest,
            "failed",
            "token response omitted access_token",
        );
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: "token response omitted access_token".into(),
        });
    };
    let refresh_token = token_body
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned);
    let granted_scopes = token_body
        .get("scope")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let now = Timestamp::now();
    let expires_at = token_body
        .get("expires_in")
        .and_then(Value::as_i64)
        .filter(|&secs| secs > 0)
        .and_then(|secs| now.checked_add(jiff::SignedDuration::from_secs(secs)).ok())
        .map(|ts| ts.to_string());

    // Verify who signed in.
    let (verified_email, subject) = match identity_cfg {
        Some(identity) => {
            let email = verify_declared_identity(identity, &access_token).map_err(|cause| {
                close_sign_in_failure(home, state_digest, "failed", &cause);
                McpError::Server {
                    server: server.name.to_string(),
                    cause,
                }
            })?;
            (email, None)
        }
        None => {
            let id_token = token_body
                .get("id_token")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    revoke_direct(
                        auth,
                        &registration.client_id,
                        client_secret,
                        refresh_token.as_deref(),
                        &access_token,
                    );
                    close_sign_in_failure(
                        home,
                        state_digest,
                        "refused",
                        "token response omitted OpenID Connect id_token",
                    );
                    McpError::Server {
                        server: server.name.to_string(),
                        cause: "token response omitted OpenID Connect id_token".into(),
                    }
                })?;
            verify_oidc_identity(
                auth,
                &registration.client_id,
                nonce,
                id_token,
                &access_token,
            )
            .map_err(|cause| {
                revoke_direct(
                    auth,
                    &registration.client_id,
                    client_secret,
                    refresh_token.as_deref(),
                    &access_token,
                );
                close_sign_in_failure(home, state_digest, "refused", &cause);
                McpError::Server {
                    server: server.name.to_string(),
                    cause,
                }
            })?
        }
    };

    if verified_email.trim().to_lowercase() != person {
        let msg = "sign-in account email did not match the speaker's email address";
        revoke_direct(
            auth,
            &registration.client_id,
            client_secret,
            refresh_token.as_deref(),
            &access_token,
        );
        close_sign_in_failure(home, state_digest, "refused", msg);
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: msg.into(),
        });
    }

    // Store login and close sign-in in one brief transaction checking generation.
    let now_str = now.to_string();
    let saved = {
        let mut db = StateDb::open(home)?;
        let path = db.path().to_path_buf();
        db.write::<_, StateDbError>(|tx| {
            let current_gen: i64 = tx
                .query_row(
                    "SELECT generation FROM mcp_generations WHERE company = ?1 AND person = ?2 AND server = ?3",
                    params![company.as_str(), person, server.name.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StateDbError::sqlite(&path))?
                .unwrap_or(0);
            if current_gen != generation {
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
                    expires_at,
                    refresh_token,
                    granted_scopes,
                    now_str,
                ],
            )
            .map_err(StateDbError::sqlite(&path))?;
            // Record told_at when at least one call is still waiting; otherwise leave told_at NULL
            // so the runner sends the contracted completion DM.
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
fn build_authorization_link(
    auth_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    scope: &str,
    state: &str,
    code_challenge: &str,
    resource: &str,
    nonce: Option<&str>,
) -> Result<String, McpError> {
    let mut url = url::Url::parse(auth_endpoint).map_err(|error| McpError::Server {
        server: auth_endpoint.to_owned(),
        cause: format!("invalid authorization_endpoint: {error}"),
    })?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("response_type", "code");
        pairs.append_pair("client_id", client_id);
        pairs.append_pair("redirect_uri", redirect_uri);
        if !scope.is_empty() {
            pairs.append_pair("scope", scope);
        }
        pairs.append_pair("state", state);
        pairs.append_pair("code_challenge", code_challenge);
        pairs.append_pair("code_challenge_method", "S256");
        pairs.append_pair("resource", resource);
        if let Some(nonce) = nonce {
            pairs.append_pair("nonce", nonce);
        }
    }
    Ok(url.into())
}

#[allow(clippy::too_many_arguments)]
fn exchange_authorization_code(
    token_endpoint: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    resource: &str,
    client_id: &str,
    client_secret: Option<&str>,
    auth_methods: &[String],
) -> Result<Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into();

    let use_post_secret = client_secret.is_some()
        && auth_methods.iter().any(|m| m == "client_secret_post")
        && !auth_methods.iter().any(|m| m == "client_secret_basic");

    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", code_verifier),
        ("redirect_uri", redirect_uri),
        ("resource", resource),
    ];

    let mut request = agent
        .post(token_endpoint)
        .header("Accept", "application/json");
    match (client_secret, use_post_secret) {
        (Some(secret), false) => {
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(format!("{client_id}:{secret}"));
            request = request.header("Authorization", format!("Basic {encoded}"));
        }
        (Some(secret), true) => {
            form.push(("client_id", client_id));
            form.push(("client_secret", secret));
        }
        (None, _) => {
            form.push(("client_id", client_id));
        }
    }

    let mut response = request
        .send_form(form)
        .map_err(|error| format!("token endpoint request failed: {error}"))?;
    let status = response.status().as_u16();
    let body_text = response
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_string()
        .unwrap_or_default();
    if !(200..300).contains(&status) {
        let code = serde_json::from_str::<Value>(&body_text)
            .ok()
            .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| format!("HTTP {status}"));
        return Err(format!("token exchange refused ({code})"));
    }
    serde_json::from_str(&body_text).map_err(|_| "token endpoint returned invalid JSON".to_owned())
}

fn verify_declared_identity(
    identity: &SignInIdentity,
    access_token: &str,
) -> Result<String, String> {
    if !instrukt_core::config::mcp_address_admitted(&identity.url) {
        return Err("identity endpoint address is not admitted".into());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into();
    let mut response = agent
        .get(&identity.url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/json")
        .call()
        .map_err(|error| format!("identity request failed: {error}"))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("identity request returned HTTP {status}"));
    }
    let body_text = response
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_string()
        .map_err(|error| format!("unreadable identity response: {error}"))?;
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

fn verify_oidc_identity(
    auth: &DiscoveredAuth,
    client_id: &str,
    expected_nonce: Option<&str>,
    id_token: &str,
    access_token: &str,
) -> Result<(String, Option<String>), String> {
    let jwks_uri = auth
        .jwks_uri
        .as_deref()
        .ok_or_else(|| "OpenID Provider metadata omits jwks_uri".to_owned())?;
    let userinfo_endpoint = auth
        .userinfo_endpoint
        .as_deref()
        .ok_or_else(|| "OpenID Provider metadata omits userinfo_endpoint".to_owned())?;
    if !instrukt_core::config::mcp_address_admitted(jwks_uri)
        || !instrukt_core::config::mcp_address_admitted(userinfo_endpoint)
    {
        return Err("OIDC endpoint address is not admitted".into());
    }

    let header = decode_header(id_token).map_err(|e| format!("invalid id_token header: {e}"))?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into();

    let mut jwks_resp = agent
        .get(jwks_uri)
        .header("Accept", "application/json")
        .call()
        .map_err(|e| format!("failed to fetch JWKS: {e}"))?;
    if jwks_resp.status().as_u16() != 200 {
        return Err(format!(
            "JWKS endpoint returned HTTP {}",
            jwks_resp.status().as_u16()
        ));
    }
    let jwks_text = jwks_resp
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_string()
        .map_err(|e| format!("unreadable JWKS: {e}"))?;
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
    let data = decode::<Value>(id_token, &decoding_key, &validation)
        .map_err(|e| format!("id_token verification failed: {e}"))?;
    let claims = data.claims;
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

    let mut ui_resp = agent
        .get(userinfo_endpoint)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/json")
        .call()
        .map_err(|e| format!("UserInfo request failed: {e}"))?;
    if ui_resp.status().as_u16() != 200 {
        return Err(format!(
            "UserInfo endpoint returned HTTP {}",
            ui_resp.status().as_u16()
        ));
    }
    let ui_text = ui_resp
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_string()
        .map_err(|e| format!("unreadable UserInfo response: {e}"))?;
    let ui: Value =
        serde_json::from_str(&ui_text).map_err(|_| "UserInfo response is not JSON".to_owned())?;
    if ui.get("sub").and_then(Value::as_str) != Some(sub.as_str()) {
        return Err("UserInfo `sub` does not match id_token `sub`".into());
    }
    if ui.get("email_verified").and_then(Value::as_bool) != Some(true) {
        return Err("UserInfo `email_verified` is not true".into());
    }
    let email = ui
        .get("email")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "UserInfo omitted `email`".to_owned())?
        .to_owned();
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
    let Some(revocation_endpoint) = auth.revocation_endpoint.as_deref() else {
        return;
    };
    if !instrukt_core::config::mcp_address_admitted(revocation_endpoint) {
        return;
    }
    let (token, hint) = match refresh_token {
        Some(rt) => (rt, "refresh_token"),
        None => (access_token, "access_token"),
    };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into();
    let use_post_secret = client_secret.is_some()
        && auth
            .token_endpoint_auth_methods_supported
            .iter()
            .any(|m| m == "client_secret_post")
        && !auth
            .token_endpoint_auth_methods_supported
            .iter()
            .any(|m| m == "client_secret_basic");
    let mut form: Vec<(&str, &str)> = vec![("token", token), ("token_type_hint", hint)];
    let mut req = agent.post(revocation_endpoint);
    match (client_secret, use_post_secret) {
        (Some(secret), false) => {
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(format!("{client_id}:{secret}"));
            req = req.header("Authorization", format!("Basic {encoded}"));
        }
        (Some(secret), true) => {
            form.push(("client_id", client_id));
            form.push(("client_secret", secret));
        }
        (None, _) => {
            form.push(("client_id", client_id));
        }
    }
    let _ = req.send_form(form);
}

enum IdentityRecheckOutcome {
    Matched,
    Mismatch(String),
    RequestFailed(String),
}

/// Re-verify who a renewed access token identifies as (`mcp-access` "Keeping and
/// renewing a login"), distinguishing transport/endpoint failures (keep login unchecked)
/// from confirmed identity mismatches (remove + revoke login and restart sign-in).
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

    let Some(userinfo_endpoint) = auth.userinfo_endpoint.as_deref() else {
        return IdentityRecheckOutcome::RequestFailed(
            "authorization server omits `userinfo_endpoint`".into(),
        );
    };
    if !instrukt_core::config::mcp_address_admitted(userinfo_endpoint) {
        return IdentityRecheckOutcome::RequestFailed(
            "userinfo_endpoint must be https (or loopback http)".into(),
        );
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into();
    let mut ui_resp = match agent
        .get(userinfo_endpoint)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/json")
        .call()
    {
        Ok(resp) => resp,
        Err(e) => {
            return IdentityRecheckOutcome::RequestFailed(format!("UserInfo request failed: {e}"));
        }
    };
    let status = ui_resp.status().as_u16();
    if status != 200 {
        return IdentityRecheckOutcome::RequestFailed(format!(
            "UserInfo endpoint returned HTTP {status}"
        ));
    }
    let ui_text = match ui_resp
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_string()
    {
        Ok(t) => t,
        Err(e) => {
            return IdentityRecheckOutcome::RequestFailed(format!(
                "unreadable UserInfo response: {e}"
            ));
        }
    };
    let Ok(ui) = serde_json::from_str::<Value>(&ui_text) else {
        return IdentityRecheckOutcome::RequestFailed("UserInfo response is not JSON".into());
    };
    if let Some(exp_sub) = expected_subject
        && ui.get("sub").and_then(Value::as_str) != Some(exp_sub)
    {
        return IdentityRecheckOutcome::Mismatch(
            "UserInfo `sub` no longer matches the recorded sign-in subject".into(),
        );
    }
    if ui.get("email_verified").and_then(Value::as_bool) != Some(true) {
        return IdentityRecheckOutcome::Mismatch("UserInfo `email_verified` is not true".into());
    }
    let Some(email) = ui
        .get("email")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return IdentityRecheckOutcome::Mismatch("UserInfo omitted `email`".into());
    };
    if email.to_lowercase() != expected_person {
        return IdentityRecheckOutcome::Mismatch(
            "UserInfo `email` no longer matches the speaker's address".into(),
        );
    }
    IdentityRecheckOutcome::Matched
}

fn clear_renewal_lease(home: &Home, company: &ProjectSlug, person: &str, server: &str) {
    match StateDb::open(home) {
        Ok(mut db) => {
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
        Err(error) => {
            tracing::warn!(event = "mcp_renewal_lease_open_failed", company = %company, server, %error);
        }
    }
}

/// Single-flight OAuth 2.1 refresh-token renewal and identity re-check per
/// `mcp-access` "Keeping and renewing a login":
/// - Enforces one renewal at a time via `renewing_until` CAS; concurrent callers wait.
/// - Stores renewed tokens with `checked = 0` and checks `UPDATE` row count + generation.
/// - Re-verifies identity (UserInfo `sub` + `email_verified` + `email`, or declared identity).
/// - Classifies failures strictly: only `invalid_grant` (or identity mismatch) removes the
///   login and falls back to sign-in; transport / 5xx / `invalid_client` / identity errors
///   fail the call while preserving the login.
/// - Revokes received tokens on stale generation (sign-out race) or identity mismatch.
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
    let token_endpoint = auth
        .token_endpoint
        .as_deref()
        .ok_or_else(|| McpError::Server {
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
        /// Another caller already completed renewal with a checked, fresh token.
        AlreadyFresh(String),
        /// Login was removed or rotated away; caller should start a new interactive sign-in.
        StartSignIn,
        /// Another renewal holds an unexpired lease; wait and poll again.
        Wait,
        /// We acquired the renewal lease and must either refresh or re-check a fresh unchecked token.
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

        // N2: Poll under read-only StateRead first so waiting callers never contend on writer.lock.
        if let Some(read) = StateRead::open(&inspection)? {
            struct PeekRow {
                resource: String,
                return_id: String,
                access_token: String,
                expires_at: Option<String>,
                checked: i32,
                renewing_until: Option<String>,
            }
            let peek = read
                .conn()
                .query_row(
                    "SELECT resource, return_id, access_token, expires_at, checked, renewing_until
                     FROM mcp_logins
                     WHERE company = ?1 AND person = ?2 AND server = ?3",
                    params![company.as_str(), person, server.name.as_str()],
                    |r| {
                        Ok(PeekRow {
                            resource: r.get(0)?,
                            return_id: r.get(1)?,
                            access_token: r.get(2)?,
                            expires_at: r.get(3)?,
                            checked: r.get(4)?,
                            renewing_until: r.get(5)?,
                        })
                    },
                )
                .optional()
                .map_err(StateDbError::sqlite(read.path()))?;
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

        // N1: 90 s lease outlasts refresh POST (30 s) + identity re-check (20 s) + margin.
        let lease_until = now
            .checked_add(jiff::SignedDuration::from_secs(RENEWAL_LEASE_SECS))
            .unwrap_or(now)
            .to_string();

        let step = {
            let mut db = StateDb::open(home)?;
            let path = db.path().to_path_buf();
            db.write::<_, StateDbError>(|tx| {
                struct Row {
                    resource: String,
                    subject: Option<String>,
                    return_id: String,
                    access_token: String,
                    expires_at: Option<String>,
                    checked: i32,
                    refresh_token: Option<String>,
                    renewing_until: Option<String>,
                }
                let row = tx
                    .query_row(
                        "SELECT resource, subject, return_id, access_token, expires_at, checked, refresh_token, renewing_until
                         FROM mcp_logins
                         WHERE company = ?1 AND person = ?2 AND server = ?3",
                        params![company.as_str(), person, server.name.as_str()],
                        |r| {
                            Ok(Row {
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
                    .map_err(StateDbError::sqlite(&path))?;

                let Some(row) = row else {
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
                // F1: An unchecked token is only re-checked directly while it is still fresh
                // and was not rejected with 401; once expired it must be refreshed again.
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

                let gen_val: i64 = tx
                    .query_row(
                        "SELECT generation FROM mcp_generations WHERE company = ?1 AND person = ?2 AND server = ?3",
                        params![company.as_str(), person, server.name.as_str()],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(StateDbError::sqlite(&path))?
                    .unwrap_or(0);
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

    // If the stored access token is unchecked AND still fresh (and not rejected),
    // skip the refresh POST and proceed directly to re-checking that token's identity.
    // Once expired, refresh it first (F1) so an expired unchecked token never locks out a speaker.
    let (candidate_access, candidate_refresh) = if !needs_refresh {
        (existing_access, existing_refresh)
    } else {
        let Some(refresh_token) = existing_refresh else {
            clear_renewal_lease(home, company, person, server.name.as_str());
            return Ok(None);
        };
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .into();
        let use_post_secret = client_secret.is_some()
            && auth
                .token_endpoint_auth_methods_supported
                .iter()
                .any(|m| m == "client_secret_post")
            && !auth
                .token_endpoint_auth_methods_supported
                .iter()
                .any(|m| m == "client_secret_basic");

        let mut form: Vec<(&str, &str)> = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh_token),
            ("resource", &server.url),
        ];
        let mut request = agent
            .post(token_endpoint)
            .header("Accept", "application/json");
        match (client_secret, use_post_secret) {
            (Some(secret), false) => {
                let encoded = base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{secret}", registration.client_id));
                request = request.header("Authorization", format!("Basic {encoded}"));
            }
            (Some(secret), true) => {
                form.push(("client_id", &registration.client_id));
                form.push(("client_secret", secret));
            }
            (None, _) => {
                form.push(("client_id", &registration.client_id));
            }
        }

        let mut response = match request.send_form(form) {
            Ok(resp) => resp,
            Err(error) => {
                clear_renewal_lease(home, company, person, server.name.as_str());
                return Err(McpError::Server {
                    server: server.name.to_string(),
                    cause: format!("token renewal request failed: {error}"),
                });
            }
        };
        let status = response.status().as_u16();
        let body_text = response
            .body_mut()
            .with_config()
            .limit(1024 * 1024)
            .read_to_string()
            .unwrap_or_default();

        if !(200..300).contains(&status) {
            let err_code = serde_json::from_str::<Value>(&body_text)
                .ok()
                .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned));
            if matches!(status, 400 | 401) && err_code.as_deref() == Some("invalid_grant") {
                // Only `invalid_grant` removes the login and falls back to interactive sign-in.
                // N4: clear lease if row wasn't deleted (e.g. refresh_token mismatch).
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
            clear_renewal_lease(home, company, person, server.name.as_str());
            return Err(McpError::Server {
                server: server.name.to_string(),
                cause: format!(
                    "token renewal failed ({})",
                    err_code.unwrap_or_else(|| format!("HTTP {status}"))
                ),
            });
        }

        let body: Value = match serde_json::from_str(&body_text) {
            Ok(v) => v,
            Err(_) => {
                clear_renewal_lease(home, company, person, server.name.as_str());
                return Err(McpError::Server {
                    server: server.name.to_string(),
                    cause: "token renewal response is not valid JSON".into(),
                });
            }
        };
        let token_type = body
            .get("token_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !token_type.eq_ignore_ascii_case("Bearer") {
            clear_renewal_lease(home, company, person, server.name.as_str());
            return Err(McpError::Server {
                server: server.name.to_string(),
                cause: format!("token renewal returned unsupported token_type `{token_type}`"),
            });
        }
        let Some(access_token) = body
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned)
        else {
            clear_renewal_lease(home, company, person, server.name.as_str());
            return Err(McpError::Server {
                server: server.name.to_string(),
                cause: "token renewal response omitted `access_token`".into(),
            });
        };
        let new_refresh = body
            .get("refresh_token")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or(refresh_token);
        let now_done = Timestamp::now();
        let expires_at = body
            .get("expires_in")
            .and_then(Value::as_i64)
            .filter(|&secs| secs > 0)
            .and_then(|secs| {
                now_done
                    .checked_add(jiff::SignedDuration::from_secs(secs))
                    .ok()
            })
            .map(|ts| ts.to_string());
        let granted_scopes = body.get("scope").and_then(Value::as_str).map(str::to_owned);

        // Persist the renewed access token with `checked = 0` (keeping renewing_until active
        // for the identity re-check), checking both generation and UPDATE row count.
        let saved = {
            let mut db = StateDb::open(home)?;
            let path = db.path().to_path_buf();
            db.write::<_, StateDbError>(|tx| {
                let current_gen: i64 = tx
                    .query_row(
                        "SELECT generation FROM mcp_generations WHERE company = ?1 AND person = ?2 AND server = ?3",
                        params![company.as_str(), person, server.name.as_str()],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(StateDbError::sqlite(&path))?
                    .unwrap_or(0);
                if current_gen != generation {
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
                            expires_at,
                            new_refresh,
                            granted_scopes,
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

    // Re-check who the renewed access token identifies as before marking `checked = 1`.
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
                    let current_gen: i64 = tx
                        .query_row(
                            "SELECT generation FROM mcp_generations WHERE company = ?1 AND person = ?2 AND server = ?3",
                            params![company.as_str(), person, server.name.as_str()],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(StateDbError::sqlite(&path))?
                        .unwrap_or(0);
                    if current_gen != generation {
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
                let current_gen: i64 = tx
                    .query_row(
                        "SELECT generation FROM mcp_generations WHERE company = ?1 AND person = ?2 AND server = ?3",
                        params![company.as_str(), person, server.name.as_str()],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(StateDbError::sqlite(&path))?
                    .unwrap_or(0);
                if current_gen == generation {
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
            // Keep login + newest refresh token with `checked = 0`; next call repeats check.
            clear_renewal_lease(home, company, person, server.name.as_str());
            Err(McpError::Server {
                server: server.name.to_string(),
                cause: format!("renewed login identity check failed: {cause}"),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use instrukt_core::config::{McpServer, SpeakerLogin};
    use instrukt_core::home::Home;
    use instrukt_core::ids::ProjectSlug;
    use instrukt_core::state_db::StateDb;
    use instrukt_core::state_db::rusqlite::params;
    use jiff::Timestamp;

    use super::resolve_sign_in_login;
    use crate::mcp::Speaker;

    /// A retry after HTTP 401 on `T1`, made once another call has already
    /// renewed the login to a checked, fresh `T2`, carries `T2` as it is: it
    /// neither discovers the authorization server nor exchanges a refresh
    /// token. The server's address is unreachable, so any renewal fails.
    #[test]
    fn a_retry_after_a_rejected_token_reuses_a_login_another_call_already_renewed() {
        let dir = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        let company = ProjectSlug::parse("acme").unwrap();
        let server = McpServer {
            name: ProjectSlug::parse("workspace").unwrap(),
            url: "http://127.0.0.1:9/mcp".into(),
            description: "A sign-in server".into(),
            speaker_login: SpeakerLogin::SignIn,
            credential: None,
            login: None,
            sign_in: None,
        };
        let speaker = Speaker {
            identity: Some("USTEVEN".into()),
            name: "Steven".into(),
            email: Some("Steven@Example.com".into()),
            is_admin: false,
            is_direct: true,
            connector: Some("slack".into()),
            conversation: Some("D1".into()),
            message: None,
        };
        let now = Timestamp::now();
        let expires = now
            .checked_add(jiff::SignedDuration::from_hours(1))
            .unwrap()
            .to_string();
        let mut db = StateDb::open(&home).unwrap();
        db.write::<_, instrukt_core::state_db::StateDbError>(|tx| {
            tx.execute(
                "INSERT INTO mcp_logins(company, person, server, resource, return_id, access_token, expires_at, checked, refresh_token, signed_in_at)
                 VALUES (?1, 'steven@example.com', ?2, ?3, 'return-1', 'token-2', ?4, 1, 'refresh-2', ?5)",
                params![company.as_str(), server.name.as_str(), server.url, expires, now.to_string()],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        drop(db);

        let (header, _) =
            resolve_sign_in_login(&home, &company, &server, &speaker, Some("token-1"), now)
                .unwrap();

        assert_eq!(header.value, "Bearer token-2");
    }
}
