//! Protocol- and shape-validated T3 WebSocket RPC, HTTP project snapshot, readiness,
//! and bounded token lifecycle (`Token` single-revoke + long-lived `ReadTokenLease`).
use std::collections::{BTreeMap, BTreeSet};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use instrukt_core::config::{Env, HarnessKind};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProviderInstanceId, SessionRef};
use instrukt_core::interfaces::{ConfiguredInstance, InstructionAssurance, InstructionHomeSource};
use instrukt_core::process::{Finished, Invocation, ProcessError};
use instrukt_runtime::DispatchError::{self, AfterFrame, BeforeFrame, BeforeWrite};
use instrukt_runtime::{
    ExecutionRootCause, LaunchInteractionMode, LaunchRuntimeMode, ObservedSnapshot, ProviderFacts,
    ProviderSnapshot, ReportedModel, ResolvedLaunchModes, RuntimeError, RuntimeStart, UsageWindow,
};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::json;
use tungstenite::{Message, client::IntoClientRequest};

const RPC_REQUEST_ID: &str = "instrukt-readiness";
/// Default limit on one T3 CLI launch and on one snapshot read.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Bounded single-attempt synchronous revoke cap for one-shot live reads and drop cleanup.
const REVOKE_TIMEOUT: Duration = Duration::from_secs(1);
/// The lifetime T3 gives a readiness / live-read token (`--ttl`).
const TOKEN_LIFETIME: &str = "5m";
const TOKEN_LIFETIME_SECONDS: i64 = 300;
/// How long before its expiry a token is replaced.
const RENEW_BEFORE_SECONDS: i64 = 60;

fn refused(cause: &str) -> RuntimeError {
    RuntimeError::Refused(format!("T3 readiness: {cause}"))
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    #[serde(default)]
    provider_instances: BTreeMap<ProviderInstanceId, InstanceSettings>,
}

#[derive(Deserialize)]
struct InstanceSettings {
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    #[serde(default)]
    environment: Vec<Variable>,
    #[serde(default)]
    config: serde_json::Value,
}

fn enabled_by_default() -> bool {
    true
}

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

fn command_failure(error: ProcessError) -> CommandFailure {
    match error {
        ProcessError::Spawn { .. } => CommandFailure::Launch,
        ProcessError::Timeout { .. } => CommandFailure::Timeout,
        ProcessError::Io { .. } => CommandFailure::Io,
    }
}

fn command_output(outcome: Result<Finished, ProcessError>) -> Result<String, CommandFailure> {
    match outcome {
        Ok(out) if out.success => Ok(out.stdout),
        Ok(_) => Err(CommandFailure::Exit),
        Err(error) => Err(command_failure(error)),
    }
}

