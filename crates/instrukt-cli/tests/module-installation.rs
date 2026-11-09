#![allow(clippy::unwrap_used, reason = "test setup")]
mod common;
#[path = "support/documented.rs"]
mod documented;
use common::{Repo, TestEnv};
use std::os::unix::fs::PermissionsExt;

fn manager(env: &mut TestEnv) {
    let dir = env.path("runtime-bin");
    for name in ["launchctl", "systemctl", "crontab"] {
        let text = format!(
            r#"#!/bin/sh
printf '%s\n' '{name}' "$@" >> '{dir}/calls'
if [ "$1" = --user ]; then shift; fi
operation=$1
if [ "$operation" = show ]; then unit=$2; if [ -f '{dir}/unit-'"$unit" ]; then echo loaded; else echo not-found; fi; exit 0; fi
case $operation in daemon-reload) exit 0;; esac
for argument in "$@"; do unit=$argument; done
unit=${{unit##*/}}; unit=${{unit%.plist}}
state='{dir}/unit-'$unit
case $operation in
    list|is-active) [ -f "$state" ];;
    unload|disable|stop) rm -f "$state"; case $unit in ai.instrukt.tick|instrukt-tick.timer) rm -f '{dir}/active';; esac;;
    *) touch "$state"; case $unit in ai.instrukt.tick|instrukt-tick.timer) touch '{dir}/active';; esac;;
esac
"#,
            dir = dir.display()
        );
        let file = dir.join(name);
        std::fs::write(&file, text).unwrap();
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
fn project(env: &TestEnv, slug: &str, modules: &str) -> Repo {
    let repo = documented::project(env, slug, &[]);
    documented::sync(env, &repo);
    std::fs::write(
        env.instrukt_home().join("instrukt.yaml"),
        "schema_version: 1\nrunner: {runtime: t3}\n",
    )
    .unwrap();
    repo.write(
        "instrukt.yaml",
        &format!(
            "schema_version: 1\nproject: {slug}\nmodules: {modules}\nrunner: {{runtime: t3}}\ngithub: {{repository: fixture/project}}\n"
        ),
    );
    repo
}
fn json(env: &TestEnv, args: &[&str]) -> serde_json::Value {
    let run = env.run(env.home(), args).success();
    serde_json::from_slice(&run.get_output().stdout).unwrap()
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI1
#[test]
fn core_has_no_infrastructure_demand_even_with_global_defaults() {
    let mut env = TestEnv::new();
    manager(&mut env);
    env.run(env.home(), ["install", "--check"]).success();
    let _core = project(&env, "core", "{todos: false, events: false}");
    std::fs::write(
        env.instrukt_home().join("instrukt.yaml"),
        "schema_version: 1\nmodules: {todos: true, events: true}\n",
    )
    .unwrap();
    env.run(env.home(), ["install"]).success();
    assert!(!env.path("runtime-bin/calls").exists());
    assert_eq!(
        json(&env, &["ticks", "status", "--json"])["mode"],
        "no_demand"
    );
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI2
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI5
#[test]
fn todos_install_one_registration_check_read_only_and_repair_missing_definition() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let _repo = project(&env, "work", "{todos: true}");
    env.run(env.home(), ["install", "--check"]).failure();
    env.run(env.home(), ["install"]).success();
    env.run(env.home(), ["install", "--check"]).success();
    let file = if cfg!(target_os = "macos") {
        env.home()
            .join("Library/LaunchAgents/ai.instrukt.tick.plist")
    } else {
        env.home().join(".config/systemd/user/instrukt-tick.timer")
    };
    let before = std::fs::read(&file).unwrap();
    let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
    env.run(env.home(), ["install"]).success();
    assert_eq!(
        modified,
        std::fs::metadata(&file).unwrap().modified().unwrap()
    );
    assert!(!env.instrukt_home().join("state/redis").exists());
    std::fs::remove_file(&file).unwrap();
    env.run(env.home(), ["install", "--check"]).failure();
    assert!(!file.exists());
    env.run(env.home(), ["install"]).success();
    assert_eq!(before, std::fs::read(file).unwrap());
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI7
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI11
#[test]
fn shared_demand_and_unreadable_projects_prevent_withdrawal_and_last_disable_preserves_data() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let first = project(&env, "first", "{todos: true}");
    let second = project(&env, "second", "{todos: true}");
    env.run(env.home(), ["install"]).success();
    first.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: first\nmodules: {todos: false}\n",
    );
    env.run(env.home(), ["install"]).success();
    second.write("instrukt.yaml", "invalid: [");
    env.run(env.home(), ["install"]).failure();
    assert!(env.path("runtime-bin/active").exists());
    second.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: second\nmodules: {todos: false}\n",
    );
    std::fs::write(env.instrukt_home().join("state/retained"), "pending work").unwrap();
    env.run(env.home(), ["install"]).success();
    assert!(!env.path("runtime-bin/active").exists());
    assert_eq!(
        std::fs::read_to_string(env.instrukt_home().join("state/retained")).unwrap(),
        "pending work"
    );
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI10
#[test]
fn ticks_off_withdraws_owned_wakeup_and_reports_operator_scheduling() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let _repo = project(&env, "work", "{todos: true}");
    env.run(env.home(), ["install"]).success();
    std::fs::write(
        env.instrukt_home().join("instrukt.yaml"),
        "schema_version: 1\nticks: off\nrunner: {runtime: t3}\n",
    )
    .unwrap();
    env.run(env.home(), ["install"]).success();
    assert_eq!(
        json(&env, &["ticks", "status", "--json"])["mode"],
        "operator"
    );
    assert!(!env.path("runtime-bin/active").exists());
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI5
#[test]
fn foreign_definition_is_never_overwritten() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let _repo = project(&env, "work", "{todos: true}");
    env.run(env.home(), ["install"]).success();
    let file = if cfg!(target_os = "macos") {
        env.home()
            .join("Library/LaunchAgents/ai.instrukt.tick.plist")
    } else {
        env.home().join(".config/systemd/user/instrukt-tick.timer")
    };
    std::fs::write(&file, "operator's changed definition").unwrap();
    env.run(env.home(), ["install"]).failure();
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        "operator's changed definition"
    );
}

