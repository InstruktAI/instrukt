//! T3 0.1.7 orchestration-v2 read protocol, pinned to upstream 6417151c09.
use super::{
    _store::{self, Conversation},
    Extraction, HistoryError,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use instrukt_core::{config::Env, fsx::LockGuard, home::Home};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    net::TcpStream,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tungstenite::{Message, WebSocket, client::IntoClientRequest};

const TIMEOUT: Duration = Duration::from_secs(30);
const VERSION: &str = "0.1.7";

fn failed(detail: &str) -> HistoryError {
    HistoryError::Source(format!("T3 history: {detail}"))
}
fn command(args: &[&str]) -> Result<String, HistoryError> {
    // T3 owns local credential issuance and offers no Rust binding.
    let out = crate::process::Invocation::program("t3", args)
        .run(TIMEOUT)
        .map_err(|_| failed("host command failed or timed out"))?;
    if !out.success {
        return Err(failed(
            "host command refused; check T3 availability and read access",
        ));
    }
    Ok(out.stdout)
}

struct Client {
    socket: WebSocket<TcpStream>,
    session: String,
    source: String,
    queued: VecDeque<Value>,
}
impl Client {
    fn open(home: &Home, env: &Env) -> Result<Self, HistoryError> {
        if command(&["--version"])?.trim() != format!("t3 v{VERSION}") {
            return Err(failed("unsupported CLI version"));
        }
        let explicit = env.get("T3CODE_HOME").filter(|s| !s.is_empty());
        let base = explicit
            .map(|s| {
                if let Some(rest) = s.strip_prefix("~/") {
                    home.user_home().join(rest)
                } else {
                    std::path::PathBuf::from(s)
                }
            })
            .unwrap_or_else(|| home.user_home().join(".t3"));
        let band = if explicit.is_none()
            && env
                .get("VITE_DEV_SERVER_URL")
                .is_some_and(|s| !s.is_empty())
        {
            "dev"
        } else {
            "userdata"
        };
        let requested = base.join(band).join("server-runtime.json");
        let file = requested
            .canonicalize()
            .map_err(|source| HistoryError::Io {
                path: requested,
                source,
            })?;
        let data = std::fs::read(&file).map_err(|source| HistoryError::Io {
            path: file.clone(),
            source,
        })?;
        let state: Value = serde_json::from_slice(&data)?;
        if state["version"] != 1 || state["serverVersion"] != VERSION {
            return Err(failed("unsupported server runtime shape/version"));
        }
        let mut endpoint = url::Url::parse(
            state["origin"]
                .as_str()
                .ok_or_else(|| failed("missing server address"))?,
        )
        .map_err(|_| failed("invalid server address"))?;
        if endpoint.scheme() != "http"
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.path() != "/"
            || endpoint.port_or_known_default().map(u64::from) != state["port"].as_u64()
        {
            return Err(failed("invalid local server address"));
        }
        // Only the local runtime source is extracted; linked computers answer their own store queries.
        if !matches!(
            endpoint.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
        ) {
            return Err(failed("source address must be loopback"));
        }
        endpoint
            .set_scheme("ws")
            .map_err(|_| failed("invalid transport"))?;
        endpoint.set_path("/ws");
        endpoint.set_query(Some("orchestrationProtocol=2"));
        let token = command(&[
            "auth",
            "session",
            "issue",
            "--scope",
            "orchestration:read",
            "--ttl",
            "24h",
            "--token-only",
            "--label",
            "instrukt-history",
        ])?;
        let token = token.trim();
        let claims = token
            .split_once('.')
            .and_then(|(p, _)| URL_SAFE_NO_PAD.decode(p).ok())
            .and_then(|p| serde_json::from_slice::<Value>(&p).ok())
            .ok_or_else(|| failed("invalid token envelope; issued token may require revocation"))?;
        let session = claims["sid"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| failed("token lacks revocable session id"))?
            .to_owned();
        if !matches!(claims["v"].as_u64(), Some(1 | 2)) || claims["kind"] != "session" {
            revoke(&session)?;
            return Err(failed("unsupported issued token shape"));
        }
        let connected = (|| {
            let addresses = endpoint
                .socket_addrs(|| None)
                .map_err(|_| failed("cannot resolve server"))?;
            let deadline = Instant::now() + TIMEOUT;
            let mut connected = None;
            for address in addresses {
                if let Ok(stream) = TcpStream::connect_timeout(
                    &address,
                    deadline
                        .saturating_duration_since(Instant::now())
                        .max(Duration::from_millis(1)),
                ) {
                    connected = Some(stream);
                    break;
                }
            }
            let stream = connected.ok_or_else(|| failed("cannot connect to server"))?;
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
                .map_err(|_| failed("cannot bound transport"))?;
            let mut request = endpoint
                .as_str()
                .into_client_request()
                .map_err(|_| failed("invalid upgrade request"))?;
            request.headers_mut().insert(
                "Authorization",
                format!("Bearer {token}")
                    .parse()
                    .map_err(|_| failed("invalid authorization"))?,
            );
            let (mut socket, _) = tungstenite::client(request, stream)
                .map_err(|_| failed("read-scoped WebSocket upgrade refused"))?;
            socket.set_config(|c| {
                c.max_message_size = Some(64 * 1024 * 1024);
                c.max_frame_size = Some(64 * 1024 * 1024);
            });
            Ok(socket)
        })();
        match connected {
            Ok(socket) => Ok(Self {
                socket,
                session,
                source: format!(
                    "t3:{}",
                    hex::encode(Sha256::digest(file.to_string_lossy().as_bytes()))
                ),
                queued: VecDeque::new(),
            }),
            Err(error) => {
                revoke(&session)?;
                Err(error)
            }
        }
    }
    fn send(&mut self, value: Value) -> Result<(), HistoryError> {
        self.socket
            .send(Message::Text(value.to_string().into()))
            .map_err(|_| failed("request send failed"))
    }
    fn finish(&mut self) -> Result<(), HistoryError> {
        revoke(&self.session)?;
        self.session.clear();
        Ok(())
    }
    fn request(&mut self, id: &str, tag: &str, payload: Value) -> Result<(), HistoryError> {
        self.send(json!({"_tag":"Request","id":id,"tag":tag,"payload":payload,"headers":[]}))
    }
    fn frames(&mut self) -> Result<Vec<Value>, HistoryError> {
        loop {
            let message = self.socket.read().map_err(|error| match error {
                tungstenite::Error::Io(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    HistoryError::StreamIdle
                }
                _ => failed("source read failed"),
            })?;
            match message {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text)?;
                    return Ok(value.as_array().cloned().unwrap_or_else(|| vec![value]));
                }
                Message::Ping(bytes) => self
                    .socket
                    .send(Message::Pong(bytes))
                    .map_err(|_| failed("keepalive failed"))?,
                Message::Pong(_) => {}
                _ => return Err(failed("server closed the source stream")),
            }
        }
    }
    fn receive(&mut self) -> Result<(), HistoryError> {
        for frame in self.frames()? {
            match frame["_tag"].as_str() {
                Some("Ping") => self.send(json!({"_tag":"Pong"}))?,
                Some("Chunk") => {
                    let values = frame["values"]
                        .as_array()
                        .ok_or_else(|| failed("invalid stream chunk"))?;
                    for value in values {
                        self.queued.push_back(value.clone());
                    }
                    self.send(json!({"_tag":"Ack","requestId":frame["requestId"]}))?;
                }
                Some("Exit") if frame["exit"]["_tag"] == "Success" => {
                    self.queued
                        .push_back(json!({"rpcResult":frame["exit"]["value"]}));
                }
                _ => return Err(failed("unrecognized or refused source RPC")),
            }
        }
        Ok(())
    }
    fn next(&mut self) -> Result<Value, HistoryError> {
        loop {
            if let Some(value) = self.queued.pop_front() {
                return Ok(value);
            }
            self.receive()?;
        }
    }
    fn call(&mut self, tag: &str, payload: Value) -> Result<Value, HistoryError> {
        self.request("read", tag, payload)?;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(index) = self
                .queued
                .iter()
                .position(|v| v.get("rpcResult").is_some())
            {
                return self
                    .queued
                    .remove(index)
                    .map(|v| v["rpcResult"].clone())
                    .ok_or_else(|| failed("missing response"));
            }
            if Instant::now() >= deadline {
                return Err(failed("source response timed out"));
            }
            match self.receive() {
                Err(HistoryError::StreamIdle) => continue,
                result => result?,
            };
        }
    }
}
fn revoke(session: &str) -> Result<(), HistoryError> {
    command(&["auth", "session", "revoke", "--", session]).map(|_| ())
}
impl Drop for Client {
    fn drop(&mut self) {
        if let Err(error) = self.socket.close(None) {
            tracing::debug!(event="history_source_close",error=%error);
        }
        if !self.session.is_empty()
            && let Err(error) = revoke(&self.session)
        {
            tracing::error!(event="history_token_revoke_failed",error=%error);
        }
    }
}

