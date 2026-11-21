//! Protocol- and shape-validated translation of T3's internal read-scoped provider snapshot.
//! Wire source: https://github.com/pingdotgg/t3code/tree/bf469bb19669671d0df2efda74ed73867a7137c0/apps/server/src
//!
//! Readiness is off the start path. A runner keeps one read-scoped token,
//! issued when it starts and renewed before it expires, and refreshes the
//! provider snapshot on its own wake ([`Readiness`]). Each snapshot read is
//! published with the time it was observed; a start reads the last one
//! published ([`last`]) and never waits on an acquisition in progress.
//!
//! The same read answers which agent instances are configured
//! ([`configured`]): the enabled providers, with the home each one's
//! settings and instance environment name, interpreted at T3 `v0.1.8`'s
//! semantics (`packages/contracts/src/settings.ts`, `providerInstance.ts`,
//! `apps/server/src/provider/Drivers/CodexDriver.ts`).
use super::{ObservedSnapshot, ProviderFacts, ProviderSnapshot, ReportedModel, UsageWindow};
use crate::{process::Invocation, sessions::SessionsError};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use instrukt_core::{
    config::{Env, HarnessKind},
    home::Home,
    ids::ProviderInstanceId,
    interfaces::{ConfiguredInstance, InstructionAssurance, InstructionHomeSource},
};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    net::TcpStream,
    path::{Path, PathBuf},
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
    settings: Settings,
}
/// The configured instances' settings, decoded apart from the provider
/// facts: what an instance's home is depends on them.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    #[serde(default)]
    provider_instances: BTreeMap<ProviderInstanceId, InstanceSettings>,
}
#[derive(Deserialize)]
struct InstanceSettings {
    /// Whether the instance is enabled; T3 decodes an absent value as
    /// enabled.
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    #[serde(default)]
    environment: Vec<Variable>,
    /// Each driver decodes its own `config`; only the home settings are read.
    #[serde(default)]
    config: serde_json::Value,
}
fn enabled_by_default() -> bool {
    true
}
/// An instance-environment variable. T3 sends a sensitive variable's value
/// redacted (empty, or a secret reference), so a selected home stays unknown.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Variable {
    name: String,
    #[serde(default)]
    value: serde_json::Value,
    #[serde(default)]
    sensitive: bool,
    #[serde(default)]
    value_redacted: bool,
}
/// The homes T3 reports for a managed Codex instance.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimePaths {
    home_path: String,
    #[serde(default)]
    shadow_home_path: Option<String>,
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
    #[serde(default, deserialize_with = "present")]
    runtime_paths: Option<RuntimePaths>,
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
        crate::process::ProcessError::Io { .. } => CommandFailure::Io,
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
        Self::issue_scoped("orchestration:read", "instrukt-readiness")
    }

    fn issue_scoped(scope: &str, label: &str) -> Result<Self, SessionsError> {
        let secret = command(
            &[
                "auth",
                "session",
                "issue",
                "--scope",
                scope,
                "--ttl",
                TOKEN_LIFETIME,
                "--token-only",
                "--label",
                label,
            ],
            if scope == "orchestration:read" {
                "cannot issue read-scoped token"
            } else {
                "cannot issue operate-scoped token"
            },
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
    tracing::debug!(event="t3_readiness_cli_release", cli_release=?cli);
    server_endpoint(home, env)
}

fn server_endpoint(home: &Home, env: &Env) -> Result<url::Url, SessionsError> {
    let location = super::_t3_home::T3Home::discover(home, env);
    let bytes = std::fs::read(location.descriptor())
        .map_err(|_| refused("cannot discover the local running server"))?;
    let state: RuntimeState =
        serde_json::from_slice(&bytes).map_err(|_| refused("unrecognized server runtime state"))?;
    if state.version != 1 || state.pid == 0 || state.port == 0 {
        return Err(refused("unsupported server runtime descriptor shape"));
    }
    tracing::debug!(event="t3_readiness_releases",server_release=?state.server_version,descriptor_version=state.version,orchestration_protocol=2);
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
                    .run(TIMEOUT);
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
// Counts bytes accepted by the TCP stream, including partial writes.
struct FrameStream {
    stream: TcpStream,
    written: usize,
}

impl std::io::Read for FrameStream {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        std::io::Read::read(&mut self.stream, bytes)
    }
}

impl std::io::Write for FrameStream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let written = std::io::Write::write(&mut self.stream, bytes)?;
        self.written += written;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::Write::flush(&mut self.stream)
    }
}

use super::DispatchError::{self, AfterFrame, BeforeFrame};