fn event_config(env: &TestEnv, local: Option<&str>, cluster: &str, ingress: Option<&str>) {
    let local = local
        .map(|url| format!("  local: {{url: '{url}'}}\n"))
        .unwrap_or_default();
    std::fs::write(env.instrukt_home().join("instrukt.yaml"),format!("schema_version: 3\nrunner: {{runtime: t3}}\nredis:\n  cluster: {{url: '{cluster}'}}\n{local}{}",ingress.unwrap_or(""))).unwrap();
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI3
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI5
#[test]
fn events_reuse_operator_local_redis_prepare_readers_and_check_without_writes() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let _repo = project(&env, "events", "{events: true}");
    let redis = common::redis::RedisServer::new(&env);
    event_config(&env, Some(&redis.url), &redis.url, None);
    env.run(env.home(), ["install"]).success();
    let before: redis::Value = redis::cmd("XINFO")
        .arg("GROUPS")
        .arg("instrukt:events")
        .query(&mut redis.connection())
        .unwrap();
    env.run(env.home(), ["install", "--check"]).success();
    let after: redis::Value = redis::cmd("XINFO")
        .arg("GROUPS")
        .arg("instrukt:events")
        .query(&mut redis.connection())
        .unwrap();
    assert_eq!(before, after);
    let status = json(&env, &["events", "status", "--json"]);
    assert_eq!(status["enabled_projects"], serde_json::json!(["events"]));
    assert_eq!(status["transport"][0]["reachable"], true);
    assert!(!env.instrukt_home().join("state/redis").exists());
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI6
#[test]
fn broken_established_local_endpoint_keeps_config_and_never_falls_back() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let _repo = project(&env, "events", "{events: true}");
    let redis = common::redis::RedisServer::new(&env);
    let missing = env.path("missing.sock");
    event_config(
        &env,
        Some(&format!("unix://{}", missing.display())),
        &redis.url,
        None,
    );
    let config = std::fs::read(env.instrukt_home().join("instrukt.yaml")).unwrap();
    env.run(env.home(), ["install"]).failure();
    assert_eq!(
        config,
        std::fs::read(env.instrukt_home().join("instrukt.yaml")).unwrap()
    );
    assert!(!env.instrukt_home().join("state/redis").exists());
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI4
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI9
#[test]
fn only_ingress_gets_receiver_and_non_ingress_needs_no_signing_material() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let repo = project(&env, "events", "{events: true}");
    repo.write("instrukt.yaml","schema_version: 1\nproject: events\nmodules: {events: true}\nwebhooks:\n  service: {secret: '${SIGNING_KEY}'}\n");
    let health = ReceiverHealth::new();
    let redis = common::redis::RedisServer::new(&env);
    event_config(
        &env,
        Some(&redis.url),
        &redis.url,
        Some(&format!(
            "ingress: {{computer: '0000000000000000', port: {}}}\n",
            health.port
        )),
    );
    env.run(env.home(), ["install"]).success();
    let receiver = if cfg!(target_os = "macos") {
        env.home().join(format!(
            "Library/LaunchAgents/ai.instrukt.webhooks-{}.plist",
            health.port
        ))
    } else {
        env.home().join(format!(
            ".config/systemd/user/instrukt-webhooks-{}.socket",
            health.port
        ))
    };
    assert!(!receiver.exists());
    env.run(env.home(), ["webhooks", "serve", "--socket-activation"])
        .failure();
    let machine = std::fs::read_to_string(env.instrukt_home().join("state/machine-id")).unwrap();
    event_config(
        &env,
        Some(&redis.url),
        &redis.url,
        Some(&format!(
            "ingress: {{computer: '{}', port: {}}}\n",
            machine.trim(),
            health.port
        )),
    );
    env.run(env.home(), ["install"]).failure();
    assert!(!receiver.exists());
    env.instrukt(env.home())
        .args(["secrets", "set", "SIGNING_KEY"])
        .write_stdin("test signing material")
        .assert()
        .success();
    env.run(env.home(), ["install"]).success();
    assert!(receiver.is_file());
    env.run(env.home(), ["install", "--check"]).success();
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI8
#[test]
fn held_socket_keeps_a_signed_request_across_receiver_restart_without_runner() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let repo = project(&env, "events", "{events: true}");
    let redis = common::redis::RedisServer::new(&env);
    event_config(&env, Some(&redis.url), &redis.url, None);
    env.run(env.home(), ["install"]).success();
    let machine = std::fs::read_to_string(env.instrukt_home().join("state/machine-id")).unwrap();
    repo.write("instrukt.yaml","schema_version: 3\nproject: events\nmodules: {events: true}\nwebhooks:\n  service: {secret: '${SIGNING_KEY}'}\n");
    env.instrukt(env.home())
        .args(["secrets", "set", "SIGNING_KEY"])
        .write_stdin("diagnostic-test-key")
        .assert()
        .success();
    let code = r#"
import os, socket, subprocess, sys, json, hmac, hashlib, signal, http.client
binary, config, machine = sys.argv[1:]
held = socket.socket()
held.bind(('127.0.0.1', 0)); held.listen()
port = held.getsockname()[1]
with open(config, 'a') as out:
    out.write('\ningress: {computer: "' + machine + '", port: ' + str(port) + '}\n')
child = None
def start():
    pid = os.fork()
    if pid == 0:
        os.dup2(held.fileno(), 3); os.set_inheritable(3, True)
        os.environ['LISTEN_FDS'] = '1'; os.environ['LISTEN_FDNAMES'] = 'instrukt'
        os.environ['LISTEN_PID'] = str(os.getpid())
        os.execv(binary, [binary, 'webhooks', 'serve', '--socket-activation'])
    return pid
def stop(pid):
    os.kill(pid, signal.SIGTERM); os.waitpid(pid, 0)
def request(body, signature):
    client = http.client.HTTPConnection('127.0.0.1', port, timeout=5)
    client.request('POST', '/hooks/inbound/project/events/service', body, {'X-Hook-Signature':signature,'Content-Type':'application/json'})
    response = client.getresponse(); response.read(); client.close(); return response.status
try:
    child = start()
    assert request(b'{}', 'bad') == 401
    stop(child); child = None
    # Send while no receiver exists. The parent owns the listening socket and
    # retains the queued connection; the second receiver adopts its descriptor.
    queued = socket.create_connection(('127.0.0.1', port), timeout=5)
    body = b'{"name":"diagnostic","payload":{"value":1}}'
    signature = 'sha256=' + hmac.new(b'diagnostic-test-key', body, hashlib.sha256).hexdigest()
    queued.sendall(b'POST /hooks/inbound/project/events/service HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nX-Hook-Signature: '+signature.encode()+b'\r\nContent-Length: '+str(len(body)).encode()+b'\r\nConnection: close\r\n\r\n'+body)
    child = start()
    response = queued.recv(4096); queued.close()
    assert response.split(b' ')[1] == b'202', response
finally:
    if child is not None: stop(child)
    held.close()
"#;
    let before: redis::Value = redis::cmd("XINFO")
        .arg("GROUPS")
        .arg("instrukt:events")
        .query(&mut redis.connection())
        .unwrap();
    let output = env
        .command(std::path::Path::new("python3"), env.home())
        .arg("-c")
        .arg(code)
        .arg(env!("CARGO_BIN_EXE_instrukt"))
        .arg(env.instrukt_home().join("instrukt.yaml"))
        .arg(machine.trim())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after: redis::Value = redis::cmd("XINFO")
        .arg("GROUPS")
        .arg("instrukt:events")
        .query(&mut redis.connection())
        .unwrap();
    // The receiver never drains or resets the installed reader's position.
    let before = redis::from_redis_value::<redis::streams::StreamInfoGroupsReply>(before).unwrap();
    let after = redis::from_redis_value::<redis::streams::StreamInfoGroupsReply>(after).unwrap();
    assert_eq!(
        before.groups[0].last_delivered_id,
        after.groups[0].last_delivered_id
    );
    assert_eq!(
        redis::cmd("XLEN")
            .arg("instrukt:events")
            .query::<u64>(&mut redis.connection())
            .unwrap(),
        1
    );
}

