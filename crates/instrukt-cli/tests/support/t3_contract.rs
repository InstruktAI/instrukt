//! Release-only fixture: all runtime state and child processes belong to one temporary home.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "contract fixture failures fail its single acceptance test"
)]
use crate::common::{self, TestEnv};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use instrukt_core::{home::Home, ids::SessionRef};
use instrukt_runner::{
    process,
    runtime::{InputState, RuntimeAdapter, T3},
    sessions::SessionState,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tungstenite::{Message, client::IntoClientRequest};

pub const TEST: &str = "t3_contract::a_release_and_its_fake_are_checked_without_a_model_account";
pub const VERSION: &str = include_str!("../../../instrukt-runner/t3.version");
const BOUND: Duration = Duration::from_secs(20);

pub fn download(env: &TestEnv, version: &str) -> Result<PathBuf> {
    ensure!(
        !version.is_empty()
            && version
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c)),
        "invalid release version"
    );
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x64",
        ("macos", "aarch64") => "darwin-arm64",
        _ => bail!("unsupported contract host"),
    };
    let asset = format!("t3-{version}-{platform}.tar.gz");
    for name in [asset.as_str(), "SHA256SUMS"] {
        let mut command = env.command(&common::tool_executable("curl"), env.home());
        command
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-time",
                "120",
                "--output",
            ])
            .arg(env.path(name))
            .arg(format!(
                "https://github.com/pingdotgg/t3code/releases/download/v{version}/{name}"
            ));
        ensure!(
            process::output(&mut command, None, Duration::from_secs(130))?
                .status
                .success(),
            "release asset acquisition failed: {name}"
        );
    }
    let sums = fs::read_to_string(env.path("SHA256SUMS"))?;
    let matches: Vec<_> = sums
        .lines()
        .filter_map(|line| line.split_once("  "))
        .filter(|(_, name)| *name == asset)
        .collect();
    ensure!(matches.len() == 1, "checksum entry is missing or ambiguous");
    let digest = hex::encode(Sha256::digest(fs::read(env.path(&asset))?));
    ensure!(digest == matches[0].0, "release checksum mismatch");
    let mut tar = env.command(&common::tool_executable("tar"), env.home());
    tar.args(["-xzf"])
        .arg(env.path(&asset))
        .arg("-C")
        .arg(env.home());
    ensure!(
        process::output(&mut tar, None, BOUND)?.status.success(),
        "release extraction failed"
    );
    let binary = env.home().join(format!("t3-{version}-{platform}/t3"));
    ensure!(binary.is_file(), "release executable is missing");
    fs::write(
        env.path("release.json"),
        json!({"version":version,"archive":asset,"sha256":digest,"platform":platform}).to_string(),
    )?;
    Ok(binary)
}

