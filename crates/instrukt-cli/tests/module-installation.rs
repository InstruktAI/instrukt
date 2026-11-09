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
    insta::assert_json_snapshot!(json(&env, &["ticks", "status", "--json"])["mode"], @r#"
"no_demand"
"#);
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
    insta::assert_json_snapshot!(std::fs::read_to_string(env.instrukt_home().join("state/retained")).unwrap(), @r#"
"pending work"
"#);
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
    insta::assert_json_snapshot!(json(&env, &["ticks", "status", "--json"])["mode"], @r#"
"operator"
"#);
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
    insta::assert_json_snapshot!(std::fs::read_to_string(file).unwrap(), @r#"
"operator's changed definition"
"#);
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
    insta::assert_json_snapshot!(status["enabled_projects"], @r#"
[
  "events"
]
"#);
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
    let owned_path = env.instrukt_home().join("state/installation/owned.json");
    let owned: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&owned_path).unwrap()).unwrap();
    for suffix in ["", "?db=0", "?db=3"] {
        repo.write("instrukt.yaml", &format!("schema_version: 3\nproject: events\nmodules: {{events: false}}\nredis: {{local: {{url: '{}{suffix}'}}}}\n", url));
        env.run(env.home(), ["install"]).success();
        for (file, contents) in owned["resources"]["redis"]["files"].as_object().unwrap() {
            assert_eq!(
                contents.as_str().unwrap().as_bytes(),
                std::fs::read(file).unwrap()
            );
        }
    }
    repo.write(
        "instrukt.yaml",
        "schema_version: 3\nproject: events\nmodules: {events: false}\n",
    );
    env.run(env.home(), ["install"]).success();
    insta::assert_json_snapshot!(redis::cmd("GET")
            .arg("instrukt:retained:test")
            .query::<String>(&mut connection)
            .unwrap(), @r#"
"pending"
"#);
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
    for denied in ["-EVALSHA", "-SCRIPT|LOAD"] {
        redis::cmd("ACL")
            .arg("SETUSER")
            .arg("probe-user")
            .arg("+@all")
            .arg(denied)
            .query::<()>(&mut redis.connection())
            .unwrap();
        env.run(env.home(), ["install", "--check"]).failure();
    }
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("probe-user")
        .arg("-@all")
        .arg("+@all")
        .arg("-XADD")
        .arg("clearselectors")
        .arg("(+XADD ~instrukt:diagnostic:*)")
        .query::<()>(&mut redis.connection())
        .unwrap();
    env.run(env.home(), ["install", "--check"]).failure();
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("probe-user")
        .arg("+@all")
        .arg("clearselectors")
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

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI11
#[test]
fn cron_withdrawal_preserves_the_entire_table_when_the_marked_line_has_drifted() {
    let mut env = TestEnv::new();
    let repo = project(&env, "cron-work", "{todos: true}");
    let git = env
        .command(std::path::Path::new("/usr/bin/which"), env.home())
        .arg("git")
        .output()
        .unwrap();
    std::os::unix::fs::symlink(
        String::from_utf8(git.stdout).unwrap().trim(),
        env.path("runtime-bin/git"),
    )
    .unwrap();
    let crontab = env.path("runtime-bin/crontab");
    let table = env.path("crontab");
    std::fs::write(&crontab, format!("#!/bin/sh\ncase $1 in -l) if [ -f '{0}' ]; then /bin/cat '{0}'; else echo 'no crontab for fixture' >&2; exit 1; fi;; -) /bin/cat > '{0}';; esac\n", table.display())).unwrap();
    std::fs::set_permissions(crontab, std::fs::Permissions::from_mode(0o755)).unwrap();
    env.set_env("PATH", "");
    env.run(env.home(), ["install"]).success();
    let installed = std::fs::read_to_string(&table).unwrap();
    let foreign =
        installed.replace(" tick ", " tick --operator-changed ") + "17 * * * * /operator/job\n";
    assert_ne!(installed, foreign);
    std::fs::write(&table, foreign.as_bytes()).unwrap();
    repo.write(
        "instrukt.yaml",
        "schema_version: 3\nproject: cron-work\nmodules: {todos: false}\n",
    );
    env.run(env.home(), ["install"]).failure();
    assert_eq!(foreign.as_bytes(), std::fs::read(&table).unwrap());
    std::fs::write(&table, &installed).unwrap();
    env.run(env.home(), ["install"]).success();
    assert!(
        !std::fs::read_to_string(table)
            .unwrap()
            .contains(installed.trim())
    );
}

fn write_owned(env: &TestEnv, journal: &serde_json::Value) {
    std::fs::write(
        env.instrukt_home().join("state/installation/owned.json"),
        serde_json::to_vec_pretty(journal).unwrap(),
    )
    .unwrap();
}
fn replay_prefixes(env: &TestEnv, kind: &str, desired: serde_json::Value) {
    let path = env.instrukt_home().join("state/installation/owned.json");
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let old = &original["resources"][kind];
    let files = desired["files"]
        .as_object()
        .unwrap()
        .iter()
        .collect::<Vec<_>>();
    for prefix in 0..=files.len() {
        let mut journal = original.clone();
        journal["updates"][kind] = serde_json::json!({"before": [old], "after": desired});
        if kind == "receiver" {
            journal["pending_receiver"] = desired.clone();
        }
        write_owned(env, &journal);
        for resource in original["resources"].as_object().unwrap().values() {
            for (file, contents) in resource["files"].as_object().unwrap() {
                std::fs::write(file, contents.as_str().unwrap()).unwrap();
            }
        }
        for (file, contents) in files.iter().take(prefix) {
            std::fs::write(file, contents.as_str().unwrap()).unwrap();
        }
        env.run(env.home(), ["install"]).success();
        let completed: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(completed["updates"].as_object().unwrap().is_empty());
        for (file, contents) in &files {
            assert_eq!(
                contents.as_str().unwrap().as_bytes(),
                std::fs::read(file).unwrap()
            );
        }
    }
    let mut journal = original;
    journal["updates"][kind] =
        serde_json::json!({"before": [journal["resources"][kind]], "after": desired});
    write_owned(env, &journal);
    let foreign = b"foreign definition after interruption";
    std::fs::write(files[0].0, foreign).unwrap();
    env.run(env.home(), ["install"]).failure();
    assert_eq!(foreign.as_slice(), std::fs::read(files[0].0).unwrap());
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI5
#[test]
fn every_wakeup_write_prefix_retains_the_old_and_new_ownership_proofs() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let _repo = project(&env, "interrupted", "{todos: true}");
    env.run(env.home(), ["install"]).success();
    let old_path = env.injected("PATH").unwrap().to_owned();
    env.set_env("PATH", &format!("{old_path}:/changed-tools"));
    let desired_path = env.injected("PATH").unwrap();
    let journal: serde_json::Value = serde_json::from_slice(
        &std::fs::read(env.instrukt_home().join("state/installation/owned.json")).unwrap(),
    )
    .unwrap();
    let mut desired = journal["resources"]["wake-up"].clone();
    for text in desired["files"].as_object_mut().unwrap().values_mut() {
        *text = serde_json::Value::String(text.as_str().unwrap().replace(&old_path, desired_path));
    }
    replay_prefixes(&env, "wake-up", desired);
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI8
#[test]
fn every_receiver_write_prefix_retains_the_old_and_new_ownership_proofs() {
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
    repo.write("instrukt.yaml", "schema_version: 3\nproject: events\nmodules: {events: true}\nwebhooks: {service: {secret: '${SIGNING_KEY}'}}\n");
    let health = ReceiverHealth::new();
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
    env.run(env.home(), ["install"]).success();
    let old_path = env.injected("PATH").unwrap().to_owned();
    env.set_env("PATH", &format!("{old_path}:/changed-tools"));
    let desired_path = env.injected("PATH").unwrap();
    let journal: serde_json::Value = serde_json::from_slice(
        &std::fs::read(env.instrukt_home().join("state/installation/owned.json")).unwrap(),
    )
    .unwrap();
    let mut desired = journal["resources"]["receiver"].clone();
    for (file, text) in desired["files"].as_object_mut().unwrap() {
        if !cfg!(target_os = "macos") || !file.ends_with(".plist") {
            *text =
                serde_json::Value::String(text.as_str().unwrap().replace(&old_path, desired_path));
        }
    }
    replay_prefixes(&env, "receiver", desired);
}

#[cfg(target_os = "macos")]
struct CachedLaunchd {
    child: std::process::Child,
    socket: std::path::PathBuf,
    _dir: tempfile::TempDir,
}
#[cfg(target_os = "macos")]
impl CachedLaunchd {
    fn new(env: &mut TestEnv) -> Self {
        let python = env
            .command(std::path::Path::new("/usr/bin/which"), env.home())
            .arg("python3")
            .output()
            .unwrap();
        let python = String::from_utf8(python.stdout).unwrap().trim().to_owned();
        let script =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/launchd.py");
        let dir = tempfile::Builder::new()
            .prefix("instrukt-launchd-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("control");
        let child = env
            .command(std::path::Path::new(&python), env.home())
            .arg(&script)
            .arg("serve")
            .arg(&socket)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let fixture = Self {
            child,
            socket,
            _dir: dir,
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !fixture.socket.exists() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        let launchctl = env.path("runtime-bin/launchctl");
        std::fs::write(
            &launchctl,
            format!(
                "#!/bin/sh\nexec '{}' '{}' call '{}' \"$@\"\n",
                python,
                script.display(),
                fixture.socket.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(launchctl, std::fs::Permissions::from_mode(0o755)).unwrap();
        fixture
    }
    fn request(&self, args: &[&str]) -> serde_json::Value {
        use std::io::{BufRead, Write};
        let mut socket = std::os::unix::net::UnixStream::connect(&self.socket).unwrap();
        serde_json::to_writer(&mut socket, args).unwrap();
        socket.write_all(b"\n").unwrap();
        let mut line = String::new();
        std::io::BufReader::new(socket)
            .read_line(&mut line)
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }
}
#[cfg(target_os = "macos")]
impl Drop for CachedLaunchd {
    fn drop(&mut self) {
        use std::io::Write;
        if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&self.socket) {
            let _ = stream.write_all(b"[\"shutdown\"]\n");
        } else {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI8
#[cfg(target_os = "macos")]
#[test]
fn launchd_cached_job_applies_the_desired_executable_and_environment_with_socket_custody() {
    let mut env = TestEnv::new();
    let fixture = CachedLaunchd::new(&mut env);
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
    repo.write("instrukt.yaml", "schema_version: 3\nproject: events\nmodules: {events: true}\nwebhooks: {service: {secret: '${SIGNING_KEY}'}}\n");
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    event_config(
        &env,
        Some(&redis.url),
        &redis.url,
        Some(&format!(
            "ingress: {{computer: '{}', port: {port}}}\n",
            machine.trim()
        )),
    );
    env.run(env.home(), ["install"]).success();
    let unit = format!("ai.instrukt.webhooks-{port}");
    let before = fixture.request(&["inspect", &unit]);
    let copied = env.path("replacement-instrukt");
    std::fs::copy(env!("CARGO_BIN_EXE_instrukt"), &copied).unwrap();
    env.set_env("PATH", "/usr/bin:/bin:/effective-runtime");
    let output = env
        .command(&copied, env.home())
        .arg("install")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let after = fixture.request(&["inspect", &unit]);
    assert_eq!(before["definition"], after["definition"]);
    assert_ne!(before["pid"], after["pid"]);
    let process = env
        .command(std::path::Path::new("/bin/ps"), env.home())
        .args(["eww", "-p", &after["pid"].to_string(), "-o", "command="])
        .output()
        .unwrap();
    assert!(process.status.success());
    let process = String::from_utf8(process.stdout).unwrap();
    assert!(process.contains(copied.to_str().unwrap()), "{process}");
    for (key, expected) in [
        ("PATH", env.injected("PATH").unwrap().to_owned()),
        ("HOME", env.home().display().to_string()),
        ("INSTRUKT_HOME", env.instrukt_home().display().to_string()),
    ] {
        assert!(process.contains(&format!("{key}={expected}")), "{process}");
    }
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_err());
    // The retained listener also accepts a real request from the replacement.
    use std::io::{Read, Write};
    let mut request = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    request
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    request.write_all(b"POST /hooks/inbound/project/events/service HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    request.read_to_string(&mut response).unwrap();
    insta::assert_snapshot!(response.lines().next().unwrap(), @"HTTP/1.1 401 Unauthorized");
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI5
#[test]
fn installation_inspection_reads_retained_foreign_project_facts_without_entering_its_checkout() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let repo = project(&env, "retained", "{todos: true}");
    env.run(env.home(), ["install"]).success();
    let before = env
        .run(env.home(), ["install", "--check", "--json"])
        .success()
        .get_output()
        .stdout
        .clone();
    std::fs::rename(repo.path(), env.path("moved-foreign-checkout")).unwrap();
    let after = env
        .run(env.home(), ["install", "--check", "--json"])
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(before, after);
}
