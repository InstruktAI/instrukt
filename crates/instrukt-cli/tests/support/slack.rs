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
/// The workspace the app is installed in.
pub const TEAM: &str = "TFAKE";

/// The most messages one history or replies page carries, so that every
/// read the scenarios make spans pages.
const PAGE: usize = 2;

/// A message the agent posted, as the Web API received it: as plain text
/// (`text`) or as Markdown Slack renders (`markdown_text`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Post {
    pub channel: String,
    pub thread: Option<String>,
    pub text: Option<String>,
    pub markdown: Option<String>,
}

/// What becomes of a post whose text begins with a given beginning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fate {
    /// Refused: nothing is committed, and the poster is told it failed.
    Refused,
    /// Committed, and its response lost: the poster is told it failed.
    Lost,
    /// Committed with a successful response that omits its timestamp.
    MissingTimestamp,
    /// Committed only once [`FakeSlack::commit_late`] is called, invisible
    /// until then; the poster is told it failed, as after its own timeout.
    Late,
}

/// One person in Slack's records: their names, address and standing.
#[derive(Debug, Clone)]
pub struct Person {
    pub id: String,
    pub display_name: String,
    pub real_name: String,
    pub email: Option<String>,
    pub team: String,
    pub deleted: bool,
    pub bot: bool,
    pub guest: bool,
    pub is_admin: bool,
}
impl Person {
    /// A full member of the workspace.
    pub fn member(id: &str, display_name: &str, real_name: &str, email: &str) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            real_name: real_name.into(),
            email: Some(email.into()),
            team: TEAM.into(),
            deleted: false,
            bot: false,
            guest: false,
            is_admin: false,
        }
    }
    /// The name Slack shows for them: the display name, the real name when
    /// that is empty, the id when there is neither.
    pub fn shown(&self) -> String {
        [&self.display_name, &self.real_name]
            .into_iter()
            .find(|name| !name.is_empty())
            .cloned()
            .unwrap_or_else(|| self.id.clone())
    }
    fn record(&self) -> Value {
        let mut profile = json!({"display_name": self.display_name, "real_name": self.real_name});
        if let Some(email) = &self.email {
            profile["email"] = json!(email);
        }
        json!({"id": self.id, "team_id": self.team, "name": self.id.to_lowercase(), "deleted": self.deleted, "is_bot": self.bot, "is_restricted": self.guest, "is_ultra_restricted": false, "is_admin": self.is_admin, "real_name": self.real_name, "profile": profile})
    }
}