pub fn worker(env: &TestEnv, stage: &str, root: &Path) -> Result<()> {
    let mut command = env.command(&std::env::current_exe()?, root);
    command
        .args(["--exact", TEST, "--nocapture"])
        .env("T3_CONTRACT_STAGE", stage)
        .env("T3_CONTRACT_ROOT", root);
    let output = process::output(&mut command, None, Duration::from_secs(100))?;
    // Worker diagnostics identify operations, never credentials or conversation text.
    ensure!(
        output.status.success(),
        "{stage} contract worker failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

pub fn cli(args: &[&str], input: Option<&str>) -> Result<String> {
    let mut invocation = process::Invocation::program("t3", args);
    if let Some(input) = input {
        invocation = invocation.input(input.as_bytes());
    }
    let output = invocation.run(BOUND)?;
    ensure!(
        output.success,
        "T3 command failed at {}",
        args.first().unwrap_or(&"CLI")
    );
    Ok(output.stdout)
}

pub struct Server {
    child: Child,
}
impl Server {
    pub fn start(root: &Path) -> Result<Self> {
        let base = std::env::var("T3CODE_HOME")?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);
        let log = fs::File::create(root.join("server-private.log"))?;
        let mut command = Command::new("t3");
        command
            .args([
                "--base-dir",
                &base,
                "--no-browser",
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--auto-bootstrap-project-from-cwd",
            ])
            .arg(root.join("project"))
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        // The server inherits the worker group owned by the parent test.
        let child = command.spawn()?;
        let mut server = Self { child };
        common::wait_until("isolated T3 descriptor", BOUND, || {
            if server.child.try_wait().unwrap().is_some() {
                panic!(
                    "isolated T3 stopped before readiness; private log retained in its temporary fixture"
                );
            }
            Path::new(&base)
                .join("userdata/server-runtime.json")
                .exists()
        });
        Ok(server)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(pid) = rustix::process::Pid::from_raw(self.child.id() as i32)
            && let Err(error) = rustix::process::kill_process(pid, rustix::process::Signal::TERM)
            && error != rustix::io::Errno::SRCH
        {
            eprintln!("T3 termination failed: {error}");
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if matches!(self.child.try_wait(), Ok(None)) {
            self.child.kill().expect("owned T3 process cleanup");
        }
        self.child.wait().expect("owned T3 reap");
    }
}

pub struct Token {
    pub secret: String,
    id: String,
    revoked: bool,
}
impl Token {
    pub fn issue(operate: bool) -> Result<Self> {
        let mut args = vec![
            "auth",
            "session",
            "issue",
            "--scope",
            "orchestration:read",
            "--ttl",
            "5m",
            "--token-only",
            "--label",
            "instrukt-contract",
        ];
        if operate {
            args.extend(["--scope", "orchestration:operate"]);
        }
        let secret = cli(&args, None)?.trim().to_owned();
        let payload = secret.split_once('.').context("token envelope")?.0;
        let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload)?)?;
        Ok(Self {
            secret,
            id: claims["sid"].as_str().context("revocable session")?.into(),
            revoked: false,
        })
    }
    pub fn revoke(&mut self) -> Result<()> {
        cli(&["auth", "session", "revoke", "--", &self.id], None)?;
        self.revoked = true;
        Ok(())
    }
}
impl Drop for Token {
    fn drop(&mut self) {
        if !self.revoked && self.revoke().is_err() {
            eprintln!(
                "temporary session revocation failed; disposable server shutdown invalidates it"
            );
        }
    }
}

pub fn connect(token: &str) -> Result<tungstenite::WebSocket<TcpStream>> {
    let descriptor: Value = serde_json::from_slice(&fs::read(
        Path::new(&std::env::var("T3CODE_HOME")?).join("userdata/server-runtime.json"),
    )?)?;
    let port = descriptor["port"].as_u64().context("descriptor port")?;
    let address = format!("127.0.0.1:{port}").parse()?;
    let stream = TcpStream::connect_timeout(&address, BOUND)?;
    stream.set_read_timeout(Some(BOUND))?;
    stream.set_write_timeout(Some(BOUND))?;
    let mut request =
        format!("ws://127.0.0.1:{port}/ws?orchestrationProtocol=2").into_client_request()?;
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {token}").parse()?);
    tungstenite::client(request, stream)
        .map(|(socket, _)| socket)
        .map_err(|_| anyhow::anyhow!("T3 WebSocket authorization/negotiation refused"))
}

pub fn rpc(token: &str, tag: &str, payload: Value) -> Result<Value> {
    let mut socket = connect(token)?;
    socket.send(Message::Text(
        json!({"_tag":"Request","id":"contract","tag":tag,"payload":payload,"headers":[]})
            .to_string()
            .into(),
    ))?;
    let deadline = Instant::now() + BOUND;
    for _ in 0..32 {
        ensure!(Instant::now() < deadline, "RPC deadline: {tag}");
        let frame = socket.read()?;
        let Message::Text(text) = frame else { continue };
        let frame: Value = serde_json::from_str(&text)?;
        let frames = frame.as_array().cloned().unwrap_or_else(|| vec![frame]);
        for frame in frames {
            if frame["_tag"] == "Ping" {
                socket.send(Message::Text(json!({"_tag":"Pong"}).to_string().into()))?;
            } else if frame["_tag"] == "Exit" && frame["requestId"] == "contract" {
                ensure!(frame["exit"]["_tag"] == "Success", "RPC rejected at {tag}");
                return Ok(frame["exit"]["value"].clone());
            }
        }
    }
    bail!("RPC frame bound: {tag}")
}

