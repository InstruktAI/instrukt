#![allow(clippy::unwrap_used, reason = "test setup")]
use crate::common;
use crate::fixtures::documented;
use common::{Repo, TestEnv};
use std::os::unix::fs::PermissionsExt;

fn manager(env: &mut TestEnv) {
    env.set_env("SERVICE_PID", &std::process::id().to_string());
    let dir = env.path("runtime-bin");
    for name in ["launchctl", "systemctl", "crontab"] {
        let text = format!(
            r#"#!/bin/sh
printf '%s\n' '{name}' "$@" >> '{dir}/calls'
if [ "$1" = --user ]; then shift; fi
operation=$1
case $operation in load|unload|remove|start|stop|enable|disable|restart|daemon-reload|debug) printf '%s\n' "$operation" >> '{dir}/mutations';; esac
if [ "$operation" = list-units ]; then
    for path in '{dir}'/unit-*; do
        [ -f "$path" ] || continue
        printf '%s loaded active running\n' "${{path##*/unit-}}"
    done
    exit 0
fi
if [ "$operation" = list ] && [ "$#" -eq 1 ]; then
    printf 'PID\tStatus\tLabel\n'
    for path in '{dir}'/unit-*; do
        [ -f "$path" ] || continue
        unit=${{path##*/unit-}}
        printf '%s\t0\t%s\n' "$SERVICE_PID" "$unit"
    done
    exit 0
fi
if [ "$operation" = show ] && [ "$3" = --property=MainPID ]; then echo "$SERVICE_PID"; exit 0; fi
if [ "$operation" = show ]; then unit=$2; if [ -f '{dir}/unit-'"$unit" ]; then echo loaded; else echo not-found; fi; exit 0; fi
case $operation in daemon-reload|debug|print) exit 0;; esac
for argument in "$@"; do unit=$argument; done
unit=${{unit##*/}}; unit=${{unit%.plist}}
state='{dir}/unit-'$unit
case $operation in
    list|is-active) [ -f "$state" ];;
    unload|disable|stop|remove) rm -f "$state"; case $unit in ai.instrukt.tick|instrukt-tick.timer) rm -f '{dir}/active';; esac;;
    *) touch "$state"; case $unit in ai.instrukt.tick|instrukt-tick.timer) touch '{dir}/active';; esac;;
