//! Built-in lightweight per-company OIDC issuer for MCP Enterprise-Managed
//! Authorization (`sign_in.mode: issuer`, ID-JAG `urn:ietf:params:oauth:grant-type:jwt-bearer`).
//!
//! Private ES256 signing keys live strictly in the company home's `secrets.env`.
//! Public discovery and JWKS documents are published read-only to cluster Redis
//! and served by ingress under `/hooks/inbound/issuers/<issuer_id>/...`.

use std::time::Duration;

use base64::Engine as _;
use instrukt_core::config::{
    self, Configuration, DEFAULT_ISSUER_SIGNING_KEY_NAME, Layer, McpServer, SecretSource,
    resolve_secret_from, set_secret_in,
};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use instrukt_core::state_db::{StateDb, StateDbError, StateRead};
use jiff::Timestamp;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::_discover::{JWT_BEARER_GRANT_TYPE, discover_auth};
use super::_redis_mcp::{MAX_ISSUER_DOC_BYTES, publish_issuer_metadata};
use super::_registrations::random_hex_128;
use super::{Login, McpError, Speaker};
use crate::state::RunnerState;
use crate::transport::ScopedTransport;

/// Maximum lifetime (5 minutes) of an ID-JAG assertion JWT.
const ID_JAG_LIFETIME_SECS: i64 = 300;
/// Renew cached access tokens when fewer than 60 seconds remain.
const RENEW_MARGIN_SECS: i64 = 60;

/// Public identity and URLs of a company's built-in OIDC issuer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssuerInfo {
    pub company: String,
    pub issuer_id: String,
    pub issuer_url: String,
    pub jwks_url: String,
    pub enabled: bool,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
struct IdJagClaims<'a> {
    iss: &'a str,
    sub: &'a str,
    email: &'a str,
    aud: &'a str,
    resource: &'a str,
    iat: i64,
    exp: i64,
    jti: String,
}

/// Generated ES256 keypair: private PKCS#8 PEM + public JWKS JSON (never contains `d`).
pub(crate) struct GeneratedKeypair {
    pub private_pem: String,
    pub public_jwks_json: String,
}

/// Generate a fresh ES256 (P-256) keypair using `ring`.
pub(crate) fn generate_es256_keypair() -> Result<GeneratedKeypair, McpError> {
    let rng = SystemRandom::new();
    let pkcs8 =
        EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).map_err(|_| {
            McpError::Server {
                server: "issuer".into(),
                cause: "failed to generate ES256 signing keypair".into(),
            }
        })?;
    let keypair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
        .map_err(|_| McpError::Server {
            server: "issuer".into(),
            cause: "failed to load generated ES256 keypair".into(),
        })?;
    let pub_bytes = keypair.public_key().as_ref();
    if pub_bytes.len() != 65 || pub_bytes[0] != 0x04 {
        return Err(McpError::Server {
            server: "issuer".into(),
            cause: "unexpected P-256 public key encoding".into(),
        });
    }
    let x = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&pub_bytes[1..33]);
    let y = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&pub_bytes[33..65]);
    // RFC 7638 JWK thumbprint (canonical member order: crv, kty, x, y).
    let thumbprint_input = format!(r#"{{"crv":"P-256","kty":"EC","x":"{x}","y":"{y}"}}"#);
    let kid = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(&Sha256::digest(thumbprint_input.as_bytes())[..16]);

    let jwks = json!({
        "keys": [{
            "kty": "EC",
            "crv": "P-256",
            "use": "sig",
            "alg": "ES256",
            "kid": kid,
            "x": x,
            "y": y,
        }]
    });
    let public_jwks_json = serde_json::to_string(&jwks).map_err(|error| McpError::Server {
        server: "issuer".into(),
        cause: format!("failed to serialize public JWKS: {error}"),
    })?;
    let private_pem = pem::encode(&pem::Pem::new("PRIVATE KEY", pkcs8.as_ref()));
    Ok(GeneratedKeypair {
        private_pem,
        public_jwks_json,
    })
}

pub(crate) fn issuer_url_for(ingress_url: &str, issuer_id: &str) -> String {
    format!(
        "{}/hooks/inbound/issuers/{issuer_id}",
        ingress_url.trim_end_matches('/')
    )
}

pub(crate) fn discovery_document(issuer_url: &str) -> String {
    json!({
        "issuer": issuer_url,
        "jwks_uri": format!("{issuer_url}/jwks.json"),
        "id_token_signing_alg_values_supported": ["ES256"],
        "subject_types_supported": ["public"],
        "response_types_supported": ["id_token"],
    })
    .to_string()
}

