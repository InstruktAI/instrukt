//! Protocol- and shape-validated translation of T3's internal read-scoped provider snapshot.
//! Wire source: https://github.com/pingdotgg/t3code/tree/6417151c09/apps/server/src
//!
//! Readiness is off the start path. A runner keeps one read-scoped token,
//! issued when it starts and renewed before it expires, and refreshes the
//! provider snapshot on its own wake ([`Readiness`]). Each snapshot read is
//! published with the time it was observed; a start reads the last one
//! published ([`last`]) and never waits on an acquisition in progress.
use super::{ObservedSnapshot, ProviderFacts, ProviderSnapshot, ReportedModel, UsageWindow};
use crate::{process::Invocation, sessions::SessionsError};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use instrukt_core::{
    config::{Env, HarnessKind},
    home::Home,
    ids::ProviderInstanceId,
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

const RPC_REQUEST_ID: &str = "instrukt-readiness";
/// The limit on one T3 CLI launch and on one snapshot read.
const TIMEOUT: Duration = Duration::from_secs(10);
/// The lifetime T3 gives a readiness token (`--ttl`).
const TOKEN_LIFETIME: &str = "5m";
const TOKEN_LIFETIME_SECONDS: i64 = 300;
/// How long before its expiry a token is replaced.
const RENEW_BEFORE_SECONDS: i64 = 60;
fn refused(cause: &str) -> SessionsError {
    SessionsError::Refused(format!("T3 readiness: {cause}"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeState {
    version: u64,
    pid: u32,
    port: u16,
    origin: String,
    server_version: Option<String>,
}
#[derive(Deserialize)]
struct Snapshot {
    providers: Vec<Provider>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Provider {
    instance_id: ProviderInstanceId,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandFailure {
    Launch,
    Timeout,
    Io,
    Exit,
}
impl CommandFailure {
    fn as_str(self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::Timeout => "timeout",
            Self::Io => "io",
            Self::Exit => "exit",
        }
    }
}
fn command_failure(error: crate::process::ProcessError) -> CommandFailure {
    match error {
        crate::process::ProcessError::Spawn { .. } => CommandFailure::Launch,
        crate::process::ProcessError::Timeout { .. } => CommandFailure::Timeout,
        crate::process::ProcessError::Io { .. } | crate::process::ProcessError::Clock { .. } => {
            CommandFailure::Io
        }
    }
}
fn command_output(
    outcome: Result<crate::process::Finished, crate::process::ProcessError>,
) -> Result<String, CommandFailure> {
    match outcome {
        Ok(out) if out.success => Ok(out.stdout),
        Ok(_) => Err(CommandFailure::Exit),
        Err(error) => Err(command_failure(error)),
    }
}
fn command(args: &[&str], operation: &str) -> Result<String, SessionsError> {
    match command_output(Invocation::program("t3", args).run(TIMEOUT)) {
        Ok(stdout) => Ok(stdout),
        Err(failure) => {
            tracing::warn!(
                event = "t3_readiness_command_failed",
                operation,
                category = failure.as_str()
            );
            Err(refused(&format!("{operation} [{}]", failure.as_str())))
        }
    }
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
                TOKEN_LIFETIME,
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
/// The local server's WebSocket endpoint, from its runtime descriptor.
fn endpoint(home: &Home, env: &Env) -> Result<url::Url, SessionsError> {
    let cli = match command(&["--version"], "cannot inspect CLI release") {
        Ok(release) => Some(release),
        Err(error) => {
            tracing::debug!(event="t3_readiness_release_unavailable",error=%error);
            None
        }
    };
    let location = super::_t3_home::T3Home::discover(home, env);
    let bytes = std::fs::read(location.descriptor())
        .map_err(|_| refused("cannot discover the local running server"))?;
    let state: RuntimeState =
        serde_json::from_slice(&bytes).map_err(|_| refused("unrecognized server runtime state"))?;
    if state.version != 1 || state.pid == 0 || state.port == 0 {
        return Err(refused("unsupported server runtime descriptor shape"));
    }
    tracing::debug!(event="t3_readiness_releases",cli_release=?cli,server_release=?state.server_version,descriptor_version=state.version,orchestration_protocol=2);
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
    Ok(endpoint)
}

/// The snapshot a start reads: the last one any refresh in this process
/// published, with the time it was observed.
static LAST: std::sync::RwLock<Option<std::sync::Arc<ObservedSnapshot>>> =
    std::sync::RwLock::new(None);

/// The last snapshot published in this process, if any.
pub(super) fn last() -> Option<std::sync::Arc<ObservedSnapshot>> {
    match LAST.read() {
        Ok(last) => last.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn publish(observed: &std::sync::Arc<ObservedSnapshot>) {
    let next = Some(std::sync::Arc::clone(observed));
    match LAST.write() {
        Ok(mut last) => *last = next,
        Err(poisoned) => *poisoned.into_inner() = next,
    }
}

/// The runner's one readiness token and the refresh of the provider
/// snapshot it reads. The token is issued on the first refresh and replaced
/// before it expires: the next is issued, then the earlier one revoked. The
/// last one is revoked when this is dropped. A revocation that fails is
/// logged; the token lapses with its lifetime.
pub(crate) struct Readiness {
    env: Env,
    slot: std::sync::Arc<std::sync::Mutex<Slot>>,
}

/// The one token the runner keeps: the token reads use and the token a stop
/// revokes are this slot's, replaced in one step.
#[derive(Default)]
struct Slot {
    /// The kept token and when it is renewed.
    kept: Option<(Token, Timestamp)>,
    /// A stop has taken the slot: no token is kept in it any more.
    claimed: bool,
}

fn lock(slot: &std::sync::Mutex<Slot>) -> std::sync::MutexGuard<'_, Slot> {
    match slot.lock() {
        Ok(slot) => slot,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl Readiness {
    pub(super) fn new(env: Env) -> Self {
        Self {
            env,
            slot: std::sync::Arc::default(),
        }
    }

    /// The secret of the token to read with at `now`, issuing or renewing
    /// it as due. A replacement is issued outside the slot and kept in one
    /// step; the earlier token is revoked after that, outside the slot. Once
    /// a stop has claimed the slot nothing is kept: a replacement issued
    /// meanwhile is revoked here.
    fn token(&mut self, now: Timestamp) -> Result<String, SessionsError> {
        {
            let slot = lock(&self.slot);
            if slot.claimed {
                return Err(refused("the runner is stopping"));
            }
            if let Some((token, renew_at)) = &slot.kept
                && now < *renew_at
            {
                return Ok(token.secret.clone());
            }
        }
        let next = Token::issue()?;
        let renew_at = now
            .checked_add(jiff::SignedDuration::from_secs(
                TOKEN_LIFETIME_SECONDS - RENEW_BEFORE_SECONDS,
            ))
            .map_err(|_| refused("cannot compute the token's renewal time"))?;
        let secret = next.secret.clone();
        let earlier = {
            let mut slot = lock(&self.slot);
            if slot.claimed {
                drop(slot);
                // Revoked as it is dropped: the stop took the slot first.
                drop(next);
                return Err(refused("the runner is stopping"));
            }
            slot.kept.replace((next, renew_at))
        };
        // Revoked as it is dropped, the slot already holding the next one.
        drop(earlier);
        Ok(secret)
    }
}

impl Drop for Readiness {
    /// A readiness that ends revokes the token it keeps.
    fn drop(&mut self) {
        let kept = {
            let mut slot = lock(&self.slot);
            slot.claimed = true;
            slot.kept.take()
        };
        drop(kept);
    }
}

impl super::Readiness for Readiness {
    fn refresh(
        &mut self,
        home: &Home,
        clock: &dyn Fn() -> Timestamp,
    ) -> Result<std::sync::Arc<ObservedSnapshot>, SessionsError> {
        let endpoint = endpoint(home, &self.env)?;
        let secret = self.token(clock())?;
        let source = read_rpc(&endpoint, &secret)?;
        // Observed when the read returned, however long it took.
        let observed = std::sync::Arc::new(ObservedSnapshot {
            snapshot: translate(source),
            at: clock(),
        });
        publish(&observed);
        Ok(observed)
    }

    fn stopper(&self) -> Box<dyn Fn() + Send + Sync> {
        let slot = std::sync::Arc::clone(&self.slot);
        Box::new(move || {
            let kept = {
                let mut slot = lock(&slot);
                slot.claimed = true;
                slot.kept.take()
            };
            let Some((mut token, _)) = kept else {
                return;
            };
            // One attempt at the kept token, bounded in real time whatever
            // the operator's clock says: past the bound its command's process
            // group is ended and reaped before the process ends.
            token.revoked = true;
            let attempt =
                Invocation::program("t3", &["auth", "session", "revoke", "--", &token.session])
                    .run_in_real_time(TIMEOUT);
            if let Err(failure) = command_output(attempt) {
                tracing::error!(
                    event = "readiness_token_cleanup_failed",
                    category = failure.as_str(),
                    message =
                        "readiness session revocation failed at stop; its five-minute TTL remains"
                );
            }
        })
    }
}

/// One snapshot read with a token of its own, revoked after it: for a
/// command that reads readiness once, never for a start.
fn acquire(home: &Home, env: &Env) -> Result<Snapshot, SessionsError> {
    let endpoint = endpoint(home, env)?;
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
                if provider.driver.is_empty()
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
fn kind(driver: &str) -> Option<HarnessKind> {
    match driver {
        "claudeAgent" => Some(HarnessKind::Claude),
        "codex" => Some(HarnessKind::Codex),
        "antigravity" => Some(HarnessKind::Antigravity),
        "cursor" => Some(HarnessKind::Cursor),
        "grok" => Some(HarnessKind::Grok),
        "fx" => Some(HarnessKind::Fx),
        "opencode" => Some(HarnessKind::OpenCode),
        "kiro" => Some(HarnessKind::Kiro),
        "pi" => Some(HarnessKind::Pi),
        "muse" => Some(HarnessKind::Muse),
        "acpRegistry" => Some(HarnessKind::AcpRegistry),
        _ => None,
    }
}
fn unavailable(provider: &Provider) -> Option<String> {
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
    // Claude emits warning/unknown and bundled models when its account probe fails.
    // Codex emits ready/unknown for a successfully probed no-auth custom configuration.
    if provider.driver == "claudeAgent"
        && matches!(provider.status, Status::Warning)
        && matches!(provider.auth.status, AuthStatus::Unknown)
    {
        return Some("Claude authentication probe failed or incomplete".into());
    }
    None
}
pub(super) fn snapshot(home: &Home, env: &Env) -> Result<ProviderSnapshot, SessionsError> {
    Ok(translate(acquire(home, env)?))
}

fn translate(source: Snapshot) -> ProviderSnapshot {
    ProviderSnapshot {
        native_picks: vec!["auto".into(), "default".into()],
        providers: source
            .providers
            .into_iter()
            .map(|provider| {
                let unavailable_reason = unavailable(&provider);
                ProviderFacts {
                    kind: kind(&provider.driver),
                    instance: provider.instance_id,
                    unavailable_reason,
                    models: provider
                        .models
                        .into_iter()
                        .map(|model| ReportedModel {
                            name: model.slug,
                            aliases: model.aliases,
                        })
                        .collect(),
                    usage: provider
                        .usage_limits
                        .into_iter()
                        .flat_map(|usage| usage.windows)
                        .map(|window| UsageWindow {
                            id: window.id,
                            used_percent: window.used_percent,
                            resets_at: window.resets_at,
                        })
                        .collect(),
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandFailure, command_output};
    use crate::process::{Finished, ProcessError};
    use std::{io, time::Duration};
    #[test]
    fn launch_failure_keeps_its_safe_category() {
        let result = command_output(Err(ProcessError::Spawn {
            program: "t3".into(),
            source: io::Error::new(io::ErrorKind::NotFound, "private detail"),
        }));
        assert_eq!(result, Err(CommandFailure::Launch));
    }
    #[test]
    fn timeout_keeps_its_safe_category() {
        let result = command_output(Err(ProcessError::Timeout {
            program: "t3".into(),
            timeout: Duration::from_secs(10),
        }));
        assert_eq!(result, Err(CommandFailure::Timeout));
    }
    #[test]
    fn io_failure_keeps_its_safe_category() {
        let result = command_output(Err(ProcessError::Io {
            program: "t3".into(),
            source: io::Error::other("private detail"),
        }));
        assert_eq!(result, Err(CommandFailure::Io));
    }
    #[test]
    fn unsuccessful_exit_keeps_its_category_and_discards_raw_output() {
        let result = command_output(Ok(Finished {
            success: false,
            stdout: "private stdout".into(),
            stderr: "private stderr".into(),
        }));
        assert_eq!(result, Err(CommandFailure::Exit));
    }
}