esac
"#,
            dir = dir.display()
        );
        let file = dir.join(name);
        std::fs::write(&file, text).unwrap();
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // The user's linger state: on unless a test switches it off.
    let loginctl = dir.join("loginctl");
    std::fs::write(
        &loginctl,
        format!(
            "#!/bin/sh\nif [ \"$1\" = show-user ]; then if [ -f '{dir}/linger-off' ]; then echo no; else echo yes; fi; fi\n",
            dir = dir.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(loginctl, std::fs::Permissions::from_mode(0o755)).unwrap();
}
fn project(env: &TestEnv, slug: &str, modules: &str) -> Repo {
    let modules = if modules.contains("history:") {
        modules.to_owned()
    } else {
        modules.replacen("{", "{history: false, ", 1)
    };
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
    // Nothing is demanded, so nothing is registered or changed; reading the
    // crontab for this home's line is all a manager is asked.
    assert!(!env.path("runtime-bin/mutations").exists());
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
    let github = env.with_fake_github();
    github.add_repo("fixture/project");
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
        "schema_version: 1\nproject: first\nmodules: {history: false, todos: false}\n",
    );
    env.run(env.home(), ["install"]).success();
    second.write("instrukt.yaml", "invalid: [");
    env.run(env.home(), ["install"]).failure();
    assert!(env.path("runtime-bin/active").exists());
    second.write(
        "instrukt.yaml",
        "schema_version: 1\nproject: second\nmodules: {history: false, todos: false}\n",
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

fn event_config(env: &TestEnv, local: Option<&str>, cluster: &str, ingress: Option<&str>) {
    let local = local
        .map(|url| format!("  local: {{url: '{url}'}}\n"))
        .unwrap_or_default();
    std::fs::write(env.instrukt_home().join("instrukt.yaml"),format!("schema_version: 6\nrunner: {{runtime: t3}}\nredis:\n  cluster: {{url: '{cluster}'}}\n{local}{}",ingress.unwrap_or(""))).unwrap();
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
    repo.write("instrukt.yaml","schema_version: 1\nproject: events\nmodules: {history: false, events: true}\nwebhooks:\n  service: {secret: '${SIGNING_KEY}'}\n");
    let health = ReceiverHealth::new(&env);
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
    // On Linux the local Redis becomes instrukt's own private service.
    let mut _private = None;
    if cfg!(target_os = "linux") {
        event_config(
            &env,
            None,
            &redis.url,
            Some(&format!(
                "ingress: {{computer: '{}', port: {}}}\n",
                machine.trim(),
                health.port
            )),
        );
        env.run(env.home(), ["install"]).success();
        _private = Some(PrivateRedis(
            env.command(std::path::Path::new("redis-server"), env.home())
                .arg(env.instrukt_home().join("state/redis/redis.conf"))
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        ));
        crate::common::wait_until(
            "the private Redis checked",
            std::time::Duration::from_secs(10),
            || {
                env.instrukt(env.home())
                    .args(["install", "--check"])
                    .output()
                    .unwrap()
                    .status
                    .success()
            },
        );
        // The user's manager does not start at boot without a login: one
        // finding names every owned unit that needs it, and installation
        // leaves it as it is.
        let off = env.path("runtime-bin").join("linger-off");
        std::fs::write(&off, "").unwrap();
        let check = env
            .run(env.home(), ["install", "--check", "--json"])
            .failure();
        let report: serde_json::Value = serde_json::from_slice(&check.get_output().stdout).unwrap();
        insta::assert_json_snapshot!(report["findings"], @r#"
        [
          "user service manager: the user's systemd manager does not start at boot without a login, so instrukt's owned units wait for one; enable it with `loginctl enable-linger`; it is needed by the receiver's socket and service, the private local Redis service, the minute timer"
        ]
        "#);
        // Installation reports the same one finding, and leaves the
        // prerequisite as it is: the check after it still reports it.
        let install = env.run(env.home(), ["install", "--json"]).failure();
        let report: serde_json::Value =
            serde_json::from_slice(&install.get_output().stdout).unwrap();
        insta::assert_json_snapshot!(report["findings"], @r#"
        [
          "user service manager: the user's systemd manager does not start at boot without a login, so instrukt's owned units wait for one; enable it with `loginctl enable-linger`; it is needed by the receiver's socket and service, the private local Redis service, the minute timer"
        ]
        "#);
        // Its one line points at the findings the JSON holds.
        insta::assert_snapshot!(
            String::from_utf8_lossy(&install.get_output().stderr),
            @"instrukt: the installation is incomplete; resolve each entry of the output's `findings` and run `instrukt install` again"
        );
        env.run(env.home(), ["install", "--check"]).failure();
        std::fs::remove_file(off).unwrap();
    }

    // A company home's source alone keeps the events dependencies and the
    // receiver while the machine's events switch is on, its signing value
    // resolving from the company's own file.
    let global = env.instrukt_home().join("instrukt.yaml");
    let mut machine_settings = std::fs::read_to_string(&global).unwrap();
    machine_settings.push_str("modules: {events: true}\n");
    std::fs::write(&global, machine_settings).unwrap();
    env.run(env.home(), ["config", "init", "--company", "acme"])
        .success();
    env.instrukt(env.home())
        .args(["secrets", "set", "COMPANY_KEY", "--company", "acme"])
        .write_stdin("company signing material")
        .assert()
        .success();
    let revision = env
        .run(
            env.home(),
            ["config", "get", "--company", "acme", "--revision"],
        )
        .success();
    let revision = String::from_utf8(revision.get_output().stdout.clone()).unwrap();
    env.run(
        env.home(),
        [
            "config",
            "patch",
            "--company",
            "acme",
            "--revision",
            revision.trim(),
            "--yaml",
            "{webhooks: {service: {secret: '${COMPANY_KEY}'}}}",
        ],
    )
    .success();
    repo.write(
        "instrukt.yaml",
        "schema_version: 6\nproject: events\nmodules: {history: false, events: false}\n",
    );
    env.run(env.home(), ["install"]).success();
    assert!(receiver.is_file());
    assert!(env.path("runtime-bin/active").exists());
    env.run(env.home(), ["install", "--check"]).success();

    // Receiving and publishing need no agent: with another computer named as
    // the one that reacts, this ingress installs and checks its receiver with
    // no runtime chosen, for the company's source alone and with the
    // project's source beside it.
    let reacting = std::fs::read_to_string(&global)
        .unwrap()
        .replace("runner: {runtime: t3}\n", "")
        .replace(
            &format!("computer: '{}',", machine.trim()),
            &format!(
                "computer: '{}', reactions: '1111111111111111',",
                machine.trim()
            ),
        );
    std::fs::write(&global, &reacting).unwrap();
    env.run(env.home(), ["install"]).success();
    assert!(receiver.is_file());
    env.run(env.home(), ["install", "--check"]).success();
    repo.write(
        "instrukt.yaml",
        "schema_version: 6\nproject: events\nmodules: {history: false, events: true}\nwebhooks:\n  service: {secret: '${SIGNING_KEY}'}\n",
    );
    env.run(env.home(), ["install"]).success();
    env.run(env.home(), ["install", "--check"]).success();
    // Where this computer reacts itself, its events run on the compiled
    // runtime, which an unset `runner.runtime` is.
    std::fs::write(
        &global,
        reacting.replace(", reactions: '1111111111111111'", ""),
    )
    .unwrap();
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
    repo.write("instrukt.yaml","schema_version: 6\nproject: events\nmodules: {history: false, events: true}\nwebhooks:\n  service: {secret: '${SIGNING_KEY}'}\n");
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
    assert int(response.split(b' ')[1]) == http.HTTPStatus.ACCEPTED, response
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
struct OwnedRedisFixture {
    _server: PrivateRedis,
    _cluster: common::redis::RedisServer,
    env: TestEnv,
    repo: Repo,
    _short: tempfile::TempDir,
    alias: std::path::PathBuf,
    dir: std::path::PathBuf,
    url: String,
    connection: redis::Connection,
}
fn owned_redis_fixture() -> OwnedRedisFixture {
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
    let mut connection = None;
    crate::common::wait_until(
        "owned Redis startup",
        std::time::Duration::from_secs(5),
        || {
            if let Ok(mut c) = client.get_connection()
                && redis::cmd("PING").query::<String>(&mut c).is_ok()
            {
                connection = Some(c);
                return true;
            }
            assert!(server.0.try_wait().unwrap().is_none());
            false
        },
    );
    let connection = connection.unwrap();
    env.run(env.home(), ["install"]).success();
    env.run(env.home(), ["install", "--check"]).success();
    OwnedRedisFixture {
        _server: server,
        _cluster: cluster,
        env,
        repo,
        _short: short,
        alias,
        dir,
        url,
        connection,
    }
}
impl OwnedRedisFixture {
    fn history_cluster(&self, url: &str) {
        let file = self.env.instrukt_home().join("instrukt.yaml");
        let mut settings: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        settings["redis"]["cluster"] = settings["redis"]["local"].clone();
        settings["redis"]["cluster"]["url"] = url.into();
        std::fs::write(file, serde_yaml_ng::to_string(&settings).unwrap()).unwrap();
        self.repo.write(
            "instrukt.yaml",
            "schema_version: 6\nproject: events\nmodules: {events: false, history: true}\n",
        );
    }
    /// The ACL an earlier release wrote, without history's permissions.
    fn legacy_acl(&self) -> (std::path::PathBuf, String, String) {
        let file = self.alias.join(".instrukt/state/redis/users.acl");
        let current = std::fs::read_to_string(&file).unwrap();
        let old = current
            .replace(" +rpush +blpop +multi +exec", "")
            .replace(" +select", "");
        std::fs::write(&file, &old).unwrap();
        (file, current, old)
    }
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI3
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI11
#[test]
fn owned_redis_requires_secret_authentication_and_withdrawal_retains_its_data() {
    let mut fixture = owned_redis_fixture();
    let env = &fixture.env;
    let repo = &fixture.repo;
    let dir = &fixture.dir;
    let url = &fixture.url;
    let mut connection = &mut fixture.connection;
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
        "schema_version: 6\nproject: events\nmodules: {history: false, events: false}\n",
    );
    let definition = if cfg!(target_os = "macos") {
        env.home()
            .join("Library/LaunchAgents/ai.instrukt.redis.plist")
    } else {
        env.home()
            .join(".config/systemd/user/instrukt-redis.service")
    };
    let files = [definition, dir.join("redis.conf"), dir.join("users.acl")];
    let written = files
        .iter()
        .map(|file| std::fs::read(file).unwrap())
        .collect::<Vec<_>>();
    for suffix in ["", "?db=0", "?db=3"] {
        repo.write("instrukt.yaml", &format!("schema_version: 6\nproject: events\nmodules: {{history: false, events: false}}\nredis: {{local: {{url: '{}{suffix}'}}}}\n", url));
        env.run(env.home(), ["install"]).success();
        let kept = files
            .iter()
            .map(|file| std::fs::read(file).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(kept, written);
    }
    repo.write(
        "instrukt.yaml",
        "schema_version: 6\nproject: events\nmodules: {history: false, events: false}\n",
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
    let bin = env.path("fallback-bin");
    std::fs::create_dir_all(&bin).unwrap();
    // Only these tools and the process-line fakes are visible to the CLI.
    for tool in ["git", "id", "rm", "touch"] {
        let resolved = env
            .command(std::path::Path::new("/usr/bin/which"), env.home())
            .arg(tool)
            .output()
            .unwrap();
        assert!(
            resolved.status.success(),
            "required fixture tool {tool} is absent"
        );
        std::os::unix::fs::symlink(
            String::from_utf8(resolved.stdout).unwrap().trim(),
            bin.join(tool),
        )
        .unwrap();
    }
    env.set_env("PATH", bin.to_str().unwrap());
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
    assert!(env.path("package-called").exists());
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
    fn new(env: &TestEnv) -> Self {
        Self::with_response(env, 401)
    }
    fn with_response(env: &TestEnv, code: u16) -> Self {
        let server = std::sync::Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = std::sync::Arc::clone(&server);
        let stopping = std::sync::Arc::clone(&stop);
        let home = env.instrukt_home();
        let thread = std::thread::spawn(move || {
            while !stopping.load(std::sync::atomic::Ordering::Acquire) {
                match reader.recv() {
                    Ok(request) => {
                        if request.url() == "/_instrukt/receiver" && code == 401 {
                            use sha2::{Digest, Sha256};
                            let bytes =
                                std::fs::read(home.join(format!("state/receiver/{port}.json")))
                                    .unwrap();
                            request
                                .respond(tiny_http::Response::from_string(serde_json::json!({"pid": std::process::id(), "invocation": hex::encode(Sha256::digest(bytes))}).to_string()))
                                .unwrap();
                        } else {
                            request.respond(tiny_http::Response::empty(code)).unwrap();
                        }
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
    std::fs::write(env.instrukt_home().join("instrukt.yaml"),format!("schema_version: 6\nrunner: {{runtime: t3}}\nredis:\n  cluster: {{url: '{}', username: '${{PROBE_USER}}', password: '${{PROBE_PASSWORD}}'}}\n  local: {{url: '{}', username: '${{PROBE_USER}}', password: '${{PROBE_PASSWORD}}'}}\n",redis.url,redis.url)).unwrap();
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
    repo.write("instrukt.yaml","schema_version: 6\nproject: events\nmodules: {history: false, events: true}\nwebhooks: {service: {secret: '${SIGNING_KEY}'}}\n");
    let first = ReceiverHealth::new(&env);
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
    // An install that changes nothing still writes the receiver's invocation
    // and restarts the receiver, keeping its socket; inspection changes
    // neither.
    use std::os::unix::fs::MetadataExt as _;
    let invocation = env
        .instrukt_home()
        .join(format!("state/receiver/{}.json", first.port));
    let written = std::fs::metadata(&invocation).unwrap().ino();
    let mutations = env.path("runtime-bin/mutations");
    std::fs::remove_file(&mutations).unwrap();
    env.run(env.home(), ["install"]).success();
    let rewritten = std::fs::metadata(&invocation).unwrap().ino();
    assert_ne!(rewritten, written);
    assert!(!std::fs::read_to_string(&mutations).unwrap().is_empty());
    std::fs::remove_file(&mutations).unwrap();
    env.run(env.home(), ["install", "--check"]).success();
    assert_eq!(std::fs::metadata(&invocation).unwrap().ino(), rewritten);
    assert!(!mutations.exists());
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
    let next = ReceiverHealth::new(&env);
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
    let broken = ReceiverHealth::with_response(&env, 503);
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
    env.remove_env("REPLACEMENT_UNIT");
    repo.write(
        "instrukt.yaml",
        "schema_version: 6\nproject: events\nmodules: {history: false, events: false}\n",
    );
    env.run(env.home(), ["install"]).success();
    assert!(!pending.exists());
    assert!(!current.exists());
    // A receiver's service systemd still lists, with no socket and no unit
    // file left, is found by its name and withdrawn.
    let service = env.path("runtime-bin/unit-instrukt-webhooks-12345.service");
    std::fs::write(&service, "").unwrap();
    env.run(env.home(), ["install"]).success();
    assert!(!service.exists());
    // Its socket and service both listed while systemd reports their unit
    // files not found: both are stopped.
    let socket = env.path("runtime-bin/unit-instrukt-webhooks-12345.socket");
    std::fs::write(&socket, "").unwrap();
    std::fs::write(&service, "").unwrap();
    let systemctl = env.path("runtime-bin/systemctl");
    let wrapper = std::fs::read_to_string(&systemctl).unwrap();
    std::fs::write(
        &systemctl,
        wrapper
            .replacen(
                "operation=$1",
                "operation=$1\nif [ \"$operation\" = show ]; then echo not-found; exit 0; fi",
                1,
            )
            .replace("loaded active running", "not-found active running"),
    )
    .unwrap();
    env.run(env.home(), ["install"]).success();
    assert!(!socket.exists());
    assert!(!service.exists());
    // A runner registration systemd keeps with its unit file gone, whose
    // environment names a home with a space: systemd prints that assignment
    // quoted. The default home leaves another home's registration, and the
    // home it names withdraws its own legacy one.
    let wrapper = std::fs::read_to_string(&systemctl).unwrap();
    std::fs::write(
        &systemctl,
        wrapper.replacen(
            "operation=$1",
            "operation=$1\nif [ \"$operation\" = show ] && [ \"$3\" = --property=Environment ]; then printf '%s\\n' \"$SYSTEMD_ENVIRONMENT\"; exit 0; fi",
            1,
        ),
    )
    .unwrap();
    let registration = env.path("runtime-bin/unit-instrukt-runner.service");
    let another = env.path("another home");
    std::fs::create_dir_all(&another).unwrap();
    env.set_env(
        "SYSTEMD_ENVIRONMENT",
        &format!(
            "HOME=/home/fixture \"INSTRUKT_HOME={}\" OTHER=value",
            another.display()
        ),
    );
    std::fs::write(&registration, "").unwrap();
    env.run(env.home(), ["install"]).success();
    assert!(registration.exists());
    std::fs::write(
        another.join("instrukt.yaml"),
        "schema_version: 6\nmodules: {history: false}\n",
    )
    .unwrap();
    env.set_env("INSTRUKT_HOME", another.to_str().unwrap());
    env.run(env.home(), ["install"]).success();
    assert!(!registration.exists());
    env.remove_env("INSTRUKT_HOME");
    // Environment systemd's quoting never prints: a stray quote ending it,
    // or a quote never closed that swallows the home. Whose the registration
    // is cannot be established, so it is left in place and reported once:
    // by the default home's install, and by the install of a home other
    // than the default one, which meets it as an earlier registration.
    let elsewhere = std::path::PathBuf::from(format!("{}\"", env.instrukt_home().display()));
    std::fs::create_dir_all(&elsewhere).unwrap();
    let base = env.path("");
    let canonical = base.canonicalize().unwrap();
    let mut reported = Vec::new();
    let swallowed = format!("HOME=\"/home/fixture INSTRUKT_HOME={}", another.display());
    for (printed, installing) in [
        (format!("INSTRUKT_HOME={}", elsewhere.display()), None),
        (swallowed.clone(), None),
        (swallowed, Some(&another)),
    ] {
        env.set_env("SYSTEMD_ENVIRONMENT", &printed);
        if let Some(home) = installing {
            env.set_env("INSTRUKT_HOME", home.to_str().unwrap());
        }
        std::fs::write(&registration, "").unwrap();
        let installed = env.run(env.home(), ["install", "--json"]).success();
        env.remove_env("INSTRUKT_HOME");
        assert!(registration.exists());
        let report: serde_json::Value =
            serde_json::from_slice(&installed.get_output().stdout).unwrap();
        let notices = report["notices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|notice| {
                notice
                    .as_str()
                    .unwrap()
                    .replace(canonical.to_str().unwrap(), "[base]")
                    .replace(base.to_str().unwrap(), "[base]")
            })
            .collect::<Vec<_>>();
        assert_eq!(notices.len(), 1);
        reported.push(notices);
    }
    insta::assert_json_snapshot!(reported, @r#"
    [
      [
        "instrukt-runner.service (systemd): instrukt cannot tell which instrukt home this registration belongs to, so it was left running. Run `instrukt install` from the home it belongs to, or remove it yourself with `systemctl --user disable --now instrukt-runner.service`"
      ],
      [
        "instrukt-runner.service (systemd): instrukt cannot tell which instrukt home this registration belongs to, so it was left running. Run `instrukt install` from the home it belongs to, or remove it yourself with `systemctl --user disable --now instrukt-runner.service`"
      ],
      [
        "instrukt-runner.service (systemd): instrukt cannot tell which instrukt home this registration belongs to, so it was left running. Run `instrukt install` from the home it belongs to, or remove it yourself with `systemctl --user disable --now instrukt-runner.service`"
      ]
    ]
    "#);
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI11
#[test]
fn cron_withdrawal_removes_only_this_homes_marked_line_even_when_it_has_drifted() {
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
    let operator = "17 * * * * /operator/job\n";
    let foreign = installed.replace(" tick ", " tick --operator-changed ") + operator;
    assert_ne!(installed, foreign);
    std::fs::write(&table, foreign.as_bytes()).unwrap();
    repo.write(
        "instrukt.yaml",
        "schema_version: 6\nproject: cron-work\nmodules: {history: false, todos: false}\n",
    );
    // The marked line is this home's whatever it now says: it is withdrawn,
    // and every other line is kept.
    env.run(env.home(), ["install"]).success();
    assert_eq!(std::fs::read_to_string(&table).unwrap(), operator);
}

#[cfg(target_os = "macos")]
struct CachedLaunchd {
    child: std::process::Child,
    socket: std::path::PathBuf,
    _dir: tempfile::TempDir,
    python: String,
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
            python: python.clone(),
        };
        crate::common::wait_until(
            "launchd fixture socket",
            std::time::Duration::from_secs(5),
            || fixture.socket.exists(),
        );
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
    repo.write("instrukt.yaml", "schema_version: 6\nproject: events\nmodules: {history: false, events: true}\nwebhooks: {service: {secret: '${SIGNING_KEY}'}}\n");
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
    assert_receiver_execution(&env, &after, &copied, port);
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
    let github = env.with_fake_github();
    github.add_repo("fixture/project");
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

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI5
#[test]
fn an_earlier_releases_installation_record_is_left_by_inspection_and_removed_by_install() {
    let mut env = TestEnv::new();
    let repo = project(&env, "migrating", "{todos: true}");
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
    std::fs::write(
        &crontab,
        format!(
            "#!/bin/sh\ncase $1 in -l) /bin/cat '{}';; *) exit 99;; esac\n",
            table.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(crontab, std::fs::Permissions::from_mode(0o755)).unwrap();
    env.set_env("PATH", "");
    std::fs::write(
        env.instrukt_home().join("instrukt.yaml"),
        "schema_version: 6\nrunner: {runtime: t3}\n",
    )
    .unwrap();
    let github = env.with_fake_github();
    github.add_repo("fixture/project");
    // Exact pre-change field set; only installation-specific addresses are substituted.
    let original = include_str!("../fixtures/installation-owned-v2-676e017-cron.json")
        .replace("__HOME__", env.home().to_str().unwrap())
        .replace("__PATH__", env.injected("PATH").unwrap())
        .replace("__EXE__", env!("CARGO_BIN_EXE_instrukt"));
    let golden: serde_json::Value = serde_json::from_str(&original).unwrap();
    std::fs::write(
        &table,
        format!(
            "{}\n",
            golden["resources"]["wake-up"]["unit"].as_str().unwrap()
        ),
    )
    .unwrap();
    let path = env.instrukt_home().join("state/installation/owned.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, original.as_bytes()).unwrap();
    let inspection_only =
        common::Snapshot::over(vec![], vec![env.home().into(), repo.path().into()]);
    env.run(repo.path(), ["install", "--check"]).success();
    assert_eq!(original.as_bytes(), std::fs::read(&path).unwrap());
    assert!(inspection_only.changed().is_empty());
    // Installing finds the wake-up by its name, unchanged, and removes the
    // record its names replace.
    let table_before = std::fs::read(&table).unwrap();
    env.run(repo.path(), ["install"]).success();
    assert!(!path.exists());
    assert_eq!(table_before, std::fs::read(table).unwrap());
}

#[cfg(target_os = "macos")]
fn assert_receiver_execution(
    env: &TestEnv,
    job: &serde_json::Value,
    executable: &std::path::Path,
    port: u16,
) {
    use std::os::unix::fs::MetadataExt;
    let pid = job["pid"].to_string();
    let kernel = env
        .command(std::path::Path::new("/bin/ps"), env.home())
        .args(["-p", &pid, "-o", "comm="])
        .output()
        .unwrap();
    assert!(kernel.status.success());
    let program = String::from_utf8(kernel.stdout).unwrap();
    let observed = std::fs::metadata(program.trim_end()).unwrap();
    let desired = std::fs::metadata(executable).unwrap();
    assert_eq!(
        (observed.dev(), observed.ino()),
        (desired.dev(), desired.ino())
    );
    let process = env
        .command(std::path::Path::new("/bin/ps"), env.home())
        .args(["eww", "-p", &pid, "-o", "command="])
        .output()
        .unwrap();
    assert!(process.status.success());
    use sha2::{Digest, Sha256};
    let invocation = std::fs::read(
        env.instrukt_home()
            .join(format!("state/receiver/{port}.json")),
    )
    .unwrap();
    let digest = hex::encode(Sha256::digest(invocation));
    let rendered = String::from_utf8(process.stdout)
        .unwrap()
        .replace(&digest, "[INVOCATION]")
        .replace(&format!("LISTEN_PID={pid}"), "LISTEN_PID=[PID]")
        .replace(env.path("").to_str().unwrap(), "[TMP]/");
    // The harness's write fence names this computer's live home.
    let fence = format!("{}=", instrukt_core::fsx::FENCE_VARIABLE);
    let rendered = rendered
        .split_whitespace()
        .filter(|field| !field.starts_with("__CF_USER_TEXT_ENCODING="))
        .map(|field| {
            if field.starts_with(&fence) {
                format!("{fence}[LIVE HOME]")
            } else {
                field.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    insta::assert_snapshot!("receiver_execution", rendered);
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI5
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI8
#[cfg(target_os = "macos")]
#[test]
fn pre_invocation_receiver_at_another_path_upgrades_without_releasing_its_socket() {
    let mut env = TestEnv::new();
    let fixture = CachedLaunchd::new(&mut env);
    env.set_env("PATH", "/usr/bin:/bin");
    let repo = project(&env, "events", "{events: true}");
    let redis = common::redis::RedisServer::new(&env);
    event_config(&env, Some(&redis.url), &redis.url, Some("ticks: off\n"));
    env.run(env.home(), ["install"]).success();
    let machine = std::fs::read_to_string(env.instrukt_home().join("state/machine-id")).unwrap();
    env.instrukt(env.home())
        .args(["secrets", "set", "SIGNING_KEY"])
        .write_stdin("test signing material")
        .assert()
        .success();
    repo.write("instrukt.yaml", "schema_version: 6\nproject: events\nmodules: {history: false, events: true}\nwebhooks: {service: {secret: '${SIGNING_KEY}'}}\n");
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    event_config(
        &env,
        Some(&redis.url),
        &redis.url,
        Some(&format!(
            "ticks: off\ningress: {{computer: '{}', port: {port}}}\n",
            machine.trim()
        )),
    );
    let original = env.path("original-instrukt");
    std::fs::write(
        &original,
        format!(
            "#!{}\n{}",
            fixture.python,
            include_str!("../support/pre-invocation-receiver.py")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o755)).unwrap();
    let golden = include_str!("../fixtures/installation-owned-v2-676e017-launchd.json")
        .replace("__HOME__", env.home().to_str().unwrap())
        .replace("__EXE__", original.to_str().unwrap())
        .replace("__PATH__", env.injected("PATH").unwrap())
        .replace("__PORT__", &port.to_string());
    let record: serde_json::Value = serde_json::from_str(&golden).unwrap();
    for (file, contents) in record["resources"]["receiver"]["files"]
        .as_object()
        .unwrap()
    {
        let file = std::path::Path::new(file);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, contents.as_str().unwrap()).unwrap();
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)).unwrap();
        fixture.request(&["load", "-w", file.to_str().unwrap()]);
    }
    std::fs::create_dir_all(env.instrukt_home().join("state/installation")).unwrap();
    std::fs::write(
        env.instrukt_home().join("state/installation/owned.json"),
        golden.as_bytes(),
    )
    .unwrap();
    let unit = record["resources"]["receiver"]["unit"].as_str().unwrap();
    fixture.request(&["kickstart", "-k", &format!("gui/1/{unit}")]);
    common::wait_until(
        "legacy receiver started",
        std::time::Duration::from_secs(15),
        || {
            fixture.request(&["inspect", unit])["pid"]
                .as_u64()
                .is_some()
        },
    );
    let before = fixture.request(&["inspect", unit]);
    let untouched = common::Snapshot::over(vec![], vec![env.home().into()]);
    env.run(env.home(), ["install", "--check"]).failure();
    assert!(untouched.changed().is_empty());
    let copied = env.path("replacement-instrukt");
    std::fs::copy(env!("CARGO_BIN_EXE_instrukt"), &copied).unwrap();
    env.set_env("PATH", "/usr/bin:/bin:/effective-runtime");
    let inspect = || {
        env.command(&copied, env.home())
            .args(["install", "--check"])
            .output()
            .unwrap()
    };
    let install = || {
        env.command(&copied, env.home())
            .arg("install")
            .output()
            .unwrap()
    };
    let output = install();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let after = fixture.request(&["inspect", unit]);
    assert_eq!(before["custody"], after["custody"]);
    assert_eq!(before["definition"], after["definition"]);
    assert_receiver_execution(&env, &after, &copied, port);
    let moved = env.path("temporarily-unavailable-checkout");
    std::fs::rename(repo.path(), &moved).unwrap();
    let inspection_only = common::Snapshot::over(vec![], vec![env.home().into()]);
    assert!(inspect().status.success());
    assert!(inspection_only.changed().is_empty());
    std::fs::rename(moved, repo.path()).unwrap();
    fixture.request(&["force-legacy", unit]);
    common::wait_until(
        "legacy receiver restarted",
        std::time::Duration::from_secs(15),
        || {
            let job = fixture.request(&["inspect", unit]);
            job["pid"].as_u64().is_some() && job["pid"] != after["pid"]
        },
    );
    let unchanged = common::Snapshot::over(vec![], vec![env.home().into()]);
    assert!(!inspect().status.success());
    assert!(unchanged.changed().is_empty());
    // A misleading unsigned 401 is insufficient when the native override did not apply.
    fixture.request(&["ignore-debug", "on"]);
    assert!(!install().status.success());
    fixture.request(&["ignore-debug", "off"]);
    assert!(install().status.success());
    let repaired = fixture.request(&["inspect", unit]);
    assert_eq!(before["custody"], repaired["custody"]);
    assert_receiver_execution(&env, &repaired, &copied, port);
    // Native debug overrides are one-shot: the running receiver prepares the next start too.
    fixture.request(&["kickstart", "-k", &format!("gui/1/{unit}")]);
    // A kickstart reply reports the fork, not receiver readiness. Wait for the
    // replacement process and its successful public installation inspection.
    common::wait_until(
        "receiver restart completed",
        std::time::Duration::from_secs(15),
        || {
            let job = fixture.request(&["inspect", unit]);
            job["pid"].as_u64().is_some()
                && job["pid"] != repaired["pid"]
                && inspect().status.success()
        },
    );
    assert!(inspect().status.success());
    let restarted = fixture.request(&["inspect", unit]);
    assert_eq!(before["custody"], restarted["custody"]);
    assert_receiver_execution(&env, &restarted, &copied, port);
    fixture.request(&["stop-process", unit]);
    common::wait_until(
        "receiver stopped",
        std::time::Duration::from_secs(15),
        || fixture.request(&["inspect", unit])["pid"].is_null(),
    );
    let cold = common::Snapshot::over(vec![], vec![env.home().into()]);
    assert!(!inspect().status.success());
    assert!(cold.changed().is_empty());
    let stopped = fixture.request(&["inspect", unit]);
    assert_eq!(stopped["queued"], serde_json::Value::Bool(false));
    assert_eq!(before["custody"], stopped["custody"]);
}

fn github_installation() -> (TestEnv, Repo, common::github::FakeGitHub) {
    let env = TestEnv::new();
    let repo = project(&env, "github-ready", "{todos: true}");
    let github = env.with_fake_github();
    github.add_repo("fixture/project");
    let file = env.instrukt_home().join("instrukt.yaml");
    let config = std::fs::read_to_string(&file).unwrap();
    std::fs::write(file, format!("{config}ticks: off\n")).unwrap();
    env.run(repo.path(), ["install"]).success();
    (env, repo, github)
}

fn github_check(env: &TestEnv, repo: &Repo, healthy: bool) -> serde_json::Value {
    let mut command = env.instrukt(repo.path());
    command.args(["install", "--check", "--module", "todos", "--json"]);
    let guard = common::Snapshot::over(vec![], vec![env.path("")]);
    let output = if healthy {
        command.assert().success()
    } else {
        command.assert().failure()
    };
    assert!(
        guard.changed().is_empty(),
        "inspection changed {:?}",
        guard.changed()
    );
    let report: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    report["findings"].clone()
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI2
#[test]
fn github_readiness_checks_existing_reflection_surfaces_without_mutating_them_or_local_files() {
    let (env, repo, github) = github_installation();
    github.add_project("fixture", 1);
    let issue = github.seed_issue(
        "fixture/project",
        common::github::IssueSeed {
            title: "Existing work".into(),
            ..Default::default()
        },
    );
    repo.write("instrukt.yaml", "schema_version: 6\nproject: github-ready\nmodules: {history: false, todos: true}\nrunner: {runtime: t3}\ngithub: {repository: fixture/project}\nboard: fixture/1\n");
    let issues = github.issues("fixture/project");
    let artifacts = github.artifacts("fixture/project", issue);
    let fields = github.project_fields("fixture", 1);
    let views = github.project_views("fixture", 1);
    let items = github.project_items("fixture", 1);
    assert_eq!(github_check(&env, &repo, true), serde_json::json!([]));
    assert_eq!(github.issues("fixture/project"), issues);
    assert_eq!(github.artifacts("fixture/project", issue), artifacts);
    assert_eq!(github.project_fields("fixture", 1), fields);
    assert_eq!(github.project_views("fixture", 1), views);
    assert_eq!(github.project_items("fixture", 1), items);
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI2
#[test]
fn github_readiness_checks_account_discovery_without_creating_a_board_in_an_empty_repository() {
    let (env, repo, github) = github_installation();
    assert_eq!(github_check(&env, &repo, true), serde_json::json!([]));
    assert!(github.issues("fixture/project").is_empty());
    assert!(github.project_numbers("fixture").is_empty());
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI2
#[test]
fn github_readiness_refuses_a_missing_protected_key_even_with_process_tokens() {
    let (mut env, repo, _github) = github_installation();
    std::fs::remove_file(env.instrukt_home().join("secrets.env")).unwrap();
    env.set_env("GH_TOKEN", "must-not-substitute");
    env.set_env("GITHUB_TOKEN", "must-not-substitute");
    insta::assert_json_snapshot!(
        "github-readiness-missing-key",
        github_check(&env, &repo, false)
    );
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI2
#[test]
fn github_readiness_refuses_an_unverified_installation_actor() {
    let (env, repo, github) = github_installation();
    github.respond_once("POST", "/graphql", common::github::CannedResponse {
        status: 200, headers: vec![], body: serde_json::json!({"data":{"viewer":{"id":"wrong","databaseId":99,"login":"wrong[bot]"}}})
    });
    insta::assert_json_snapshot!(
        "github-readiness-wrong-actor",
        github_check(&env, &repo, false)
    );
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI2
#[test]
fn github_readiness_refuses_missing_account_board_permissions() {
    let (env, repo, github) = github_installation();
    github.respond_once("GET", "/repos/fixture/project/installation", common::github::CannedResponse {
        status: 200, headers: vec![], body: serde_json::json!({"id":1,"app_id":1,"permissions":{"issues":"write","contents":"read","pull_requests":"read"}})
    });
    insta::assert_json_snapshot!(
        "github-readiness-board-permissions",
        github_check(&env, &repo, false)
    );
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI2
#[test]
fn github_readiness_reports_unavailable_artifact_reflection_without_creating_a_probe_issue() {
    let (env, repo, github) = github_installation();
    let issue = github.seed_issue("fixture/project", common::github::IssueSeed::default());
    github.respond_once(
        "GET",
        &format!("/repos/fixture/project/issues/{issue}/artifacts"),
        common::github::CannedResponse {
            status: 404,
            headers: vec![],
            body: serde_json::json!({"message":"Not Found"}),
        },
    );
    insta::assert_json_snapshot!(
        "github-readiness-artifacts-unavailable",
        github_check(&env, &repo, false)
    );
    assert_eq!(github.issues("fixture/project").len(), 1);
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI2
#[test]
fn github_readiness_reports_rate_limiting_as_unmet_and_preserves_local_state() {
    let (env, repo, github) = github_installation();
    github.respond_once(
        "GET",
        "/repos/fixture/project/issues",
        common::github::CannedResponse {
            status: 429,
            headers: vec![
                ("X-RateLimit-Remaining".into(), "0".into()),
                ("X-RateLimit-Reset".into(), "4070908800".into()),
            ],
            body: serde_json::json!({"message":"rate limited"}),
        },
    );
    insta::assert_json_snapshot!(
        "github-readiness-rate-limited",
        github_check(&env, &repo, false)
    );
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI2
#[test]
fn github_readiness_refuses_disabled_repository_issues() {
    let (env, repo, github) = github_installation();
    github.respond_once(
        "GET",
        "/repos/fixture/project",
        common::github::CannedResponse {
            status: 200,
            headers: vec![],
            body: serde_json::json!({"has_issues":false}),
        },
    );
    insta::assert_json_snapshot!(
        "github-readiness-disabled-issues",
        github_check(&env, &repo, false)
    );
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI2
#[test]
fn github_readiness_reports_an_unavailable_configured_board_without_replacing_it() {
    let (env, repo, github) = github_installation();
    repo.write("instrukt.yaml", "schema_version: 6\nproject: github-ready\nmodules: {history: false, todos: true}\nrunner: {runtime: t3}\ngithub: {repository: fixture/project}\nboard: fixture/99\n");
    insta::assert_json_snapshot!(
        "github-readiness-missing-board",
        github_check(&env, &repo, false)
    );
    assert!(github.project_numbers("fixture").is_empty());
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI2
#[test]
fn github_readiness_is_outside_an_events_only_inspection() {
    let (env, repo, github) = github_installation();
    std::fs::remove_file(env.instrukt_home().join("secrets.env")).unwrap();
    github.clear_requests();
    env.run(repo.path(), ["install", "--check", "--module", "events"])
        .success();
    assert!(github.requests().is_empty());
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI2
#[test]
fn github_readiness_reports_unavailable_account_discovery() {
    let (env, repo, github) = github_installation();
    github.set_project_discovery_available(false);
    insta::assert_json_snapshot!(
        "github-readiness-discovery-unavailable",
        github_check(&env, &repo, false)
    );
    assert!(github.issues("fixture/project").is_empty());
    assert!(github.project_numbers("fixture").is_empty());
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI2
#[test]
fn github_readiness_checks_older_issue_artifacts_after_a_full_page_of_pull_requests() {
    let (env, repo, github) = github_installation();
    let issue = github.seed_issue("fixture/project", common::github::IssueSeed::default());
    for index in 0..100 {
        github.seed_pull_request(
            "fixture/project",
            common::github::IssueSeed {
                title: format!("Newer pull request {index}"),
                ..Default::default()
            },
        );
    }
    let issues = github.issues("fixture/project");
    let artifacts = github.artifacts("fixture/project", issue);
    github.respond_once(
        "GET",
        &format!("/repos/fixture/project/issues/{issue}/artifacts"),
        common::github::CannedResponse {
            status: 404,
            headers: vec![],
            body: serde_json::json!({"message":"Not Found"}),
        },
    );
    insta::assert_json_snapshot!(
        "github-readiness-paginated-artifacts-unavailable",
        github_check(&env, &repo, false)
    );
    assert_eq!(github.issues("fixture/project"), issues);
    assert_eq!(github.artifacts("fixture/project", issue), artifacts);
}

fn history_cluster_fixture() -> (TestEnv, Repo, common::redis::RedisServer) {
    let mut env = TestEnv::new();
    manager(&mut env);
    let repo = project(
        &env,
        "history",
        "{history: true, events: false, todos: false}",
    );
    repo.write(
        "instrukt.yaml",
        "schema_version: 6\nproject: history\nmodules: {events: false, todos: false}\n",
    );
    let redis = common::redis::RedisServer::new(&env);
    event_config(&env, None, &redis.url, None);
    let file = env.instrukt_home().join("instrukt.yaml");
    let original = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        format!("{original}  local: {{url: 'unix:///missing/socket', password: '${{MISSING}}'}}\n"),
    )
    .unwrap();
    (env, repo, redis)
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI3
#[test]
fn history_converges_cluster_without_event_resources_or_local_credentials() {
    let (env, _repo, redis) = history_cluster_fixture();
    let report = json(&env, &["install", "--json"]);
    assert!(report["findings"].as_array().unwrap().is_empty());
    assert_eq!(report["resources"].as_array().unwrap().len(), 1);
    insta::assert_json_snapshot!(report["resources"][0]["projects"], @r#"
    [
      "history"
    ]
    "#);
    assert!(!env.path("runtime-bin/mutations").exists());
    assert!(!env.instrukt_home().join("state/redis").exists());
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg("instrukt:*")
        .query(&mut redis.connection())
        .unwrap();
    assert!(keys.is_empty());
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI3
#[test]
fn history_inspection_checks_only_cluster_without_changing_files() {
    let (env, repo, redis) = history_cluster_fixture();
    let guard = common::Snapshot::over(vec![], vec![env.home().into(), repo.path().into()]);
    let result = env
        .run(
            repo.path(),
            ["install", "--module", "history", "--check", "--json"],
        )
        .success();
    let report: serde_json::Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert!(report["findings"].as_array().unwrap().is_empty());
    assert_eq!(report["resources"].as_array().unwrap().len(), 1);
    insta::assert_json_snapshot!(report["resources"][0]["projects"], @r#"
    [
      "history"
    ]
    "#);
    assert!(guard.changed().is_empty());
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg("instrukt:*")
        .query(&mut redis.connection())
        .unwrap();
    assert!(keys.is_empty());
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI2
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI3
#[test]
fn todos_filter_keeps_history_prerequisites_out_of_its_scope() {
    let mut env = TestEnv::new();
    manager(&mut env);
    let repo = project(&env, "work", "{todos: true}");
    repo.write("instrukt.yaml", "schema_version: 6\nproject: work\nmodules: {todos: true}\ngithub: {repository: fixture/project}\n");
    let github = env.with_fake_github();
    github.add_repo("fixture/project");
    env.run(env.home(), ["install", "--module", "todos"])
        .success();
    env.run(env.home(), ["install", "--module", "todos", "--check"])
        .success();
    let full = env
        .run(env.home(), ["install", "--check", "--json"])
        .failure();
    let result: serde_json::Value = serde_json::from_slice(&full.get_output().stdout).unwrap();
    assert_eq!(result["findings"].as_array().unwrap().len(), 1);
    insta::assert_json_snapshot!(result["findings"], @r#"
    [
      "history Redis: `redis.cluster.url` required for enabled Redis transport"
    ]
    "#);
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI5
#[test]
fn history_upgrades_legacy_owned_acl_through_a_qualified_socket_and_keeps_query_database() {
    let mut f = owned_redis_fixture();
    let url = format!("{}?db=3", f.url);
    f.history_cluster(&url);
    let (file, current, _) = f.legacy_acl();
    f.env
        .run(f.env.home(), ["install", "--module", "history"])
        .success();
    assert_eq!(std::fs::read_to_string(file).unwrap(), current);
    f.env
        .run(
            f.env.home(),
            [
                "history",
                "search",
                "test",
                "--computer",
                "bbbbbbbbbbbbbbbb",
                "--timeout",
                "1",
                "--json",
            ],
        )
        .failure();
    let mut info = redis::IntoConnectionInfo::into_connection_info(url.as_str()).unwrap();
    let password = instrukt_core::config::resolve_secret(
        &instrukt_core::home::Home::at_user_home(f.env.home().into()),
        "${INSTRUKT_REDIS_PASSWORD}",
    )
    .unwrap();
    let auth = info
        .redis_settings()
        .clone()
        .set_username("instrukt")
        .set_password(&password);
    info = info.set_redis_settings(auth);
    let mut selected = redis::Client::open(info).unwrap().get_connection().unwrap();
    assert!(
        redis::cmd("EXISTS")
            .arg("instrukt:history:requests:bbbbbbbbbbbbbbbb")
            .query::<bool>(&mut selected)
            .unwrap()
    );
    assert!(
        !redis::cmd("EXISTS")
            .arg("instrukt:history:requests:bbbbbbbbbbbbbbbb")
            .query::<bool>(&mut f.connection)
            .unwrap()
    );
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI5
#[test]
fn history_repairs_owned_acl_through_a_canonical_parent_alias() {
    let f = owned_redis_fixture();
    let alias = f._short.path().join("other");
    std::os::unix::fs::symlink(f.env.home(), &alias).unwrap();
    let url = format!(
        "unix://{}",
        alias.join(".instrukt/state/redis/redis.sock").display()
    );
    f.history_cluster(&url);
    let (file, current, _) = f.legacy_acl();
    f.env
        .run(f.env.home(), ["install", "--module", "history"])
        .success();
    assert_eq!(std::fs::read_to_string(file).unwrap(), current);
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI7
#[test]
fn history_only_demand_keeps_a_shared_owned_redis_registered() {
    let f = owned_redis_fixture();
    f.history_cluster(&f.url);
    f.env.run(f.env.home(), ["install"]).success();
    f.env.run(f.repo.path(), ["install", "--check"]).success();
    assert!(f.dir.join("redis.sock").exists());
    assert!(f.dir.join("users.acl").exists());
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI5
#[test]
fn history_acl_upgrade_inspection_leaves_legacy_files_and_services_unchanged() {
    let f = owned_redis_fixture();
    f.history_cluster(&f.url);
    let (file, _, old) = f.legacy_acl();
    let calls = std::fs::read(f.env.path("runtime-bin/mutations")).unwrap();
    f.env
        .run(f.repo.path(), ["install", "--module", "history", "--check"])
        .failure();
    assert_eq!(std::fs::read_to_string(file).unwrap(), old);
    assert_eq!(
        std::fs::read(f.env.path("runtime-bin/mutations")).unwrap(),
        calls
    );
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI3
#[test]
fn events_filtered_install_does_not_repair_history_only_redis() {
    let f = owned_redis_fixture();
    f.history_cluster(&f.url);
    let (file, _, old) = f.legacy_acl();
    let calls = std::fs::read(f.env.path("runtime-bin/mutations")).unwrap();
    f.env
        .run(f.env.home(), ["install", "--module", "events"])
        .success();
    assert_eq!(std::fs::read_to_string(file).unwrap(), old);
    assert_eq!(
        std::fs::read(f.env.path("runtime-bin/mutations")).unwrap(),
        calls
    );
}
// spec: project/instrukt/spec/feature/runner/module-installation UC-MI3
#[test]
fn events_filtered_inspection_does_not_check_history_only_redis() {
    let f = owned_redis_fixture();
    f.history_cluster(&f.url);
    f.legacy_acl();
    f.env
        .run(f.repo.path(), ["install", "--module", "events", "--check"])
        .success();
}

fn blueprint_machine() -> TestEnv {
    let mut env = TestEnv::new();
    let bin = env.path("blueprint-bin");
    std::fs::create_dir_all(&bin).unwrap();
    for tool in ["git", "kill", "mkdir", "cat", "chmod"] {
        let found = env
            .command(std::path::Path::new("/usr/bin/which"), env.home())
            .arg(tool)
            .output()
            .unwrap();
        assert!(found.status.success());
        std::os::unix::fs::symlink(
            String::from_utf8(found.stdout).unwrap().trim(),
            bin.join(tool),
        )
        .unwrap();
    }
    env.set_env("PATH", bin.to_str().unwrap());
    env
}

fn fake_uv(env: &TestEnv) {
    let pin = include_str!("../../../instrukt-blueprint/graphify.version").trim();
    let root = env.instrukt_home();
    let script = format!(
        r#"#!/bin/sh
set -eu
[ "$1" = tool ] && [ "$2" = install ] && [ "$3" = 'graphifyy=={pin}' ]
[ "$UV_TOOL_DIR" = '{root}/tools/uv' ]
[ "$UV_TOOL_BIN_DIR" = '{root}/tools/bin' ]
mkdir -p "$UV_TOOL_DIR/graphifyy" "$UV_TOOL_BIN_DIR"
cat > "$UV_TOOL_BIN_DIR/graphify" <<'TOOL'
#!/bin/sh
[ "$1" = --version ] && [ "$PYTHONDONTWRITEBYTECODE" = 1 ] || exit 9
echo 'graphify {pin}'
TOOL
chmod +x "$UV_TOOL_BIN_DIR/graphify"
"#,
        root = root.display()
    );
    let file = env.path("runtime-bin/uv");
    std::fs::write(&file, script).unwrap();
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI12
#[test]
fn blueprint_installs_private_pin_reuses_it_and_repairs_drift() {
    let env = blueprint_machine();
    let _repo = project(&env, "blueprint", "{blueprint: true}");
    fake_uv(&env);
    let before = common::Snapshot::over(vec![], vec![env.home().into()]);
    env.run(env.home(), ["install", "--check", "--module", "blueprint"])
        .failure();
    assert!(before.changed().is_empty(), "{:?}", before.changed());
    env.run(env.home(), ["install", "--module", "blueprint"])
        .success();
    let binary = env.instrukt_home().join("tools/bin/graphify");
    assert!(binary.is_file());
    let installed = std::fs::read(&binary).unwrap();
    // A matching install needs no package operation (nor a working uv).
    std::fs::write(env.path("runtime-bin/uv"), "#!/bin/sh\nexit 23\n").unwrap();
    env.run(env.home(), ["install", "--module", "blueprint"])
        .success();
    env.run(env.home(), ["install", "--check"]).success();
    assert!(!env.path("runtime-bin/calls").exists());
    for tool in ["#!/bin/sh\necho 'graphify 0.0.0'\n", "#!/bin/sh\nexit 8\n"] {
        std::fs::write(&binary, tool).unwrap();
        let before = common::Snapshot::over(vec![], vec![env.home().into()]);
        env.run(env.home(), ["install", "--check"]).failure();
        assert!(before.changed().is_empty(), "{:?}", before.changed());
        fake_uv(&env);
        env.run(env.home(), ["install"]).success();
        assert_eq!(installed, std::fs::read(&binary).unwrap());
    }
    std::fs::remove_file(&binary).unwrap();
    env.run(env.home(), ["install"]).success();
    env.run(env.home(), ["install", "--check"]).success();
    assert_eq!(installed, std::fs::read(&binary).unwrap());
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI12
#[test]
fn blueprint_missing_uv_or_failed_uv_is_unmet_and_disable_retains_tool() {
    let env = blueprint_machine();
    let repo = project(&env, "blueprint", "{blueprint: true}");
    env.run(env.home(), ["install"]).failure();
    assert!(!env.instrukt_home().join("tools/bin/graphify").exists());
    fake_uv(&env);
    std::fs::write(env.path("runtime-bin/uv"), "#!/bin/sh\nexit 23\n").unwrap();
    env.run(env.home(), ["install"]).failure();
    fake_uv(&env);
    env.run(env.home(), ["install"]).success();
    let binary = env.instrukt_home().join("tools/bin/graphify");
    let bytes = std::fs::read(&binary).unwrap();
    repo.write(
        "instrukt.yaml",
        "schema_version: 6\nproject: blueprint\nmodules: {history: false, blueprint: false}\n",
    );
    std::fs::remove_file(env.path("runtime-bin/uv")).unwrap();
    env.run(env.home(), ["install"]).success();
    env.run(env.home(), ["install", "--check"]).success();
    assert_eq!(bytes, std::fs::read(binary).unwrap());
}

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI15
#[test]
fn install_over_definitions_instrukt_wrote_without_its_records() {
    use crate::fixtures::conversing::{Company, MEMBERS, company_home, slack_settings, wait};
    use crate::fixtures::slack::{APP_TOKEN, BOT_TOKEN};
    let mut c = Company::new("");
    let manager = ServiceManager::new(&mut c.env);
    let macos = cfg!(target_os = "macos");
    if !macos {
        // The user's systemd manager starts at boot without a login.
        manager.linger(true);
    }
    let units = |env: &TestEnv| {
        if macos {
            env.home().join("Library/LaunchAgents")
        } else {
            env.home().join(".config/systemd/user")
        }
    };
    let runner_definition = |env: &TestEnv| -> std::path::PathBuf {
        let output = env
            .run(env.home(), ["events", "status", "--json"])
            .success();
        let status: serde_json::Value =
            serde_json::from_slice(&output.get_output().stdout).unwrap();
        status["runner_service"]["paths"][0]
            .as_str()
            .unwrap()
            .into()
    };
    // A definition under another name, and an earlier release's record of
    // what installation wrote.
    std::fs::create_dir_all(units(&c.env)).unwrap();
    // Its name shares the runner's prefix and is still another name.
    let other = units(&c.env).join(if macos {
        "ai.instrukt.runner-old.plist"
    } else {
        "instrukt-runner-old.service"
    });
    std::fs::write(&other, "a definition under another name\n").unwrap();
    let record = c.env.instrukt_home().join("state/installation/owned.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(&record, "{}").unwrap();
    c.env.run(c.env.home(), ["install"]).success();
    wait("the service's connection", || c.slack.connections() == 1);
    assert!(!record.exists());
    // The runner's definition is changed by hand, with no record of what
    // installation wrote: installing writes it again and restarts the service.
    let definition = runner_definition(&c.env);
    let written = std::fs::read_to_string(&definition).unwrap();
    let path = c.env.injected("PATH").unwrap().to_owned();
    std::fs::write(&definition, written.replace(&path, "/changed")).unwrap();
    let pid = manager.pid();
    c.env.run(c.env.home(), ["install"]).success();
    assert_eq!(std::fs::read_to_string(&definition).unwrap(), written);
    assert_ne!(manager.pid(), pid);
    wait("the restarted service's connection", || {
        c.slack.connections() == 2
    });
    assert_eq!(
        std::fs::read(&other).unwrap(),
        b"a definition under another name\n"
    );
    // A second home on the computer, not the default one, whose wake-up an
    // earlier release registered under the default name.
    let second = c.env.path("second-home");
    std::fs::create_dir_all(&second).unwrap();
    std::fs::copy(
        c.env.instrukt_home().join("instrukt.yaml"),
        second.join("instrukt.yaml"),
    )
    .unwrap();
    let root = second.display().to_string();
    let legacy = if macos {
        units(&c.env).join("ai.instrukt.tick.plist")
    } else {
        units(&c.env).join("instrukt-tick.service")
    };
    std::fs::write(
        &legacy,
        if macos {
            format!(
                "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Label</key><string>ai.instrukt.tick</string><key>EnvironmentVariables</key><dict><key>INSTRUKT_HOME</key><string>{root}</string></dict></dict></plist>\n"
            )
        } else {
            format!("[Service]\nEnvironment=\"INSTRUKT_HOME={root}\"\n")
        },
    )
    .unwrap();
    let first = std::fs::read(&definition).unwrap();
    c.env.set_env("INSTRUKT_HOME", &root);
    company_home(
        &c.env,
        "beta",
        APP_TOKEN,
        BOT_TOKEN,
        &slack_settings(MEMBERS, ""),
    );
    // While a company home of the second home cannot be read, its demand is
    // incomplete: the earlier unit naming it is not withdrawn.
    let broken = second.join("companies/broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("instrukt.yaml"), "invalid: [").unwrap();
    c.env.run(c.env.home(), ["install"]).failure();
    assert!(legacy.exists());
    std::fs::remove_dir_all(&broken).unwrap();
    let installed = c.env.run(c.env.home(), ["install", "--json"]).success();
    let report: serde_json::Value = serde_json::from_slice(&installed.get_output().stdout).unwrap();
    // It installs under its own names: the first home's runner definition is
    // byte-identical, and the earlier default-named unit naming it is gone.
    let suffixed: std::path::PathBuf = report["resources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|resource| resource["name"] == "runner service")
        .unwrap()["paths"][0]
        .as_str()
        .unwrap()
        .into();
    assert_ne!(suffixed, definition);
    assert_eq!(suffixed.parent(), definition.parent());
    // The manager runs the second home's service with the second home.
    let unit = if macos {
        suffixed.file_stem()
    } else {
        suffixed.file_name()
    }
    .unwrap()
    .to_str()
    .unwrap();
    let running = manager.request(&["inspect", unit]);
    assert!(running["pid"].as_u64().is_some());
    let environment = if macos {
        &running["definition"]["EnvironmentVariables"]
    } else {
        &running["environment"]
    };
    assert_eq!(environment["INSTRUKT_HOME"].as_str(), Some(root.as_str()));
    assert_eq!(std::fs::read(&definition).unwrap(), first);
    assert!(!legacy.exists());
    c.env.remove_env("INSTRUKT_HOME");
    assert_eq!(runner_definition(&c.env), definition);

    // The private Redis's password is gone from `secrets.env`: installing
    // generates one, writes Redis's configuration and ACL from it and restarts
    // it, and what Redis keeps stays readable with the new password.
    let mut env = TestEnv::new();
    let short = tempfile::Builder::new()
        .prefix("ik-home-")
        .tempdir_in("/tmp")
        .unwrap();
    let alias = short.path().join("h");
    std::os::unix::fs::symlink(env.home(), &alias).unwrap();
    env.set_env("HOME", alias.to_str().unwrap());
    let redis_manager = ServiceManager::new(&mut env);
    if !macos {
        redis_manager.linger(true);
    }
    let _repo = project(&env, "events", "{events: true}");
    let cluster = common::redis::RedisServer::new(&env);
    event_config(&env, None, &cluster.url, None);
    let global = env.instrukt_home().join("instrukt.yaml");
    let settings = std::fs::read_to_string(&global).unwrap();
    std::fs::write(&global, format!("{settings}ticks: off\n")).unwrap();
    // Under the default runner name, a unit an earlier release wrote for
    // another home: this default home demands no runner, and its install
    // leaves that unit as it is.
    let elsewhere = env.path("elsewhere/.instrukt").display().to_string();
    let foreign = if macos {
        env.home()
            .join("Library/LaunchAgents/ai.instrukt.runner.plist")
    } else {
        env.home()
            .join(".config/systemd/user/instrukt-runner.service")
    };
    std::fs::create_dir_all(foreign.parent().unwrap()).unwrap();
    let foreign_text = if macos {
        format!(
            "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Label</key><string>ai.instrukt.runner</string><key>EnvironmentVariables</key><dict><key>INSTRUKT_HOME</key><string>{elsewhere}</string></dict></dict></plist>\n"
        )
    } else {
        format!("[Service]\nEnvironment=\"INSTRUKT_HOME={elsewhere}\"\n")
    };
    std::fs::write(&foreign, &foreign_text).unwrap();
    env.run(env.home(), ["install"]).success();
    assert_eq!(std::fs::read_to_string(&foreign).unwrap(), foreign_text);
    let home = instrukt_core::home::Home::at_user_home(env.home().into());
    let url = format!(
        "unix://{}",
        alias.join(".instrukt/state/redis/redis.sock").display()
    );
    let connect = |password: &str| {
        let mut info = redis::IntoConnectionInfo::into_connection_info(url.as_str()).unwrap();
        let auth = info
            .redis_settings()
            .clone()
            .set_username("instrukt")
            .set_password(password);
        info = info.set_redis_settings(auth);
        redis::Client::open(info).unwrap().get_connection()
    };
    let before =
        instrukt_core::config::resolve_secret(&home, "${INSTRUKT_REDIS_PASSWORD}").unwrap();
    redis::cmd("SET")
        .arg("instrukt:retained:test")
        .arg("kept")
        .query::<()>(&mut connect(&before).unwrap())
        .unwrap();
    let secrets = env.instrukt_home().join("secrets.env");
    let kept = std::fs::read_to_string(&secrets)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with("INSTRUKT_REDIS_PASSWORD="))
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    std::fs::write(&secrets, kept).unwrap();
    // The Redis it restarts is still loading its data when it first
    // answers: installation waits until it has loaded, then succeeds. The
    // Redis executable is now one that answers as a loading Redis does for as
    // long as the test says.
    let loading = env.path("rl");
    std::fs::create_dir_all(&loading).unwrap();
    let real = common::tool_executable("redis-server");
    let server = env.path("runtime-bin/redis-server");
    std::fs::write(
        &server,
        format!(
            "#!/bin/sh\ncase \"$1\" in *.conf) ;; *) exec '{real}' \"$@\" ;; esac\nrm -f '{rl}/i.sock'\n'{python}' '{stand_in}' \"$1\" '{rl}/i.sock' '{rl}' &\nexec '{real}' \"$1\" --unixsocket '{rl}/i.sock'\n",
            real = real.display(),
            rl = loading.display(),
            python = common::tool_executable("python3").display(),
            stand_in = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/support/loading-redis.py")
                .display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(loading.join("loading"), "").unwrap();
    let mut install = env
        .instrukt_process(env.home())
        .arg("install")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait("the restarted Redis answering as it loads", || {
        loading.join("asked").exists()
    });
    std::fs::remove_file(loading.join("loading")).unwrap();
    assert!(install.wait().unwrap().success());
    let after = instrukt_core::config::resolve_secret(&home, "${INSTRUKT_REDIS_PASSWORD}").unwrap();
    assert_ne!(after, before);
    let value: String = redis::cmd("GET")
        .arg("instrukt:retained:test")
        .query(&mut connect(&after).unwrap())
        .unwrap();
    assert_eq!(value, "kept");
    assert!(connect(&before).is_err());

    // The private Redis named only as history's cluster endpoint, its
    // password gone again: installing history generates one before the
    // endpoint's references are resolved, and the data stays readable.
    std::fs::write(
        &global,
        format!(
            "schema_version: 6\nrunner: {{runtime: t3}}\nticks: off\nredis:\n  cluster: {{url: '{url}', username: '${{INSTRUKT_REDIS_USERNAME}}', password: '${{INSTRUKT_REDIS_PASSWORD}}'}}\n"
        ),
    )
    .unwrap();
    _repo.write(
        "instrukt.yaml",
        "schema_version: 6\nproject: events\nmodules: {events: false, history: true}\n",
    );
    let kept = std::fs::read_to_string(&secrets)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with("INSTRUKT_REDIS_PASSWORD="))
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    std::fs::write(&secrets, kept).unwrap();
    env.run(env.home(), ["install", "--module", "history"])
        .success();
    let history =
        instrukt_core::config::resolve_secret(&home, "${INSTRUKT_REDIS_PASSWORD}").unwrap();
    assert_ne!(history, after);
    let value: String = redis::cmd("GET")
        .arg("instrukt:retained:test")
        .query(&mut connect(&history).unwrap())
        .unwrap();
    assert_eq!(value, "kept");
    assert_eq!(std::fs::read_to_string(&foreign).unwrap(), foreign_text);

    // The other home's runner stays registered under the default name with
    // its definition file gone: the environment the manager keeps for it
    // names that home, and this default home's install leaves it running.
    let job = if macos {
        format!(
            "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Label</key><string>ai.instrukt.runner</string><key>ProgramArguments</key><array><string>/bin/sleep</string><string>600</string></array><key>RunAtLoad</key><true/><key>EnvironmentVariables</key><dict><key>INSTRUKT_HOME</key><string>{elsewhere}</string></dict></dict></plist>\n"
        )
    } else {
        format!(
            "[Service]\nExecStart=/bin/sleep 600\nEnvironment=\"INSTRUKT_HOME={elsewhere}\"\n\n[Install]\nWantedBy=default.target\n"
        )
    };
    std::fs::write(&foreign, &job).unwrap();
    let unit = if macos {
        redis_manager.request(&["load", "-w", foreign.to_str().unwrap()]);
        "ai.instrukt.runner"
    } else {
        redis_manager.request(&["daemon-reload"]);
        redis_manager.request(&["enable", "--now", "instrukt-runner.service"]);
        "instrukt-runner.service"
    };
    let running = redis_manager.request(&["inspect", unit])["pid"].as_u64();
    assert!(running.is_some());
    std::fs::remove_file(&foreign).unwrap();
    env.run(env.home(), ["install"]).success();
    if macos {
        assert_eq!(redis_manager.request(&["list", unit])["ok"], true);
    }
    assert_eq!(
        redis_manager.request(&["inspect", unit])["pid"].as_u64(),
        running
    );
}

// spec: project/instrukt/spec/feature/configuration/configuration-editing UC-CE3
#[test]
fn installation_reports_event_responses_without_the_event_transport() {
    let mut env = TestEnv::new();
    manager(&mut env);
    // The project's layer arrives with event responses on; the machine names
    // no event transport.
    let _repo = project(&env, "events", "{events: true}");
    let checked = env
        .run(env.home(), ["install", "--check", "--json"])
        .failure();
    let report: serde_json::Value = serde_json::from_slice(&checked.get_output().stdout).unwrap();
    let base = env.path("");
    let canonical = base.canonicalize().unwrap();
    let findings = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| {
            finding
                .as_str()
                .unwrap()
                .replace(canonical.to_str().unwrap(), "[base]/")
                .replace(base.to_str().unwrap(), "[base]/")
                .replace("[base]//", "[base]/")
        })
        .collect::<Vec<_>>();
    insta::assert_json_snapshot!(findings, @r#"
    [
      "project events: `events` [base]/home/.instrukt/state/configuration/events.yaml has no retained desired configuration; run instrukt install to record it",
      "[base]/events/instrukt.yaml `modules.events`: event responses need the event transport, and `redis.cluster.url` is not set; set it in the global layer first (`instrukt config patch --global`)"
    ]
    "#);
}

/// The computer's service manager as a stand-in at the process boundary:
/// launchd on macOS, the user's systemd manager (with `loginctl`) on Linux.
/// It runs each definition's real command line and environment, keeps
/// loaded, enabled and running apart, restarts exited services by their own
/// policy and starts them again after a simulated restart of the computer.
struct ServiceManager {
    child: std::process::Child,
    socket: std::path::PathBuf,
    _dir: tempfile::TempDir,
    unit: &'static str,
    agents: std::path::PathBuf,
}
impl ServiceManager {
    fn new(env: &mut TestEnv) -> Self {
        let python = env
            .command(std::path::Path::new("/usr/bin/which"), env.home())
            .arg("python3")
            .output()
            .unwrap();
        let python = String::from_utf8(python.stdout).unwrap().trim().to_owned();
        let (script, unit, commands): (&str, &str, &[&str]) = if cfg!(target_os = "macos") {
            ("launchd.py", "ai.instrukt.runner", &["launchctl"])
        } else {
            (
                "systemd.py",
                "instrukt-runner.service",
                &["systemctl", "loginctl"],
            )
        };
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/support")
            .join(script);
        let dir = tempfile::Builder::new()
            .prefix("instrukt-manager-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("control");
        let units = env.home().join(".config/systemd/user");
        let child = env
            .command(std::path::Path::new(&python), env.home())
            .arg(&script)
            .arg("serve")
            .arg(&socket)
            .arg(&units)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        crate::common::wait_until(
            "the service manager's socket",
            std::time::Duration::from_secs(5),
            || socket.exists(),
        );
        for command in commands {
            let wrapper = env.path("runtime-bin").join(command);
            std::fs::write(
                &wrapper,
                format!(
                    "#!/bin/sh\nexec '{python}' '{}' call '{}' \"$@\"\n",
                    script.display(),
                    socket.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self {
            child,
            socket,
            _dir: dir,
            unit: if cfg!(target_os = "macos") {
                "ai.instrukt.runner"
            } else {
                unit
            },
            agents: env.home().join("Library/LaunchAgents"),
        }
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
    /// The service's live process id, if it has one.
    fn pid(&self) -> Option<u64> {
        self.request(&["inspect", self.unit])["pid"].as_u64()
    }
    /// How many times the manager has started the service's process.
    fn starts(&self) -> u64 {
        self.request(&["inspect", self.unit])["starts"]
            .as_u64()
            .unwrap()
    }
    /// The service's process dies; the manager's own policy restarts it.
    fn kill(&self) {
        self.request(&["kill-process", self.unit]);
    }
    /// The service stays registered with no process.
    fn hold(&self) {
        let operation = if cfg!(target_os = "macos") {
            "hold"
        } else {
            "stop"
        };
        self.request(&[operation, self.unit]);
    }
    /// The service is started.
    fn start(&self) {
        let operation = if cfg!(target_os = "macos") {
            "kickstart"
        } else {
            "start"
        };
        self.request(&[operation, self.unit]);
    }
    /// The computer restarts: every process ends and the manager starts
    /// what is registered to start.
    fn reboot(&self) {
        if cfg!(target_os = "macos") {
            self.request(&["reboot", self.agents.to_str().unwrap()]);
        } else {
            self.request(&["reboot"]);
        }
    }
    /// Whether the manager still has the service registered.
    fn registered(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.request(&["list", self.unit])["ok"] == true
        } else {
            self.request(&["inspect", self.unit])["enabled"] == true
        }
    }
    /// The user's linger state, on Linux.
    fn linger(&self, on: bool) {
        self.request(&["linger", if on { "yes" } else { "no" }]);
    }
}
impl Drop for ServiceManager {
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

// spec: project/instrukt/spec/feature/runner/module-installation UC-MI13
#[test]
fn an_owned_service_keeps_the_runner_running_for_a_companys_conversations() {
    use crate::fixtures::conversing::{Company, Running, wait};
    let mut c = Company::new("");
    // The installed executable sits where an upgrade replaces it.
    let installed = c.env.path("installed/instrukt");
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_instrukt"), &installed).unwrap();
    let manager = ServiceManager::new(&mut c.env);
    let instrukt = |c: &Company, args: &[&str]| {
        c.env
            .command(&installed, c.env.home())
            .args(args)
            .output()
            .unwrap()
    };
    let status = |c: &Company| -> instrukt_runner::installation::ServiceStatus {
        let output = instrukt(c, &["events", "status", "--json"]);
        assert!(output.status.success());
        let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        serde_json::from_value(status["runner_service"].clone()).unwrap()
    };
    let state = |c: &Company| status(c).state;
    use instrukt_runner::installation::ServiceState;
    // Demanded and not installed: inspection reports it unmet.
    assert!(!instrukt(&c, &["install", "--check"]).status.success());
    assert_eq!(state(&c), ServiceState::Unmet);
    // Installing starts the runner as a service: it connects the company's
    // connector without anyone starting it.
    let first = instrukt(&c, &["install", "--json"]);
    wait("the service's connector connection", || {
        c.slack.connections() == 1
    });
    if cfg!(target_os = "linux") {
        // The user's manager does not start at boot without a login: that
        // prerequisite is reported, and installation leaves it as it is.
        assert!(!first.status.success());
        let report: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
        insta::assert_json_snapshot!(report["findings"], @r#"
[
  "user service manager: the user's systemd manager does not start at boot without a login, so instrukt's owned units wait for one; enable it with `loginctl enable-linger`; it is needed by the runner service"
]
"#);
        assert!(!instrukt(&c, &["install", "--check"]).status.success());
        // Status shows the running service with that cause.
        let service = status(&c);
        assert_eq!(service.state, ServiceState::Running);
        insta::assert_json_snapshot!(service.cause, @r#""the user's systemd manager does not start at boot without a login, so instrukt's owned units wait for one; enable it with `loginctl enable-linger`""#);
        manager.linger(true);
    } else {
        assert!(first.status.success());
    }
    // Installing again restarts the service: a new process, which connects
    // the connector again.
    let pid = manager.pid();
    assert!(instrukt(&c, &["install"]).status.success());
    assert_ne!(manager.pid(), pid);
    wait("the reinstalled service's connection", || {
        c.slack.connections() == 2
    });
    assert!(instrukt(&c, &["install", "--check"]).status.success());
    assert_eq!(state(&c), ServiceState::Running);
    // While the service runs, a run started by hand is refused naming it.
    let refused = instrukt(&c, &["run"]);
    assert!(!refused.status.success());
    insta::assert_snapshot!(
        "second-run-refused-while-the-service-runs",
        String::from_utf8_lossy(&refused.stderr)
    );
    // Its process is killed: the manager starts it again.
    manager.kill();
    wait("the restarted service's connection", || {
        c.slack.connections() == 3
    });
    // Registered but stopped, it is unmet until installation starts it.
    manager.hold();
    assert!(!instrukt(&c, &["install", "--check"]).status.success());
    assert_eq!(state(&c), ServiceState::Unmet);
    assert!(instrukt(&c, &["install"]).status.success());
    wait("the started service's connection", || {
        c.slack.connections() == 4
    });
    // The executable is upgraded in place, its definition unchanged: the
    // service is still loaded and alive, and installation restarts it on the
    // new executable.
    let definition = |c: &Company| {
        status(c)
            .paths
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect::<Vec<_>>()
    };
    let before = definition(&c);
    if cfg!(target_os = "linux") {
        // Disabled while its process keeps running, it is enabled again and
        // restarted.
        manager.request(&["disable", manager.unit]);
        assert!(manager.pid().is_some());
        assert!(!manager.registered());
    }
    std::fs::remove_file(&installed).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_instrukt"), &installed).unwrap();
    assert!(instrukt(&c, &["install", "--check"]).status.success());
    let pid = manager.pid();
    assert!(instrukt(&c, &["install"]).status.success());
    assert_ne!(manager.pid(), pid);
    wait("the upgraded service's connection", || {
        c.slack.connections() == 5
    });
    assert_eq!(definition(&c), before);
    assert_eq!(state(&c), ServiceState::Running);
    assert!(manager.registered());
    // The runner exits at once while its configuration cannot be read; the
    // manager keeps starting it, and once the configuration is back the
    // service runs again.
    let global = c.env.instrukt_home().join("instrukt.yaml");
    let configuration = std::fs::read(&global).unwrap();
    std::fs::write(&global, "invalid: [").unwrap();
    let starts = manager.starts();
    manager.kill();
    wait("two failed starts", || manager.starts() >= starts + 2);
    std::fs::write(&global, &configuration).unwrap();
    wait("the recovered service's connection", || {
        c.slack.connections() == 6
    });
    // The definition changes: installation repairs it and restarts the
    // service with it.
    let path = c.env.injected("PATH").unwrap().to_owned();
    c.env.set_env("PATH", &format!("{path}:/changed-tools"));
    assert!(instrukt(&c, &["install"]).status.success());
    wait("the repaired service's connection", || {
        c.slack.connections() == 7
    });
    assert_ne!(definition(&c), before);
    // A run started by hand holds the runner lock, and a second one started
    // by hand waits for it, saying so.
    manager.hold();
    let by_hand = Running::spawn(
        c.env
            .command(&installed, c.env.home())
            .arg("run")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()),
    );
    wait("the hand-started run's connection", || {
        c.slack.connections() == 8
    });
    let said = c.env.path("waiting-run.stderr");
    let mut waiting = Running::spawn(
        c.env
            .command(&installed, c.env.home())
            .arg("run")
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(&said).unwrap()),
    );
    wait("the second run waiting", || {
        std::fs::read_to_string(&said).is_ok_and(|text| !text.is_empty())
    });
    // The service started meanwhile runs and waits too.
    manager.start();
    wait("the waiting service", || state(&c) == ServiceState::Running);
    assert_eq!(c.slack.connections(), 8);
    // The holder dies without letting go while the run waiting by hand is
    // held still: the service takes the lock. Resumed, that run is refused
    // naming the service.
    waiting.signal(rustix::process::Signal::STOP);
    wait("the waiting run held still", || waiting.stopped());
    drop(by_hand);
    wait("the service taking over", || c.slack.connections() == 9);
    waiting.signal(rustix::process::Signal::CONT);
    wait("the waiting run's end", || waiting.exited().is_some());
    assert!(!waiting.exited().unwrap().success());
    insta::assert_snapshot!(
        "a-waiting-run-refused-when-the-service-takes-the-lock",
        std::fs::read_to_string(&said).unwrap()
    );
    // The computer restarts: the manager starts the runner again.
    let pid = manager.pid();
    manager.reboot();
    wait("the service's connection after the restart", || {
        c.slack.connections() == 10
    });
    assert_ne!(manager.pid(), pid);
    // A neighbouring company home that cannot be read leaves the demand
    // unresolved; the readable company's service still runs.
    let broken = c.env.instrukt_home().join("companies/broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("instrukt.yaml"), "invalid: [").unwrap();
    assert!(!instrukt(&c, &["install"]).status.success());
    assert_eq!(state(&c), ServiceState::Running);
    // Removing the company's last declared connector ends the known demand,
    // but while that home stays unreadable nothing is withdrawn.
    let document = c.env.instrukt_home().join("companies/acme/instrukt.yaml");
    let mut settings: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(&document).unwrap()).unwrap();
    settings.as_mapping_mut().unwrap().remove("connectors");
    std::fs::write(&document, serde_yaml_ng::to_string(&settings).unwrap()).unwrap();
    assert_eq!(state(&c), ServiceState::Unmet);
    assert!(!instrukt(&c, &["install"]).status.success());
    assert!(manager.registered());
    // Its definition file is gone while the manager keeps it registered:
    // it is still found, reported obsolete and withdrawn.
    for path in status(&c).paths {
        std::fs::remove_file(path).unwrap();
    }
    assert!(manager.registered());
    // Once it can be read, the demand is resolved: the next install
    // withdraws the service.
    std::fs::remove_dir_all(&broken).unwrap();
    assert_eq!(state(&c), ServiceState::Obsolete);
    assert!(instrukt(&c, &["install"]).status.success());
    assert!(!manager.registered());
    assert_eq!(state(&c), ServiceState::NoDemand);
    // A computer with no service manager that keeps a process running: the
    // prerequisite is reported and installation claims no success.
    let bare = Company::new("");
    let unmet = bare.env.run(bare.env.home(), ["install"]).failure();
    insta::assert_snapshot!(
        "runner-service-without-a-service-manager",
        String::from_utf8_lossy(&unmet.get_output().stdout)
    );
}