/// Initialize (or rotate) a company's built-in OIDC issuer:
/// 1. Verify `ingress.url` and cluster Redis reachability FIRST (fail fast, zero local mutation).
/// 2. Retain/create a stable 128-bit `issuer_id` and generate an ES256 keypair.
/// 3. Publish bounded public discovery + JWKS JSON to cluster Redis BEFORE updating `secrets.env`.
/// 4. Store private PEM in the company's `secrets.env`, upsert `company_issuers`, and enable `issuer:`.
pub fn init_company_issuer(
    home: &Home,
    company: &ProjectSlug,
    now: Timestamp,
) -> Result<IssuerInfo, McpError> {
    let company_cfg = Configuration::load_company(home, company)?;
    let global_cfg = Configuration::load_global(home)?;
    let ingress_url = global_cfg.ingress_url()?.ok_or_else(|| McpError::Server {
        server: company.to_string(),
        cause: "global `ingress.url` must be configured before initializing a company issuer"
            .into(),
    })?;

    let existing_key_name = company_cfg
        .company_issuer()?
        .and_then(|c| c.signing_key)
        .and_then(|r| {
            r.strip_prefix("${")
                .and_then(|s| s.strip_suffix('}'))
                .map(str::to_owned)
        })
        .unwrap_or_else(|| DEFAULT_ISSUER_SIGNING_KEY_NAME.to_owned());

    // Verify cluster Redis is reachable before writing any private key or SQLite row.
    let transport =
        ScopedTransport::cluster_only(&global_cfg, &RunnerState::new(home)).map_err(|error| {
            McpError::Server {
                server: company.to_string(),
                cause: format!("cluster Redis unavailable for issuer publication: {error}"),
            }
        })?;
    super::_redis_mcp::ping_cluster(&transport).map_err(|error| McpError::Server {
        server: company.to_string(),
        cause: format!("cluster Redis unreachable for issuer publication: {error}"),
    })?;

    let existing_issuer_id = if let Some(read) = StateRead::open(home)? {
        read.conn()
            .query_row(
                "SELECT issuer_id FROM company_issuers WHERE company = ?1",
                [company.as_str()],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(StateDbError::sqlite(read.path()))?
    } else {
        None
    };
    let issuer_id = match existing_issuer_id {
        Some(id) => id,
        None => random_hex_128()?,
    };

    let keypair = generate_es256_keypair()?;
    let now_str = now.to_string();

    // Publish bounded public discovery and JWKS JSON to cluster Redis BEFORE updating secrets.env.
    let issuer_url = issuer_url_for(&ingress_url, &issuer_id);
    let discovery_json = discovery_document(&issuer_url);
    if discovery_json.len() > MAX_ISSUER_DOC_BYTES
        || keypair.public_jwks_json.len() > MAX_ISSUER_DOC_BYTES
    {
        return Err(McpError::Server {
            server: company.to_string(),
            cause: "public issuer metadata exceeds bounded size limit".into(),
        });
    }
    publish_issuer_metadata(
        &transport,
        &issuer_id,
        &discovery_json,
        &keypair.public_jwks_json,
    )
    .map_err(|error| McpError::Server {
        server: company.to_string(),
        cause: format!("failed to publish public issuer metadata to cluster Redis: {error}"),
    })?;

    // Record stable issuer_id + public JWKS in company_issuers BEFORE overwriting secrets.env,
    // so a DB failure never leaves secrets.env holding a newer private key than company_issuers.
    {
        let mut db = StateDb::open(home)?;
        let path = db.path().to_path_buf();
        db.write::<_, StateDbError>(|tx| {
            tx.execute(
                "INSERT INTO company_issuers (company, issuer_id, public_jwks_json, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(company) DO UPDATE SET
                   public_jwks_json = excluded.public_jwks_json,
                   updated_at = excluded.updated_at",
                params![
                    company.as_str(),
                    issuer_id,
                    keypair.public_jwks_json,
                    now_str
                ],
            )
            .map_err(StateDbError::sqlite(&path))?;
            Ok(())
        })?;
    }

    // Store private key in company home's owner-only secrets.env (never in DB or Redis).
    set_secret_in(
        home,
        &SecretSource::Company(company.clone()),
        &existing_key_name,
        &keypair.private_pem,
    )?;

    // Enable `issuer:` in the company home's company.yaml.
    let layer = Layer::Company(company.clone());
    let rev = config::revision(home, &layer)?;
    let patch_yaml =
        format!("issuer:\n  enabled: true\n  signing_key: '${{{existing_key_name}}}'\n");
    let patch_val = config::parse_patch(&patch_yaml)?;
    config::patch(home, &layer, &rev, &patch_val)?;

    Ok(IssuerInfo {
        company: company.to_string(),
        issuer_id,
        jwks_url: format!("{issuer_url}/jwks.json"),
        issuer_url,
        enabled: true,
        updated_at: now_str,
    })
}

/// Idempotently rebuild derived public discovery + JWKS documents in cluster Redis
/// from authoritative `company_issuers` + current `ingress.url`, without rotating keys.
pub fn republish_company_issuers(
    home: &Home,
    only_company: Option<&ProjectSlug>,
) -> Result<Vec<IssuerInfo>, McpError> {
    let inspection = home.for_inspection();
    let global_cfg = Configuration::load_global(&inspection)?;
    let ingress_url = global_cfg.ingress_url()?.ok_or_else(|| McpError::Server {
        server: "issuer".into(),
        cause: "global `ingress.url` is not configured".into(),
    })?;
    let Some(read) = StateRead::open(&inspection)? else {
        return Ok(Vec::new());
    };
    let rows: Vec<(String, String, String, String)> = match only_company {
        Some(company) => read
            .conn()
            .query_row(
                "SELECT company, issuer_id, public_jwks_json, updated_at FROM company_issuers WHERE company = ?1",
                [company.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(StateDbError::sqlite(read.path()))?
            .into_iter()
            .collect(),
        None => {
            let mut stmt = read
                .conn()
                .prepare("SELECT company, issuer_id, public_jwks_json, updated_at FROM company_issuers ORDER BY company")
                .map_err(StateDbError::sqlite(read.path()))?;
            let mapped = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(StateDbError::sqlite(read.path()))?;
            let mut out = Vec::new();
            for row in mapped {
                out.push(row.map_err(StateDbError::sqlite(read.path()))?);
            }
            out
        }
    };
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let transport = ScopedTransport::cluster_only(&global_cfg, &RunnerState::new(&inspection))
        .map_err(|error| McpError::Server {
            server: "issuer".into(),
            cause: format!("cluster Redis unavailable for issuer publication: {error}"),
        })?;
    let mut published = Vec::new();
    for (company_name, issuer_id, public_jwks_json, updated_at) in rows {
        let Ok(slug) = ProjectSlug::parse(&company_name) else {
            continue;
        };
        let enabled = Configuration::load_company(&inspection, &slug)
            .ok()
            .and_then(|c| c.company_issuer().ok().flatten())
            .is_some_and(|i| i.enabled);
        if !enabled {
            continue;
        }
        let issuer_url = issuer_url_for(&ingress_url, &issuer_id);
        let discovery_json = discovery_document(&issuer_url);
        if discovery_json.len() > MAX_ISSUER_DOC_BYTES
            || public_jwks_json.len() > MAX_ISSUER_DOC_BYTES
        {
            return Err(McpError::Server {
                server: company_name,
                cause: "public issuer metadata exceeds bounded size limit".into(),
            });
        }
        publish_issuer_metadata(&transport, &issuer_id, &discovery_json, &public_jwks_json)
            .map_err(|error| McpError::Server {
                server: company_name.clone(),
                cause: format!(
                    "failed to republish public issuer metadata to cluster Redis: {error}"
                ),
            })?;
        published.push(IssuerInfo {
            company: company_name,
            issuer_id,
            jwks_url: format!("{issuer_url}/jwks.json"),
            issuer_url,
            enabled: true,
            updated_at,
        });
    }
    Ok(published)
}

/// Show a company's built-in OIDC issuer URL and JWKS URL.
pub fn show_company_issuer(home: &Home, company: &ProjectSlug) -> Result<IssuerInfo, McpError> {
    let company_cfg = Configuration::load_company(home, company)?;
    let global_cfg = Configuration::load_global(home)?;
    let ingress_url = global_cfg.ingress_url()?.ok_or_else(|| McpError::Server {
        server: company.to_string(),
        cause: "global `ingress.url` is not configured".into(),
    })?;
    let enabled = company_cfg
        .company_issuer()?
        .is_some_and(|issuer| issuer.enabled);
    let db = StateRead::open(home)?.ok_or_else(|| McpError::Server {
        server: company.to_string(),
        cause: format!(
            "company `{company}` has no initialized OIDC issuer; run `instrukt mcp issuer init --company {company}`"
        ),
    })?;
    let row: Option<(String, String)> = db
        .conn()
        .query_row(
            "SELECT issuer_id, updated_at FROM company_issuers WHERE company = ?1",
            [company.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(StateDbError::sqlite(db.path()))?;
    let (issuer_id, updated_at) = row.ok_or_else(|| McpError::Server {
        server: company.to_string(),
        cause: format!(
            "company `{company}` has no initialized OIDC issuer; run `instrukt mcp issuer init --company {company}`"
        ),
    })?;
    let issuer_url = issuer_url_for(&ingress_url, &issuer_id);
    Ok(IssuerInfo {
        company: company.to_string(),
        issuer_id,
        jwks_url: format!("{issuer_url}/jwks.json"),
        issuer_url,
        enabled,
        updated_at,
    })
}

/// Obtain a speaker's Bearer access token for a `sign_in.mode: issuer` server:
/// reuses a valid cached token from `mcp_logins`, or signs a fresh ES256 ID-JAG
/// JWT asserting the speaker's verified email and exchanges it via `jwt-bearer`.
/// A cached token other than `rejected_access_token`, which the server just
/// refused, is reused even on a retry: another call may already have renewed it.
pub(crate) fn obtain_issuer_login(
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
                "speaker {speaker} has no recorded email address, which `sign_in.mode: issuer` requires"
            ),
        })?;
    let person = email.to_lowercase();

    // 1. Verify company issuer is enabled and capture generation + issuer_id + cached login
    //    in one read-only snapshot so cached tokens are invalidated if issuer changes or is disabled.
    let company_cfg = Configuration::load_company(&home.for_inspection(), company)?;
    let issuer_cfg = company_cfg
        .company_issuer()?
        .filter(|c| c.enabled)
        .ok_or_else(|| McpError::Server {
            server: server.name.to_string(),
            cause: format!(
                "company `{company}` does not have its built-in OIDC issuer enabled; run `instrukt mcp issuer init --company {company}`"
            ),
        })?;
    let (generation, issuer_id, public_jwks_json, cached_login) = {
        let read = StateRead::open(home)?.ok_or_else(|| McpError::Server {
            server: server.name.to_string(),
            cause: format!(
                "company `{company}` has no initialized OIDC issuer; run `instrukt mcp issuer init --company {company}`"
            ),
        })?;
        let generation: i64 = read
            .conn()
            .query_row(
                "SELECT generation FROM mcp_generations WHERE company = ?1 AND person = ?2 AND server = ?3",
                params![company.as_str(), person, server.name.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(StateDbError::sqlite(read.path()))?
            .unwrap_or(0);
        let (issuer_id, public_jwks_json): (String, String) = read
            .conn()
            .query_row(
                "SELECT issuer_id, public_jwks_json FROM company_issuers WHERE company = ?1",
                [company.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(StateDbError::sqlite(read.path()))?
            .ok_or_else(|| McpError::Server {
                server: server.name.to_string(),
                cause: format!(
                    "company `{company}` has no initialized OIDC issuer; run `instrukt mcp issuer init --company {company}`"
                ),
            })?;
        let cached: Option<(String, String, String, Option<String>, i32)> = read
            .conn()
            .query_row(
                "SELECT resource, return_id, access_token, expires_at, checked
                 FROM mcp_logins
                 WHERE company = ?1 AND person = ?2 AND server = ?3",
                params![company.as_str(), person, server.name.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(StateDbError::sqlite(read.path()))?;
        (generation, issuer_id, public_jwks_json, cached)
    };

    let expected_return_id = format!("issuer:{issuer_id}");
    if let Some((resource, return_id, access_token, expires_at, checked)) = cached_login
        && resource == server.url
        && return_id == expected_return_id
        && cached_login_usable(
            &access_token,
            checked,
            expires_at.as_deref(),
            rejected_access_token,
            now,
        )
    {
        return Ok(bearer_logins(&server.name, access_token));
    }

    // 2. Load global ingress.url + public kid + private PEM.
    let global_cfg = Configuration::load_global(&home.for_inspection())?;
    let ingress_url = global_cfg.ingress_url()?.ok_or_else(|| McpError::Server {
        server: server.name.to_string(),
        cause: "global `ingress.url` is not configured".into(),
    })?;
    let kid = extract_first_kid(&public_jwks_json).ok_or_else(|| McpError::Server {
        server: server.name.to_string(),
        cause: "company issuer JWKS has no key id (`kid`)".into(),
    })?;
    let key_ref = issuer_cfg
        .signing_key
        .unwrap_or_else(|| format!("${{{DEFAULT_ISSUER_SIGNING_KEY_NAME}}}"));
    let private_pem = resolve_secret_from(home, &SecretSource::Company(company.clone()), &key_ref)?;

    // 4. Discover authorization server's token endpoint and issuer (`aud`).
    let auth = discover_auth(&server.url)?;
    let token_endpoint = auth.token_endpoint().ok_or_else(|| McpError::Server {
        server: server.name.to_string(),
        cause: "authorization server metadata omits `token_endpoint`".into(),
    })?;
    if !instrukt_core::config::mcp_address_admitted(token_endpoint) {
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: "token endpoint must be https (or loopback http)".into(),
        });
    }

    // 5. Sign short-lived (<= 5 min) ES256 ID-JAG JWT asserting speaker's verified email.
    let iss = issuer_url_for(&ingress_url, &issuer_id);
    let iat = now.as_second();
    let exp = iat + ID_JAG_LIFETIME_SECS;
    let claims = IdJagClaims {
        iss: &iss,
        sub: &person,
        email: &person,
        aud: &auth.issuer,
        resource: &server.url,
        iat,
        exp,
        jti: random_hex_128()?,
    };
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(kid);
    header.typ = Some("oauth-id-jag+jwt".to_owned());
    let encoding_key =
        EncodingKey::from_ec_pem(private_pem.as_bytes()).map_err(|error| McpError::Server {
            server: server.name.to_string(),
            cause: format!("invalid ES256 issuer signing key: {error}"),
        })?;
    let assertion = jsonwebtoken::encode(&header, &claims, &encoding_key).map_err(|error| {
        McpError::Server {
            server: server.name.to_string(),
            cause: format!("failed to sign ID-JAG assertion: {error}"),
        }
    })?;

    // 6. Exchange assertion at token_endpoint via `urn:ietf:params:oauth:grant-type:jwt-bearer`.
    let scope_str = server
        .sign_in
        .as_ref()
        .filter(|s| !s.scopes.is_empty())
        .map(|s| s.scopes.join(" "));
    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", JWT_BEARER_GRANT_TYPE),
        ("assertion", &assertion),
        ("resource", &server.url),
    ];
    if let Some(ref scope) = scope_str {
        form.push(("scope", scope));
    }

    let http_client = super::_http::client(
        server.name.as_str(),
        Duration::from_secs(10),
        Duration::from_secs(30),
        false,
    )?;
    let (status, body_text) = super::_http::run_async(server.name.as_str(), async {
        let response = http_client
            .post(token_endpoint)
            .header(http::header::ACCEPT, "application/json")
            .form(&form)
            .send()
            .await
            .map_err(|error| McpError::Server {
                server: server.name.to_string(),
                cause: format!("issuer token exchange failed: {error}"),
            })?;
        let status = response.status().as_u16();
        let bytes = super::_http::read_bounded_body(response, super::_http::MAX_OAUTH_BODY_BYTES)
            .await
            .unwrap_or_default();
        Ok::<_, McpError>((status, String::from_utf8_lossy(&bytes).into_owned()))
    })??;
    if !(200..300).contains(&status) {
        let code = serde_json::from_str::<Value>(&body_text)
            .ok()
            .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| format!("HTTP {status}"));
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: format!("issuer jwt-bearer token exchange refused ({code})"),
        });
    }
    let body: Value = serde_json::from_str(&body_text).map_err(|_| McpError::Server {
        server: server.name.to_string(),
        cause: "issuer token exchange returned invalid JSON".into(),
    })?;
    let token_type = body
        .get("token_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !token_type.eq_ignore_ascii_case("Bearer") {
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: format!("issuer token exchange returned unsupported token_type `{token_type}`"),
        });
    }
    let access_token = body
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| McpError::Server {
            server: server.name.to_string(),
            cause: "issuer token exchange omitted `access_token`".into(),
        })?
        .to_owned();
    let expires_at = body
        .get("expires_in")
        .and_then(Value::as_i64)
        .filter(|&secs| secs > 0)
        .and_then(|secs| now.checked_add(jiff::SignedDuration::from_secs(secs)).ok())
        .map(|ts| ts.to_string());
    let granted_scopes = body
        .get("scope")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or(scope_str);

    // 7. Cache the Bearer token in mcp_logins in one brief write transaction, checking generation.
    let now_str = now.to_string();
    let return_id = format!("issuer:{issuer_id}");
    let mut db = StateDb::open(home)?;
    let path = db.path().to_path_buf();
    let stored = db.write::<_, StateDbError>(|tx| {
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
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, NULL, ?9, ?10, ?10, NULL)
             ON CONFLICT(company, person, server) DO UPDATE SET
               resource = excluded.resource,
               subject = excluded.subject,
               return_id = excluded.return_id,
               access_token = excluded.access_token,
               expires_at = excluded.expires_at,
               checked = 1,
               refresh_token = NULL,
               scopes = excluded.scopes,
               renewed_at = excluded.renewed_at,
               renewing_until = NULL",
            params![
                company.as_str(),
                person,
                server.name.as_str(),
                server.url,
                person,
                return_id,
                access_token,
                expires_at,
                granted_scopes,
                now_str,
            ],
        )
        .map_err(StateDbError::sqlite(&path))?;
        Ok(true)
    })?;
    if !stored {
        return Err(McpError::Server {
            server: server.name.to_string(),
            cause: "the speaker's login was signed out while completing token exchange".into(),
        });
    }

    Ok(bearer_logins(&server.name, access_token))
}