fn command_within(
    args: &[&str],
    operation: &str,
    timeout: Duration,
) -> Result<String, RuntimeError> {
    match command_output(Invocation::program("t3", args).run(timeout)) {
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

fn command(args: &[&str], operation: &str) -> Result<String, RuntimeError> {
    command_within(args, operation, TIMEOUT)
}

struct Token {
    secret: String,
    session: String,
    revoked: bool,
}

impl Token {
    fn issue() -> Result<Self, RuntimeError> {
        Self::issue_scoped_within("orchestration:read", "instrukt-readiness", TIMEOUT)
    }

    fn issue_scoped(scope: &str, label: &str) -> Result<Self, RuntimeError> {
        Self::issue_scoped_within(scope, label, TIMEOUT)
    }

    fn issue_scoped_within(
        scope: &str,
        label: &str,
        timeout: Duration,
    ) -> Result<Self, RuntimeError> {
        let secret = command_within(
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
            timeout,
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
        let claims: Claims = URL_SAFE_NO_PAD
            .decode(payload)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or_else(|| {
                refused(
                    "unrecognized issued token payload; short-lived session may require manual revocation",
                )
            })?;
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

    /// Runs one synchronous revoke attempt, marking `revoked = true` BEFORE
    /// invoking the subprocess so `Drop` never attempts a second revoke.
    fn revoke_with_timeout(&mut self, timeout: Duration) -> Result<(), RuntimeError> {
        if self.revoked {
            return Ok(());
        }
        self.revoked = true;
        command_within(
            &["auth", "session", "revoke", "--", &self.session],
            "cannot revoke readiness session",
            timeout,
        )?;
        Ok(())
    }

    fn revoke(&mut self) -> Result<(), RuntimeError> {
        self.revoke_with_timeout(TIMEOUT)
    }

    fn revoke_quick(&mut self) -> Result<(), RuntimeError> {
        self.revoke_with_timeout(REVOKE_TIMEOUT)
    }
}

impl Drop for Token {
    fn drop(&mut self) {
        if !self.revoked && self.revoke_quick().is_err() {
            tracing::error!(
                event = "readiness_token_cleanup_failed",
                message = "readiness session revocation failed; its five-minute TTL remains"
            );
        }
    }
}

/// Shared `orchestration:read` session-token lease held by long-lived `T3` adapters
/// (`instrukt run`), eliminating per-poll auth subprocesses while refreshing once on HTTP 401.
#[derive(Default)]
struct LeaseSlot {
    kept: Option<(Token, Instant)>,
    closed: bool,
}

#[derive(Default)]
pub(crate) struct ReadTokenLease {
    slot: Mutex<LeaseSlot>,
}

impl ReadTokenLease {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn lock_slot(&self) -> MutexGuard<'_, LeaseSlot> {
        match self.slot.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn get_or_issue(&self, timeout: Duration) -> Result<String, RuntimeError> {
        let now = Instant::now();
        {
            let slot = self.lock_slot();
            if slot.closed {
                return Err(refused("the runner is stopping"));
            }
            if let Some((token, renew_at)) = &slot.kept
                && now < *renew_at
            {
                return Ok(token.secret.clone());
            }
        }
        let next = Token::issue_scoped_within("orchestration:read", "instrukt-live-read", timeout)?;
        let renew_at = Instant::now()
            + Duration::from_secs((TOKEN_LIFETIME_SECONDS - RENEW_BEFORE_SECONDS) as u64);
        let secret = next.secret.clone();
        let earlier = {
            let mut slot = self.lock_slot();
            if slot.closed {
                drop(slot);
                drop(next);
                return Err(refused("the runner is stopping"));
            }
            slot.kept.replace((next, renew_at))
        };
        drop(earlier);
        Ok(secret)
    }

    fn invalidate_if_matching(&self, stale: &str) {
        let stale_token = {
            let mut slot = self.lock_slot();
            if slot
                .kept
                .as_ref()
                .is_some_and(|(token, _)| token.secret == stale)
            {
                slot.kept.take()
            } else {
                None
            }
        };
        if let Some((mut token, _)) = stale_token
            && let Err(error) = token.revoke_quick()
        {
            tracing::warn!(event = "t3_live_read_token_cleanup_failed", %error);
        }
    }

    pub(crate) fn shutdown(&self) {
        let kept = {
            let mut slot = self.lock_slot();
            slot.closed = true;
            slot.kept.take()
        };
        if let Some((mut token, _)) = kept
            && let Err(error) = token.revoke_quick()
        {
            tracing::warn!(event = "t3_live_read_token_shutdown_failed", %error);
        }
    }
}

impl Drop for ReadTokenLease {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn endpoint(home: &Home, env: &Env) -> Result<url::Url, RuntimeError> {
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

fn server_endpoint(home: &Home, env: &Env) -> Result<url::Url, RuntimeError> {
    let location = crate::_t3_home::T3Home::discover(home, env);
    let bytes = std::fs::read(location.descriptor())
        .map_err(|_| refused("cannot discover the local running server"))?;
    let state: RuntimeState =
        serde_json::from_slice(&bytes).map_err(|_| refused("unrecognized server runtime state"))?;
    if state.version != 1 || state.pid == 0 || state.port == 0 {
        return Err(refused("unsupported server runtime descriptor shape"));
    }
    tracing::debug!(
        event = "t3_readiness_releases",
        server_release = ?state.server_version,
        descriptor_version = state.version,
        orchestration_protocol = 2
    );
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

static LAST: RwLock<Option<Arc<ObservedSnapshot>>> = RwLock::new(None);

pub(crate) fn last() -> Option<Arc<ObservedSnapshot>> {
    match LAST.read() {
        Ok(last) => last.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn publish(observed: &Arc<ObservedSnapshot>) {
    let next = Some(Arc::clone(observed));
    match LAST.write() {
        Ok(mut last) => *last = next,
        Err(poisoned) => *poisoned.into_inner() = next,
    }
}

pub(crate) struct Readiness {
    env: Env,
    slot: Arc<Mutex<Slot>>,
}

#[derive(Default)]
struct Slot {
    kept: Option<(Token, Timestamp)>,
    claimed: bool,
}

fn lock(slot: &Mutex<Slot>) -> MutexGuard<'_, Slot> {
    match slot.lock() {
        Ok(slot) => slot,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl Readiness {
    pub(crate) fn new(env: Env) -> Self {
        Self {
            env,
            slot: Arc::default(),
        }
    }

    fn token(&mut self, now: Timestamp) -> Result<String, RuntimeError> {
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
                drop(next);
                return Err(refused("the runner is stopping"));
            }
            slot.kept.replace((next, renew_at))
        };
        drop(earlier);
        Ok(secret)
    }
}

impl Drop for Readiness {
    fn drop(&mut self) {
        let kept = {
            let mut slot = lock(&self.slot);
            slot.claimed = true;
            slot.kept.take()
        };
        drop(kept);
    }
}

impl instrukt_runtime::Readiness for Readiness {
    fn refresh(
        &mut self,
        home: &Home,
        clock: &dyn Fn() -> Timestamp,
    ) -> Result<Arc<ObservedSnapshot>, RuntimeError> {
        let endpoint = endpoint(home, &self.env)?;
        let secret = self.token(clock())?;
        let source = read_rpc(&endpoint, &secret)?;
        let observed = Arc::new(ObservedSnapshot {
            snapshot: translate(source),
            at: clock(),
        });
        publish(&observed);
        Ok(observed)
    }

    fn stopper(&self) -> Box<dyn Fn() + Send + Sync> {
        let slot = Arc::clone(&self.slot);
        Box::new(move || {
            let kept = {
                let mut slot = lock(&slot);
                slot.claimed = true;
                slot.kept.take()
            };
            let Some((mut token, _)) = kept else {
                return;
            };
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

fn acquire(home: &Home, env: &Env) -> Result<Snapshot, RuntimeError> {
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

enum RpcFailure {
    Unauthorized(RuntimeError),
    Dispatch(DispatchError),
}

impl RpcFailure {
    fn into_dispatch(self) -> DispatchError {
        match self {
            Self::Unauthorized(err) => BeforeFrame(err),
            Self::Dispatch(err) => err,
        }
    }
}

pub(crate) fn dispatch(
    home: &Home,
    env: &Env,
    label: &str,
    operation: &str,
    payload: serde_json::Value,
    before_write: &mut (dyn FnMut() -> Result<(), RuntimeError> + Send),
) -> Result<(), DispatchError> {
    let endpoint = server_endpoint(home, env).map_err(BeforeFrame)?;
    let mut token = Token::issue_scoped("orchestration:operate", label).map_err(BeforeFrame)?;
    let outcome = rpc_within(
        &endpoint,
        &token.secret,
        "orchestration.dispatchCommand",
        operation,
        payload,
        TIMEOUT,
        before_write,
    )
    .map_err(RpcFailure::into_dispatch)
    .and_then(|value| {
        value["sequence"]
            .as_u64()
            .map(|_| ())
            .ok_or_else(|| AfterFrame(refused("invalid dispatch receipt")))
    });
    if let Err(error) = token.revoke_quick() {
        tracing::warn!(event = "t3_dispatch_token_cleanup_failed", %error);
    }
    outcome
}

/// Calls `orchestration.getThreadProjection` over a held `ReadTokenLease` (in long-lived mode)
/// or a short-lived one-shot `orchestration:read` token bounded to `timeout`.
pub(crate) fn get_thread_projection(
    home: &Home,
    env: &Env,
    lease: Option<&ReadTokenLease>,
    thread_id: &str,
    timeout: Duration,
) -> Result<serde_json::Value, RuntimeError> {
    let endpoint = server_endpoint(home, env)?;
    let payload = json!({ "threadId": thread_id });
    if let Some(lease) = lease {
        let token = lease.get_or_issue(timeout)?;
        match rpc_within(
            &endpoint,
            &token,
            "orchestration.getThreadProjection",
            "thread projection",
            payload.clone(),
            timeout,
            &mut || Ok(()),
        ) {
            Ok(value) => return Ok(value),
            Err(RpcFailure::Unauthorized(_)) => {
                lease.invalidate_if_matching(&token);
                let fresh = lease.get_or_issue(timeout)?;
                return rpc_within(
                    &endpoint,
                    &fresh,
                    "orchestration.getThreadProjection",
                    "thread projection",
                    payload,
                    timeout,
                    &mut || Ok(()),
                )
                .map_err(|err| err.into_dispatch().into_inner());
            }
            Err(err) => return Err(err.into_dispatch().into_inner()),
        }
    }

    // One-shot CLI path: reserve <=1s for synchronous revoke, clamp issue+RPC to remaining budget.
    let deadline = Instant::now() + timeout;
    let revoke_reserve = REVOKE_TIMEOUT.min(timeout / 3);
    let rpc_budget = timeout
        .saturating_sub(revoke_reserve)
        .max(Duration::from_millis(250));
    let mut token =
        Token::issue_scoped_within("orchestration:read", "instrukt-thread-read", rpc_budget)?;
    let remaining_for_rpc = deadline
        .saturating_duration_since(Instant::now())
        .saturating_sub(revoke_reserve)
        .max(Duration::from_millis(200));
    let result = rpc_within(
        &endpoint,
        &token.secret,
        "orchestration.getThreadProjection",
        "thread projection",
        payload,
        remaining_for_rpc,
        &mut || Ok(()),
    )
    .map_err(|err| err.into_dispatch().into_inner());
    let revoke_bound = deadline
        .saturating_duration_since(Instant::now())
        .min(REVOKE_TIMEOUT)
        .max(Duration::from_millis(100));
    if let Err(error) = token.revoke_with_timeout(revoke_bound) {
        tracing::warn!(event = "t3_thread_read_token_cleanup_failed", %error);
    }
    result
}

fn decode_rpc_exit_error(
    tag: &str,
    operation: &str,
    payload: &serde_json::Value,
    exit: &serde_json::Value,
) -> RuntimeError {
    if matches!(
        tag,
        "orchestration.dispatchCommand"
            | "orchestration.launchThread"
            | "orchestration.getThreadProjection"
    ) {
        let error_tag = exit
            .pointer("/cause/0/error/_tag")
            .and_then(serde_json::Value::as_str);
        let detail = exit
            .pointer("/cause/0/error/detail")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                exit.pointer("/cause/0/error/message")
                    .and_then(serde_json::Value::as_str)
            })
            .unwrap_or("T3 refused the WebSocket dispatch");
        if error_tag == Some("ProjectionStoreThreadNotFoundError") {
            let thread_id = exit
                .pointer("/cause/0/error/threadId")
                .and_then(serde_json::Value::as_str)
                .or_else(|| payload.get("threadId").and_then(serde_json::Value::as_str))
                .unwrap_or_default()
                .to_owned();
            return RuntimeError::ThreadNotFound { thread_id };
        }
        if error_tag == Some("OrchestratorCommandPreviouslyRejectedError") {
            let command_id = exit
                .pointer("/cause/0/error/commandId")
                .and_then(serde_json::Value::as_str)
                .or_else(|| payload.get("commandId").and_then(serde_json::Value::as_str))
                .unwrap_or_default()
                .to_owned();
            return RuntimeError::CommandPreviouslyRejected {
                command_id,
                detail: detail.into(),
            };
        }
        return RuntimeError::Failed {
            runtime: "t3".into(),
            operation: operation.into(),
            detail: detail.into(),
        };
    }
    refused("unrecognized or refused snapshot response")
}

fn rpc_within(
    endpoint: &url::Url,
    token: &str,
    tag: &str,
    operation: &str,
    payload: serde_json::Value,
    timeout: Duration,
    before_write: &mut (dyn FnMut() -> Result<(), RuntimeError> + Send),
) -> Result<serde_json::Value, RpcFailure> {
    let addresses = endpoint.socket_addrs(|| None).map_err(|_| {
        RpcFailure::Dispatch(BeforeFrame(refused("cannot resolve local server address")))
    })?;
    let deadline = Instant::now() + timeout;
    let mut connected = None;
    for address in addresses {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if let Ok(stream) = TcpStream::connect_timeout(&address, remaining) {
            connected = Some(stream);
            break;
        }
    }
    let stream = connected.ok_or_else(|| {
        RpcFailure::Dispatch(BeforeFrame(refused("cannot connect to local server")))
    })?;
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .map_err(|_| {
            RpcFailure::Dispatch(BeforeFrame(refused("cannot bound readiness transport")))
        })?;
    let mut request = endpoint.as_str().into_client_request().map_err(|_| {
        RpcFailure::Dispatch(BeforeFrame(refused("invalid readiness upgrade request")))
    })?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {token}").parse().map_err(|_| {
            RpcFailure::Dispatch(BeforeFrame(refused("invalid issued bearer token")))
        })?,
    );
    let (mut socket, _) = tungstenite::client(request, FrameStream { stream, written: 0 })
        .map_err(|err| {
            let unauthorized = matches!(
                err,
                tungstenite::handshake::HandshakeError::Failure(tungstenite::Error::Http(ref resp))
                    if resp.status().as_u16() == 401
            );
            let error = refused("WebSocket upgrade refused or protocol version unsupported");
            if unauthorized {
                RpcFailure::Unauthorized(error)
            } else {
                RpcFailure::Dispatch(BeforeFrame(error))
            }
        })?;
    socket.set_config(|config| {
        config.max_message_size = Some(8 * 1024 * 1024);
        config.max_frame_size = Some(8 * 1024 * 1024);
    });
    let frame = json!({
        "_tag": "Request",
        "id": RPC_REQUEST_ID,
        "tag": tag,
        "payload": &payload,
        "headers": []
    })
    .to_string();
    before_write().map_err(|err| RpcFailure::Dispatch(BeforeWrite(err)))?;
    socket.get_mut().written = 0;
    if socket.send(Message::Text(frame.into())).is_err() {
        let error = refused("RPC request write failed");
        return Err(RpcFailure::Dispatch(if socket.get_mut().written != 0 {
            AfterFrame(error)
        } else {
            BeforeFrame(error)
        }));
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
            .map_err(|_| RpcFailure::Dispatch(AfterFrame(refused("cannot bound snapshot read"))))?;
        let message = socket.read().map_err(|_| {
            RpcFailure::Dispatch(AfterFrame(refused("snapshot response failed or timed out")))
        })?;
        let Message::Text(text) = message else {
            if matches!(message, Message::Ping(_) | Message::Pong(_)) {
                continue;
            }
            return Err(RpcFailure::Dispatch(AfterFrame(refused(
                "unexpected snapshot frame",
            ))));
        };
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|_| RpcFailure::Dispatch(AfterFrame(refused("unrecognized RPC response"))))?;
        let frames = if let Some(frames) = value.as_array() {
            frames.clone()
        } else {
            vec![value]
        };
        for frame in frames {
            if frame["_tag"] == "Ping" {
                socket
                    .send(Message::Text(json!({"_tag":"Pong"}).to_string().into()))
                    .map_err(|_| {
                        RpcFailure::Dispatch(AfterFrame(refused("RPC keepalive failed")))
                    })?;
                continue;
            }
            if frame["_tag"] != "Exit" || frame["requestId"] != RPC_REQUEST_ID {
                return Err(RpcFailure::Dispatch(AfterFrame(refused(
                    "unrecognized RPC response",
                ))));
            }
            let _ = socket.close(None);
            if frame["exit"]["_tag"] == "Success" {
                return Ok(frame["exit"]["value"].clone());
            }
            return Err(RpcFailure::Dispatch(AfterFrame(decode_rpc_exit_error(
                tag,
                operation,
                &payload,
                &frame["exit"],
            ))));
        }
    }
    Err(RpcFailure::Dispatch(AfterFrame(refused(
        "RPC response did not complete",
    ))))
}

fn read_rpc(endpoint: &url::Url, token: &str) -> Result<Snapshot, RuntimeError> {
    let value = rpc_within(
        endpoint,
        token,
        "server.getConfig",
        "readiness",
        json!({}),
        TIMEOUT,
        &mut || Ok(()),
    )
    .map_err(|err| err.into_dispatch().into_inner())?;
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
    if provider.driver == "claudeAgent"
        && matches!(provider.status, Status::Warning)
        && matches!(provider.auth.status, AuthStatus::Unknown)
    {
        return Some("Claude authentication probe failed or incomplete".into());
    }
    None
}

pub(crate) fn snapshot(home: &Home, env: &Env) -> Result<ProviderSnapshot, RuntimeError> {
    Ok(translate(acquire(home, env)?))
}

pub(crate) fn configured(home: &Home, env: &Env) -> Result<Vec<ConfiguredInstance>, RuntimeError> {
    Ok(configured_instances(
        acquire(home, env)?,
        home.machine_user_home(),
        env,
    ))
}

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

pub(crate) struct LiveProject {
    pub id: String,
    pub root: PathBuf,
}

fn root_unavailable(checkout: &Path, cause: ExecutionRootCause) -> RuntimeError {
    RuntimeError::ExecutionRootUnavailable {
        checkout: checkout.to_path_buf(),
        cause,
    }
}

fn canonical_root(path: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(path).ok()?;
    Some(absolute.canonicalize().unwrap_or(absolute))
}

pub(crate) fn fetch_live_projects(
    endpoint: &url::Url,
    token: &str,
) -> Result<Vec<LiveProject>, ExecutionRootCause> {
    let mut origin = endpoint.clone();
    origin
        .set_scheme("http")
        .map_err(|_| ExecutionRootCause::MalformedMetadata)?;
    origin.set_path("/api/projects");
    origin.set_query(None);
    let client = instrukt_outside::http::build_client(&instrukt_outside::http::ClientConfig {
        request_timeout: TIMEOUT,
        follow_redirects: false,
        ..Default::default()
    })
    .map_err(|error| {
        tracing::warn!(event = "t3_project_snapshot_failed", %error);
        ExecutionRootCause::Unreachable
    })?;
    let url = origin.as_str().to_owned();
    let bearer = format!("Bearer {token}");
    let text = instrukt_outside::runtime::block_on(async {
        let response = client
            .get(&url)
            .header(reqwest::header::AUTHORIZATION, bearer.as_str())
            .send()
            .await
            .map_err(|error| error.without_url().to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("HTTP {status}"));
        }
        response
            .text()
            .await
            .map_err(|error| error.without_url().to_string())
    })
    .map_err(|error| {
        tracing::warn!(event = "t3_project_snapshot_failed", %error);
        ExecutionRootCause::Unreachable
    })?
    .map_err(|error| {
        tracing::warn!(event = "t3_project_snapshot_failed", %error);
        ExecutionRootCause::Unreachable
    })?;
    live_projects(&text)
}

fn live_projects(text: &str) -> Result<Vec<LiveProject>, ExecutionRootCause> {
    let malformed = ExecutionRootCause::MalformedMetadata;
    let snapshot: serde_json::Value = serde_json::from_str(text).map_err(|_| malformed)?;
    let mut live = Vec::new();
    for project in snapshot["projects"].as_array().ok_or(malformed)? {
        let id = project["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or(malformed)?;
        let root = project["workspaceRoot"].as_str().ok_or(malformed)?;
        let deleted = project.get("deletedAt").ok_or(malformed)?;
        if deleted.is_null() {
            live.push(LiveProject {
                id: id.to_owned(),
                root: root.into(),
            });
        }
    }
    Ok(live)
}

pub(crate) fn exact_project_id_in(
    projects: &[LiveProject],
    checkout: &Path,
) -> Result<Option<String>, RuntimeError> {
    let wanted = canonical_root(checkout)
        .ok_or_else(|| root_unavailable(checkout, ExecutionRootCause::Missing))?;
    let mut found: Option<&LiveProject> = None;
    for project in projects {
        if canonical_root(&project.root).as_ref() == Some(&wanted) {
            if found.is_some() {
                return Err(root_unavailable(
                    checkout,
                    ExecutionRootCause::DuplicateRoots,
                ));
            }
            found = Some(project);
        }
    }
    Ok(found.map(|project| project.id.clone()))
}

pub(crate) fn exact_project_id(
    home: &Home,
    env: &Env,
    checkout: &Path,
) -> Result<Option<String>, RuntimeError> {
    let endpoint = server_endpoint(home, env).map_err(|_| {
        let cause = if crate::_t3_home::T3Home::discover(home, env)
            .descriptor()
            .exists()
        {
            ExecutionRootCause::MalformedMetadata
        } else {
            ExecutionRootCause::Unreachable
        };
        root_unavailable(checkout, cause)
    })?;
    let mut token = Token::issue_scoped("orchestration:read", "instrukt-project-snapshot")
        .map_err(|_| root_unavailable(checkout, ExecutionRootCause::Unreachable))?;
    let result = fetch_live_projects(&endpoint, &token.secret)
        .map_err(|cause| root_unavailable(checkout, cause))
        .and_then(|projects| exact_project_id_in(&projects, checkout));
    if let Err(cleanup) = token.revoke_quick() {
        tracing::warn!(event = "t3_project_snapshot_token_cleanup_failed", %cleanup);
    }
    result
}

pub(crate) fn resolve_launch(
    home: &Home,
    env: &Env,
    input: &RuntimeStart,
    existing: Option<&SessionRef>,
) -> Result<RuntimeStart, RuntimeError> {
    let endpoint = server_endpoint(home, env)?;
    let mut token = Token::issue_scoped("orchestration:read", "instrukt-launch-metadata")?;
    let result = (|| {
        let mut request = input.clone();
        if request
            .native_launch
            .as_ref()
            .is_none_or(|launch| launch.project_id.is_empty())
        {
            let projects = fetch_live_projects(&endpoint, &token.secret)
                .map_err(|cause| root_unavailable(&request.checkout, cause))?;
            let project_id = exact_project_id_in(&projects, &request.checkout)?
                .ok_or_else(|| root_unavailable(&request.checkout, ExecutionRootCause::Missing))?;
            let launch = request.native_launch.as_mut().ok_or_else(|| {
                launch_error("native IDs must be reserved before metadata acquisition")
            })?;
            launch.project_id = project_id;
        }
        if request.launch_modes.is_none() {
            let modes = if let Some(session) = existing {
                let projection = rpc_within(
                    &endpoint,
                    &token.secret,
                    "orchestration.getThreadProjection",
                    "thread projection",
                    json!({"threadId": session.id}),
                    TIMEOUT,
                    &mut || Ok(()),
                )
                .map_err(|err| err.into_dispatch().into_inner())?;
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
                let config = rpc_within(
                    &endpoint,
                    &token.secret,
                    "server.getConfig",
                    "readiness",
                    json!({}),
                    TIMEOUT,
                    &mut || Ok(()),
                )
                .map_err(|err| err.into_dispatch().into_inner())?;
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
                let selection = request
                    .selection
                    .as_ref()
                    .ok_or_else(|| launch_error("missing model selection"))?;
                let providers = config["providers"]
                    .as_array()
                    .ok_or_else(|| launch_error("missing providers"))?;
                let provider = providers
                    .iter()
                    .find(|p| p["instanceId"].as_str() == Some(selection.instance.as_str()))
                    .ok_or_else(|| launch_error("selected native provider instance missing"))?;
                crate::_t3_options::admit(provider, selection)?;
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
    if let Err(cleanup) = token.revoke_quick() {
        tracing::warn!(event = "t3_launch_metadata_token_cleanup_failed", %cleanup);
    }
    result
}

fn launch_error(cause: &str) -> RuntimeError {
    RuntimeError::Failed {
        runtime: "t3".into(),
        operation: "launch".into(),
        detail: cause.into(),
    }
}

pub(crate) fn launch(
    home: &Home,
    env: &Env,
    payload: serde_json::Value,
) -> Result<serde_json::Value, RuntimeError> {
    let endpoint = server_endpoint(home, env)?;
    let mut token = Token::issue_scoped("orchestration:operate", "instrukt-keyed-launch")?;
    let result = rpc_within(
        &endpoint,
        &token.secret,
        "orchestration.launchThread",
        "launch",
        payload,
        TIMEOUT,
        &mut || Ok(()),
    )
    .map_err(|error| launch_error(&error.into_dispatch().into_inner().to_string()));
    if let Err(error) = token.revoke_quick() {
        tracing::warn!(event = "t3_launch_token_cleanup_failed", %error);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{
        CommandFailure, Snapshot, Token, command_output, configured_instances,
        decode_rpc_exit_error,
    };
    use instrukt_core::ids::ProviderInstanceId;
    use instrukt_core::interfaces::ConfiguredInstance;
    use instrukt_core::process::{Finished, ProcessError};
    use instrukt_runtime::{ExecutionRootCause, RuntimeError};
    use serde_json::json;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn provider(id: &str, driver: &str, enabled: bool) -> serde_json::Value {
        json!({"instanceId":id,"driver":driver,"enabled":enabled,"installed":true,
            "status":if enabled {"ready"} else {"disabled"},"auth":{"status":"unknown"},
            "checkedAt":"2026-11-19T10:00:00.000Z","models":[]})
    }

    fn variable(name: &str, value: &str) -> serde_json::Value {
        json!({"name":name,"value":value,"sensitive":false})
    }

    #[test]
    fn decodes_typed_t3_rpc_errors_from_cause_tag() {
        let not_found = decode_rpc_exit_error(
            "orchestration.getThreadProjection",
            "thread projection",
            &json!({"threadId": "thr-1"}),
            &json!({
                "_tag": "Failure",
                "cause": [{"_tag": "Fail", "error": {
                    "_tag": "ProjectionStoreThreadNotFoundError",
                    "threadId": "thr-1",
                    "message": "Thread not found"
                }}]
            }),
        );
        assert!(matches!(
            not_found,
            RuntimeError::ThreadNotFound { ref thread_id } if thread_id == "thr-1"
        ));

        let rejected = decode_rpc_exit_error(
            "orchestration.dispatchCommand",
            "send",
            &json!({"commandId": "cmd-9"}),
            &json!({
                "_tag": "Failure",
                "cause": [{"_tag": "Fail", "error": {
                    "_tag": "OrchestratorCommandPreviouslyRejectedError",
                    "commandId": "cmd-9",
                    "detail": "previously rejected"
                }}]
            }),
        );
        assert!(matches!(
            rejected,
            RuntimeError::CommandPreviouslyRejected { ref command_id, ref detail }
                if command_id == "cmd-9" && detail == "previously rejected"
        ));
    }

    #[test]
    fn token_marks_revoked_before_running_revoke_preventing_double_revoke_on_drop() {
        let mut token = Token {
            secret: "payload.sig".into(),
            session: "sid-1".into(),
            revoked: false,
        };
        // Even if the revoke command fails, `revoked` is already marked true so Drop won't retry.
        let _ = token.revoke_with_timeout(Duration::from_millis(1));
        assert!(token.revoked);
    }

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
        let snapshot: Snapshot = serde_json::from_value(value).unwrap();
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
            instance: ProviderInstanceId::parse(id).unwrap(),
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
        let snapshot: Snapshot = serde_json::from_value(value).unwrap();
        let unknown = |id: &str, path: &str| ConfiguredInstance {
            instance: ProviderInstanceId::parse(id).unwrap(),
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
        let snapshot: Snapshot = serde_json::from_value(value).unwrap();
        assert_eq!(
            configured_instances(
                snapshot,
                Path::new("/home/user"),
                &instrukt_core::config::Env::default()
            ),
            vec![ConfiguredInstance {
                instance: ProviderInstanceId::parse("codex-overlay").unwrap(),
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
    fn command_failure_categories_preserve_safe_classification() {
        assert_eq!(
            command_output(Err(ProcessError::Spawn {
                program: "t3".into(),
                source: io::Error::new(io::ErrorKind::NotFound, "private detail"),
            })),
            Err(CommandFailure::Launch)
        );
        assert_eq!(
            command_output(Err(ProcessError::Timeout {
                program: "t3".into(),
                timeout: Duration::from_secs(10),
            })),
            Err(CommandFailure::Timeout)
        );
        assert_eq!(
            command_output(Err(ProcessError::Io {
                program: "t3".into(),
                source: io::Error::other("private detail"),
            })),
            Err(CommandFailure::Io)
        );
        assert_eq!(
            command_output(Ok(Finished {
                success: false,
                stdout: "private stdout".into(),
                stderr: "private stderr".into(),
            })),
            Err(CommandFailure::Exit)
        );
    }

    fn snapshot_of(roots: &[(&str, &Path, bool)]) -> Vec<super::LiveProject> {
        let projects: Vec<_> = roots
            .iter()
            .map(|(id, root, deleted)| {
                json!({"id": id, "workspaceRoot": root, "deletedAt": if *deleted { json!("2026-11-01T00:00:00Z") } else { json!(null) }})
            })
            .collect();
        super::live_projects(&json!({ "projects": projects }).to_string()).unwrap()
    }

    #[test]
    fn exact_project_root_matching_rules() {
        let root = std::env::temp_dir().join(format!(
            "instrukt-runtime-t3-root-test-{}",
            std::process::id()
        ));
        let home = root.join("home");
        let company = home.join(".instrukt/companies/acme");
        instrukt_core::fsx::create_dir_all(&company).unwrap();
        let projects = snapshot_of(&[("home", &home, false), ("acme", &company, false)]);
        assert_eq!(
            super::exact_project_id_in(&projects, &company).unwrap(),
            Some("acme".to_owned())
        );
        let ancestor_only = snapshot_of(&[("home", &home, false)]);
        assert_eq!(
            super::exact_project_id_in(&ancestor_only, &company).unwrap(),
            None
        );
        let dup = snapshot_of(&[("one", &company, false), ("two", &company, false)]);
        assert!(matches!(
            super::exact_project_id_in(&dup, &company),
            Err(RuntimeError::ExecutionRootUnavailable {
                cause: ExecutionRootCause::DuplicateRoots,
                ..
            })
        ));
        let deleted = snapshot_of(&[("gone", &company, true)]);
        assert_eq!(
            super::exact_project_id_in(&deleted, &company).unwrap(),
            None
        );
        let _ = instrukt_core::fsx::remove_dir_all(&root);
    }
}