pub(super) fn dispatch(
    home: &Home,
    env: &Env,
    payload: serde_json::Value,
    before_write: &mut dyn FnMut() -> Result<(), SessionsError>,
) -> Result<(), DispatchError> {
    let endpoint = server_endpoint(home, env).map_err(BeforeFrame)?;
    let mut token = Token::issue_scoped("orchestration:operate", "instrukt-conversation-send")
        .map_err(BeforeFrame)?;
    let outcome = rpc(
        &endpoint,
        &token.secret,
        "orchestration.dispatchCommand",
        payload,
        before_write,
    )
    .and_then(|value| {
        value["sequence"]
            .as_u64()
            .map(|_| ())
            .ok_or_else(|| AfterFrame(refused("invalid dispatch receipt")))
    });
    // A cleanup failure cannot undo an accepted dispatch.
    if let Err(error) = token.revoke() {
        tracing::warn!(event="t3_dispatch_token_cleanup_failed", error=%error);
    }
    outcome
}

fn rpc(
    endpoint: &url::Url,
    token: &str,
    tag: &str,
    payload: serde_json::Value,
    before_write: &mut dyn FnMut() -> Result<(), SessionsError>,
) -> Result<serde_json::Value, DispatchError> {
    let addresses = endpoint
        .socket_addrs(|| None)
        .map_err(|_| BeforeFrame(refused("cannot resolve local server address")))?;
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
    let stream = connected.ok_or_else(|| BeforeFrame(refused("cannot connect to local server")))?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|_| BeforeFrame(refused("cannot bound readiness transport")))?;
    let mut request = endpoint
        .as_str()
        .into_client_request()
        .map_err(|_| BeforeFrame(refused("invalid readiness upgrade request")))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {token}")
            .parse()
            .map_err(|_| BeforeFrame(refused("invalid issued bearer token")))?,
    );
    let (mut socket, _) = tungstenite::client(request, FrameStream { stream, written: 0 })
        .map_err(|_| {
            BeforeFrame(refused(
                "WebSocket upgrade refused or protocol version unsupported",
            ))
        })?;
    socket.set_config(|config| {
        config.max_message_size = Some(8 * 1024 * 1024);
        config.max_frame_size = Some(8 * 1024 * 1024);
    });
    let frame =
        json!({"_tag":"Request","id":RPC_REQUEST_ID,"tag":tag,"payload":payload,"headers":[]})
            .to_string();
    before_write().map_err(BeforeFrame)?;
    socket.get_mut().written = 0;
    if socket.send(Message::Text(frame.into())).is_err() {
        let error = refused("RPC request write failed");
        return Err(if socket.get_mut().written != 0 {
            AfterFrame(error)
        } else {
            BeforeFrame(error)
        });
    }
    for _ in 0..32 {
        socket
            .get_mut()
            .stream
            .set_read_timeout(Some(
                deadline
                    .saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(1)),
            ))
            .map_err(|_| AfterFrame(refused("cannot bound snapshot read")))?;
        let message = socket
            .read()
            .map_err(|_| AfterFrame(refused("snapshot response failed or timed out")))?;
        let Message::Text(text) = message else {
            if matches!(message, Message::Ping(_) | Message::Pong(_)) {
                continue;
            }
            return Err(AfterFrame(refused("unexpected snapshot frame")));
        };
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|_| AfterFrame(refused("unrecognized RPC response")))?;
        let frames = if let Some(frames) = value.as_array() {
            frames.clone()
        } else {
            vec![value]
        };
        for frame in frames {
            if frame["_tag"] == "Ping" {
                socket
                    .send(Message::Text(json!({"_tag":"Pong"}).to_string().into()))
                    .map_err(|_| AfterFrame(refused("RPC keepalive failed")))?;
                continue;
            }
            if frame["_tag"] != "Exit" || frame["requestId"] != RPC_REQUEST_ID {
                return Err(AfterFrame(refused("unrecognized RPC response")));
            }
            let _ = socket.close(None);
            if frame["exit"]["_tag"] == "Success" {
                return Ok(frame["exit"]["value"].clone());
            }
            if matches!(
                tag,
                "orchestration.dispatchCommand"
                    | "orchestration.launchThread"
                    | "orchestration.getThreadProjection"
            ) {
                let exit = &frame["exit"];
                let detail = exit
                    .pointer("/cause/0/error/detail")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| {
                        exit.pointer("/cause/0/error/message")
                            .and_then(serde_json::Value::as_str)
                    })
                    .unwrap_or("T3 refused the WebSocket dispatch");
                return Err(AfterFrame(SessionsError::Failed {
                    runtime: "t3".into(),
                    operation: match tag {
                        "orchestration.launchThread" => "launch",
                        "orchestration.getThreadProjection" => "thread projection",
                        _ => "send",
                    }
                    .into(),
                    detail: detail.into(),
                }));
            }
            return Err(AfterFrame(refused(
                "unrecognized or refused snapshot response",
            )));
        }
    }
    Err(AfterFrame(refused("RPC response did not complete")))
}