pub(crate) fn is_token_fresh(expires_at: Option<&str>, now: Timestamp) -> bool {
    let Some(expires_at) = expires_at else {
        // Unknown lifetime: usable until the server answers 401.
        return true;
    };
    let Ok(expiry) = expires_at.parse::<Timestamp>() else {
        return false;
    };
    expiry.as_second() > now.as_second() + RENEW_MARGIN_SECS
}

/// Whether a cached sign-in access token may be sent as it is: its identity
/// was checked, it is fresh, and it is not the token the server just refused.
/// A different token on a retry is one another call already renewed.
pub(crate) fn cached_login_usable(
    access_token: &str,
    checked: i32,
    expires_at: Option<&str>,
    rejected_access_token: Option<&str>,
    now: Timestamp,
) -> bool {
    checked != 0
        && rejected_access_token.is_none_or(|rejected| rejected != access_token)
        && is_token_fresh(expires_at, now)
}

pub(crate) fn bearer_logins(server: &ProjectSlug, access_token: String) -> (Login, Vec<Login>) {
    let label = format!("<{server} sign-in login>");
    let header = Login {
        value: format!("Bearer {access_token}"),
        label: label.clone(),
    };
    let scrub_entries = vec![
        header.clone(),
        Login {
            value: access_token,
            label,
        },
    ];
    (header, scrub_entries)
}

