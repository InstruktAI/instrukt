//! A stand-in for Slack at the network line: a local Web API over HTTP and a
//! local Socket Mode endpoint over WebSocket. The connector under test stays
//! real; it reaches this fake through `INSTRUKT_SLACK_API_URL`. Real Slack is
//! never contacted.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::panic,
    reason = "shared test setup: a failed step fails the test"
)]
use std::collections::VecDeque;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

/// The app token the fake accepts.
pub const APP_TOKEN: &str = "xapp-fake-app";
/// The bot token the fake accepts.
pub const BOT_TOKEN: &str = "xoxb-fake-bot";
/// The agent's bot user and bot ids in the fake workspace.
pub const BOT_USER: &str = "UAGENT";
pub const BOT_ID: &str = "BAGENT";

/// The most messages one history or replies page carries, so that every
/// read the scenarios make spans pages.
const PAGE: usize = 2;

/// A message the agent posted, as the Web API received it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Post {
    pub channel: String,
    pub thread: Option<String>,
    pub text: String,
}

#[derive(Default)]
struct Workspace {
    /// Every message written, in order: `channel`, `ts`, optional `thread_ts`,
    /// `user`, optional `bot_id`, `text`.
    messages: Vec<Value>,
    posts: Vec<Post>,
    outgoing: VecDeque<String>,
    acknowledged: Vec<String>,
    connections: usize,
    next: u64,
    refuse: bool,
    hold_reads: bool,
    reads: usize,
}