pub(super) fn extract(home: &Home, env: &Env, live: bool) -> Result<Extraction, HistoryError> {
    extract_until(home, env, live, &AtomicBool::new(false))
}
fn extract_until(
    home: &Home,
    env: &Env,
    live: bool,
    stop: &AtomicBool,
) -> Result<Extraction, HistoryError> {
    let mut client = Client::open(home, env)?;
    let _extraction = LockGuard::acquire(&home.history_lock_file(), TIMEOUT)?;
    let (position, initialized) = _store::position(home, &client.source)?;
    let mut payload = json!({"requestCompletionMarker":true});
    if position > 0 {
        payload["afterSequence"] = json!(position);
    }
    client.request("shell", "orchestration.subscribeShell", payload)?;
    let mut report = Extraction {
        loaded: 0,
        position,
    };
    if !initialized {
        let archive = client.call("orchestration.getArchivedShellSnapshot", json!({}))?;
        let rows = archive["threads"]
            .as_array()
            .ok_or_else(|| failed("missing archived thread list"))?;
        let conversations = rows
            .iter()
            .map(|row| read_conversation(home, &mut client, row))
            .collect::<Result<Vec<_>, _>>()?;
        _store::load(home, &client.source, position, true, &conversations)?;
        report.loaded += conversations.len();
    }
    // Snapshot transport may split one position over many frames. Commit its
    // position only after the completion marker and every conversation read.
    let mut batch = Vec::new();
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let item = match client.next() {
            Err(HistoryError::StreamIdle) if Instant::now() < deadline => continue,
            result => result?,
        };
        if item["kind"] == "synchronized" {
            break;
        }
        batch.push(item);
    }
    let mut conversations = Vec::new();
    for item in batch {
        let (position, mut loaded) = prepare(home, &mut client, &item, false)?;
        report.position = report.position.max(position);
        conversations.append(&mut loaded);
    }
    for id in _store::pending(home, &client.source)? {
        if !conversations.iter().any(|c| c.native_id == id) {
            conversations.push(read_conversation(home, &mut client, &json!({"id":id}))?);
        }
    }
    _store::load(home, &client.source, report.position, false, &conversations)?;
    report.loaded += conversations.len();
    drop(_extraction);
    if live {
        while !stop.load(Ordering::Relaxed) {
            let item = match client.next() {
                Err(HistoryError::StreamIdle) => continue,
                result => result?,
            };
            let _extraction = LockGuard::acquire(&home.history_lock_file(), TIMEOUT)?;
            let (position, conversations) = prepare(home, &mut client, &item, true)?;
            _store::load(home, &client.source, position, false, &conversations)?;
        }
    }
    client.finish()?;
    Ok(report)
}

