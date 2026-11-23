//! Cluster Redis storage for MCP sign-in return entries and public per-company
//! OIDC issuer metadata (`mcp-access`, "The return" & "Built-in per-company OIDC issuer").
//!
//! No private signing key or OAuth client secret ever enters Redis.

use serde::{Deserialize, Serialize};

use crate::transport::{EventScope, ScopedTransport, TransportError};

/// Prefix for short-lived sign-in return entries keyed by `SHA-256(state)`.
const SIGN_IN_RETURN_PREFIX: &str = "instrukt:mcp:sign-in:return:";
/// Sorted set tracking live sign-in return entries scored by expiry unix time.
const SIGN_IN_ACTIVE_ZSET: &str = "instrukt:mcp:sign-in:active";
/// Sign-in return entries expire after 10 minutes unclaimed.
pub const SIGN_IN_RETURN_TTL_SECS: u64 = 600;
/// Maximum concurrent sign-in return entries held in cluster Redis.
pub const SIGN_IN_MAX_ENTRIES: usize = 1000;

/// Prefix for public per-company OIDC issuer documents in cluster Redis.
const ISSUER_PREFIX: &str = "instrukt:mcp:issuer:";
/// Maximum byte length of a published public discovery or JWKS document.
pub const MAX_ISSUER_DOC_BYTES: usize = 64 * 1024;

/// One sign-in return written by ingress and claimed by the waiting computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignInReturnEntry {
    pub return_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
}

/// Outcome of storing one sign-in return entry in cluster Redis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOutcome {
    Stored,
    Duplicate,
    Full,
}

/// Store a sign-in return entry keyed by `state_digest`, enforcing the 1,000
/// active entry cap and 10-minute TTL without overwriting an existing entry.
pub fn store_sign_in_return(
    transport: &ScopedTransport,
    state_digest: &str,
    entry: &SignInReturnEntry,
    now_unix: i64,
) -> Result<StoreOutcome, TransportError> {
    let key = format!("{SIGN_IN_RETURN_PREFIX}{state_digest}");
    let payload = serde_json::to_string(entry).unwrap_or_default();
    let script = redis::Script::new(
        r#"
        local now = tonumber(ARGV[3])
        local ttl = tonumber(ARGV[2])
        local cap = tonumber(ARGV[4])
        redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', now)
        if redis.call('EXISTS', KEYS[1]) == 1 then
            return 0
        end
        if redis.call('ZCARD', KEYS[2]) >= cap then
            return -1
        end
        local ok = redis.call('SET', KEYS[1], ARGV[1], 'EX', ttl, 'NX')
        if not ok then
            return 0
        end
        redis.call('ZADD', KEYS[2], now + ttl, ARGV[5])
        return 1
        "#,
    );
    let code: i64 = transport.endpoint(EventScope::Cluster).run(|conn| {
        script
            .key(&key)
            .key(SIGN_IN_ACTIVE_ZSET)
            .arg(&payload)
            .arg(SIGN_IN_RETURN_TTL_SECS)
            .arg(now_unix)
            .arg(SIGN_IN_MAX_ENTRIES)
            .arg(state_digest)
            .invoke(conn)
    })?;
    Ok(match code {
        1 => StoreOutcome::Stored,
        0 => StoreOutcome::Duplicate,
        _ => StoreOutcome::Full,
    })
}

/// Atomically read and remove the sign-in return entry for `state_digest`.
pub fn claim_sign_in_return(
    transport: &ScopedTransport,
    state_digest: &str,
) -> Result<Option<SignInReturnEntry>, TransportError> {
    let key = format!("{SIGN_IN_RETURN_PREFIX}{state_digest}");
    let script = redis::Script::new(
        r#"
        local val = redis.call('GET', KEYS[1])
        if val then
            redis.call('DEL', KEYS[1])
            redis.call('ZREM', KEYS[2], ARGV[1])
        end
        return val
        "#,
    );
    let raw: Option<String> = transport.endpoint(EventScope::Cluster).run(|conn| {
        script
            .key(&key)
            .key(SIGN_IN_ACTIVE_ZSET)
            .arg(state_digest)
            .invoke(conn)
    })?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    Ok(serde_json::from_str(&raw).ok())
}

/// Publish bounded public discovery and JWKS JSON for `issuer_id` into cluster Redis.
pub fn publish_issuer_metadata(
    transport: &ScopedTransport,
    issuer_id: &str,
    discovery_json: &str,
    jwks_json: &str,
) -> Result<(), TransportError> {
    let discovery_key = format!("{ISSUER_PREFIX}{issuer_id}:discovery");
    let jwks_key = format!("{ISSUER_PREFIX}{issuer_id}:jwks");
    transport.endpoint(EventScope::Cluster).run(|conn| {
        redis::pipe()
            .atomic()
            .set(&discovery_key, discovery_json)
            .set(&jwks_key, jwks_json)
            .query::<()>(conn)
    })
}

/// Read public OIDC discovery JSON for `issuer_id` from cluster Redis.
pub fn read_issuer_discovery(
    transport: &ScopedTransport,
    issuer_id: &str,
) -> Result<Option<String>, TransportError> {
    let key = format!("{ISSUER_PREFIX}{issuer_id}:discovery");
    transport
        .endpoint(EventScope::Cluster)
        .command(redis::cmd("GET").arg(&key))
}

/// Read public JWKS JSON for `issuer_id` from cluster Redis.
pub fn read_issuer_jwks(
    transport: &ScopedTransport,
    issuer_id: &str,
) -> Result<Option<String>, TransportError> {
    let key = format!("{ISSUER_PREFIX}{issuer_id}:jwks");
    transport
        .endpoint(EventScope::Cluster)
        .command(redis::cmd("GET").arg(&key))
}

/// Verify cluster Redis responds to `PING`.
pub fn ping_cluster(transport: &ScopedTransport) -> Result<(), TransportError> {
    let _: String = transport
        .endpoint(EventScope::Cluster)
        .command(&mut redis::cmd("PING"))?;
    Ok(())
}
