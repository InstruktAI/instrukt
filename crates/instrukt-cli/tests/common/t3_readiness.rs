//! T3's local WebSocket endpoint, owned by the temporary test machine.
use super::TestEnv;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    net::{TcpListener, TcpStream},
    path::PathBuf,
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
            json!({"providers":providers}).to_string(),
        )
        .unwrap();
        let runtime = env.home().join(".t3/userdata/server-runtime.json");
        std::fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        std::fs::write(runtime,json!({"version":1,"pid":std::process::id(),"port":address.port(),"origin":format!("http://{address}"),"serverVersion":"0.1.7"}).to_string()).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopping.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else {
                    break;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let header_dir = dir.clone();
                let accepted = tungstenite::accept_hdr(
                    stream,
                    move |request: &tungstenite::handshake::server::Request,
                          response: tungstenite::handshake::server::Response| {
                        let auth = request
                            .headers()
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.strip_prefix("Bearer "))
                            .is_some_and(|token| {
                                hex::encode(Sha256::digest(token.as_bytes()))
                                    == std::fs::read_to_string(header_dir.join("token-digest"))
                                        .unwrap()
                            });
                        if !auth {
                            return Err(tungstenite::http::Response::builder()
                                .status(401)
                                .body(Some("unauthorized".into()))
                                .unwrap());
                        }
                        std::fs::write(
                            header_dir.join("authenticated"),
                            if auth { "yes" } else { "no" },
                        )
                        .unwrap();
                        std::fs::write(header_dir.join("path"), request.uri().to_string()).unwrap();
                        Ok(response)
                    },
                );
                let Ok(mut socket) = accepted else {
                    continue;
                };
                let Ok(message) = socket.read() else {
                    continue;
                };
                let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                std::fs::write(dir.join("request.json"), request.to_string()).unwrap();
                let value: Value = serde_json::from_str(
                    &std::fs::read_to_string(dir.join("snapshot.json")).unwrap(),
                )
                .unwrap();
                let response = if dir.join("malformed").exists() {
                    json!({"unknown":true})
                } else {
                    json!({"_tag":"Exit","requestId":request["id"],"exit":{"_tag":"Success","value":value}})
                };
                let _ = socket.send(tungstenite::Message::Text(response.to_string().into()));
                let _ = socket.close(None);
            }
        });
        Self {
            address,
            stop,
            worker: Some(worker),
        }
    }
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
