//! T3's local WebSocket endpoint, owned by the temporary test machine.
use super::TestEnv;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

pub struct Server {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Server {
    #[allow(
        clippy::result_large_err,
        reason = "tungstenite fixes the handshake callback error to an HTTP response"
    )]
    pub fn new(env: &TestEnv) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let dir = env.path("t3-readiness");
        std::fs::create_dir_all(&dir).unwrap();
        let mut providers = Vec::new();
        let agents =
            instrukt_core::config::Agents::parse(instrukt_core::config::SHIPPED_AGENTS).unwrap();
        for profile in agents.profiles.values() {
            let mut models = std::collections::BTreeSet::new();
            for chain in [
                &profile.light,
                &profile.standard,
                &profile.strong,
                &profile.premium,
            ] {
                for entry in chain {
                    models.insert(entry["model"].as_str().unwrap().to_owned());
                }
            }
            let kind = serde_json::to_value(profile.kind).unwrap();
            let driver = if kind == "claude" {
                "claudeAgent"
            } else {
                kind.as_str().unwrap()
            };
            providers.push(json!({"instanceId":profile.instance,"driver":driver,"enabled":true,"installed":true,"status":"ready","auth":{"status":"authenticated"},"availability":"available","models":models.into_iter().map(|slug|json!({"slug":slug})).collect::<Vec<_>>() }));
        }
        // A restarted endpoint keeps the snapshot it served before.
        if !dir.join("snapshot.json").exists() {
            std::fs::write(
                dir.join("snapshot.json"),
                json!({"providers":providers,"settings":{"providerInstances":{},"defaultRuntimeMode":"full-access","projectSettingsOverrides":{}}}).to_string(),
            )
            .unwrap();
        }
        let runtime =
            Path::new(env.injected("T3CODE_HOME").unwrap()).join("userdata/server-runtime.json");
        std::fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        std::fs::write(runtime,json!({"version":1,"pid":std::process::id(),"port":address.port(),"origin":format!("http://{address}"),"serverVersion":"0.1.7"}).to_string()).unwrap();
        std::fs::write(
            dir.join("fixture-environment.json"),
            serde_json::to_vec(&env.vars).unwrap(),
        )
        .unwrap();
        let home = env.home().to_path_buf();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        // Held reads, numbered in the order they arrived.
        let held = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopping.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else {
                    break;
                };
                // Each connection on its own, so a held read holds no other.
                let dir = dir.clone();
                let held = Arc::clone(&held);
                let home = home.clone();
                std::thread::spawn(move || serve(&dir, &home, stream, &held));
            }
        });
        Self {
            address,
            stop,
            worker: Some(worker),
        }
    }
}
/// The time T3 judges a token's expiry by: the operator's clock the runner
/// issued it under, when it had one.
fn now(dir: &Path) -> jiff::Timestamp {
    let source = std::fs::read_to_string(dir.join("clock-source")).unwrap_or_default();
    let text = match source.strip_prefix('@') {
        Some(file) => std::fs::read_to_string(file).unwrap_or_default(),
        None => source,
    };
    text.trim()
        .parse()
        .unwrap_or_else(|_| jiff::Timestamp::now())
}

/// The session whose live token `token` is: issued, not revoked, not past
/// its expiry.
fn session(dir: &Path, token: &str) -> Option<String> {
    let digest = hex::encode(Sha256::digest(token.as_bytes()));
    let now = now(dir);
    std::fs::read_dir(dir.join("tokens"))
        .ok()?
        .find_map(|entry| {
            let path = entry.ok()?.path();
            let record: Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
            let expires: jiff::Timestamp = record["expires"].as_str()?.parse().ok()?;
            (record["digest"] == digest.as_str() && record["revoked"] == false && expires > now)
                .then(|| path.file_stem().unwrap().to_string_lossy().into_owned())
        })
}

fn record(dir: &Path, file: &str, line: &str) {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(file))
        .unwrap();
    writeln!(file, "{line}").unwrap();
}