fn prepare(
    home: &Home,
    client: &mut Client,
    item: &Value,
    completion_only: bool,
) -> Result<(u64, Vec<Conversation>), HistoryError> {
    let mut conversations = Vec::new();
    let position = match item["kind"].as_str() {
        Some("snapshot") if completion_only => return Ok((0, Vec::new())),
        Some("snapshot") => {
            let snapshot = &item["snapshot"];
            for row in snapshot["threads"]
                .as_array()
                .ok_or_else(|| failed("missing snapshot threads"))?
                .iter()
                .chain(snapshot["archivedThreads"].as_array().into_iter().flatten())
            {
                conversations.push(read_conversation(home, client, row)?);
            }
            snapshot["snapshotSequence"]
                .as_u64()
                .ok_or_else(|| failed("missing snapshot position"))?
        }
        Some("thread.updated") => {
            let row = &item["thread"];
            let status = row["status"]
                .as_str()
                .ok_or_else(|| failed("missing thread status"))?;
            if !completion_only
                || matches!(
                    status,
                    "idle" | "completed" | "failed" | "interrupted" | "cancelled" | "rolled_back"
                )
            {
                conversations.push(read_conversation(home, client, row)?);
            } else {
                let id = row["id"]
                    .as_str()
                    .ok_or_else(|| failed("missing thread id"))?;
                let position = item["sequence"]
                    .as_u64()
                    .ok_or_else(|| failed("missing event position"))?;
                _store::defer(home, &client.source, id, position)?;
            }
            item["sequence"]
                .as_u64()
                .ok_or_else(|| failed("missing event position"))?
        }
        Some("thread.removed") => {
            let id = item["threadId"]
                .as_str()
                .ok_or_else(|| failed("missing removed thread id"))?;
            _store::remove_pending(home, &client.source, id)?;
            item["sequence"]
                .as_u64()
                .ok_or_else(|| failed("missing event position"))?
        }
        Some("project.updated" | "project.removed") => item["sequence"]
            .as_u64()
            .ok_or_else(|| failed("missing event position"))?,
        Some("synchronized") => return Ok((0, Vec::new())),
        _ => return Err(failed("unknown shell stream item")),
    };
    Ok((position, conversations))
}
fn read_conversation(
    home: &Home,
    client: &mut Client,
    row: &Value,
) -> Result<Conversation, HistoryError> {
    let id = row["id"]
        .as_str()
        .ok_or_else(|| failed("thread lacks native id"))?;
    let projection = client.call("orchestration.getThreadProjection", json!({"threadId":id}))?;
    if projection["thread"]["id"] != id {
        return Err(failed("projection identity mismatch"));
    }
    let title = projection["thread"]["title"]
        .as_str()
        .ok_or_else(|| failed("missing title"))?
        .to_owned();
    let parent = projection["thread"]["lineage"]["parentThreadId"]
        .as_str()
        .map(str::to_owned);
    let mut text = Vec::new();
    for message in projection["messages"]
        .as_array()
        .ok_or_else(|| failed("missing messages"))?
    {
        text.push(
            message["text"]
                .as_str()
                .ok_or_else(|| failed("invalid message text"))?
                .to_owned(),
        );
    }
    for item in projection["turnItems"]
        .as_array()
        .ok_or_else(|| failed("missing turn items"))?
    {
        for key in ["text", "input", "output", "markdown", "diffStr"] {
            if let Some(s) = item[key].as_str() {
                text.push(s.to_owned());
            }
        }
    }
    let active = projection["runs"]
        .as_array()
        .ok_or_else(|| failed("missing run state"))?
        .iter()
        .any(|run| {
            !matches!(
                run["status"].as_str(),
                Some("completed" | "failed" | "interrupted" | "cancelled" | "rolled_back")
            )
        });
    Ok(Conversation {
        active,
        native_id:id.into(),
        computer:instrukt_core::machine::machine_id(home)?.to_string(),
        title,
        parent,
        aliases:vec![id.into()],
        body:text.join("\n\n"),
        document:projection,
        limitations:vec!["T3 exposes only provider-persisted reasoning; tool outputs and diffs may be bounded or omitted by its wire projection.".into()],
    })
}

