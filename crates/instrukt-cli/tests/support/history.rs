#![allow(clippy::panic, reason = "fixture protocol violations fail the test")]
use crate::common::TestEnv;
use instrukt_core::state_db::rusqlite::Connection;
use serde_json::{Value, json};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Child;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub struct T3Store {
    file: PathBuf,
    subscribers: Arc<Mutex<Vec<mpsc::Sender<Value>>>>,
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl T3Store {
    pub fn new(env: &TestEnv) -> Self {
        computer(env, "aaaaaaaaaaaaaaaa");
        let root = env.home().join(".t3/userdata");
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("fixture.json");
        let thread = |id: &str, parent: Option<&str>| json!({"thread":{"id":id,"projectId":"project","title":"Local jewels","updatedAt":"2026-11-08T10:00:00Z","lineage":{"parentThreadId":parent,"relationshipToParent":parent.map(|_|"subagent")}},"messages":[{"id":format!("message-{id}"),"role":"assistant","text":"Keep the jewels searchable.","createdAt":"2026-11-08T10:00:00Z"}],"turnItems":[],"runs":[],"updatedAt":"2026-11-08T10:00:00Z"});
        std::fs::write(
            &file,
            json!({"sequence":2,"threads":[thread("parent",None),thread("child",Some("parent"))]})
                .to_string(),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::fs::write(root.join("server-runtime.json"),json!({"version":1,"pid":std::process::id(),"port":address.port(),"origin":format!("http://{address}"),"serverVersion":"0.1.7"}).to_string()).unwrap();
        let bin = env.path("runtime-bin/t3");
        std::fs::write(&bin, r#"#!/usr/bin/env python3
import sys,json,base64
a=sys.argv[1:]
if a==['--version']: print('t3 v0.1.7')
elif a[:3]==['auth','session','issue']: print(base64.urlsafe_b64encode(json.dumps({'v':1,'kind':'session','sid':'fixture-session'}).encode()).decode().rstrip('=')+'.signature')
elif a[:3]==['auth','session','revoke']: pass
else: sys.exit(1)
"#).unwrap();
        std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let subscribers: Arc<Mutex<Vec<mpsc::Sender<Value>>>> = Arc::new(Mutex::new(Vec::new()));
        let listeners = subscribers.clone();
        let data_file = file.clone();
        let worker = std::thread::spawn(move || {
            let mut connections = Vec::new();
            for stream in listener.incoming() {
                if stopping.load(Ordering::Relaxed) {
                    break;
                }
                let data_file = data_file.clone();
                let stopping = stopping.clone();
                let listeners = listeners.clone();
                connections.push(std::thread::spawn(move || {
                    let stream = stream.unwrap();
                    stream.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
                    let Ok(mut socket) = tungstenite::accept(stream) else { return; };
                    let (updates,receiver) = mpsc::channel::<Value>();
                    while !stopping.load(Ordering::Relaxed) {
                        for event in receiver.try_iter() {
                            if socket.send(tungstenite::Message::Text(json!({"_tag":"Chunk","requestId":"shell","values":[event]}).to_string().into())).is_err() { return; }
                        }
                        let message = match socket.read() {
                            Ok(message) => message,
                            Err(tungstenite::Error::Io(error)) if matches!(error.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut) => continue,
                            Err(_) => break,
                        };
                        let tungstenite::Message::Text(text) = message else { break; };
                        let request: Value = serde_json::from_str(&text).unwrap();
                        if request["_tag"] != "Request" { continue; }
                        let data: Value = serde_json::from_slice(&std::fs::read(&data_file).unwrap()).unwrap();
                        let send = |socket: &mut tungstenite::WebSocket<TcpStream>, value: Value, streaming: bool| {
                            let response = if streaming { json!({"_tag":"Chunk","requestId":request["id"],"values":[value]}) } else { json!({"_tag":"Exit","requestId":request["id"],"exit":{"_tag":"Success","value":value}}) };
                            socket.send(tungstenite::Message::Text(response.to_string().into())).unwrap();
                        };
                        match request["tag"].as_str().unwrap() {
                            "orchestration.subscribeShell" => {
                                listeners.lock().unwrap().push(updates.clone());
                                if request["payload"]["afterSequence"].as_u64().unwrap_or(0) < data["sequence"].as_u64().unwrap() {
                                    let shells: Vec<Value> = data["threads"].as_array().unwrap().iter().map(|p|p["thread"].clone()).collect();
                                    send(&mut socket,json!({"kind":"snapshot","snapshot":{"snapshotSequence":data["sequence"],"threads":shells,"archivedThreads":[]}}),true);
                                }
                                send(&mut socket,json!({"kind":"synchronized"}),true);
                            }
                            "orchestration.getArchivedShellSnapshot" => send(&mut socket,json!({"snapshotSequence":data["sequence"],"threads":[]}),false),
                            "orchestration.getThreadProjection" => {
                                let projection = data["threads"].as_array().unwrap().iter().find(|p|p["thread"]["id"]==request["payload"]["threadId"]).unwrap();
                                send(&mut socket, projection.clone(),false);
                            }
                            _ => panic!("unexpected fixture RPC"),
                        }
                    }
                }));
            }
            for connection in connections {
                connection.join().unwrap();
            }
        });
        Self {
            file,
            subscribers,
            address,
            stop,
            worker: Some(worker),
        }
    }
    pub fn remove_threads(&self) {
        self.write(json!({"sequence":3,"threads":[]}));
    }
    pub fn active_child(&self, append: bool) {
        let mut data: Value = serde_json::from_slice(&std::fs::read(&self.file).unwrap()).unwrap();
        data["threads"][1]["runs"] = json!([{"status":"running"}]);
        if append {
            data["threads"][1]["messages"].as_array_mut().unwrap().push(json!({"id":"hidden-update","role":"assistant","text":"Unreported child progress.","createdAt":"2026-11-08T11:00:00Z"}));
        }
        // Native-child transcript changes need not change the shell sequence.
        self.write(data);
    }
    fn write(&self, data: Value) {
        let temporary = self.file.with_extension("new");
        std::fs::write(&temporary, data.to_string()).unwrap();
        std::fs::rename(temporary, &self.file).unwrap();
    }
    pub fn complete(&self) {
        let mut data: Value = serde_json::from_slice(&std::fs::read(&self.file).unwrap()).unwrap();
        data["sequence"] = json!(3);
        data["threads"][1]["messages"].as_array_mut().unwrap().push(json!({"id":"completion","role":"assistant","text":"Finished the next turn.","createdAt":"2026-11-08T11:00:00Z"}));
        data["threads"][1]["thread"]["status"] = json!("completed");
        let event = json!({"kind":"thread.updated","sequence":3,"location":"active","thread":data["threads"][1]["thread"]});
        self.write(data);
        self.subscribers
            .lock()
            .unwrap()
            .retain(|sender| sender.send(event.clone()).is_ok());
    }
    pub fn wait_for_readers(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.subscribers.lock().unwrap().len() < count {
            assert!(
                Instant::now() < deadline,
                "history source subscriber did not connect"
            );
            std::thread::yield_now();
        }
    }
}
impl Drop for T3Store {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        self.worker.take().unwrap().join().unwrap();
    }
}

pub fn computer(env: &TestEnv, id: &str) {
    std::fs::create_dir_all(env.instrukt_home().join("state")).unwrap();
    std::fs::write(env.instrukt_home().join("state/machine-id"), id).unwrap();
}
pub fn mirror(env: &TestEnv) -> PathBuf {
    let file = env.home().join(".teleclaude/mirrors.db");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let db = Connection::open(&file).unwrap();
    db.execute_batch("PRAGMA user_version=11;
CREATE TABLE mirrors(id INTEGER PRIMARY KEY,computer TEXT,title TEXT,conversation_text TEXT,metadata TEXT,native_session_id TEXT,source_identity TEXT,session INTEGER,project_path TEXT,timestamp_start TEXT,timestamp_end TEXT);
CREATE TABLE mirror_sessions(id INTEGER PRIMARY KEY,session_id TEXT,initiator_session_id TEXT);
CREATE TABLE mirror_sources(id INTEGER PRIMARY KEY,mirror INTEGER,source_identity TEXT,native_session_id TEXT,agent TEXT,transcript_path TEXT,position INTEGER,archival_text TEXT);
CREATE TABLE mirror_messages(source INTEGER,ordinal INTEGER,entry_index INTEGER,block_ordinal INTEGER,timestamp TEXT,role TEXT,text TEXT);
INSERT INTO mirror_sessions VALUES(1,'archive-session',NULL);
INSERT INTO mirrors VALUES(1,'old-computer','Retained archive','The archive jewels.','{}','native-session','source-alias',1,'/project','2026-01-01T00:00:00Z',NULL);
INSERT INTO mirror_sources VALUES(1,1,'source-alias','native-session','claude','/gone/transcript',0,'The archive jewels.');").unwrap();
    file
}
pub fn configure_cluster(env: &TestEnv, url: &str) {
    std::fs::write(env.instrukt_home().join("instrukt.yaml"),format!("schema_version: 1\nmodules: {{events: true, todos: false}}\nrunner: {{runtime: t3}}\nredis:\n  cluster:\n    url: {url}\n")).unwrap();
}
pub struct Runner(Child);
impl Drop for Runner {
    fn drop(&mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }
}
pub fn runner(env: &TestEnv) -> Runner {
    let child = env
        .command(
            std::path::Path::new(env!("CARGO_BIN_EXE_instrukt")),
            env.home(),
        )
        .arg("run")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // The query worker publishes readiness only after opening its request queue.
    let deadline = Instant::now() + Duration::from_secs(15);
    let config: Value = serde_yaml_ng::from_str(
        &std::fs::read_to_string(env.instrukt_home().join("instrukt.yaml")).unwrap(),
    )
    .unwrap();
    let mut conn = redis::Client::open(config["redis"]["cluster"]["url"].as_str().unwrap())
        .unwrap()
        .get_connection()
        .unwrap();
    while !redis::cmd("EXISTS")
        .arg(format!(
            "instrukt:history:ready:{}",
            std::fs::read_to_string(env.instrukt_home().join("state/machine-id"))
                .unwrap()
                .trim()
        ))
        .query::<bool>(&mut conn)
        .unwrap()
    {
        assert!(Instant::now() < deadline, "history responder did not start");
        std::thread::yield_now();
    }
    Runner(child)
}