fn read_rpc(endpoint: &url::Url, token: &str) -> Result<Snapshot, SessionsError> {
    let value = rpc(endpoint, token, "server.getConfig", json!({}), &mut || {
        Ok(())
    })
    .map_err(DispatchError::into_inner)?;
    let snapshot: Snapshot = serde_json::from_value(value)
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
    Ok(snapshot)
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
/// The model family a usage window is scoped to. At the pinned release T3
/// reports a Claude window scoped to one model as `seven_day_<name>`
/// (`apps/server/src/provider/claudeUsageLimits.ts`, `scopedWindowId`):
/// its name lowercased. Every other window applies to all models.
fn scoped_family(id: &str) -> Option<String> {
    id.strip_prefix("seven_day_")
        .filter(|name| !name.is_empty())
        .map(str::to_lowercase)
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

/// The enabled instances T3 reports, each with the home this adapter resolves
/// for it.
pub(super) fn configured(home: &Home, env: &Env) -> Result<Vec<ConfiguredInstance>, SessionsError> {
    Ok(configured_instances(
        acquire(home, env)?,
        home.machine_user_home(),
        env,
    ))
}

/// An instance with a settings entry is enabled exactly when its settings
/// say so: T3 reports an instance it failed to create as disabled in the
/// provider snapshot, and publication can repair only what it is told of.
/// The snapshot's flag decides only for a default instance (`codex`,
/// `claudeAgent`) that T3 runs without a settings entry. Homes are
/// interpreted here and only here, at T3 `v0.1.8`'s semantics.
fn configured_instances(source: Snapshot, user_home: &Path, env: &Env) -> Vec<ConfiguredInstance> {
    let mut settings = source.settings.provider_instances;
    source
        .providers
        .into_iter()
        .filter_map(|provider| {
            let instance = settings.remove(&provider.instance_id);
            let enabled = instance
                .as_ref()
                .map_or(provider.enabled, |settings| settings.enabled);
            if !enabled {
                return None;
            }
            let driver = instrukt_core::docs::AgentCli::named(&provider.driver)?;
            let runtime = if driver.name() == "claudeAgent" {
                "claude".to_owned()
            } else {
                driver.name().to_owned()
            };
            let read = Instance {
                settings: instance.as_ref(),
                user_home,
            };
            let (variable, default) = match provider.driver.as_str() {
                "codex" => (Some("CODEX_HOME"), Some(".codex")),
                "claudeAgent" => (Some("CLAUDE_CONFIG_DIR"), Some(".claude")),
                "opencode" => (Some("OPENCODE_CONFIG_DIR"), Some(".config/opencode")),
                "fx" => (Some("FX_HOME"), Some(".fx")),
                "kiro" => (Some("KIRO_HOME"), Some(".kiro")),
                "pi" => (Some("PI_CODING_AGENT_DIR"), Some(".pi/agent")),
                "cursor" => (None, Some(".cursor")),
                "grok" => (None, Some(".grok")),
                "antigravity" => (None, Some(".gemini")),
                _ => (None, None),
            };
            let native_runtime = default.map(|_| runtime.clone());
            let mut home = None;
            let mut opaque = match provider.driver.as_str() {
                "codex" if provider.runtime_paths.is_none() => {
                    read.opaque_config("homePath") || read.opaque_config("shadowHomePath")
                }
                "claudeAgent" => read.opaque_config("homePath"),
                _ => false,
            };
            let mut link_folders = Vec::new();
            let registry_agent = (runtime == "acpRegistry")
                .then(|| read.text("agentId").map(str::to_owned))
                .flatten();
            if runtime == "codex" {
                if let Some(paths) = &provider.runtime_paths {
                    home = read.path(&paths.home_path);
                    link_folders
                        .extend(paths.shadow_home_path.as_deref().and_then(|p| read.path(p)));
                } else {
                    home = read.config("homePath");
                    link_folders.extend(read.config("shadowHomePath"));
                }
            } else if provider.driver == "claudeAgent" {
                home = read.config("homePath");
            }
            if home.is_none()
                && !opaque
                && let Some(name) = variable
            {
                // v0.1.8 ignores instance CODEX_HOME when shadowHomePath is set.
                let instance_allowed = runtime != "codex" || link_folders.is_empty();
                let selected = instance_allowed
                    .then(|| {
                        read.settings
                            .and_then(|s| s.environment.iter().rev().find(|v| v.name == name))
                    })
                    .flatten();
                if let Some(value) = selected {
                    opaque = value.sensitive || value.value_redacted || !value.value.is_string();
                    if !opaque {
                        home = value.value.as_str().and_then(|v| read.path(v));
                    }
                } else {
                    // A supplied empty instance value replaces the inherited value too.
                    // Inherited Codex/Claude values are not shell-expanded by pinned T3.
                    home = env.get(name).and_then(|value| {
                        if matches!(runtime.as_str(), "codex" | "claude") {
                            let value = value.trim();
                            (!value.is_empty()).then(|| PathBuf::from(value))
                        } else {
                            read.path(value)
                        }
                    });
                }
            }
            let home_source = if opaque || default.is_none() {
                InstructionHomeSource::Unknown
            } else if home.is_some() {
                InstructionHomeSource::NativeVariable
            } else {
                InstructionHomeSource::Default
            };
            if home.is_none() && !opaque {
                home = default.map(|p| user_home.join(p));
            }
            let mut assurance = Vec::new();
            if matches!(runtime.as_str(), "claude" | "fx")
                && home_source == InstructionHomeSource::NativeVariable
            {
                assurance.push(InstructionAssurance::InstructionRelocationUnverified);
            }
            if runtime == "antigravity"
                || (runtime == "grok"
                    && read
                        .variable("GROK_HOME")
                        .as_ref()
                        .zip(home.as_ref())
                        .is_some_and(|(actual, target)| actual != target))
            {
                assurance.push(InstructionAssurance::RuntimeHomeDiffers);
            }
            // T3 resolves a relative home against its own process working
            // directory, which it does not report (`ServerConfig.cwd` is its
            // workspace): such a home cannot be known, and nothing is guessed.
            let unknown_home = home
                .iter()
                .chain(&link_folders)
                .find(|path| path.is_relative())
                .cloned();
            if unknown_home.is_some() {
                home = None;
                link_folders.clear();
            }
            Some(ConfiguredInstance {
                instance: provider.instance_id,
                runtime,
                native_runtime,
                registry_agent,
                home,
                link_folders,
                unknown_home,
                home_source,
                assurance,
            })
        })
        .collect()
}
/// One instance's settings, read for its home.
struct Instance<'a> {
    settings: Option<&'a InstanceSettings>,
    user_home: &'a Path,
}
impl Instance<'_> {
    fn text(&self, key: &str) -> Option<&str> {
        self.settings?.config.get(key)?.as_str()
    }
    fn opaque_config(&self, key: &str) -> bool {
        self.settings
            .and_then(|settings| settings.config.get(key))
            .is_some_and(|value| !value.is_string())
    }
    fn config(&self, key: &str) -> Option<PathBuf> {
        self.path(self.text(key)?)
    }
    /// The instance environment only; the server's own is not read.
    fn variable(&self, name: &str) -> Option<PathBuf> {
        let variable = self
            .settings?
            .environment
            .iter()
            .rev()
            .find(|variable| variable.name == name)?;
        if variable.sensitive || variable.value_redacted {
            return None;
        }
        self.path(variable.value.as_str()?)
    }
    /// A non-empty value, with a leading `~` expanded against the user's home.
    fn path(&self, value: &str) -> Option<PathBuf> {
        let value = value.trim();
        match value.strip_prefix('~') {
            _ if value.is_empty() => None,
            Some("") => Some(self.user_home.to_path_buf()),
            Some(rest) if rest.starts_with('/') => {
                Some(self.user_home.join(rest.trim_start_matches('/')))
            }
            _ => Some(PathBuf::from(value)),
        }
    }
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
                            model_family: scoped_family(&window.id),
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

/// Launch metadata is acquired independently of provider readiness/capacity.
pub(super) fn resolve_launch(
    home: &Home,
    env: &Env,
    input: &super::RuntimeStart,
    existing: Option<&instrukt_core::ids::SessionRef>,
) -> Result<super::RuntimeStart, SessionsError> {
    use super::{LaunchInteractionMode, LaunchRuntimeMode, ResolvedLaunchModes};
    let endpoint = server_endpoint(home, env)?;
    let mut token = Token::issue_scoped("orchestration:read", "instrukt-launch-metadata")?;
    let result = (|| {
        let mut request = input.clone();
        if request
            .native_launch
            .as_ref()
            .is_none_or(|launch| launch.project_id.is_empty())
        {
            let mut origin = endpoint.clone();
            origin
                .set_scheme("http")
                .map_err(|_| launch_error("invalid project snapshot origin"))?;
            origin.set_path("/api/projects");
            origin.set_query(None);
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(TIMEOUT))
                .build()
                .into();
            let snapshot_text = agent
                .get(origin.as_str())
                .header("Authorization", &format!("Bearer {}", token.secret))
                .call()
                .map_err(|e| launch_error(&format!("project snapshot: {e}")))?
                .body_mut()
                .read_to_string()
                .map_err(|e| launch_error(&format!("project snapshot: {e}")))?;
            let snapshot: serde_json::Value = serde_json::from_str(&snapshot_text)
                .map_err(|e| launch_error(&format!("project snapshot: {e}")))?;
            let projects = snapshot["projects"]
                .as_array()
                .ok_or_else(|| launch_error("invalid project snapshot"))?;
            let absolute =
                std::path::absolute(&request.checkout).map_err(|e| launch_error(&e.to_string()))?;
            let checkout = absolute.canonicalize().unwrap_or(absolute);
            let mut selected = None;
            for project in projects {
                let id = project["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| launch_error("invalid native project ID"))?;
                let root = project["workspaceRoot"]
                    .as_str()
                    .ok_or_else(|| launch_error("invalid native workspace root"))?;
                let deleted = project
                    .get("deletedAt")
                    .ok_or_else(|| launch_error("missing native project deletion state"))?;
                if !deleted.is_null() {
                    continue;
                }
                let absolute =
                    std::path::absolute(root).map_err(|e| launch_error(&e.to_string()))?;
                let root = absolute.canonicalize().unwrap_or(absolute);
                let depth = root.components().count();
                if checkout.starts_with(&root)
                    && selected.as_ref().is_none_or(|(_, prior)| depth > *prior)
                {
                    selected = Some((id.to_owned(), depth));
                }
            }
            let project_id = selected
                .ok_or_else(|| {
                    launch_error("checkout has no containing registered native project")
                })?
                .0;
            let launch = request.native_launch.as_mut().ok_or_else(|| {
                launch_error("native IDs must be reserved before metadata acquisition")
            })?;
            launch.project_id = project_id;
        }
        if request.launch_modes.is_none() {
            let modes = if let Some(session) = existing {
                let projection = rpc(
                    &endpoint,
                    &token.secret,
                    "orchestration.getThreadProjection",
                    json!({"threadId": session.id}),
                    &mut || Ok(()),
                )
                .map_err(super::DispatchError::into_inner)?;
                let thread = projection
                    .get("thread")
                    .ok_or_else(|| launch_error("missing legacy thread projection"))?;
                if thread["id"].as_str() != Some(session.id.as_str()) {
                    return Err(launch_error(
                        "legacy projection differs from the bound native ID",
                    ));
                }
                if thread["projectId"].as_str()
                    != request
                        .native_launch
                        .as_ref()
                        .map(|launch| launch.project_id.as_str())
                {
                    return Err(launch_error(
                        "legacy native project contradicts retained/resolved project",
                    ));
                }
                ResolvedLaunchModes {
                    runtime: serde_json::from_value(thread["runtimeMode"].clone())
                        .map_err(|e| launch_error(&format!("legacy runtime mode: {e}")))?,
                    interaction: serde_json::from_value(thread["interactionMode"].clone())
                        .map_err(|e| launch_error(&format!("legacy interaction mode: {e}")))?,
                    source: "native legacy thread projection".into(),
                }
            } else {
                let config = rpc(
                    &endpoint,
                    &token.secret,
                    "server.getConfig",
                    json!({}),
                    &mut || Ok(()),
                )
                .map_err(super::DispatchError::into_inner)?;
                let settings = &config["settings"];
                let project_id = &request
                    .native_launch
                    .as_ref()
                    .ok_or_else(|| launch_error("missing native identity"))?
                    .project_id;
                let effective = settings
                    .pointer(&format!(
                        "/projectSettingsOverrides/{}/defaultRuntimeMode",
                        project_id.replace('~', "~0").replace('/', "~1")
                    ))
                    .unwrap_or(&settings["defaultRuntimeMode"]);
                let mut mode: LaunchRuntimeMode = serde_json::from_value(effective.clone())
                    .map_err(|e| launch_error(&format!("effective runtime mode: {e}")))?;
                let mut source =
                    "native effective project runtime default; standalone default interaction"
                        .to_owned();
                let ceiling = match config.pointer("/managedPolicy/limits/maxRuntimeMode") {
                    Some(value) => serde_json::from_value(value.clone())
                        .map_err(|e| launch_error(&format!("managed runtime ceiling: {e}")))?,
                    None => LaunchRuntimeMode::FullAccess,
                };
                if mode > ceiling {
                    mode = ceiling;
                    source.push_str(
                        "; native caught project-default escalation uses managed ceiling",
                    );
                }
                let instance = &request
                    .selection
                    .as_ref()
                    .ok_or_else(|| launch_error("missing model selection"))?
                    .instance;
                let providers = config["providers"]
                    .as_array()
                    .ok_or_else(|| launch_error("missing providers"))?;
                let provider = providers
                    .iter()
                    .find(|p| p["instanceId"].as_str() == Some(instance.as_str()))
                    .ok_or_else(|| launch_error("selected native provider instance missing"))?;
                if let Some(value) = provider.get("supportedRuntimeModes") {
                    let supported: Vec<LaunchRuntimeMode> =
                        serde_json::from_value(value.clone())
                            .map_err(|e| launch_error(&format!("supported runtime modes: {e}")))?;
                    if !supported.is_empty() && !supported.contains(&mode) {
                        mode = supported
                            .into_iter()
                            .filter(|candidate| *candidate <= mode)
                            .max()
                            .unwrap_or(LaunchRuntimeMode::ApprovalRequired);
                        source.push_str("; native provider inherited-mode adaptation");
                    }
                }
                ResolvedLaunchModes {
                    runtime: mode,
                    interaction: LaunchInteractionMode::Default,
                    source,
                }
            };
            request.launch_modes = Some(modes);
        }
        Ok(request)
    })();
    let cleanup = token.revoke();
    match (result, cleanup) {
        (Ok(request), Ok(())) => Ok(request),
        (Err(error), Err(cleanup)) => {
            tracing::warn!(event="t3_launch_metadata_token_cleanup_failed",%cleanup);
            Err(error)
        }
        (Ok(request), Err(cleanup)) => {
            // A cleanup failure does not erase observed metadata; the read-only credential expires.
            tracing::warn!(event="t3_launch_metadata_token_cleanup_failed",%cleanup);
            Ok(request)
        }
        (Err(error), Ok(())) => Err(error),
    }
}

fn launch_error(cause: &str) -> SessionsError {
    SessionsError::Failed {
        runtime: "t3".into(),
        operation: "launch".into(),
        detail: cause.into(),
    }
}

pub(super) fn launch(
    home: &Home,
    env: &Env,
    payload: serde_json::Value,
) -> Result<serde_json::Value, SessionsError> {
    let endpoint = server_endpoint(home, env)?;
    let mut token = Token::issue_scoped("orchestration:operate", "instrukt-keyed-launch")?;
    let result = rpc(
        &endpoint,
        &token.secret,
        "orchestration.launchThread",
        payload,
        &mut || Ok(()),
    )
    .map_err(|error| launch_error(&error.into_inner().to_string()));
    if let Err(error) = token.revoke() {
        tracing::warn!(event="t3_launch_token_cleanup_failed", %error);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{CommandFailure, Snapshot, command_output, configured_instances};
    use crate::process::{Finished, ProcessError};
    use instrukt_core::{ids::ProviderInstanceId, interfaces::ConfiguredInstance};
    use serde_json::json;
    use std::{
        io,
        path::{Path, PathBuf},
        time::Duration,
    };

    /// A provider entry as T3 `v0.1.8` reports it (`ServerProvider`), with
    /// only the fields this adapter decodes.
    fn provider(id: &str, driver: &str, enabled: bool) -> serde_json::Value {
        json!({"instanceId":id,"driver":driver,"enabled":enabled,"installed":true,
            "status":if enabled {"ready"} else {"disabled"},"auth":{"status":"unknown"},
            "checkedAt":"2026-11-19T10:00:00.000Z","models":[]})
    }
    fn variable(name: &str, value: &str) -> serde_json::Value {
        json!({"name":name,"value":value,"sensitive":false})
    }

    /// u15: the `server.getConfig` value of a T3 `v0.1.8` server, shaped by
    /// its contracts (`ServerConfig`, `ProviderInstanceConfig`, the redaction
    /// of `redactServerSettingsForClient`), resolved to homes.
    #[test]
    fn configured_instances_carry_the_homes_the_adapter_resolves() {
        let mut managed = provider("codex-managed", "codex", true);
        managed["runtimePaths"] =
            json!({"homePath":"/srv/t3/codex","shadowHomePath":"/srv/t3/codex-auth"});
        let value = json!({
            "providers": [
                provider("codex", "codex", true),
                provider("claudeAgent", "claudeAgent", true),
                provider("pi-work", "pi", true),
                provider("opencode", "opencode", true),
                provider("kiro", "kiro", false),
                provider("acp-gemini", "acpRegistry", true),
                provider("claude-secret", "claudeAgent", true),
                provider("codex-overlay", "codex", true),
                provider("codex-overlay-env", "codex", true),
                managed,
                provider("cursor", "cursor", true),
                provider("grok", "grok", true),
                provider("antigravity", "antigravity", true),
                provider("future", "FutureHarness", true),
                provider("opencode-xdg", "opencode", true),
                provider("pi-empty", "pi", true),
                provider("unrecognised-claude", "claude", true),
            ],
            "settings": {"providerInstances": {
                "pi-work": {"driver":"pi","enabled":true,
                    "environment":[variable("PI_CODING_AGENT_DIR","~/agents/pi")]},
                "opencode": {"driver":"opencode","enabled":true,"environment":[
                    variable("XDG_CONFIG_HOME","/srv/xdg"),
                    variable("OPENCODE_CONFIG_DIR","/srv/opencode")]},
                "kiro": {"driver":"kiro","enabled":false,
                    "environment":[variable("KIRO_HOME","/srv/kiro")]},
                "acp-gemini": {"driver":"acpRegistry","config":{"agentId":"gemini"}},
                "claude-secret": {"driver":"claudeAgent","environment":[
                    {"name":"CLAUDE_CONFIG_DIR","value":"","sensitive":true,"valueRedacted":true}]},
                "codex-overlay": {"driver":"codex","config":{
                    "homePath":"/srv/codex","shadowHomePath":"/srv/codex-personal"}},
                "codex-overlay-env": {"driver":"codex",
                    "environment":[variable("CODEX_HOME","/srv/env-codex")],
                    "config":{"homePath":"","shadowHomePath":"/srv/codex-work"}},
                "codex-managed": {"driver":"codex","config":{"setupMode":"managed"}},
                "grok": {"driver":"grok","environment":[variable("HOME","/must-not-use"), variable("GROK_HOME","/srv/grok")]},
                "opencode-xdg": {"driver":"opencode","environment":[variable("XDG_CONFIG_HOME","/must-not-use")]},
                "pi-empty": {"driver":"pi","environment":[variable("PI_CODING_AGENT_DIR", "")]},
            }},
        });
        let snapshot: Snapshot = serde_json::from_value(value).expect("v0.1.8 config value");
        let instances = configured_instances(
            snapshot,
            Path::new("/home/user"),
            &instrukt_core::config::Env::from_pairs([
                ("CODEX_HOME", "/srv/inherited-codex"),
                ("CLAUDE_CONFIG_DIR", "/srv/inherited-claude"),
                ("OPENCODE_CONFIG_DIR", "/srv/inherited-opencode"),
                ("HOME", "/must-not-use"),
                ("PI_CODING_AGENT_DIR", "/srv/inherited-pi"),
            ]),
        );
        let instance = |id: &str,
                        runtime: &str,
                        registry_agent: Option<&str>,
                        home: Option<&str>,
                        link_folders: &[&str]| ConfiguredInstance {
            instance: ProviderInstanceId::parse(id).expect("instance id"),
            runtime: runtime.into(),
            native_runtime: if id == "unrecognised-claude"
                || matches!(runtime, "acpRegistry" | "FutureHarness")
            {
                None
            } else {
                Some(runtime.into())
            },
            registry_agent: registry_agent.map(str::to_owned),
            home: home.map(PathBuf::from).or_else(|| match runtime {
                "codex" => Some(PathBuf::from("/home/user/.codex")),
                "claude" if !matches!(id, "claude-secret" | "unrecognised-claude") => {
                    Some(PathBuf::from("/home/user/.claude"))
                }
                _ => None,
            }),
            link_folders: link_folders.iter().map(PathBuf::from).collect(),
            unknown_home: None,
            home_source: if matches!(id, "claude-secret" | "unrecognised-claude")
                || matches!(runtime, "acpRegistry" | "FutureHarness")
            {
                instrukt_core::interfaces::InstructionHomeSource::Unknown
            } else if id == "pi-empty" || matches!(runtime, "cursor" | "grok" | "antigravity") {
                instrukt_core::interfaces::InstructionHomeSource::Default
            } else if home.is_some() {
                instrukt_core::interfaces::InstructionHomeSource::NativeVariable
            } else {
                instrukt_core::interfaces::InstructionHomeSource::Default
            },
            assurance: if matches!(runtime, "grok" | "antigravity") {
                vec![instrukt_core::interfaces::InstructionAssurance::RuntimeHomeDiffers]
            } else if runtime == "claude" && home.is_some() {
                vec![instrukt_core::interfaces::InstructionAssurance::InstructionRelocationUnverified]
            } else {
                vec![]
            },
        };
        assert_eq!(
            instances,
            vec![
                instance("codex", "codex", None, Some("/srv/inherited-codex"), &[]),
                instance(
                    "claudeAgent",
                    "claude",
                    None,
                    Some("/srv/inherited-claude"),
                    &[]
                ),
                instance("pi-work", "pi", None, Some("/home/user/agents/pi"), &[]),
                instance("opencode", "opencode", None, Some("/srv/opencode"), &[]),
                instance("acp-gemini", "acpRegistry", Some("gemini"), None, &[]),
                instance("claude-secret", "claude", None, None, &[]),
                instance(
                    "codex-overlay",
                    "codex",
                    None,
                    Some("/srv/codex"),
                    &["/srv/codex-personal"],
                ),
                instance(
                    "codex-overlay-env",
                    "codex",
                    None,
                    Some("/srv/inherited-codex"),
                    &["/srv/codex-work"]
                ),
                instance(
                    "codex-managed",
                    "codex",
                    None,
                    Some("/srv/t3/codex"),
                    &["/srv/t3/codex-auth"],
                ),
                instance("cursor", "cursor", None, Some("/home/user/.cursor"), &[]),
                instance("grok", "grok", None, Some("/home/user/.grok"), &[]),
                instance(
                    "antigravity",
                    "antigravity",
                    None,
                    Some("/home/user/.gemini"),
                    &[]
                ),
                instance("future", "FutureHarness", None, None, &[]),
                instance(
                    "opencode-xdg",
                    "opencode",
                    None,
                    Some("/srv/inherited-opencode"),
                    &[]
                ),
                instance("pi-empty", "pi", None, Some("/home/user/.pi/agent"), &[]),
                instance("unrecognised-claude", "claude", None, None, &[]),
            ]
        );
    }

    /// T3 resolves a relative home against its own process working
    /// directory, which it does not report: such an instance's home is not
    /// known, and none of its paths is reported for publication to write.
    #[test]
    fn a_home_named_relative_to_t3s_own_working_directory_is_not_known() {
        let value = json!({
            "providers": [
                provider("codex-relative", "codex", true),
                provider("codex-dot", "codex", true),
                provider("codex-shadow", "codex", true),
            ],
            "settings": {"providerInstances": {
                "codex-relative": {"driver":"codex","config":{"homePath":"contract-codex-shared"}},
                "codex-dot": {"driver":"codex","config":{"homePath":"."}},
                "codex-shadow": {"driver":"codex","config":{
                    "homePath":"/srv/codex","shadowHomePath":"overlay"}},
            }},
        });
        let snapshot: Snapshot = serde_json::from_value(value).expect("v0.1.8 config value");
        let unknown = |id: &str, path: &str| ConfiguredInstance {
            instance: ProviderInstanceId::parse(id).expect("instance id"),
            runtime: "codex".into(),
            native_runtime: Some("codex".into()),
            registry_agent: None,
            home: None,
            link_folders: Vec::new(),
            unknown_home: Some(PathBuf::from(path)),
            home_source: instrukt_core::interfaces::InstructionHomeSource::NativeVariable,
            assurance: vec![],
        };
        assert_eq!(
            configured_instances(
                snapshot,
                Path::new("/home/user"),
                &instrukt_core::config::Env::default()
            ),
            vec![
                unknown("codex-relative", "contract-codex-shared"),
                unknown("codex-dot", "."),
                unknown("codex-shadow", "overlay"),
            ]
        );
    }

    /// T3 reports an instance it failed to create as disabled in the provider
    /// snapshot; its settings still enable it, so it is reported with its
    /// link folder and publication can repair the link. A default instance
    /// without a settings entry follows the snapshot, and a settings entry
    /// switched off is not reported.
    #[test]
    fn an_instance_t3_failed_to_create_is_still_configured_by_its_settings() {
        let mut failed = provider("codex-overlay", "codex", false);
        failed["unavailableReason"] = json!(
            "Driver 'codex' failed to create instance: Cannot create Codex shadow home entry 'AGENTS.md'"
        );
        let value = json!({
            "providers": [
                failed,
                provider("claudeAgent", "claudeAgent", false),
                provider("codex-off", "codex", true),
            ],
            "settings": {"providerInstances": {
                "codex-overlay": {"driver":"codex","enabled":true,"config":{
                    "homePath":"/srv/codex","shadowHomePath":"/srv/codex-personal"}},
                "codex-off": {"driver":"codex","enabled":false},
            }},
        });
        let snapshot: Snapshot = serde_json::from_value(value).expect("v0.1.8 config value");
        assert_eq!(
            configured_instances(
                snapshot,
                Path::new("/home/user"),
                &instrukt_core::config::Env::default()
            ),
            vec![ConfiguredInstance {
                instance: ProviderInstanceId::parse("codex-overlay").expect("instance id"),
                runtime: "codex".into(),
                native_runtime: Some("codex".into()),
                registry_agent: None,
                home: Some(PathBuf::from("/srv/codex")),
                link_folders: vec![PathBuf::from("/srv/codex-personal")],
                unknown_home: None,
                home_source: instrukt_core::interfaces::InstructionHomeSource::NativeVariable,
                assurance: vec![],
            }]
        );
    }
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