/// The people of the fake workspace: the agent's bot, three full members
/// and a guest.
fn people() -> std::collections::BTreeMap<String, Person> {
    let mut agent = Person::member(BOT_USER, "agent", "agent", "agent@example.com");
    agent.bot = true;
    let mut stranger = Person::member(
        "USTRANGER",
        "Visitor",
        "Visiting Guest",
        "guest@other.example",
    );
    stranger.guest = true;
    [
        agent,
        Person::member(
            "USTEVEN",
            "Steven",
            "Steven Sonneveld",
            "steven@example.com",
        ),
        Person::member("UANNA", "", "Anna", "anna@example.com"),
        Person::member("UBRAM", "", "", "bram@example.com"),
        stranger,
    ]
    .into_iter()
    .map(|person| (person.id.clone(), person))
    .collect()
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum PostResponse {
    #[default]
    Send,
    Hold,
    Timeout,
}

#[derive(Default)]
struct Workspace {
    /// Slack's records of its people, by id.
    people: std::collections::BTreeMap<String, Person>,
    /// Every status line call: channel, thread and text.
    statuses: Vec<(String, String, String)>,
    /// The status line each place shows now: a call sets it, and a post of
    /// the app's own in that place drops it, as Slack does.
    shown: std::collections::BTreeMap<(String, String), String>,
    /// `users.info` answers that Slack cannot be asked.
    refuse_lookups: bool,
    /// The next `users.info` is limited: HTTP 429 with this `Retry-After`.
    limit_lookup: Option<u64>,
    /// How many times `users.info` was called.
    lookups: usize,
    /// Every message written, in order: `channel`, `ts`, optional `thread_ts`,
    /// `user`, optional `bot_id`, `text`.
    messages: Vec<Value>,
    posts: Vec<Post>,
    rendered_posts: std::collections::BTreeMap<String, String>,
    outgoing: VecDeque<String>,
    acknowledged: Vec<String>,
    connections: usize,
    disconnections: usize,
    next: u64,
    refuse: bool,
    hold_reads: bool,
    hold_later_history_pages: bool,
    refuse_replies: bool,
    read_back_channels: Vec<String>,
    hold_read_back_channel: Option<String>,
    refuse_app_connection: bool,
    scope_header: Option<String>,
    app_token: Option<String>,
    refused_app_connections: usize,
    read_back_limit: Option<(String, u64)>,
    delete_thread_on_scan: Option<(String, String)>,
    deleted_thread_scan_calls: usize,
    unavailable_read_back_channels: std::collections::BTreeSet<String>,
    history_pages: Vec<(String, Option<String>)>,
    hold_reads_from: std::collections::BTreeSet<String>,
    metadata_miss_before_hold: Option<String>,
    refuse_metadata_reads: bool,
    metadata_reads: usize,
    /// A post Slack accepted is answered only once released.
    post_response: PostResponse,
    /// Every post fails, accepting nothing.
    fail_posts: bool,
    /// A post waits until released, then fails, accepting nothing.
    hold_failing_posts: bool,
    /// How many posts waited to fail.
    held_failing_posts: usize,
    /// How many accepted posts were held back.
    held_posts: usize,
    /// `users.info` answers wait until released.
    hold_lookups: bool,
    /// How many lookups were held back.
    held_lookups: usize,
    reads: usize,
    held: usize,
    /// The fate of the next posts beginning with each beginning: how many
    /// more it meets (`usize::MAX` until cleared).
    fates: Vec<(String, Fate, usize)>,
    /// Posts committed late, not yet visible.
    late: Vec<(Value, Post)>,
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
            people: people(),
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
                    // Each request is served on its own, as Slack serves
                    // them: one that is held holds no other.
                    let workspace = Arc::clone(&workspace);
                    let stop = Arc::clone(&stop);
                    let socket_url = socket_url.clone();
                    std::thread::spawn(move || {
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
                        let mut counted = false;
                        while method.starts_with("conversations.")
                            && {
                                let workspace = workspace.lock().unwrap();
                                workspace.hold_reads
                                    || (workspace.hold_later_history_pages
                                        && method == "conversations.history"
                                        && body
                                            .split('&')
                                            .any(|field| field.starts_with("cursor=")))
                                    || (method == "conversations.history"
                                        && workspace.hold_read_back_channel.as_ref().is_some_and(
                                            |channel| {
                                                body == format!("channel={channel}&limit=200")
                                            },
                                        ))
                                    || workspace.hold_reads_from.iter().any(|oldest| {
                                        body.split('&').any(|field| {
                                            field.strip_prefix("oldest=") == Some(oldest)
                                        }) && body
                                            .split('&')
                                            .any(|field| field == "latest=9999999999.999999")
                                    })
                            }
                            && !stop.load(Ordering::Acquire)
                        {
                            if !counted {
                                workspace.lock().unwrap().held += 1;
                                counted = true;
                            }
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        let mut counted = false;
                        while method == "users.info"
                            && workspace.lock().unwrap().hold_lookups
                            && !stop.load(Ordering::Acquire)
                        {
                            if !counted {
                                workspace.lock().unwrap().held_lookups += 1;
                                counted = true;
                            }
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        let limited = {
                            let mut w = workspace.lock().unwrap();
                            if method == "users.info" {
                                w.lookups += 1;
                                w.limit_lookup.take()
                            } else if method == "conversations.history" {
                                let args = form(&body);
                                let channel = args.get("channel").unwrap();
                                let cursor = args.get("cursor").cloned();
                                w.history_pages.push((channel.clone(), cursor.clone()));
                                if cursor.is_some()
                                    && w.read_back_limit
                                        .as_ref()
                                        .is_some_and(|(target, _)| target == channel)
                                {
                                    w.read_back_limit.take().map(|(_, seconds)| seconds)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        };
                        if let Some(seconds) = limited {
                            // As Slack does: HTTP 429, and the wait in Retry-After.
                            let header =
                                tiny_http::Header::from_bytes("Retry-After", seconds.to_string())
                                    .unwrap();
                            request
                                .respond(
                                    tiny_http::Response::from_string(
                                        json!({"ok": false, "error": "ratelimited"}).to_string(),
                                    )
                                    .with_status_code(429)
                                    .with_header(header),
                                )
                                .unwrap();
                            return;
                        }
                        // A post held to fail waits, then is refused, nothing
                        // accepted.
                        if method == "chat.postMessage"
                            && workspace.lock().unwrap().hold_failing_posts
                        {
                            workspace.lock().unwrap().held_failing_posts += 1;
                            while workspace.lock().unwrap().hold_failing_posts
                                && !stop.load(Ordering::Acquire)
                            {
                                std::thread::sleep(Duration::from_millis(20));
                            }
                            let _ = request.respond(tiny_http::Response::from_string(
                                json!({"ok": false, "error": "not_in_channel"}).to_string(),
                            ));
                            return;
                        }
                        let answer =
                            Self::answer(&workspace, &socket_url, &method, &token, &form(&body));
                        // A held post is accepted, and its answer waits: the
                        // poster may be gone by the time it is sent.
                        if method == "chat.postMessage"
                            && workspace.lock().unwrap().post_response == PostResponse::Hold
                        {
                            workspace.lock().unwrap().held_posts += 1;
                            while workspace.lock().unwrap().post_response == PostResponse::Hold
                                && !stop.load(Ordering::Acquire)
                            {
                                std::thread::sleep(Duration::from_millis(20));
                            }
                            let response = if workspace.lock().unwrap().post_response
                                == PostResponse::Timeout
                            {
                                json!({"ok": false, "error": "request_timeout"})
                            } else {
                                answer
                            };
                            let _ = request
                                .respond(tiny_http::Response::from_string(response.to_string()));
                            return;
                        }
                        let mut response = tiny_http::Response::from_string(answer.to_string());
                        if method == "auth.test" {
                            let workspace = workspace.lock().unwrap();
                            let scopes = workspace.scope_header.as_deref().unwrap_or(
                                "app_mentions:read,chat:write,channels:history,channels:read,groups:history,groups:read,im:history,im:read,users:read,users:read.email"
                            );
                            response = response.with_header(tiny_http::Header::from_bytes("x-oauth-scopes", scopes).unwrap());
                        }
                        request.respond(response).unwrap();
                    });
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
                    workspace.lock().unwrap().disconnections += 1;
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
        let accepted = if app {
            w.app_token.as_deref().unwrap_or(APP_TOKEN)
        } else {
            BOT_TOKEN
        };
        if app && w.refuse_app_connection {
            w.refused_app_connections += 1;
            return json!({"ok": false, "error": "invalid_auth"});
        }
        if w.refuse || token != accepted {
            return json!({"ok": false, "error": "invalid_auth"});
        }
        let arg = |name: &str| args.get(name).map(String::as_str);
        let bound = |m: &Value, name: &str, below: bool| {
            arg(name).is_none_or(|edge| {
                let edge: f64 = edge.parse().unwrap();
                if ts(m) == edge && arg("inclusive") == Some("true") {
                    true
                } else if below {
                    ts(m) < edge
                } else {
                    ts(m) > edge
                }
            })
        };
        match method {
            "apps.connections.open" => json!({"ok": true, "url": socket_url}),
            "auth.test" => {
                json!({"ok": true, "user_id": BOT_USER, "bot_id": BOT_ID, "team_id": TEAM})
            }
            "users.conversations" => json!({
                "ok": true,
                "channels": w.read_back_channels.iter()
                    .map(|id| json!({"id": id, "is_im": id.starts_with('D')}))
                    .collect::<Vec<_>>(),
            }),
            "conversations.info" => {
                let channel = arg("channel").unwrap();
                if channel.starts_with('D') {
                    let users = w
                        .messages
                        .iter()
                        .filter(|message| message["channel"].as_str() == Some(channel))
                        .filter_map(|message| message["user"].as_str())
                        .filter(|user| *user != BOT_USER)
                        .collect::<std::collections::BTreeSet<_>>();
                    if users.len() == 1 {
                        json!({"ok":true,"channel":{"id":channel,"is_im":true,"user":users.into_iter().next().unwrap()}})
                    } else {
                        json!({"ok":true,"channel":{"id":channel,"is_im":false,"is_mpim":true}})
                    }
                } else {
                    json!({"ok":true,"channel":{"id":channel,"name":channel.to_ascii_lowercase()}})
                }
            }
            "users.info" if w.refuse_lookups => json!({"ok": false, "error": "ratelimited"}),
            "users.info" => match w.people.get(arg("user").unwrap()) {
                Some(person) => json!({"ok": true, "user": person.record()}),
                None => json!({"ok": false, "error": "user_not_found"}),
            },
            "assistant.threads.setStatus" => {
                let call = (
                    arg("channel_id").unwrap().to_owned(),
                    arg("thread_ts").unwrap().to_owned(),
                    arg("status").unwrap_or_default().to_owned(),
                );
                w.shown
                    .insert((call.0.clone(), call.1.clone()), call.2.clone());
                w.statuses.push(call);
                json!({"ok": true})
            }
            "conversations.history" | "conversations.replies" => {
                if method == "conversations.replies"
                    && arg("latest").is_none()
                    && w.delete_thread_on_scan
                        .as_ref()
                        .is_some_and(|(channel, parent)| {
                            arg("channel") == Some(channel) && arg("ts") == Some(parent)
                        })
                {
                    w.deleted_thread_scan_calls += 1;
                    if w.deleted_thread_scan_calls == 1 {
                        let (channel, parent) = w.delete_thread_on_scan.clone().unwrap();
                        w.messages.retain(|message| {
                            message["channel"] != channel
                                || (message["ts"] != parent && message["thread_ts"] != parent)
                        });
                    }
                    return json!({"ok": false, "error": "thread_not_found"});
                }

                if method == "conversations.history"
                    && arg("limit") == Some("200")
                    && arg("channel")
                        .is_some_and(|channel| w.unavailable_read_back_channels.contains(channel))
                {
                    return json!({"ok": false, "error": "channel_not_found"});
                }

                if method == "conversations.replies" && w.refuse_replies {
                    return json!({"ok": false, "error": "thread_not_found"});
                }
                if arg("include_all_metadata") == Some("true") {
                    w.metadata_reads += 1;
                    if w.refuse_metadata_reads {
                        return json!({"ok": false, "error": "missing_scope"});
                    }
                }
                let channel = arg("channel").unwrap();
                let thread = arg("ts");
                let mut found: Vec<Value> = w
                    .messages
                    .iter()
                    .filter(|m| m["channel"] == channel)
                    .filter(|m| match thread {
                        Some(root) => m["ts"] == root || m["thread_ts"] == root,
                        None => {
                            m["thread_ts"].is_null()
                                || m["thread_ts"] == m["ts"]
                                || m["subtype"] == "thread_broadcast"
                        }
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
                let mut page: Vec<Value> = found.iter().skip(from).take(size).cloned().collect();
                if thread.is_none() && w.read_back_channels.iter().any(|id| id == channel) {
                    for parent in &mut page {
                        let latest = w
                            .messages
                            .iter()
                            .filter(|message| {
                                message["channel"] == channel
                                    && message["thread_ts"] == parent["ts"]
                                    && message["ts"] != parent["ts"]
                            })
                            .max_by(|a, b| ts(a).total_cmp(&ts(b)));
                        if let Some(latest) = latest {
                            parent["latest_reply"] = latest["ts"].clone();
                        }
                    }
                }
                let include_metadata = arg("include_all_metadata") == Some("true");
                let miss = include_metadata
                    && w.metadata_miss_before_hold
                        .as_deref()
                        .is_some_and(|oldest| Some(oldest) == arg("oldest"));
                if miss && let Some(oldest) = w.metadata_miss_before_hold.take() {
                    w.hold_reads_from.insert(oldest);
                }
                if !include_metadata || miss {
                    for message in &mut page {
                        if let Some(metadata) = message.get_mut("metadata") {
                            *metadata = json!({"event_type": metadata["event_type"]});
                        }
                    }
                }
                let more = from + size < found.len();
                w.reads += 1;
                json!({"ok": true, "messages": page, "has_more": more, "response_metadata": {"next_cursor": if more { (from + size).to_string() } else { String::new() }}})
            }
            // As Slack does: Markdown goes alone, never together with `text`
            // or `blocks`, and a post carries one of them.
            "chat.postMessage"
                if arg("markdown_text").is_some()
                    && (arg("text").is_some() || arg("blocks").is_some()) =>
            {
                json!({"ok": false, "error": "markdown_text_conflict"})
            }
            "chat.postMessage" if arg("markdown_text").is_none() && arg("text").is_none() => {
                json!({"ok": false, "error": "no_text"})
            }
            "chat.postMessage" if w.fail_posts => {
                json!({"ok": false, "error": "not_in_channel"})
            }
            "conversations.open" => {
                let user = arg("users").unwrap_or_default();
                json!({"ok": true, "channel": {"id": format!("DM-{user}")}})
            }
            "chat.postMessage" => {
                let post = Post {
                    channel: arg("channel").unwrap().to_owned(),
                    thread: arg("thread_ts").map(str::to_owned),
                    text: arg("text").map(str::to_owned),
                    markdown: arg("markdown_text").map(str::to_owned),
                };
                let shown = post
                    .markdown
                    .clone()
                    .or(post.text.clone())
                    .unwrap_or_default();
                let fate = w
                    .fates
                    .iter_mut()
                    .find(|(beginning, _, left)| *left > 0 && shown.starts_with(beginning.as_str()))
                    .map(|(_, fate, left)| {
                        *left = left.saturating_sub(usize::from(*left != usize::MAX));
                        *fate
                    });
                if fate == Some(Fate::Refused) {
                    return json!({"ok": false, "error": "not_in_channel"});
                }
                let ts = Self::posted(&mut w);
                let rendered = w.rendered_posts.get(&shown).unwrap_or(&shown);
                let mut message = json!({"channel": post.channel, "ts": ts, "user": BOT_USER, "bot_id": BOT_ID, "text": rendered});
                if let Some(metadata) = arg("metadata") {
                    message["metadata"] = serde_json::from_str(metadata).unwrap();
                }
                if let Some(thread) = &post.thread {
                    message["thread_ts"] = json!(thread);
                }
                if fate == Some(Fate::Late) {
                    w.late.push((message, post));
                    return json!({"ok": false, "error": "request_timeout"});
                }
                w.messages.push(message);
                // Slack drops the app's status in the place it posts to: the
                // thread it answers in, or the direct message.
                w.shown.retain(|(channel, thread), _| {
                    channel != &post.channel
                        || post.thread.as_ref().is_some_and(|posted| posted != thread)
                });
                w.posts.push(post);
                if fate == Some(Fate::Lost) {
                    return json!({"ok": false, "error": "request_timeout"});
                }
                if fate == Some(Fate::MissingTimestamp) {
                    return json!({"ok": true});
                }
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

    /// The time of a post of the app's own: after every message the
    /// workspace holds, those given an explicit time among them.
    fn posted(w: &mut Workspace) -> String {
        let micros = |ts: &str| -> i64 {
            let (seconds, fraction) = ts.split_once('.').unwrap();
            seconds.parse::<i64>().unwrap() * 1_000_000 + fraction.parse::<i64>().unwrap()
        };
        let stamp = Self::stamp(w);
        let latest = w
            .messages
            .iter()
            .filter_map(|m| m["ts"].as_str())
            .map(micros)
            .max()
            .unwrap_or(0);
        let at = micros(&stamp).max(latest + 100);
        format!("{}.{:06}", at / 1_000_000, at % 1_000_000)
    }

    pub fn disconnections(&self) -> usize {
        self.workspace.lock().unwrap().disconnections
    }

    pub fn scopes(&self, scopes: Option<&str>) {
        self.workspace.lock().unwrap().scope_header = scopes.map(str::to_owned);
    }

    pub fn app_token(&self, token: &str) {
        self.workspace.lock().unwrap().app_token = Some(token.to_owned());
    }

    pub fn refuse_app_connection(&self, refuse: bool) {
        self.workspace.lock().unwrap().refuse_app_connection = refuse;
    }

    pub fn refused_app_connections(&self) -> usize {
        self.workspace.lock().unwrap().refused_app_connections
    }

    pub fn delete_thread_on_scan(&self, channel: &str, parent: &str) {
        self.workspace.lock().unwrap().delete_thread_on_scan =
            Some((channel.to_owned(), parent.to_owned()));
    }

    pub fn deleted_thread_scan_calls(&self) -> usize {
        self.workspace.lock().unwrap().deleted_thread_scan_calls
    }

    pub fn unavailable_for_read_back(&self, channel: &str, unavailable: bool) {
        let mut workspace = self.workspace.lock().unwrap();
        if unavailable {
            workspace
                .unavailable_read_back_channels
                .insert(channel.to_owned());
        } else {
            workspace.unavailable_read_back_channels.remove(channel);
        }
    }

    pub fn throttle_later_history_page(&self, channel: &str, seconds: u64) {
        self.workspace.lock().unwrap().read_back_limit = Some((channel.to_owned(), seconds));
    }

    pub fn history_pages(&self, channel: &str) -> Vec<Option<String>> {
        self.workspace
            .lock()
            .unwrap()
            .history_pages
            .iter()
            .filter(|(seen, _)| seen == channel)
            .map(|(_, cursor)| cursor.clone())
            .collect()
    }

    pub fn broadcast(&self, channel: &str, ts: &str) {
        let mut workspace = self.workspace.lock().unwrap();
        let message = workspace
            .messages
            .iter_mut()
            .find(|message| message["channel"] == channel && message["ts"] == ts)
            .unwrap();
        message["subtype"] = json!("thread_broadcast");
    }

    pub fn hold_read_back_channel(&self, channel: Option<&str>) {
        self.workspace.lock().unwrap().hold_read_back_channel = channel.map(str::to_owned);
    }

    pub fn read_back_channels(&self, channels: &[&str]) {
        self.workspace.lock().unwrap().read_back_channels = channels
            .iter()
            .map(|channel| (*channel).to_owned())
            .collect();
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
        self.notify_mention(&ts, channel, user, text, thread);
        ts
    }

    /// Deliver the tag event for a message already visible through Slack's API.
    pub fn notify_mention(
        &self,
        ts: &str,
        channel: &str,
        user: &str,
        text: &str,
        thread: Option<&str>,
    ) {
        let mut event = json!({"type": "app_mention", "channel": channel, "user": user, "text": text, "ts": ts, "event_ts": ts});
        if let Some(thread) = thread {
            event["thread_ts"] = json!(thread);
        }
        self.deliver(&format!("envelope-{ts}"), event);
    }

    /// A person writes to the agent in a direct message.
    pub fn direct(&self, channel: &str, user: &str, text: &str) -> String {
        self.direct_as(channel, user, text, None, false)
    }

    /// A direct message Slack sends with `subtype`, such as `me_message` for
    /// one written with `/me`, carrying a file when `file` is set.
    pub fn direct_as(
        &self,
        channel: &str,
        user: &str,
        text: &str,
        subtype: Option<&str>,
        file: bool,
    ) -> String {
        let ts = self.say(channel, user, text, None);
        let mut event = json!({"type": "message", "channel_type": "im", "channel": channel, "user": user, "text": text, "ts": ts, "event_ts": ts});
        Self::dress(&mut event, subtype, file);
        self.dress_stored(&ts, subtype, file);
        self.deliver(&format!("envelope-{ts}"), event);
        ts
    }

    /// The person edits their direct message `ts` to `text`: Slack sends a
    /// record of the change, as `message_changed`, carrying the message.
    pub fn edit_direct(&self, channel: &str, user: &str, ts: &str, text: &str) {
        let at = self.rewrite(ts, text);
        let message = json!({"type": "message", "user": user, "text": text, "ts": ts, "edited": {"user": user, "ts": at}});
        self.deliver(
            &format!("envelope-{at}"),
            json!({"type": "message", "subtype": "message_changed", "hidden": true, "channel_type": "im", "channel": channel, "ts": at, "event_ts": at, "message": message}),
        );
    }

    /// The person deletes their direct message `ts`: Slack sends a record of
    /// the deletion, as `message_deleted`.
    pub fn delete_direct(&self, channel: &str, ts: &str) {
        let at = {
            let mut w = self.workspace.lock().unwrap();
            w.messages
                .retain(|m| m["channel"] != channel || m["ts"] != ts);
            Self::stamp(&mut w)
        };
        self.deliver(
            &format!("envelope-{at}"),
            json!({"type": "message", "subtype": "message_deleted", "hidden": true, "channel_type": "im", "channel": channel, "ts": at, "event_ts": at, "deleted_ts": ts}),
        );
    }

    /// A person tags the agent in a channel in a message that carries a
    /// file, which Slack records as `file_share`.
    pub fn mention_with_file(
        &self,
        channel: &str,
        user: &str,
        text: &str,
        thread: Option<&str>,
    ) -> String {
        let ts = self.say(channel, user, text, thread);
        let mut event = json!({"type": "app_mention", "channel": channel, "user": user, "text": text, "ts": ts, "event_ts": ts});
        if let Some(thread) = thread {
            event["thread_ts"] = json!(thread);
        }
        Self::dress(&mut event, Some("file_share"), true);
        self.dress_stored(&ts, Some("file_share"), true);
        self.deliver(&format!("envelope-{ts}"), event);
        ts
    }

    /// A person edits their channel message `ts` so that it tags the agent:
    /// Slack sends the mention marked as edited.
    pub fn edit_into_mention(
        &self,
        channel: &str,
        user: &str,
        ts: &str,
        text: &str,
        thread: Option<&str>,
    ) {
        let at = self.rewrite(ts, text);
        let mut event = json!({"type": "app_mention", "channel": channel, "user": user, "text": text, "ts": ts, "event_ts": at, "edited": {"user": user, "ts": at}});
        if let Some(thread) = thread {
            event["thread_ts"] = json!(thread);
        }
        self.deliver(&format!("envelope-{at}"), event);
    }

    /// Changes the stored message `ts` to `text`, marked as edited, and
    /// returns the time of the edit.
    fn rewrite(&self, ts: &str, text: &str) -> String {
        let mut w = self.workspace.lock().unwrap();
        let at = Self::stamp(&mut w);
        let message = w.messages.iter_mut().find(|m| m["ts"] == ts).unwrap();
        message["text"] = json!(text);
        message["edited"] = json!({"user": message["user"].clone(), "ts": at});
        at
    }

    /// Gives an event or a message its subtype and, for a file, the files
    /// Slack lists with it.
    fn dress(message: &mut Value, subtype: Option<&str>, file: bool) {
        if let Some(subtype) = subtype {
            message["subtype"] = json!(subtype);
        }
        if file {
            message["files"] =
                json!([{"id": "F1", "name": "invoice.pdf", "mimetype": "application/pdf"}]);
            message["upload"] = json!(true);
        }
    }

    fn dress_stored(&self, ts: &str, subtype: Option<&str>, file: bool) {
        let mut w = self.workspace.lock().unwrap();
        let message = w.messages.iter_mut().find(|m| m["ts"] == ts).unwrap();
        Self::dress(message, subtype, file);
    }

    /// One of Slack's own notices in a channel, such as a person joining it.
    pub fn notice(
        &self,
        channel: &str,
        subtype: &str,
        user: &str,
        text: &str,
        thread: Option<&str>,
    ) -> String {
        let mut w = self.workspace.lock().unwrap();
        let ts = Self::stamp(&mut w);
        let mut message =
            json!({"channel": channel, "ts": ts, "user": user, "subtype": subtype, "text": text});
        if let Some(thread) = thread {
            message["thread_ts"] = json!(thread);
        }
        w.messages.push(message);
        ts
    }

    /// Slack's record of the person `id`.
    pub fn person(&self, id: &str) -> Person {
        self.workspace.lock().unwrap().people[id].clone()
    }

    /// Changes Slack's record of the person `id`.
    pub fn change(&self, id: &str, change: impl FnOnce(&mut Person)) {
        change(self.workspace.lock().unwrap().people.get_mut(id).unwrap());
    }

    /// Whether `users.info` answers that Slack cannot be asked.
    pub fn refuse_lookups(&self, refuse: bool) {
        self.workspace.lock().unwrap().refuse_lookups = refuse;
    }

    /// The next `users.info` is limited by Slack for `seconds`.
    pub fn limit_lookup(&self, seconds: u64) {
        self.workspace.lock().unwrap().limit_lookup = Some(seconds);
    }

    /// How many times `users.info` was called.
    pub fn lookups(&self) -> usize {
        self.workspace.lock().unwrap().lookups
    }

    /// The status line Slack shows on a channel's thread, or on a direct
    /// message's message: the last text set there, `None` once cleared or
    /// dropped by a post of the app's own in that place.
    pub fn status(&self, channel: &str, thread: &str) -> Option<String> {
        self.workspace
            .lock()
            .unwrap()
            .shown
            .get(&(channel.to_owned(), thread.to_owned()))
            .cloned()
            .filter(|text| !text.is_empty())
    }

    /// How many times a status was set or cleared on a channel's thread or
    /// a direct message's message.
    pub fn status_calls(&self, channel: &str, thread: &str) -> usize {
        self.workspace
            .lock()
            .unwrap()
            .statuses
            .iter()
            .filter(|(c, t, _)| c == channel && t == thread)
            .count()
    }

    /// Every text set as the status line on a channel's thread or a direct
    /// message's message, in order; a clearing is an empty text.
    pub fn status_texts(&self, channel: &str, thread: &str) -> Vec<String> {
        self.workspace
            .lock()
            .unwrap()
            .statuses
            .iter()
            .filter(|(c, t, _)| c == channel && t == thread)
            .map(|(_, _, text)| text.clone())
            .collect()
    }

    /// Every later post waits until [`FakeSlack::fail_held_posts`], then
    /// fails, accepting nothing.
    pub fn hold_posts_to_fail(&self) {
        self.workspace.lock().unwrap().hold_failing_posts = true;
    }

    pub fn fail_held_posts(&self) {
        self.workspace.lock().unwrap().hold_failing_posts = false;
    }

    /// How many posts waited to fail.
    pub fn held_failing_posts(&self) -> usize {
        self.workspace.lock().unwrap().held_failing_posts
    }

    /// Every later post fails, until [`FakeSlack::accept_posts`].
    pub fn fail_posts(&self) {
        self.workspace.lock().unwrap().fail_posts = true;
    }

    pub fn accept_posts(&self) {
        self.workspace.lock().unwrap().fail_posts = false;
    }

    /// Sets whether Slack reports `user` as a workspace administrator.
    pub fn set_admin(&self, user: &str, is_admin: bool) {
        self.workspace
            .lock()
            .unwrap()
            .people
            .get_mut(user)
            .unwrap()
            .is_admin = is_admin;
    }

    /// Lookups of people wait until [`FakeSlack::release_lookups`].
    pub fn hold_lookups(&self) {
        self.workspace.lock().unwrap().hold_lookups = true;
    }

    pub fn release_lookups(&self) {
        self.workspace.lock().unwrap().hold_lookups = false;
    }

    /// How many lookups were held back while lookups were held.
    pub fn held_lookups(&self) -> usize {
        self.workspace.lock().unwrap().held_lookups
    }

    /// History and replies reads wait until [`FakeSlack::release_reads`].
    /// Every later post is accepted, and its answer held until released.
    pub fn hold_posts(&self) {
        self.workspace.lock().unwrap().post_response = PostResponse::Hold;
    }

    pub fn release_posts(&self) {
        self.workspace.lock().unwrap().post_response = PostResponse::Send;
    }

    /// End accepted, held posts with a provider timeout on demand.
    pub fn time_out_held_posts(&self) {
        self.workspace.lock().unwrap().post_response = PostResponse::Timeout;
    }

    /// How many accepted posts had their answer held.
    pub fn held_posts(&self) -> usize {
        self.workspace.lock().unwrap().held_posts
    }

    pub fn refuse_replies(&self, refused: bool) {
        self.workspace.lock().unwrap().refuse_replies = refused;
    }

    /// Keep the first history page available and hold continuations.
    pub fn hold_later_history_pages(&self) {
        self.workspace.lock().unwrap().hold_later_history_pages = true;
    }

    pub fn hold_reads(&self) {
        self.workspace.lock().unwrap().hold_reads = true;
    }

    pub fn refuse_metadata_reads(&self) {
        self.workspace.lock().unwrap().refuse_metadata_reads = true;
    }

    pub fn metadata_reads(&self) -> usize {
        self.workspace.lock().unwrap().metadata_reads
    }

    /// Omit the receipt on the first metadata read and hold later reads
    /// of the same range until the test releases them.
    pub fn miss_metadata_read_before_holding(&self, oldest: &str) {
        self.workspace.lock().unwrap().metadata_miss_before_hold = Some(oldest.into());
    }

    pub fn hold_reads_from(&self, oldest: &str) {
        self.workspace
            .lock()
            .unwrap()
            .hold_reads_from
            .insert(oldest.into());
    }

    pub fn release_reads_from(&self, oldest: &str) {
        self.workspace
            .lock()
            .unwrap()
            .hold_reads_from
            .remove(oldest);
    }

    pub fn release_reads(&self) {
        let mut workspace = self.workspace.lock().unwrap();
        workspace.hold_reads = false;
        workspace.hold_later_history_pages = false;
        workspace.hold_reads_from.clear();
    }

    /// How many reads were held back while reads were held.
    pub fn held(&self) -> usize {
        self.workspace.lock().unwrap().held
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

    /// What Slack returns as text for the submitted Markdown.
    pub fn render_post(&self, submitted: &str, returned: &str) {
        self.workspace
            .lock()
            .unwrap()
            .rendered_posts
            .insert(submitted.into(), returned.into());
    }

    /// The next `times` posts beginning with `beginning` meet `fate`
    /// (`usize::MAX`: every one, until [`FakeSlack::clear_fates`]).
    pub fn fate(&self, beginning: &str, fate: Fate, times: usize) {
        self.workspace
            .lock()
            .unwrap()
            .fates
            .push((beginning.to_owned(), fate, times));
    }

    pub fn clear_fates(&self) {
        self.workspace.lock().unwrap().fates.clear();
    }

    /// Every post committed late becomes visible, as it lands at Slack.
    pub fn commit_late(&self) {
        let mut w = self.workspace.lock().unwrap();
        for (message, post) in std::mem::take(&mut w.late) {
            w.messages.push(message);
            w.posts.push(post);
        }
    }

    /// The posts that begin with `beginning`, as Slack shows them.
    pub fn posts_beginning(&self, beginning: &str) -> usize {
        self.posts()
            .iter()
            .filter(|post| {
                post.markdown
                    .as_deref()
                    .or(post.text.as_deref())
                    .is_some_and(|text| text.starts_with(beginning))
            })
            .count()
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