pub fn observe(id: &str) -> Result<Value> {
    let adapter = T3::default();
    let session = SessionRef::parse(&format!("t3:{id}"))?;
    let state = adapter.read_state(&session)?;
    ensure!(
        state.state == SessionState::Running,
        "preparing must be operational running"
    );
    let now = "2026-11-10T12:00:00Z".parse()?;
    let deferred = adapter.observe_input(&session, "contract deferred input", now)?;
    let queued = adapter.observe_input(&session, "contract queued input", now)?;
    ensure!(
        deferred.state == InputState::Accepted && queued.state == InputState::Accepted,
        "allocation must not prove input entry"
    );
    let listed = adapter.list(&std::env::current_dir()?.join("project"))?;
    ensure!(
        listed.iter().any(|thread| thread.session == session),
        "project listing omitted the fixture thread"
    );
    ensure!(
        adapter
            .read_state(&SessionRef::parse("t3:contract-missing")?)?
            .state
            == SessionState::Gone,
        "unknown thread meaning"
    );
    let conversation = adapter.read_conversation(&session)?;
    ensure!(
        conversation.messages.len() == 1,
        "queued input must remain outside the recent transcript"
    );
    let deferred_run = deferred
        .reference
        .as_deref()
        .context("deferred input has no allocated run")?;
    let queued_run = queued
        .reference
        .as_deref()
        .context("queued input has no allocated run")?;
    ensure!(
        deferred_run != queued_run,
        "independent queued input reused the opening run"
    );
    let messages = conversation
        .messages
        .iter()
        .map(|message| -> Result<Value> {
            Ok(json!({"role": message.role, "text": message.text,
            "run": canonical_run(message.run_id.as_deref(), deferred_run, queued_run)?}))
        })
        .collect::<Result<Vec<_>>>()?;
    let inputs = json!({
        "deferred": {"state": input_state(deferred.state), "run": canonical_run(deferred.reference.as_deref(), deferred_run, queued_run)?},
        "queued": {"state": input_state(queued.state), "run": canonical_run(queued.reference.as_deref(), deferred_run, queued_run)?}
    });
    Ok(
        json!({"operational_running":true,"project_listed":true,"unknown_gone":true,
        "title_present":state.title.is_some(),"project_present":state.project_id.is_some(),
        "activity_present":state.activity.is_some(),"provider":state.provider,"model":state.model,
        "settled":state.settled,"worktree_present":state.worktree.is_some(),"branch_present":state.branch.is_some(),"title":state.title,
        "transcript":{"session":conversation.session.to_string(),"limit":conversation.limit,"bounded":conversation.bounded,"messages":messages},
        "inputs":inputs,"relationships":input_relationships(id,deferred_run,queued_run)?}),
    )
}

fn input_state(state: InputState) -> &'static str {
    match state {
        InputState::Unknown => "unknown",
        InputState::Accepted => "accepted",
        InputState::Consumed => "consumed",
    }
}

fn canonical_run(reference: Option<&str>, deferred: &str, queued: &str) -> Result<Value> {
    match reference {
        None => Ok(Value::Null),
        Some(id) if id == deferred => Ok(json!("deferred-run")),
        Some(id) if id == queued => Ok(json!("queued-run")),
        Some(_) => bail!("input/transcript references an unrelated run"),
    }
}