pub struct FakeSlack {
    pub api_url: String,
    workspace: Arc<Mutex<Workspace>>,
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

fn form(body: &str) -> std::collections::BTreeMap<String, String> {
    form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect()
}

fn ts(value: &Value) -> f64 {
    value["ts"].as_str().unwrap().parse().unwrap()
}

impl FakeSlack {
    pub fn start() -> Self {
        let workspace = Arc::new(Mutex::new(Workspace {
            next: 1,
            ..Workspace::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let sockets = TcpListener::bind("127.0.0.1:0").unwrap();
        let socket_url = format!("ws://{}/", sockets.local_addr().unwrap());
        let http = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let api_url = format!("http://{}/api", http.server_addr().to_ip().unwrap());
        let mut threads = Vec::new();
        {
            let workspace = Arc::clone(&workspace);
            let stop = Arc::clone(&stop);
            threads.push(std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let Ok(Some(mut request)) = http.recv_timeout(Duration::from_millis(50)) else {
                        continue;
                    };
                    let method = request.url().trim_start_matches("/api/").to_owned();
                    let token = request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("Authorization"))
                        .map(|h| h.value.as_str().trim_start_matches("Bearer ").to_owned())
                        .unwrap_or_default();
                    let mut body = String::new();
                    request.as_reader().read_to_string(&mut body).unwrap();
                    // Held reads answer only once released.
                    while method.starts_with("conversations.")
                        && workspace.lock().unwrap().hold_reads
                        && !stop.load(Ordering::Acquire)
                    {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    let answer =
                        Self::answer(&workspace, &socket_url, &method, &token, &form(&body));
                    request
                        .respond(tiny_http::Response::from_string(answer.to_string()))
                        .unwrap();
                }
            }));
        }
        {
            let workspace = Arc::clone(&workspace);
            let stop = Arc::clone(&stop);
            sockets.set_nonblocking(true).unwrap();
            threads.push(std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let stream = match sockets.accept() {
                        Ok((stream, _)) => stream,
                        Err(_) => {
                            std::thread::sleep(Duration::from_millis(20));
                            continue;
                        }
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_millis(20)))
                        .unwrap();
                    let Ok(mut socket) = tungstenite::accept(stream) else {
                        continue;
                    };
                    workspace.lock().unwrap().connections += 1;
                    socket
                        .send(tungstenite::Message::text(
                            json!({"type":"hello","num_connections":1}).to_string(),
                        ))
                        .unwrap();
                    while !stop.load(Ordering::Acquire) {
                        let next = workspace.lock().unwrap().outgoing.pop_front();
                        if let Some(next) = next
                            && socket.send(tungstenite::Message::text(next)).is_err()
                        {
                            break;
                        }
                        match socket.read() {
                            Ok(tungstenite::Message::Text(text)) => {
                                let ack: Value = serde_json::from_str(&text).unwrap();
                                if let Some(id) = ack["envelope_id"].as_str() {
                                    workspace.lock().unwrap().acknowledged.push(id.to_owned());
                                }
                            }
                            Ok(tungstenite::Message::Close(_)) => break,
                            Ok(_) => {}
                            Err(tungstenite::Error::Io(error))
                                if matches!(
                                    error.kind(),
                                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                                ) => {}
                            Err(_) => break,
                        }
                    }
                }
            }));
        }
        Self {
            api_url,
            workspace,
            stop,
            threads,
        }
    }

    fn answer(
        workspace: &Mutex<Workspace>,
        socket_url: &str,
        method: &str,
        token: &str,
        args: &std::collections::BTreeMap<String, String>,
    ) -> Value {
        let mut w = workspace.lock().unwrap();
        let app = method == "apps.connections.open";
        let accepted = if app { APP_TOKEN } else { BOT_TOKEN };
        if w.refuse || token != accepted {
            return json!({"ok": false, "error": "invalid_auth"});
        }
        let arg = |name: &str| args.get(name).map(String::as_str);
        let bound = |m: &Value, name: &str, below: bool| {
            arg(name).is_none_or(|edge| {
                let edge: f64 = edge.parse().unwrap();
                if below { ts(m) < edge } else { ts(m) > edge }
            })
        };
        match method {
            "apps.connections.open" => json!({"ok": true, "url": socket_url}),
            "auth.test" => json!({"ok": true, "user_id": BOT_USER, "bot_id": BOT_ID}),
            "conversations.history" | "conversations.replies" => {
                let channel = arg("channel").unwrap();
                let thread = arg("ts");
                let mut found: Vec<Value> = w
                    .messages
                    .iter()
                    .filter(|m| m["channel"] == channel)
                    .filter(|m| match thread {
                        Some(root) => m["ts"] == root || m["thread_ts"] == root,
                        None => m["thread_ts"].is_null() || m["thread_ts"] == m["ts"],
                    })
                    .filter(|m| bound(m, "latest", true) && bound(m, "oldest", false))
                    .cloned()
                    .collect();
                // As Slack does: channel history newest first, a thread's
                // replies oldest first, in pages no larger than asked and at
                // most `PAGE`, continued with a cursor.
                if thread.is_none() {
                    found.reverse();
                }
                let size = arg("limit")
                    .map_or(PAGE, |l| l.parse::<usize>().unwrap())
                    .min(PAGE);
                let from: usize = arg("cursor").map_or(0, |c| c.parse().unwrap());
                let page: Vec<Value> = found.iter().skip(from).take(size).cloned().collect();
                let more = from + size < found.len();
                w.reads += 1;
                json!({"ok": true, "messages": page, "has_more": more, "response_metadata": {"next_cursor": if more { (from + size).to_string() } else { String::new() }}})
            }
            "chat.postMessage" => {
                let post = Post {
                    channel: arg("channel").unwrap().to_owned(),
                    thread: arg("thread_ts").map(str::to_owned),
                    text: arg("text").unwrap().to_owned(),
                };
                let ts = Self::stamp(&mut w);
                let mut message = json!({"channel": post.channel, "ts": ts, "user": BOT_USER, "bot_id": BOT_ID, "text": post.text});
                if let Some(thread) = &post.thread {
                    message["thread_ts"] = json!(thread);
                }
                w.messages.push(message);
                w.posts.push(post);
                json!({"ok": true, "ts": ts})
            }
            other => json!({"ok": false, "error": format!("unknown_method {other}")}),
        }
    }

    fn stamp(w: &mut Workspace) -> String {
        let ts = format!("1700000{:03}.000100", w.next);
        w.next += 1;
        ts
    }

    /// A person writes a message nobody addresses to the agent: Slack sends
    /// instrukt nothing for it.
    pub fn say(&self, channel: &str, user: &str, text: &str, thread: Option<&str>) -> String {
        let ts = Self::stamp(&mut self.workspace.lock().unwrap());
        self.say_at(&ts, channel, user, text, thread)
    }

    /// A message written at the Slack time `ts`.
    pub fn say_at(
        &self,
        ts: &str,
        channel: &str,
        user: &str,
        text: &str,
        thread: Option<&str>,
    ) -> String {
        let mut w = self.workspace.lock().unwrap();
        let ts = ts.to_owned();
        let mut message = json!({"channel": channel, "ts": ts, "user": user, "text": text});
        if let Some(thread) = thread {
            message["thread_ts"] = json!(thread);
        }
        w.messages.push(message);
        ts
    }

    fn deliver(&self, envelope: &str, event: Value) {
        self.workspace.lock().unwrap().outgoing.push_back(
            json!({"envelope_id": envelope, "type": "events_api", "accepts_response_payload": false, "payload": {"type": "event_callback", "event": event}})
                .to_string(),
        );
    }

    /// A person tags the agent in a channel, in `thread` or starting one.
    pub fn mention(&self, channel: &str, user: &str, text: &str, thread: Option<&str>) -> String {
        let ts = Self::stamp(&mut self.workspace.lock().unwrap());
        self.mention_at(&ts, channel, user, text, thread)
    }

    /// A tag written at the Slack time `ts`.
    pub fn mention_at(
        &self,
        ts: &str,
        channel: &str,
        user: &str,
        text: &str,
        thread: Option<&str>,
    ) -> String {
        let ts = self.say_at(ts, channel, user, text, thread);
        let mut event = json!({"type": "app_mention", "channel": channel, "user": user, "text": text, "ts": ts, "event_ts": ts});
        if let Some(thread) = thread {
            event["thread_ts"] = json!(thread);
        }
        self.deliver(&format!("envelope-{ts}"), event);
        ts
    }

    /// A person writes to the agent in a direct message.
    pub fn direct(&self, channel: &str, user: &str, text: &str) -> String {
        let ts = self.say(channel, user, text, None);
        self.deliver(
            &format!("envelope-{ts}"),
            json!({"type": "message", "channel_type": "im", "channel": channel, "user": user, "text": text, "ts": ts, "event_ts": ts}),
        );
        ts
    }

    /// History and replies reads wait until [`FakeSlack::release_reads`].
    pub fn hold_reads(&self) {
        self.workspace.lock().unwrap().hold_reads = true;
    }

    pub fn release_reads(&self) {
        self.workspace.lock().unwrap().hold_reads = false;
    }

    /// How many history and replies pages were read.
    pub fn reads(&self) -> usize {
        self.workspace.lock().unwrap().reads
    }

    /// The Web API refuses every token from now on.
    pub fn refuse_tokens(&self) {
        self.workspace.lock().unwrap().refuse = true;
    }

    pub fn posts(&self) -> Vec<Post> {
        self.workspace.lock().unwrap().posts.clone()
    }

    pub fn acknowledged(&self) -> Vec<String> {
        self.workspace.lock().unwrap().acknowledged.clone()
    }

    pub fn connections(&self) -> usize {
        self.workspace.lock().unwrap().connections
    }
}

impl Drop for FakeSlack {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
