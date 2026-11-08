//! Version-pinned translation of T3's internal read-scoped provider snapshot.
//! Wire source: https://github.com/pingdotgg/t3code/tree/6417151c09/apps/server/src
use super::RuntimeStart;
use crate::{process::Invocation, sessions::SessionsError};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use instrukt_core::{
    config::{AgentSelection, Agents, Configuration, Env, HarnessKind},
    home::Home,
};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::BTreeSet,
    net::TcpStream,
    time::{Duration, Instant},
};
use tungstenite::{Message, client::IntoClientRequest};

const T3_VERSION: &str = "0.1.7";
const RPC_REQUEST_ID: &str = "instrukt-readiness";
const TIMEOUT: Duration = Duration::from_secs(10);
const QUOTA_DEFAULT_SECONDS: i64 = 3600;
fn refused(cause: &str) -> SessionsError {
    SessionsError::Refused(format!("T3 readiness: {cause}; start was not invoked"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeState {
    version: u64,
    pid: u32,
    port: u16,
    origin: String,
    server_version: String,
}
#[derive(Deserialize)]
struct Snapshot {
    providers: Vec<Provider>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Provider {
    instance_id: String,
    driver: String,
    enabled: bool,
    installed: bool,
    status: Status,
    auth: Auth,
    #[serde(default, deserialize_with = "present")]
    availability: Option<Available>,
    #[serde(default, deserialize_with = "present")]
    unavailable_reason: Option<String>,
    models: Vec<Model>,
    #[serde(default, deserialize_with = "present")]
    usage_limits: Option<Usage>,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Ready,
    Warning,
    Error,
    Disabled,
}
#[derive(Deserialize)]
struct Auth {
    status: AuthStatus,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum AuthStatus {
    Authenticated,
    Unauthenticated,
    Unknown,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Available {
    Available,
    Unavailable,
}
#[derive(Deserialize)]
struct Model {
    slug: String,
    #[serde(default)]
    aliases: Vec<String>,
}
#[derive(Deserialize)]
struct Usage {
    windows: Vec<Window>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Window {
    id: String,
    used_percent: f64,
    #[serde(default, deserialize_with = "present")]
    resets_at: Option<Timestamp>,
}

fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn command(args: &[&str], operation: &str) -> Result<String, SessionsError> {
    let out = Invocation::program("t3", args)
        .run(TIMEOUT)
        .map_err(|_| refused(operation))?;
    if !out.success {
        return Err(refused(operation));
    }
    Ok(out.stdout)
}
struct Token {
    secret: String,
    session: String,
    revoked: bool,
}
impl Token {
    fn issue() -> Result<Self, SessionsError> {
        let secret = command(
            &[
                "auth",
                "session",
                "issue",
                "--scope",
                "orchestration:read",
                "--ttl",
                "5m",
                "--token-only",
                "--label",
                "instrukt-readiness",
            ],
            "cannot issue read-scoped token",
        )?
        .trim()
        .to_owned();
        let (payload, signature) = secret
            .split_once('.')
            .ok_or_else(|| refused("unrecognized issued token envelope"))?;
        if signature.is_empty() || signature.contains('.') {
            return Err(refused("unrecognized issued token envelope"));
        }
        #[derive(Deserialize)]
        struct Claims {
            v: u64,
            kind: String,
            sid: String,
        }
        let claims:Claims=URL_SAFE_NO_PAD.decode(payload).ok().and_then(|bytes|serde_json::from_slice(&bytes).ok()).ok_or_else(||refused("unrecognized issued token payload; short-lived session may require manual revocation"))?;
        if claims.sid.is_empty() {
            return Err(refused("issued token has no revocable session identity"));
        }
        let mut issued = Self {
            secret,
            session: claims.sid,
            revoked: false,
        };
        if !matches!(claims.v, 1 | 2) || claims.kind != "session" {
            issued.revoke()?;
            return Err(refused("unrecognized issued token payload"));
        }
        Ok(issued)
    }
    fn revoke(&mut self) -> Result<(), SessionsError> {
        command(
            &["auth", "session", "revoke", "--", &self.session],
            "cannot revoke readiness session",
        )?;
        self.revoked = true;
        Ok(())
    }
}
impl Drop for Token {
    fn drop(&mut self) {
        if !self.revoked && self.revoke().is_err() {
            tracing::error!(
                event = "readiness_token_cleanup_failed",
                message = "readiness session revocation failed; its five-minute TTL remains"
            );
        }
    }
}
fn snapshot(home: &Home, env: &Env) -> Result<Snapshot, SessionsError> {
    let cli = command(&["--version"], "cannot inspect CLI version")?;
    if cli.trim().strip_prefix("t3 v") != Some(T3_VERSION) {
        return Err(refused(
            "unsupported CLI version; update the adapter's verified protocol support",
        ));
    }
    let explicit = env.get("T3CODE_HOME").filter(|v| !v.trim().is_empty());
    let base = explicit
        .map(|value| {
            let value = value.trim();
            if let Some(rest) = value.strip_prefix("~/") {
                home.machine_user_home().join(rest)
            } else {
                std::path::PathBuf::from(value)
            }
        })
        .unwrap_or_else(|| home.machine_user_home().join(".t3"));
    let band = if explicit.is_none()
        && env
            .get("VITE_DEV_SERVER_URL")
            .is_some_and(|v| !v.is_empty())
    {
        "dev"
    } else {
        "userdata"
    };
    let bytes = std::fs::read(base.join(band).join("server-runtime.json"))
        .map_err(|_| refused("cannot discover the local running server"))?;
    let state: RuntimeState =
        serde_json::from_slice(&bytes).map_err(|_| refused("unrecognized server runtime state"))?;
    if state.version != 1 || state.server_version != T3_VERSION || state.pid == 0 || state.port == 0
    {
        return Err(refused(
            "unsupported server runtime version or shape; update the adapter's verified protocol support",
        ));
    }
    let mut endpoint =
        url::Url::parse(&state.origin).map_err(|_| refused("invalid local server address"))?;
    if endpoint.scheme() != "http"
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.port_or_known_default() != Some(state.port)
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || endpoint.path() != "/"
    {
        return Err(refused("unrecognized local server address"));
    }
    endpoint
        .set_scheme("ws")
        .map_err(|_| refused("invalid server transport"))?;
    endpoint.set_path("/ws");
    endpoint.set_query(Some("orchestrationProtocol=2"));
    let mut token = Token::issue()?;
    let result = read_rpc(&endpoint, &token.secret);
    let cleanup = token.revoke();
    match (result, cleanup) {
        (Ok(snapshot), Ok(())) => Ok(snapshot),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(_), Err(_)) => Err(refused(
            "snapshot read and readiness token revocation both failed",
        )),
    }
}
fn read_rpc(endpoint: &url::Url, token: &str) -> Result<Snapshot, SessionsError> {
    let addresses = endpoint
        .socket_addrs(|| None)
        .map_err(|_| refused("cannot resolve local server address"))?;
    let deadline = Instant::now() + TIMEOUT;
    let mut connected = None;
    for address in addresses {
        if let Ok(stream) =
            TcpStream::connect_timeout(&address, deadline.saturating_duration_since(Instant::now()))
        {
            connected = Some(stream);
            break;
        }
    }
    let stream = connected.ok_or_else(|| refused("cannot connect to local server"))?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|_| refused("cannot bound readiness transport"))?;
    let mut request = endpoint
        .as_str()
        .into_client_request()
        .map_err(|_| refused("invalid readiness upgrade request"))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {token}")
            .parse()
            .map_err(|_| refused("invalid issued bearer token"))?,
    );
    let (mut socket, _) = tungstenite::client(request, stream)
        .map_err(|_| refused("WebSocket upgrade refused or protocol version unsupported"))?;
    socket.set_config(|config| {
        config.max_message_size = Some(8 * 1024 * 1024);
        config.max_frame_size = Some(8 * 1024 * 1024);
    });
    socket.send(Message::Text(json!({"_tag":"Request","id":RPC_REQUEST_ID,"tag":"server.getConfig","payload":{},"headers":[]}).to_string().into())).map_err(|_|refused("snapshot request failed"))?;
    for _ in 0..32 {
        socket
            .get_mut()
            .set_read_timeout(Some(
                deadline
                    .saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(1)),
            ))
            .map_err(|_| refused("cannot bound snapshot read"))?;
        let message = socket
            .read()
            .map_err(|_| refused("snapshot response failed or timed out"))?;
        let Message::Text(text) = message else {
            if matches!(message, Message::Ping(_) | Message::Pong(_)) {
                continue;
            }
            return Err(refused("unexpected snapshot frame"));
        };
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|_| refused("unrecognized RPC response"))?;
        let frames = if let Some(frames) = value.as_array() {
            frames.clone()
        } else {
            vec![value]
        };
        for frame in frames {
            if frame["_tag"] == "Ping" {
                socket
                    .send(Message::Text(json!({"_tag":"Pong"}).to_string().into()))
                    .map_err(|_| refused("RPC keepalive failed"))?;
                continue;
            }
            if frame["_tag"] != "Exit"
                || frame["requestId"] != RPC_REQUEST_ID
                || frame["exit"]["_tag"] != "Success"
            {
                return Err(refused("unrecognized or refused snapshot response"));
            }
            let snapshot: Snapshot = serde_json::from_value(frame["exit"]["value"].clone())
                .map_err(|_| refused("unrecognized provider snapshot shape"))?;
            let mut ids = BTreeSet::new();
            for provider in &snapshot.providers {
                if provider.instance_id.is_empty()
                    || provider.driver.is_empty()
                    || !ids.insert(&provider.instance_id)
                    || provider.models.iter().any(|m| m.slug.is_empty())
                    || provider.usage_limits.as_ref().is_some_and(|usage| {
                        usage.windows.iter().any(|window| {
                            window.id.is_empty()
                                || !window.used_percent.is_finite()
                                || !(0.0..=100.0).contains(&window.used_percent)
                        })
                    })
                {
                    return Err(refused("invalid provider snapshot facts"));
                }
            }
            let _ = socket.close(None);
            return Ok(snapshot);
        }
    }
    Err(refused("snapshot response did not complete"))
}
fn driver(kind: HarnessKind) -> &'static str {
    match kind {
        HarnessKind::Claude => "claudeAgent",
        HarnessKind::Codex => "codex",
        HarnessKind::Antigravity => "antigravity",
        HarnessKind::Cursor => "cursor",
        HarnessKind::Grok => "grok",
        HarnessKind::Fx => "fx",
        HarnessKind::OpenCode => "opencode",
        HarnessKind::Kiro => "kiro",
        HarnessKind::Pi => "pi",
        HarnessKind::Muse => "muse",
        HarnessKind::AcpRegistry => "acpRegistry",
    }
}
fn unavailable(provider: &Provider, selection: &AgentSelection) -> Option<String> {
    if provider.driver != driver(selection.kind) {
        return Some("instance harness kind differs from configured kind".into());
    }
    if !provider.enabled || matches!(provider.status, Status::Disabled) {
        return Some("instance disabled".into());
    }
    if !provider.installed {
        return Some("harness not installed".into());
    }
    if matches!(provider.auth.status, AuthStatus::Unauthenticated) {
        return Some("instance unauthenticated".into());
    }
    if matches!(provider.availability, Some(Available::Unavailable)) {
        return Some(
            provider
                .unavailable_reason
                .clone()
                .unwrap_or_else(|| "provider unavailable".into()),
        );
    }
    if matches!(provider.status, Status::Error) {
        return Some("provider status error".into());
    }
    if !matches!(selection.model.as_str(), "auto" | "default")
        && !provider
            .models
            .iter()
            .any(|model| model.slug == selection.model || model.aliases.contains(&selection.model))
    {
        return Some("model not reported by instance".into());
    }
    None
}

pub(super) fn prepare(
    home: &Home,
    env: &Env,
    request: &RuntimeStart,
    now: Timestamp,
) -> Result<RuntimeStart, SessionsError> {
    let snapshot = snapshot(home, env)?;
    let config = Configuration::load(home, Some(&request.checkout))?;
    let agents = Agents::load(home)?;
    let default_until = now
        .checked_add(jiff::SignedDuration::from_secs(QUOTA_DEFAULT_SECONDS))
        .map_err(|_| refused("cannot compute the one-hour quota expiry"))?;
    for (name, profile) in &agents.profiles {
        let Some(provider) = snapshot
            .providers
            .iter()
            .find(|p| p.instance_id == profile.instance)
        else {
            continue;
        };
        let exhausted = provider
            .usage_limits
            .as_ref()
            .into_iter()
            .flat_map(|u| &u.windows)
            .filter(|w| w.used_percent == 100.0)
            .map(|w| {
                let default = w.resets_at.is_none();
                let until = w.resets_at.unwrap_or(default_until);
                (
                    until,
                    format!(
                        "quota: window {} at 100%{}",
                        w.id,
                        if default {
                            "; one-hour default expiry (resetsAt missing)"
                        } else {
                            ""
                        }
                    ),
                )
            })
            .filter(|(until, _)| *until > now)
            .max_by_key(|(until, _)| *until);
        if let Some((until, cause)) = exhausted {
            let mut models = BTreeSet::new();
            for tier in [
                instrukt_core::config::Tier::Light,
                instrukt_core::config::Tier::Standard,
                instrukt_core::config::Tier::Strong,
                instrukt_core::config::Tier::Premium,
            ] {
                for entry in agents.model_chain(name, tier)? {
                    models.insert(entry.model);
                }
            }
            crate::agents::mark_quota(
                home,
                &crate::agents::QuotaMark {
                    profile: name.clone(),
                    models,
                    until,
                    cause,
                },
            )?;
        }
    }
    let candidates = if request.preferences.is_empty() {
        request.selection.iter().cloned().collect()
    } else {
        crate::agents::selections(&config, home, &request.preferences, now)?
    };
    let mut tried = Vec::new();
    for selection in candidates {
        let cause = snapshot
            .providers
            .iter()
            .find(|p| p.instance_id == selection.instance)
            .map(|provider| unavailable(provider, &selection))
            .unwrap_or_else(|| Some("instance missing".into()));
        if let Some(cause) = cause {
            tried.push(format!(
                "{} / {}: {cause}",
                selection.profile, selection.model
            ));
            tracing::warn!(event="agent_readiness_skip",profile=%selection.profile,instance=%selection.instance,model=%selection.model,reason=%cause);
            continue;
        }
        let mut prepared = request.clone();
        prepared.selection = Some(selection);
        return Ok(prepared);
    }
    Err(SessionsError::OrderExhausted { tried })
}