fn input_database() -> Result<instrukt_core::state_db::rusqlite::Connection> {
    Ok(
        instrukt_core::state_db::rusqlite::Connection::open_with_flags(
            Path::new(&std::env::var("T3CODE_HOME")?).join("userdata/statev2.sqlite"),
            instrukt_core::state_db::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?,
    )
}

fn input_relationships(thread: &str, deferred: &str, queued: &str) -> Result<Value> {
    let db = input_database()?;
    let mut statement = db.prepare("SELECT m.message_id,m.payload_json,r.payload_json FROM orchestration_v2_projection_messages m LEFT JOIN orchestration_v2_projection_runs r ON r.thread_id=m.thread_id AND r.run_id=json_extract(m.payload_json,'$.runId') WHERE m.thread_id=?1 AND m.role='user' ORDER BY m.rowid")?;
    let rows = statement
        .query_map([thread], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    ensure!(
        rows.len() == 2,
        "persisted input set differs from the two accepted inputs"
    );
    let mut identities = std::collections::BTreeMap::new();
    for (id, message, _) in &rows {
        let message: Value = serde_json::from_str(message)?;
        let label = match message["runId"].as_str() {
            Some(id) if id == deferred => "deferred-input",
            Some(id) if id == queued => "queued-input",
            _ => bail!("persisted input references an unrelated run"),
        };
        identities.insert(id.clone(), json!(label));
    }
    let mut relationships = Vec::new();
    for (_, message, run) in rows {
        let message: Value = serde_json::from_str(&message)?;
        let run: Value = serde_json::from_str(&run)?;
        let message_id = message["id"]
            .as_str()
            .context("persisted message identity")?;
        let opening = run["userMessageId"]
            .as_str()
            .context("persisted opening identity")?;
        let entry = if run["startedAt"].is_null() {
            Value::Null
        } else {
            run["startedAt"]
                .as_str()
                .context("entry timestamp shape")?
                .parse::<jiff::Timestamp>()?;
            json!("<timestamp>")
        };
        relationships.push(json!({
            "message":{"id":identities.get(message_id).context("unrelated persisted message identity")?,"thread":message["threadId"],"role":message["role"],"text":message["text"],"run":canonical_run(message["runId"].as_str(),deferred,queued)?},
            "run":{"id":canonical_run(run["id"].as_str(),deferred,queued)?,"thread":run["threadId"],"opening_input":identities.get(opening).context("unrelated opening input")?,"status":run["status"],"startedAt":entry}
        }));
    }
    Ok(json!(relationships))
}

pub fn readiness() -> Result<Value> {
    let mut token = Token::issue(false)?;
    let config = rpc(&token.secret, "server.getConfig", json!({}))?;
    let providers = config["providers"]
        .as_array()
        .context("native provider array")?;
    token.revoke()?;
    ensure!(
        connect(&token.secret).is_err(),
        "readiness credential revocation did not deny reuse"
    );
    let translated = T3::default().provider_snapshot(&Home::from_env()?)?;
    ensure!(
        providers.len() == translated.providers.len(),
        "readiness lost a provider instance"
    );
    let mut unready = 0;
    for raw in providers {
        let id = raw["instanceId"]
            .as_str()
            .context("native provider identity")?;
        let facts = translated
            .providers
            .iter()
            .find(|facts| facts.instance.as_str() == id)
            .context("translated provider identity")?;
        let explicitly_unready = raw["enabled"] == false
            || raw["installed"] == false
            || matches!(raw["status"].as_str(), Some("disabled" | "error"))
            || raw["auth"]["status"] == "unauthenticated"
            || raw["availability"] == "unavailable"
            || (raw["driver"] == "claudeAgent"
                && raw["status"] == "warning"
                && raw["auth"]["status"] == "unknown");
        if explicitly_unready {
            unready += 1;
            ensure!(
                facts
                    .unavailable_reason
                    .as_ref()
                    .is_some_and(|reason| !reason.is_empty()),
                "readiness promoted an explicitly unready provider to available: {id}"
            );
        }
    }
    ensure!(
        unready > 0,
        "provider-free readiness produced no explicitly unready evidence"
    );
    let mut profiles=translated.providers.iter().map(|facts| json!({
        "instance":facts.instance,"kind":facts.kind,"unavailable":facts.unavailable_reason.is_some(),
        "reason_present":facts.unavailable_reason.as_ref().is_some_and(|reason|!reason.is_empty()),
        "models":facts.models.iter().map(|model|json!({"name":model.name,"aliases":model.aliases})).collect::<Vec<_>>(),
        "usage":facts.usage.iter().map(|window|json!({"id":window.id,"used_percent":window.used_percent,"resets_at":window.resets_at})).collect::<Vec<_>>()
    })).collect::<Vec<_>>();
    profiles.sort_by(|a, b| a["instance"].as_str().cmp(&b["instance"].as_str()));
    Ok(
        json!({"providers":providers,"translated":{"profiles":profiles,"native_picks":translated.native_picks},"explicitly_unready":unready,"revoked_denied":true}),
    )
}

/// Source-grounded steering data is a fixture control, not live provider execution.
pub fn fake_steering(root: &Path) -> Result<Value> {
    let path = root.join("t3-state/contract-thread.json");
    let mut thread: Value = serde_json::from_slice(&fs::read(&path)?)?;
    let run = thread["messages"][0]["runId"].clone();
    thread["status"] = json!("running");
    thread["messages"]
        .as_array_mut()
        .context("fixture messages")?
        .push(json!({"role":"user","text":"contract steering input","runId":run}));
    fs::write(path, thread.to_string())?;
    let adapter = T3::default();
    let session = SessionRef::parse("t3:contract-thread")?;
    let now = "2026-11-10T12:00:00Z".parse()?;
    ensure!(
        adapter
            .observe_input(&session, "contract queued input", now)?
            .state
            == InputState::Accepted,
        "unrelated steering invalidated queued input recovery"
    );
    ensure!(
        adapter
            .observe_input(&session, "contract steering input", now)?
            .state
            == InputState::Accepted,
        "an earlier run start proved later steering entry"
    );
    ensure!(
        adapter
            .observe_input(&session, "contract deferred input", now)?
            .state
            == InputState::Consumed,
        "opening input lost its own entry evidence"
    );
    Ok(
        json!({"opening_entered":true,"queued_accepted":true,"unproved_steering_accepted":true,"provenance":"synthetic fixture from v0.1.8 steering producer; no provider executed"}),
    )
}

pub fn real(root: &Path) -> Result<()> {
    grammar()?;
    let _server = Server::start(root)?;
    let adapter = T3::default();
    let home = Home::from_env()?;
    let env = instrukt_core::config::Env::from_process();
    let readiness = readiness()?;
    let mut token = Token::issue(true)?;
    let db = instrukt_core::state_db::rusqlite::Connection::open_with_flags(
        Path::new(&std::env::var("T3CODE_HOME")?).join("userdata/statev2.sqlite"),
        instrukt_core::state_db::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    use instrukt_core::state_db::rusqlite::OptionalExtension as _;
    let find_project = || -> Result<Option<String>> {
        Ok(db
            .query_row(
                "SELECT project_id FROM projection_projects WHERE workspace_root=?1",
                [root.join("project").to_str().unwrap()],
                |row| row.get(0),
            )
            .optional()?)
    };
    if find_project()?.is_none() {
        cli(
            &[
                "project",
                "add",
                root.join("project").to_str().unwrap(),
                "--title",
                "Contract",
            ],
            None,
        )?;
    }
    let project = find_project()?.context("registered temporary project")?;
    let id = "contract-thread";
    rpc(
        &token.secret,
        "orchestration.dispatchCommand",
        json!({"type":"thread.create","commandId":"contract-create","threadId":id,"projectId":project,"title":"contract · instrukt:contract","modelSelection":{"instanceId":"codex","model":"default"},"runtimeMode":"approval-required","interactionMode":"default","createdBy":"user","creationSource":"web","branch":null,"worktreePath":null}),
    )?;
    rpc(
        &token.secret,
        "orchestration.dispatchCommand",
        json!({"type":"message.dispatch","commandId":"contract-defer","threadId":id,"messageId":"contract-input","text":"contract deferred input","attachments":[],"createdBy":"user","creationSource":"web","dispatchMode":{"type":"defer_start"}}),
    )?;
    let shown: Value = serde_json::from_str(&cli(
        &["thread", "show", "--json", "--messages", "100", id],
        None,
    )?)?;
    let source = adapter.conversation_source(&home, &env)?;
    let mut position = None;
    let mut records = Vec::new();
    loop {
        let changes = source.changed_since(position.as_ref())?;
        position = Some(changes.position);
        records.extend(changes.records);
        if !changes.has_more {
            break;
        }
    }
    ensure!(
        records.iter().any(|r| r.conversation_id == id),
        "real projection payloads absent"
    );
    let mut unchanged = source.changed_since(position.as_ref())?;
    ensure!(
        unchanged.records.is_empty() && !unchanged.has_more,
        "unchanged source duplicated records"
    );
    let mut notifications = source.subscribe()?;
    adapter.send(
        &SessionRef::parse(&format!("t3:{id}"))?,
        "contract queued input",
    )?;
    ensure!(
        notifications.wait(BOUND)?,
        "native source notification absent"
    );
    let mut changed = false;
    loop {
        let changes = source.changed_since(Some(&unchanged.position))?;
        changed |= !changes.records.is_empty();
        unchanged = changes;
        if !unchanged.has_more {
            break;
        }
    }
    ensure!(changed, "durable input change was not observed");
    let result = observe(id)?;
    ensure!(
        adapter
            .read_state(&SessionRef::parse("t3:contract-missing")?)?
            .state
            == SessionState::Gone,
        "unknown thread meaning"
    );
    let mut history = Command::new(env!("CARGO_BIN_EXE_instrukt"));
    history
        .current_dir(root.join("project"))
        .args(["history", "extract", "--json"]);
    let output = process::output(&mut history, None, BOUND)?;
    ensure!(
        output.status.success(),
        "real instrukt history boundary failed"
    );
    fs::write(
        root.join("real.json"),
        json!({"adapter":result,"fixture":shown,"readiness":readiness}).to_string(),
    )?;
    token.revoke()?;
    Ok(())
}

fn grammar() -> Result<()> {
    let commands: [(&str, &[&str], &[&str]); 4] = [
        ("server", &["--help"], &["--base-dir", "--no-browser"]),
        (
            "start",
            &["thread", "start", "--help"],
            &["--title", "--provider", "--model", "--worktree", "--branch"],
        ),
        (
            "list",
            &["thread", "list", "--help"],
            &["--project", "--json"],
        ),
        (
            "show",
            &["thread", "show", "--help"],
            &["--messages", "--json"],
        ),
    ];
    let mut grammar = std::collections::BTreeMap::new();
    for (command, args, consumed) in commands {
        let help = cli(args, None)?;
        let declarations: Vec<_> = help
            .lines()
            .filter_map(|line| {
                let declaration = line.trim().split_once("  ")?.0;
                consumed
                    .contains(&declaration.split_whitespace().next()?)
                    .then(|| declaration.to_owned())
            })
            .collect();
        grammar.insert(command, declarations);
    }
    insta::assert_json_snapshot!(grammar, @r###"
    {
      "list": [
        "--project string",
        "--json"
      ],
      "server": [
        "--base-dir string",
        "--no-browser"
      ],
      "show": [
        "--messages integer",
        "--json"
      ],
      "start": [
        "--title string",
        "--provider string",
        "--model string",
        "--worktree string",
        "--branch string"
      ]
    }
    "###);
    Ok(())
}