#[allow(
    clippy::result_large_err,
    reason = "tungstenite fixes the handshake callback error to an HTTP response"
)]
fn serve(dir: &Path, home: &Path, stream: TcpStream, held: &std::sync::atomic::AtomicUsize) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut prefix = [0; 32];
    if stream.peek(&mut prefix).unwrap_or(0) > 0 && prefix.starts_with(b"GET /api/projects ") {
        use std::io::{Read, Write};
        let mut stream = stream;
        let mut bytes = Vec::new();
        let mut byte = [0; 1];
        while stream.read_exact(&mut byte).is_ok() {
            bytes.push(byte[0]);
            if bytes.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8(bytes).unwrap();
        let authorized = request
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                    .and_then(|(_, value)| value.trim().strip_prefix("Bearer "))
            })
            .and_then(|token| session(dir, token))
            .is_some();
        let value =
            json!({"projects":projects(dir),"updatedAt":"2026-11-06T10:00:00Z"}).to_string();
        let status = if authorized {
            "200 OK"
        } else {
            "401 Unauthorized"
        };
        write!(stream,"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{value}",value.len()).unwrap();
        return;
    }
    let header_dir = dir.to_path_buf();
    let mut authenticated = None;
    let accepted = tungstenite::accept_hdr(
        stream,
        |request: &tungstenite::handshake::server::Request,
         response: tungstenite::handshake::server::Response| {
            let used = request
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .and_then(|token| session(&header_dir, token));
            // Which session each read used, or that it was refused.
            record(&header_dir, "reads", used.as_deref().unwrap_or("refused"));
            if used.is_none() {
                return Err(tungstenite::http::Response::builder()
                    .status(401)
                    .body(Some("unauthorized".into()))
                    .unwrap());
            }
            authenticated = used;
            std::fs::write(header_dir.join("authenticated"), "yes").unwrap();
            std::fs::write(header_dir.join("path"), request.uri().to_string()).unwrap();
            Ok(response)
        },
    );
    let Ok(mut socket) = accepted else {
        return;
    };
    let Ok(message) = socket.read() else {
        return;
    };
    let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
    if request["tag"] == "orchestration.launchThread"
        || request["tag"] == "orchestration.getThreadProjection"
    {
        let exit = if request["tag"] == "orchestration.launchThread" {
            {
                let sid = authenticated.as_ref().unwrap();
                let token: Value = serde_json::from_slice(
                    &std::fs::read(dir.join(format!("tokens/{sid}.json"))).unwrap(),
                )
                .unwrap();
                launch(
                    dir,
                    home,
                    &request["payload"],
                    token["owner"].as_u64().unwrap(),
                )
            }
        } else {
            let id = request["payload"]["threadId"].as_str().unwrap();
            match std::fs::read(
                dir.parent()
                    .unwrap()
                    .join("t3-state")
                    .join(format!("{id}.json")),
            ) {
                Ok(bytes) => {
                    let mut thread: Value = serde_json::from_slice(&bytes).unwrap();
                    thread["id"] = thread["threadId"].clone();
                    json!({"_tag":"Success","value":{"thread":thread}})
                }
                Err(_) => {
                    json!({"_tag":"Failure","cause":[{"_tag":"Fail","error":{"detail":"Thread not found."}}]})
                }
            }
        };
        if request["tag"] == "orchestration.launchThread"
            && dir.parent().unwrap().join("t3-state/silent-start").exists()
        {
            std::fs::remove_file(dir.parent().unwrap().join("t3-state/silent-start")).unwrap();
            let _ = socket.close(None);
            return;
        }
        let _ = socket.send(tungstenite::Message::Text(
            json!({"_tag":"Exit","requestId":request["id"],"exit":exit})
                .to_string()
                .into(),
        ));
        return;
    }
    if request["tag"] == "orchestration.dispatchCommand" {
        let sid = authenticated.unwrap();
        let token = std::fs::read(dir.join(format!("tokens/{sid}.json"))).unwrap();
        let token: Value = serde_json::from_slice(&token).unwrap();
        assert_eq!(token["scope"], "orchestration:operate");
        let exit = dispatch(dir, home, &request["payload"]);
        let reply = json!({"_tag":"Exit","requestId":request["id"],"exit":exit});
        let _ = socket.send(tungstenite::Message::Text(reply.to_string().into()));
        let _ = socket.close(None);
        return;
    }
    std::fs::write(dir.join("request.json"), request.to_string()).unwrap();
    // A held read stays open until the test releases it, every held read or
    // this one by its number, unless its client gives up first: it is
    // recorded when it arrived, and whether the client ended it.
    let session_record: Value = serde_json::from_slice(
        &std::fs::read(dir.join(format!("tokens/{}.json", authenticated.as_ref().unwrap())))
            .unwrap(),
    )
    .unwrap();
    if dir.join("hold-read").exists() && session_record["label"] != "instrukt-launch-metadata" {
        let number = held.fetch_add(1, Ordering::SeqCst) + 1;
        let record = dir.join("held-reads").join(number.to_string());
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, "arrived").unwrap();
        socket
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        while !dir.join("release-read").exists()
            && !dir.join(format!("release-read-{number}")).exists()
        {
            match socket.read() {
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                _ => {
                    std::fs::write(&record, "closed").unwrap();
                    return;
                }
            }
        }
        std::fs::write(&record, "released").unwrap();
    }
    let value: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("snapshot.json")).unwrap()).unwrap();
    let response = if dir.join("malformed").exists()
        && session_record["label"] != "instrukt-launch-metadata"
    {
        json!({"unknown":true})
    } else {
        json!({"_tag":"Exit","requestId":request["id"],"exit":{"_tag":"Success","value":value}})
    };
    let _ = socket.send(tungstenite::Message::Text(response.to_string().into()));
    let _ = socket.close(None);
}