pub(crate) struct Worker {
    stop: Arc<AtomicBool>,
    retry: Arc<(Mutex<()>, Condvar)>,
    handle: Option<JoinHandle<()>>,
}
impl Worker {
    pub(super) fn start(home: &Home, env: &Env) -> Result<Self, HistoryError> {
        let home = home.clone();
        let env = env.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let retry = Arc::new((Mutex::new(()), Condvar::new()));
        let retry_wait = retry.clone();
        let handle = std::thread::Builder::new()
            .name("history-extraction".into())
            .spawn(move || {
                while !stopping.load(Ordering::Relaxed) {
                    if let Err(error) = extract_until(&home, &env, true, &stopping) {
                        tracing::error!(event="history_stream_disconnected",error=%error);
                    }
                    // Reconnect the ordered stream; this never initiates a periodic source scan.
                    let guard = retry_wait
                        .0
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let waited =
                        retry_wait
                            .1
                            .wait_timeout_while(guard, Duration::from_secs(5), |_| {
                                !stopping.load(Ordering::Relaxed)
                            });
                    if waited.is_err() {
                        break;
                    }
                }
            })
            .map_err(|source| HistoryError::Io {
                path: home_path(),
                source,
            })?;
        Ok(Self {
            stop,
            retry,
            handle: Some(handle),
        })
    }
    pub(crate) fn check(&self) -> Result<(), HistoryError> {
        if self.handle.as_ref().is_some_and(|h| h.is_finished()) {
            return Err(failed(
                "extraction stream stopped; restore T3 connectivity and restart instrukt run",
            ));
        }
        Ok(())
    }
}
fn home_path() -> std::path::PathBuf {
    std::path::PathBuf::from("history worker")
}
impl Drop for Worker {
    fn drop(&mut self) {
        {
            let _guard = self
                .retry
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.stop.store(true, Ordering::Relaxed);
            self.retry.1.notify_all();
        }
        if let Some(handle) = self.handle.take()
            && let Err(error) = handle.join()
        {
            tracing::error!(event="history_extraction_panicked",error=?error);
        }
    }
}