struct PrivateRedis(std::process::Child);
impl Drop for PrivateRedis {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI3
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI11
#[test]
fn owned_redis_requires_secret_authentication_and_withdrawal_retains_its_data() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let short = tempfile::Builder::new()
        .prefix("ik-home-")
        .tempdir_in("/tmp")
        .unwrap();
    let alias = short.path().join("h");
    std::os::unix::fs::symlink(env.home(), &alias).unwrap();
    env.set_env("HOME", alias.to_str().unwrap());
    let repo = project(&env, "events", "{events: true}");
    let cluster = common::redis::RedisServer::new(&env);
    event_config(&env, None, &cluster.url, None);
    // Fake manager records the registration; Redis itself is started below as
    // a disposable test child from the definition installation produced.
    env.run(env.home(), ["install"]).success();
    let dir = env.instrukt_home().join("state/redis");
    let mut server = PrivateRedis(
        env.command(std::path::Path::new("redis-server"), env.home())
            .arg(dir.join("redis.conf"))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let home = instrukt_core::home::Home::at_user_home(env.home().into());
    let password =
        instrukt_core::config::resolve_secret(&home, "${INSTRUKT_REDIS_PASSWORD}").unwrap();
    let url = format!(
        "unix://{}",
        alias.join(".instrukt/state/redis/redis.sock").display()
    );
    let mut info = redis::IntoConnectionInfo::into_connection_info(url.as_str()).unwrap();
    let auth = info
        .redis_settings()
        .clone()
        .set_username("instrukt")
        .set_password(&password);
    info = info.set_redis_settings(auth);
    let client = redis::Client::open(info).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut connection = loop {
        if let Ok(mut c) = client.get_connection()
            && redis::cmd("PING").query::<String>(&mut c).is_ok()
        {
            break c;
        }
        assert!(server.0.try_wait().unwrap().is_none());
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    };
    env.run(env.home(), ["install"]).success();
    env.run(env.home(), ["install", "--check"]).success();
    let mut anonymous = redis::Client::open(url.as_str())
        .unwrap()
        .get_connection()
        .unwrap();
    assert!(redis::cmd("PING").query::<String>(&mut anonymous).is_err());
    let socket = std::fs::metadata(dir.join("redis.sock")).unwrap();
    assert_eq!(socket.permissions().mode() & 0o077, 0);
    redis::cmd("SET")
        .arg("instrukt:retained:test")
        .arg("pending")
        .query::<()>(&mut connection)
        .unwrap();
    repo.write(
        "instrukt.yaml",
        "schema_version: 3\nproject: events\nmodules: {events: false}\n",
    );
    env.run(env.home(), ["install"]).success();
    assert_eq!(
        redis::cmd("GET")
            .arg("instrukt:retained:test")
            .query::<String>(&mut connection)
            .unwrap(),
        "pending"
    );
    assert!(dir.join("users.acl").is_file());
    assert!(env.instrukt_home().join("secrets.env").is_file());
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI6
#[test]
fn unconfigured_local_fallback_is_verified_and_check_never_provisions() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let _repo = project(&env, "events", "{events: true}");
    let cluster = common::redis::RedisServer::new(&env);
    event_config(&env, None, &cluster.url, None);
    env.set_env("PATH", "/usr/bin:/bin");
    let brew = env.path("runtime-bin/brew");
    std::fs::write(
        &brew,
        format!(
            "#!/bin/sh\ntouch '{}/package-called'\nexit 1\n",
            env.path("").display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&brew, std::fs::Permissions::from_mode(0o755)).unwrap();
    for name in ["apt-get", "dpkg-deb"] {
        let file = env.path(&format!("runtime-bin/{name}"));
        std::fs::write(
            &file,
            format!(
                "#!/bin/sh\ntouch '{}/package-called'\nexit 1\n",
                env.path("").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    env.run(env.home(), ["install", "--check"]).failure();
    assert!(!env.path("package-called").exists());
    env.run(env.home(), ["install"]).success();
    let report = json(&env, &["install", "--check", "--json"]);
    assert!(report["findings"].as_array().unwrap().is_empty());
    assert!(!env.instrukt_home().join("state/redis").exists());
}

struct ReceiverHealth {
    port: u16,
    server: std::sync::Arc<tiny_http::Server>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl ReceiverHealth {
    fn new() -> Self {
        Self::with_response(401)
    }
    fn with_response(code: u16) -> Self {
        let server = std::sync::Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = std::sync::Arc::clone(&server);
        let stopping = std::sync::Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !stopping.load(std::sync::atomic::Ordering::Acquire) {
                match reader.recv() {
                    Ok(request) => {
                        request.respond(tiny_http::Response::empty(code)).unwrap();
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            port,
            server,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for ReceiverHealth {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI5
#[test]
fn shared_local_cluster_identity_still_checks_local_socket_permissions() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let _repo = project(&env, "events", "{events: true}");
    let redis = common::redis::RedisServer::new(&env);
    event_config(&env, Some(&redis.url), &redis.url, None);
    env.run(env.home(), ["install"]).success();
    let socket = std::path::Path::new(redis.url.strip_prefix("unix://").unwrap());
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o777)).unwrap();
    env.run(env.home(), ["install", "--check"]).failure();
    let status = json(&env, &["events", "status", "--json"]);
    assert!(
        !status["infrastructure"]["findings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o700)).unwrap();
    env.run(env.home(), ["install", "--check"]).success();
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI3
#[test]
fn redis_older_than_seven_is_an_unmet_prerequisite_without_mutating_transport() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let _repo = project(&env, "events", "{events: true}");
    let redis = common::redis::RedisServer::new(&env);
    event_config(&env, Some(&redis.url), &redis.url, None);
    env.run(env.home(), ["install"]).success();
    let old = common::redis_proxy::DeliveryGate::reporting_version(&redis, "6.2.16");
    event_config(&env, Some(&old.url), &old.url, None);
    let before: redis::Value = redis::cmd("XINFO")
        .arg("GROUPS")
        .arg("instrukt:events")
        .query(&mut redis.connection())
        .unwrap();
    let result = env
        .run(env.home(), ["install", "--check", "--json"])
        .failure();
    let report: serde_json::Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    insta::assert_json_snapshot!(report["findings"], @r#"
    [
      "Redis: Redis 7.0 or later is required (server reports 6.2.16); upgrade Redis before installation"
    ]
    "#);
    let after: redis::Value = redis::cmd("XINFO")
        .arg("GROUPS")
        .arg("instrukt:events")
        .query(&mut redis.connection())
        .unwrap();
    assert_eq!(before, after);
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("probe-user")
        .arg("on")
        .arg(">test-password")
        .arg("~instrukt:*")
        .arg("+@all")
        .arg("-SET")
        .query::<()>(&mut redis.connection())
        .unwrap();
    for (name, value) in [
        ("PROBE_USER", "probe-user"),
        ("PROBE_PASSWORD", "test-password"),
    ] {
        env.instrukt(env.home())
            .args(["secrets", "set", name])
            .write_stdin(value)
            .assert()
            .success();
    }
    std::fs::write(env.instrukt_home().join("instrukt.yaml"),format!("schema_version: 3\nrunner: {{runtime: t3}}\nredis:\n  cluster: {{url: '{}', username: '${{PROBE_USER}}', password: '${{PROBE_PASSWORD}}'}}\n  local: {{url: '{}', username: '${{PROBE_USER}}', password: '${{PROBE_PASSWORD}}'}}\n",redis.url,redis.url)).unwrap();
    env.run(env.home(), ["install", "--check"]).failure();
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("probe-user")
        .arg("+SET")
        .query::<()>(&mut redis.connection())
        .unwrap();
    env.run(env.home(), ["install", "--check"]).success();
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI8
#[test]
fn receiver_restart_preserves_registration_and_port_change_starts_replacement_first() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let repo = project(&env, "events", "{events: true}");
    let redis = common::redis::RedisServer::new(&env);
    event_config(&env, Some(&redis.url), &redis.url, None);
    env.run(env.home(), ["install"]).success();
    let machine = std::fs::read_to_string(env.instrukt_home().join("state/machine-id")).unwrap();
    env.instrukt(env.home())
        .args(["secrets", "set", "SIGNING_KEY"])
        .write_stdin("test signing material")
        .assert()
        .success();
    repo.write("instrukt.yaml","schema_version: 3\nproject: events\nmodules: {events: true}\nwebhooks: {service: {secret: '${SIGNING_KEY}'}}\n");
    let first = ReceiverHealth::new();
    event_config(
        &env,
        Some(&redis.url),
        &redis.url,
        Some(&format!(
            "ingress: {{computer: '{}', port: {}}}\n",
            machine.trim(),
            first.port
        )),
    );
    env.run(env.home(), ["install"]).success();
    for name in ["launchctl", "systemctl"] {
        let file = env.path(&format!("runtime-bin/{name}"));
        let before = std::fs::read_to_string(&file).unwrap();
        let guard = r#"
case "$*" in *webhooks*)
case "${FORBID_SOCKET_STOP:-}:$*" in
    1:*unload*|1:*disable*) exit 57;;
esac
if [ -n "${REPLACEMENT_UNIT:-}" ]; then
    case "$*" in *unload*|*disable*) [ -f "$REPLACEMENT_UNIT" ] || exit 58;; esac
fi
;; esac
"#;
        std::fs::write(
            file,
            before.replacen("#!/bin/sh\n", &format!("#!/bin/sh\n{guard}"), 1),
        )
        .unwrap();
    }
    env.set_env("FORBID_SOCKET_STOP", "1");
    let new_path = format!(
        "{}:/additional-runtime-tools",
        env.injected("PATH").unwrap()
    );
    env.set_env("PATH", &new_path);
    env.run(env.home(), ["install"]).success();
    env.run(env.home(), ["install", "--check"]).success();
    env.remove_env("FORBID_SOCKET_STOP");
    let next = ReceiverHealth::new();
    let next_unit = if cfg!(target_os = "macos") {
        format!("ai.instrukt.webhooks-{}", next.port)
    } else {
        format!("instrukt-webhooks-{}.socket", next.port)
    };
    env.set_env(
        "REPLACEMENT_UNIT",
        env.path(&format!("runtime-bin/unit-{next_unit}"))
            .to_str()
            .unwrap(),
    );
    event_config(
        &env,
        Some(&redis.url),
        &redis.url,
        Some(&format!(
            "ingress: {{computer: '{}', port: {}}}\n",
            machine.trim(),
            next.port
        )),
    );
    let groups: redis::Value = redis::cmd("XINFO")
        .arg("GROUPS")
        .arg("instrukt:events")
        .query(&mut redis.connection())
        .unwrap();
    env.run(env.home(), ["install"]).success();
    env.run(env.home(), ["install", "--check"]).success();
    assert_eq!(
        groups,
        redis::cmd("XINFO")
            .arg("GROUPS")
            .arg("instrukt:events")
            .query::<redis::Value>(&mut redis.connection())
            .unwrap()
    );
    let broken = ReceiverHealth::with_response(503);
    event_config(
        &env,
        Some(&redis.url),
        &redis.url,
        Some(&format!(
            "ingress: {{computer: '{}', port: {}}}\n",
            machine.trim(),
            broken.port
        )),
    );
    env.run(env.home(), ["install"]).failure();
    let pending = if cfg!(target_os = "macos") {
        format!("ai.instrukt.webhooks-{}", broken.port)
    } else {
        format!("instrukt-webhooks-{}.socket", broken.port)
    };
    let pending = env.path(&format!("runtime-bin/unit-{pending}"));
    assert!(pending.is_file());
    let current = env.path(&format!("runtime-bin/unit-{next_unit}"));
    assert!(current.is_file());
    repo.write(
        "instrukt.yaml",
        "schema_version: 3\nproject: events\nmodules: {events: false}\n",
    );
    env.run(env.home(), ["install"]).success();
    assert!(!pending.exists());
    assert!(!current.exists());
}