/// T3's project table of the test rooted at `root`: a project exists only
/// where a fixture or an emulated `t3 project add` placed it, never because
/// a folder under test exists (`project/instrukt/spec/feature/runner/t3-contract`).
pub fn projects_file(root: &Path) -> PathBuf {
    root.join("t3-state/projects.table")
}

/// The id T3's fake gives a project it adds at `root`.
pub fn default_project_id(root: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    format!(
        "project-{}",
        hex::encode(Sha256::digest(root.to_string_lossy().as_bytes()))
    )
}

/// Edits the project table under the lock the fake `t3` takes too, and
/// replaces it whole.
fn edit_projects(root: &Path, edit: impl FnOnce(&mut Vec<Value>)) {
    let file = projects_file(root);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file.with_file_name("projects.lock"))
        .unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive).unwrap();
    let mut projects: Vec<Value> = std::fs::read(&file)
        .ok()
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
        .unwrap_or_default();
    edit(&mut projects);
    let temporary = file.with_extension("table.fixture");
    std::fs::write(&temporary, serde_json::to_vec(&projects).unwrap()).unwrap();
    std::fs::rename(temporary, file).unwrap();
    drop(lock);
}

/// Places a live project rooted exactly at `checkout`, with `id`, replacing
/// any project at that root.
pub fn place_project(root: &Path, checkout: &Path, id: &str) {
    let workspace = checkout
        .canonicalize()
        .unwrap_or_else(|_| checkout.to_path_buf());
    edit_projects(root, |projects| {
        projects.retain(|project| project["workspaceRoot"] != json!(workspace));
        projects.push(json!({"id":id,"workspaceRoot":workspace,"deletedAt":null}));
    });
}

/// Marks the project rooted at `checkout` deleted, as T3 keeps it.
pub fn withdraw_project(root: &Path, checkout: &Path) {
    let workspace = checkout
        .canonicalize()
        .unwrap_or_else(|_| checkout.to_path_buf());
    edit_projects(root, |projects| {
        for project in projects
            .iter_mut()
            .filter(|project| project["workspaceRoot"] == json!(workspace))
        {
            project["deletedAt"] = json!("2026-11-06T10:00:00Z");
        }
    });
}

