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
        std::fs::write(
            dir.join("snapshot.json"),
            json!({"providers":providers,"settings":{"providerInstances":{}}}).to_string(),
        )
        .unwrap();
        let runtime = env.home().join(".t3/userdata/server-runtime.json");
        std::fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        std::fs::write(runtime,json!({"version":1,"pid":std::process::id(),"port":address.port(),"origin":format!("http://{address}"),"serverVersion":"0.1.7"}).to_string()).unwrap();
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
                std::thread::spawn(move || serve(&dir, stream, &held));
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
fn serve(dir: &Path, stream: TcpStream, held: &std::sync::atomic::AtomicUsize) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let header_dir = dir.to_path_buf();
    let accepted = tungstenite::accept_hdr(
        stream,
        move |request: &tungstenite::handshake::server::Request,
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
    std::fs::write(dir.join("request.json"), request.to_string()).unwrap();
    // A held read stays open until the test releases it, every held read or
    // this one by its number, unless its client gives up first: it is
    // recorded when it arrived, and whether the client ended it.
    if dir.join("hold-read").exists() {
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
    let response = if dir.join("malformed").exists() {
        json!({"unknown":true})
    } else {
        json!({"_tag":"Exit","requestId":request["id"],"exit":{"_tag":"Success","value":value}})
    };
    let _ = socket.send(tungstenite::Message::Text(response.to_string().into()));
    let _ = socket.close(None);
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
    env.home().join(".t3/userdata/server-runtime.json")
}