fn extract_first_kid(jwks_json: &str) -> Option<String> {
    let doc: Value = serde_json::from_str(jwks_json).ok()?;
    doc.get("keys")?
        .as_array()?
        .first()?
        .get("kid")?
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{cached_login_usable, generate_es256_keypair};
    use jiff::Timestamp;
    use serde_json::Value;

    #[test]
    fn a_retry_reuses_a_checked_fresh_token_other_than_the_one_refused() {
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let expires = Timestamp::from_second(1_800_003_600).unwrap().to_string();

        assert!(cached_login_usable(
            "renewed",
            1,
            Some(&expires),
            Some("refused"),
            now
        ));
    }

    #[test]
    fn a_retry_does_not_reuse_the_token_the_server_refused() {
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let expires = Timestamp::from_second(1_800_003_600).unwrap().to_string();

        assert!(!cached_login_usable(
            "refused",
            1,
            Some(&expires),
            Some("refused"),
            now
        ));
    }

    #[test]
    fn generated_es256_jwks_contains_only_public_ec_coordinates() {
        let pair = generate_es256_keypair().unwrap();
        assert!(pair.private_pem.contains("BEGIN PRIVATE KEY"));
        let jwks: Value = serde_json::from_str(&pair.public_jwks_json).unwrap();
        let key = &jwks["keys"][0];
        assert_eq!(key["kty"], "EC");
        assert_eq!(key["crv"], "P-256");
        assert_eq!(key["alg"], "ES256");
        assert_eq!(key["use"], "sig");
        assert!(
            key.get("d").is_none(),
            "public JWKS must never contain private `d`"
        );
        assert!(key["x"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(key["y"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(key["kid"].as_str().is_some_and(|s| !s.is_empty()));
    }
}