fn projects(dir: &Path) -> Vec<Value> {
    std::fs::read(projects_file(dir.parent().unwrap()))
        .ok()
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
        .unwrap_or_default()
}
fn launch(dir: &Path, _home: &Path, payload: &Value, owner: u64) -> Value {
    use std::io::Write;
    let Some(project) = projects(dir)
        .into_iter()
        .find(|p| p["id"] == payload["projectId"] && p["deletedAt"].is_null())
    else {
        return json!({"_tag":"Failure","cause":[{"_tag":"Fail","error":{"detail":"Project not found."}}]});
    };
    let root = dir.parent().unwrap();
    let mut command = std::process::Command::new(root.join("runtime-bin/t3"));
    command.args([
        "thread",
        "start",
        "--title",
        payload["title"].as_str().unwrap(),
        "--provider",
        payload["modelSelection"]["instanceId"].as_str().unwrap(),
        "--model",
        payload["modelSelection"]["model"].as_str().unwrap(),
    ]);
    if payload["workspaceStrategy"]["type"] == "worktree" {
        command.args([
            "--worktree",
            payload["workspaceStrategy"]["baseRef"].as_str().unwrap(),
            "--branch",
            payload["workspaceStrategy"]["branch"].as_str().unwrap(),
        ]);
    }
    let mut child = command
        .args(["--", project["workspaceRoot"].as_str().unwrap(), "-"])
        .envs(
            serde_json::from_slice::<std::collections::BTreeMap<String, String>>(
                &std::fs::read(dir.join("fixture-environment.json")).unwrap(),
            )
            .unwrap(),
        )
        .env("T3_TEST_LAUNCH", payload.to_string())
        .env("T3_TEST_OPERATION_OWNER", owner.to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            payload["initialMessage"]["text"]
                .as_str()
                .unwrap_or("")
                .as_bytes(),
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    if !output.status.success() {
        return json!({"_tag":"Failure","cause":[{"_tag":"Fail","error":{"detail":String::from_utf8_lossy(&output.stderr)}}]});
    }
    let id = payload["threadId"].as_str().unwrap();
    let thread: Value = serde_json::from_slice(
        &std::fs::read(root.join("t3-state").join(format!("{id}.json"))).unwrap(),
    )
    .unwrap();
    json!({"_tag":"Success","value":{"threadId":id,"projection":{"thread":thread},"resumed":false,"sequence":1}})
}

/// `thread.archive` as the pinned release takes it: the thread leaves the
/// listing, its projection row records when it was archived, and a second
/// archive of it is refused. Each archive that succeeds is recorded.
fn archive(dir: &Path, payload: &Value) -> Value {
    let refused = |detail: String| json!({"_tag":"Failure","cause":[{"_tag":"Fail","error":{"detail":detail}}]});
    let fields: std::collections::BTreeSet<&str> = payload
        .as_object()
        .map(|fields| fields.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let (Some(command), Some(thread)) = (
        payload["commandId"].as_str().filter(|id| !id.is_empty()),
        payload["threadId"].as_str().filter(|id| !id.is_empty()),
    ) else {
        return refused("invalid thread.archive request".into());
    };
    if fields != ["commandId", "threadId", "type"].into() {
        return refused("invalid thread.archive request".into());
    }
    let state = dir.parent().unwrap().join("t3-state");
    let file = state.join(format!("{thread}.json"));
    let Ok(bytes) = std::fs::read(&file) else {
        return refused(format!("Thread '{thread}' was not found."));
    };
    let mut native: Value = serde_json::from_slice(&bytes).unwrap();
    if native["archived"] == true {
        return refused(format!("Thread '{thread}' is already archived."));
    }
    native["archived"] = true.into();
    let staged = state.join(format!("{thread}.archiving"));
    std::fs::write(&staged, serde_json::to_vec(&native).unwrap()).unwrap();
    std::fs::rename(&staged, &file).unwrap();
    let environment: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&std::fs::read(dir.join("fixture-environment.json")).unwrap())
            .unwrap();
    let database = Path::new(&environment["T3CODE_HOME"]).join("userdata/statev2.sqlite");
    let db = instrukt_core::state_db::rusqlite::Connection::open(database).unwrap();
    db.execute(
        "UPDATE orchestration_v2_projection_threads SET archived_at=?2,\
         payload_json=json_set(payload_json,'$.archivedAt',?2) WHERE thread_id=?1",
        [thread, &now(dir).to_string()],
    )
    .unwrap();
    record(dir, "archive-commands", &format!("{command} {thread}"));
    json!({"_tag":"Success","value":{"sequence":1}})
}

fn dispatch(dir: &Path, _home: &Path, payload: &Value) -> Value {
    use std::io::Write as _;
    if payload["type"] == "thread.archive" {
        return archive(dir, payload);
    }
    // The required message.dispatch shape exercised by the pinned release
    // contract. Reject malformed requests before the fake accepts input.
    for field in ["createdBy", "creationSource"] {
        if payload.get(field).is_none() {
            return json!({
                "_tag":"Failure",
                "cause":[{"_tag":"Die","defect":format!("Missing key\n  at [{field:?}]")}]
            });
        }
    }
    let required = ["commandId", "threadId", "messageId"];
    if payload["type"] != "message.dispatch"
        || required
            .iter()
            .any(|field| payload[*field].as_str().is_none_or(str::is_empty))
        || !payload["text"].is_string()
        || !matches!(
            payload["createdBy"].as_str(),
            Some("user" | "agent" | "system")
        )
        || !matches!(
            payload["creationSource"].as_str(),
            Some("web" | "mobile" | "mcp" | "provider" | "server")
        )
        || !payload["attachments"].is_array()
        || payload["dispatchMode"]["type"] != "queue_after_active"
    {
        return json!({"_tag":"Failure","cause":[{"_tag":"Fail","error":{"detail":"invalid message.dispatch request"}}]});
    }
    let id = payload["commandId"].as_str().unwrap();
    let thread = payload["threadId"].as_str().unwrap();
    let text = payload["text"].as_str().unwrap();
    assert_eq!(payload["messageId"], id);
    assert_eq!(payload["createdBy"], "agent");
    assert_eq!(payload["creationSource"], "mcp");
    assert_eq!(payload["attachments"], json!([]));
    let root = dir.parent().unwrap();
    let mut child = std::process::Command::new(root.join("runtime-bin/t3"))
        .args(["thread", "send", "--", thread, "-"])
        .envs(
            serde_json::from_slice::<std::collections::BTreeMap<String, String>>(
                &std::fs::read(dir.join("fixture-environment.json")).unwrap(),
            )
            .unwrap(),
        )
        .env("T3_TEST_COMMAND_ID", id)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    if output.status.success() {
        json!({"_tag":"Success","value":{"sequence":1}})
    } else {
        let detail = String::from_utf8_lossy(&output.stderr);
        json!({"_tag":"Failure","cause":[{"_tag":"Fail","error":{"detail":detail.trim()}}]})
    }
}

/// The threads archived through `thread.archive`, in order.
pub fn archived(env: &TestEnv) -> Vec<String> {
    std::fs::read_to_string(env.path("t3-readiness").join("archive-commands"))
        .unwrap_or_default()
        .lines()
        .map(|line| line.split_once(' ').unwrap().1.to_owned())
        .collect()
}

/// How many snapshot reads were held so far.
pub fn held_reads(env: &TestEnv) -> usize {
    std::fs::read_dir(env.path("t3-readiness/held-reads")).map_or(0, Iterator::count)
}

/// Whether the `n`th held read (from 1) is still open: neither ended by its
/// client nor released.
pub fn held_read_open(env: &TestEnv, n: usize) -> bool {
    std::fs::read_to_string(env.path(&format!("t3-readiness/held-reads/{n}")))
        .is_ok_and(|state| state == "arrived")
}

/// Lets the `n`th held read (from 1) through, and no other.
#[allow(dead_code, reason = "shared by the readiness scenarios")]
pub fn release_read(env: &TestEnv, n: usize) {
    std::fs::write(env.path(&format!("t3-readiness/release-read-{n}")), "").unwrap();
}

/// The session each snapshot read used, in order; `refused` for a read
/// whose token was not live.
pub fn reads(env: &TestEnv) -> Vec<String> {
    std::fs::read_to_string(env.path("t3-readiness/reads"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Whether the session `sid` was revoked.
pub fn revoked(env: &TestEnv, sid: &str) -> bool {
    std::fs::read(env.path(&format!("t3-readiness/tokens/{sid}.json")))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|record| record["revoked"] == true)
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

pub fn update(env: &TestEnv, edit: impl FnOnce(&mut Value)) {
    env.readiness_server();
    let file = env.path("t3-readiness/snapshot.json");
    let mut snapshot: Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    edit(&mut snapshot);
    std::fs::write(file, snapshot.to_string()).unwrap();
}
pub fn state_file(env: &TestEnv) -> PathBuf {
    Path::new(env.injected("T3CODE_HOME").unwrap()).join("userdata/server-runtime.json")
}

/// One provider instance as T3's snapshot reports it: installed, ready,
/// authenticated, `enabled` as given.
pub fn provider(instance: &str, driver: &str, enabled: bool) -> Value {
    json!({"instanceId":instance,"driver":driver,"enabled":enabled,"installed":true,"status":"ready","auth":{"status":"authenticated"},"availability":"available","models":[]})
}

/// Gives T3 exactly `providers`, with `instances` as its settings'
/// `providerInstances`: the agent runtimes it reports as configured.
pub fn configure(env: &TestEnv, providers: Vec<Value>, instances: Value) {
    update(env, |snapshot| {
        snapshot["providers"] = Value::Array(providers);
        snapshot["settings"]["providerInstances"] = instances;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::os::unix::fs::MetadataExt as _;

    fn issue_token(env: &TestEnv, label: &str) -> String {
        let output = env
            .command(&env.path("runtime-bin/t3"), env.home())
            .args([
                "auth",
                "session",
                "issue",
                "--scope",
                "orchestration:read",
                "--ttl",
                "5m",
                "--token-only",
                "--label",
                label,
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "t3 auth session issue ({label}) failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn projects_status(port: u16, token: &str) -> u16 {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            stream,
            "GET /api/projects HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap()
    }

    #[test]
    fn clock_source_is_replaced_atomically_across_concurrent_token_issues() {
        let mut env = TestEnv::new();
        // An operator clock well before wall-clock time: any spurious fallback
        // to jiff::Timestamp::now() immediately treats 5m tokens as expired.
        let first_now = "2026-11-19T10:10:00Z";
        let second_now = "2026-11-19T10:11:00Z";
        env.set_env("INSTRUKT_NOW", first_now);
        crate::fixtures::t3::FakeT3::create(&env);
        let runtime: Value =
            serde_json::from_slice(&std::fs::read(state_file(&env)).unwrap()).unwrap();
        let port = runtime["port"].as_u64().unwrap() as u16;
        let dir = env.path("t3-readiness");
        let clock_source = dir.join("clock-source");

        let token1 = issue_token(&env, "first");
        assert_eq!(projects_status(port, &token1), 200);

        // Hold an open handle to the first clock-source inode, modelling a
        // reader in `now(dir)` that opened the file before a concurrent rewrite.
        let mut held = std::fs::File::open(&clock_source).unwrap();
        let before_ino = held.metadata().unwrap().ino();

        env.set_env("INSTRUKT_NOW", second_now);
        let token2 = issue_token(&env, "second");
        assert_eq!(projects_status(port, &token2), 200);

        // An atomic write-to-temp + rename preserves the old inode's bytes and
        // replaces the directory entry with a new inode; an in-place O_TRUNC
        // overwrite mutates the same inode under the reader's open handle.
        let mut held_contents = String::new();
        held.read_to_string(&mut held_contents).unwrap();
        assert_eq!(held_contents, first_now);
        assert_ne!(std::fs::metadata(&clock_source).unwrap().ino(), before_ino);
        assert_eq!(std::fs::read_to_string(&clock_source).unwrap(), second_now);

        // Concurrent `t3 auth session issue` subprocesses must not collide on
        // temp filenames or cause spurious 401 rejections on live requests.
        std::thread::scope(|scope| {
            for index in 0..4 {
                let env_ref = &env;
                let token_ref = &token1;
                scope.spawn(move || {
                    for round in 0..3 {
                        let issued = issue_token(env_ref, &format!("concurrent-{index}-{round}"));
                        assert_eq!(projects_status(port, token_ref), 200);
                        assert_eq!(projects_status(port, &issued), 200);
                    }
                });
            }
        });

        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("clock-source.") && name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");

        // Without INSTRUKT_NOW, wall-clock fallback still accepts a live token.
        env.remove_env("INSTRUKT_NOW");
        let live_token = issue_token(&env, "wall-clock");
        assert_eq!(projects_status(port, &live_token), 200);
        assert_eq!(projects_status(port, &token1), 401);
    }
}
