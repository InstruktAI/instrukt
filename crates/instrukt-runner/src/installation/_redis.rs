use super::_demand::Demand;
use super::{_services, Disposition, Report, Request, ResourceKind};
use crate::install::Scheduler;
use crate::process::Invocation;
use crate::transport::{EVENT_STREAM, Endpoint};
use instrukt_core::config::Configuration;
use instrukt_core::config::{self, Layer};
use serde_json::{Value, json};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;

pub(super) fn ensure(
    request: &Request<'_>,
    owned: &mut _services::Owned,
    demand: &Demand,
    report: &mut Report,
) -> anyhow::Result<()> {
    let config = Configuration::load_global(request.home)?;
    // A missing cluster prerequisite is independent of a local setup. Never pick a cluster host.
    let events_required =
        !demand.events.is_empty() && request.module != Some(instrukt_core::config::Module::History);
    let settings = if events_required {
        config.redis(request.env)?
    } else {
        let cluster = config.cluster_redis()?;
        config::RedisSettings {
            local: Some(cluster.clone()),
            cluster,
        }
    };
    let mut projects = if events_required {
        demand.events.clone()
    } else {
        Vec::new()
    };
    projects.extend(demand.history_owned_redis.clone());
    projects.sort();
    projects.dedup();
    let redis_dir = request.home.state_dir().join("redis");
    let local_owned = settings
        .local
        .as_ref()
        .map(|endpoint| super::_demand::owned_server(&endpoint.url, &redis_dir.join("redis.sock")))
        .transpose()?
        .unwrap_or(true);
    if local_owned {
        let local = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                request.scheduler != Scheduler::Crontab,
                "owned Redis requires launchd or systemd user services; configure an operator-owned local Unix-socket Redis"
            );
            let server = super::_package::server(request)?;
            let version = Invocation::program(&server.display().to_string(), &["--version"])
                .run(Duration::from_secs(10))?;
            anyhow::ensure!(
                version.success,
                "redis-server --version failed; repair the executable"
            );
            let password_ref = "${INSTRUKT_REDIS_PASSWORD}";
            if !request.check && config::resolve_secret(request.home, password_ref).is_err() {
                // Only generate for an unconfigured local endpoint; established credentials
                // failing to resolve remain a finding rather than being rotated.
                anyhow::ensure!(
                    settings.local.is_none(),
                    "owned local credential is missing/unreadable; restore its secrets.env value before repair"
                );
                config::generate_secret(request.home, "INSTRUKT_REDIS_PASSWORD")?;
            }
            let password = config::resolve_secret(request.home, password_ref)?;
            use sha2::{Digest, Sha256};
            let hash = hex::encode(Sha256::digest(password.as_bytes()));
            let conf = redis_dir.join("redis.conf");
            let acl = redis_dir.join("users.acl");
            let expected_acl = format!(
                "user default off\nuser instrukt on #{hash} ~instrukt:* +@stream +@scripting +ping +info +command +acl|whoami +acl|dryrun +get +set +del +exists +expire +ttl +type +rpush +blpop +multi +exec +select\n"
            );
            let quote = |p: &Path| {
                format!(
                    "\"{}\"",
                    p.display()
                        .to_string()
                        .replace('\\', "\\\\")
                        .replace('"', "\\\"")
                )
            };
            let expected_conf = format!(
                "port 0\nunixsocket {}\nunixsocketperm 600\ndir {}\nappendonly yes\naclfile {}\ndaemonize no\n",
                quote(&redis_dir.join("redis.sock")),
                quote(&redis_dir),
                quote(&acl)
            );
            if !request.check {
                std::fs::create_dir_all(&redis_dir)?;
                std::fs::set_permissions(&redis_dir, std::fs::Permissions::from_mode(0o700))?;
            }
            owned.retained_file(request, &conf, &expected_conf)?;
            let mut definition = _services::redis(request, &server, &conf);
            owned.redis_acl(request, &mut definition, &acl, expected_acl)?;
            owned.reconcile(
                request,
                &definition,
                true,
                demand.complete,
                &projects,
                report,
            )?;
            // Authenticate and check the private socket before recording its endpoint.
            let endpoint = config::RedisEndpoint {
                url: format!("unix://{}", redis_dir.join("redis.sock").display()),
                username: Some("instrukt".into()),
                password: Some(password),
                max_connections: 50,
                socket_timeout: 15,
            };
            let mut conn = connect(&endpoint)?;
            private_socket(&endpoint)?;
            if !request.check {
                diagnose(&mut conn)?;
            }
            if settings.local.is_none() && !request.check {
                let patch = serde_yaml_ng::from_str(&format!(
                    "redis:\n  local:\n    url: '{}'\n    username: '${{INSTRUKT_REDIS_USERNAME}}'\n    password: '{password_ref}'\n",
                    endpoint.url
                ))?;
                config::set_secret(request.home, "INSTRUKT_REDIS_USERNAME", "instrukt")?;
                config::patch(
                    request.home,
                    &Layer::Global,
                    &config::revision(request.home, &Layer::Global)?,
                    &patch,
                )?;
            }
            Ok(())
        })();
        if let Err(error) = local {
            if settings.local.is_some() {
                return Err(error);
            }
            // Only a fully verified cluster can carry unconfigured-local fallback.
            let verified = (|| -> anyhow::Result<()> {
                let mut cluster = connect(&settings.cluster)?;
                permissions(&mut cluster)
            })();
            if let Err(fallback) = verified {
                anyhow::bail!(
                    "local setup failed: {error}; cluster fallback failed: {fallback}; resolve prerequisites and rerun instrukt install"
                );
            }
            report.resource(
                ResourceKind::LocalRedis,
                Disposition::ExternallySupplied,
                vec![],
                &projects,
                format!("verified cluster fallback; local Redis remains unconfigured: {error}"),
            );
        }
    } else {
        report.resource(
            ResourceKind::LocalRedis,
            Disposition::ExternallySupplied,
            vec![],
            &projects,
            "operator-owned local Redis: configuration and service unchanged",
        );
    }
    if !events_required {
        return Ok(());
    }
    let config = Configuration::load_global(request.home)?;
    let settings = config.redis(request.env)?;
    let mut identities = Vec::new();
    let state = crate::state::RunnerState::new(request.home);
    for (scope, endpoint) in [
        ("cluster", Some(&settings.cluster)),
        ("local", settings.local.as_ref()),
    ] {
        let Some(endpoint) = endpoint else {
            continue;
        };
        if scope == "local" {
            private_socket(endpoint)?;
        }
        let identity = Endpoint::new(endpoint, scope, &state)?.identity;
        if identities.contains(&identity) {
            continue;
        }
        identities.push(identity);
        let mut connection = connect(endpoint)?;
        permissions(&mut connection)?;
        if !request.check {
            diagnose(&mut connection)?;
        }
    }
    if !request.check {
        crate::consumers::prepare_installation(request.home, request.env)?;
    }
    inspect(request, report)?;
    Ok(())
}
fn connect(settings: &config::RedisEndpoint) -> anyhow::Result<redis::Connection> {
    use redis::IntoConnectionInfo;
    let mut info = settings.url.as_str().into_connection_info().map_err(|_| {
        anyhow::anyhow!("invalid Redis endpoint; configure a URL without inline credentials")
    })?;
    let mut auth = info.redis_settings().clone();
    anyhow::ensure!(
        auth.password().is_none() && auth.username().is_none(),
        "Redis URL contains inline credentials; use secrets.env references"
    );
    if let Some(username) = &settings.username {
        auth = auth.set_username(username);
    }
    if let Some(password) = &settings.password {
        auth = auth.set_password(password);
    }
    info = info.set_redis_settings(auth);
    let client =
        redis::Client::open(info).map_err(|_| anyhow::anyhow!("invalid Redis endpoint"))?;
    let connection=client.get_connection_with_timeout(Duration::from_secs(3)).map_err(|e|anyhow::anyhow!("Redis connection/authentication failed ({:?}); restore endpoint and secret references",e.kind()))?;
    connection.set_read_timeout(Some(Duration::from_secs(3)))?;
    connection.set_write_timeout(Some(Duration::from_secs(3)))?;
    Ok(connection)
}
fn private_socket(endpoint: &config::RedisEndpoint) -> anyhow::Result<()> {
    let socket = endpoint
        .url
        .strip_prefix("unix://")
        .ok_or_else(|| anyhow::anyhow!("local Redis needs a Unix socket"))?;
    let socket = socket.split('?').next().unwrap_or(socket);
    let metadata = std::fs::symlink_metadata(socket)?;
    anyhow::ensure!(
        metadata.file_type().is_socket() && metadata.permissions().mode() & 0o077 == 0,
        "local Redis socket must be owner-only; repair socket permissions before installation"
    );
    Ok(())
}
fn server_version(connection: &mut redis::Connection) -> anyhow::Result<()> {
    let server: redis::InfoDict =
        redis::cmd("INFO")
            .arg("server")
            .query(connection)
            .map_err(|e| {
                anyhow::anyhow!(
                    "cannot inspect Redis version ({:?}); grant INFO and use Redis 7.0 or later",
                    e.kind()
                )
            })?;
    let version = server.get::<String>("redis_version").ok_or_else(|| {
        anyhow::anyhow!("Redis server did not report a version; Redis 7.0 or later is required")
    })?;
    let major = version
        .split('.')
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| {
            anyhow::anyhow!("unrecognized Redis version; Redis 7.0 or later is required")
        })?;
    anyhow::ensure!(
        major >= 7,
        "Redis 7.0 or later is required (server reports {version}); upgrade Redis before installation"
    );
    Ok(())
}
fn permissions(connection: &mut redis::Connection) -> anyhow::Result<()> {
    server_version(connection)?;
    let commands = [
        "XADD",
        "XGROUP",
        "XREADGROUP",
        "XACK",
        "XAUTOCLAIM",
        "EVAL",
        "EVALSHA",
        "SCRIPT",
        "ACL",
    ];
    let info: Vec<redis::Value> = redis::cmd("COMMAND")
        .arg("INFO")
        .arg(&commands)
        .query(connection)
        .map_err(|e| {
            anyhow::anyhow!(
                "cannot inspect required Redis commands ({:?}); grant Streams, Lua and ACL access",
                e.kind()
            )
        })?;
    anyhow::ensure!(
        info.len() == commands.len() && info.iter().all(|v| !matches!(v, redis::Value::Nil)),
        "Redis lacks required Streams, stale-pending reclaim, Lua or ACL support; upgrade Redis"
    );
    let user: String = redis::cmd("ACL")
        .arg("WHOAMI")
        .query(connection)
        .map_err(|e| anyhow::anyhow!("Redis ACL inspection failed ({:?})", e.kind()))?;
    let claim_key = format!(
        "instrukt:ingress:claim:{}",
        crate::events::occurrence_id(&["installation-check"])
    );
    for args in [
        vec!["XADD", "instrukt:diagnostic:check", "*", "event", "check"],
        vec![
            "XGROUP",
            "CREATE",
            "instrukt:events",
            "diagnostic",
            "$",
            "MKSTREAM",
        ],
        vec![
            "XREADGROUP",
            "GROUP",
            "diagnostic",
            "check",
            "STREAMS",
            "instrukt:events",
            ">",
        ],
        vec![
            "XAUTOCLAIM",
            "instrukt:events",
            "diagnostic",
            "check",
            "0",
            "0-0",
        ],
        vec!["EVAL", "return 1", "0"],
        vec![
            "EVALSHA",
            "0000000000000000000000000000000000000000",
            "2",
            EVENT_STREAM,
            &claim_key,
        ],
        vec!["SCRIPT", "LOAD", "return 1"],
        vec!["XADD", EVENT_STREAM, "*", "event", "diagnostic"],
        vec![
            "XADD",
            crate::transport::DEAD_LETTER_STREAM,
            "*",
            "record",
            "diagnostic",
        ],
        vec!["DEL", "instrukt:diagnostic:check"],
        vec!["GET", &claim_key],
        vec!["SET", &claim_key, "diagnostic", "EX", "1"],
        vec!["XACK", "instrukt:events", "diagnostic", "0-0"],
        vec!["XPENDING", "instrukt:events", "diagnostic"],
    ] {
        let result: redis::RedisResult<String> = redis::cmd("ACL")
            .arg("DRYRUN")
            .arg(&user)
            .arg(args)
            .query(connection);
        match result {
            Ok(answer) => anyhow::ensure!(
                answer == "OK",
                "Redis ACL denies required transport operations"
            ),
            Err(e) => anyhow::bail!(
                "Redis ACL capability check failed ({:?}); grant transport permissions",
                e.kind()
            ),
        }
    }
    Ok(())
}
fn diagnose(connection: &mut redis::Connection) -> anyhow::Result<()> {
    let key = format!(
        "instrukt:diagnostic:{}",
        instrukt_core::machine::random_hex(16)?
    );
    let checked = (|| -> redis::RedisResult<()> {
        redis::cmd("XADD")
            .arg(&key)
            .arg("*")
            .arg("event")
            .arg("diagnostic")
            .query::<String>(connection)?;
        redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(&key)
            .arg("diagnostic")
            .arg("0")
            .query::<()>(connection)?;
        redis::cmd("XAUTOCLAIM")
            .arg(&key)
            .arg("diagnostic")
            .arg("check")
            .arg(0)
            .arg("0-0")
            .query::<redis::Value>(connection)?;
        redis::cmd("EVAL")
            .arg("return redis.call('XLEN', KEYS[1])")
            .arg(1)
            .arg(&key)
            .query::<u64>(connection)?;
        Ok(())
    })();
    let cleaned = redis::cmd("DEL").arg(&key).query::<u64>(connection);
    checked.map_err(|e| {
        anyhow::anyhow!(
            "Redis diagnostic failed ({:?}); grant Streams, Lua and ACL operations",
            e.kind()
        )
    })?;
    cleaned.map_err(|e| anyhow::anyhow!("Redis diagnostic cleanup failed ({:?})", e.kind()))?;
    Ok(())
}
pub(super) fn inspect(request: &Request<'_>, report: &mut Report) -> anyhow::Result<Vec<Value>> {
    let settings = Configuration::load_global(request.home)?.redis(request.env)?;
    let machine=std::fs::read_to_string(request.home.machine_id_file()).map_err(|_|anyhow::anyhow!("machine identity missing/unreadable; run instrukt install or restore the recorded machine-id before reader inspection"))?;
    anyhow::ensure!(
        instrukt_core::machine::MachineId::parse(machine.trim()).is_some(),
        "invalid recorded machine identity; restore it before reader inspection"
    );
    let group = format!("consumer-procedure-activation-{}", machine.trim());
    let mut result = Vec::new();
    let mut checked = std::collections::BTreeMap::<String, Value>::new();
    let state = crate::state::RunnerState::new(request.home);
    for (scope, endpoint) in [
        ("cluster", Some(&settings.cluster)),
        ("local", settings.local.as_ref()),
    ] {
        let Some(endpoint) = endpoint else {
            result.push(json!({"scope":scope,"mode":"unconfigured_local_cluster_fallback"}));
            continue;
        };
        let identity = Endpoint::new(endpoint, scope, &state)?.identity;
        // Local filesystem requirements still apply when both scopes share a connection.
        if scope == "local"
            && let Err(error) = private_socket(endpoint)
        {
            report.finding("Redis local", error);
            let reachable = checked
                .get(&identity)
                .and_then(|value| value.get("reachable"))
                .cloned()
                .unwrap_or(Value::Null);
            result.push(
                json!({"scope":scope,"reachable":reachable,"healthy":false,"socket_private":false}),
            );
            continue;
        }

        if let Some(shared) = checked.get(&identity) {
            let mut value = shared.clone();
            value["scope"] = scope.into();
            value["shared_connection"] = true.into();
            result.push(value);
            continue;
        }
        let inspected = (|| -> anyhow::Result<Value> {
            let mut connection = connect(endpoint)?;
            if scope == "local" {
                private_socket(endpoint)?;
            }
            permissions(&mut connection)?;
            let groups: redis::streams::StreamInfoGroupsReply = redis::cmd("XINFO")
                .arg("GROUPS")
                .arg(EVENT_STREAM)
                .query(&mut connection)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "retained reader inspection failed ({:?}); run instrukt install",
                        e.kind()
                    )
                })?;
            use instrukt_core::state_db::rusqlite::OptionalExtension;
            let prepared=state.read(|conn,path| conn.query_row("SELECT prepared FROM consumer_sources WHERE consumer = 'procedure-activation' AND source = ?1 AND state = 'active'",[&identity],|row|row.get::<_,bool>(0)).optional().map_err(crate::state::sql(path)))?.flatten().unwrap_or(false);
            anyhow::ensure!(
                prepared,
                "local retained-reader coverage is unprepared; run instrukt install"
            );
            let reader = groups
                .groups
                .into_iter()
                .find(|g| g.name == group)
                .ok_or_else(|| {
                    anyhow::anyhow!("required retained reader is unprepared; run instrukt install")
                })?;
            Ok(
                json!({"scope":scope,"reachable":true,"consumer":group,"last_delivered_id":reader.last_delivered_id,"pending":reader.pending,"lag":reader.lag}),
            )
        })();
        match inspected {
            Ok(value) => {
                checked.insert(identity, value.clone());
                result.push(value);
            }
            Err(error) => {
                report.finding(format!("Redis {scope}"), error);
                result.push(json!({"scope":scope,"reachable":false}));
            }
        }
    }
    Ok(result)
}

pub(super) fn dead_letters(request: &Request<'_>) -> anyhow::Result<Vec<Value>> {
    let settings = Configuration::load_global(request.home)?.redis(request.env)?;
    let state = crate::state::RunnerState::new(request.home);
    let mut seen = Vec::new();
    let mut result = Vec::new();
    for (scope, endpoint) in [
        ("cluster", Some(&settings.cluster)),
        ("local", settings.local.as_ref()),
    ] {
        let Some(endpoint) = endpoint else {
            continue;
        };
        let source = Endpoint::new(endpoint, scope, &state)?.identity;
        if seen.contains(&source) {
            continue;
        }
        seen.push(source.clone());
        let mut connection = connect(endpoint)?;
        let entries: redis::streams::StreamRangeReply = redis::cmd("XRANGE")
            .arg(crate::transport::DEAD_LETTER_STREAM)
            .arg("-")
            .arg("+")
            .query(&mut connection)
            .map_err(|e| {
                anyhow::anyhow!(
                    "dead-letter inspection failed for {scope} ({:?}); restore Redis access",
                    e.kind()
                )
            })?;
        for entry in entries.ids {
            let record = entry.get::<String>("record").ok_or_else(|| {
                anyhow::anyhow!("invalid retained dead-letter record at {}", entry.id)
            })?;
            let record: Value = serde_json::from_str(&record)?;
            let mut facts = serde_json::Map::new();
            for field in [
                "consumer",
                "consumer_id",
                "source",
                "original_stream",
                "original_entry_id",
                "attempt_count",
                "first_attempt_at",
                "last_attempt_at",
                "failure",
            ] {
                if let Some(value) = record.get(field) {
                    facts.insert(field.into(), value.clone());
                }
            }
            facts.insert("dead_letter_id".into(), entry.id.into());
            facts.insert("transport_source".into(), source.clone().into());
            result.push(Value::Object(facts));
        }
    }
    Ok(result)
}

/// History requires cluster request/reply only; event groups and local services are separate demand.
pub(super) fn history(
    request: &Request<'_>,
    projects: &[String],
    report: &mut Report,
) -> anyhow::Result<()> {
    let endpoint = Configuration::load_global(request.home)?.cluster_redis()?;
    let mut connection = connect(&endpoint)?;
    server_version(&mut connection)?;
    let user: String = redis::cmd("ACL").arg("WHOAMI").query(&mut connection)?;
    for args in [
        vec!["RPUSH", "instrukt:history:requests:check", "check"],
        vec!["BLPOP", "instrukt:history:requests:check", "1"],
        vec!["RPUSH", "instrukt:history:reply:check", "check"],
        vec!["BLPOP", "instrukt:history:reply:check", "1"],
        vec!["EXPIRE", "instrukt:history:reply:check", "120"],
        vec!["EXPIRE", "instrukt:history:requests:check", "120"],
        vec!["SET", "instrukt:history:ready:check", "1", "EX", "5"],
        vec!["MULTI"],
        vec!["EXEC"],
    ] {
        let answer: String = redis::cmd("ACL")
            .arg("DRYRUN")
            .arg(&user)
            .arg(args)
            .query(&mut connection)?;
        anyhow::ensure!(
            answer == "OK",
            "Redis ACL denies history request/reply; grant history transport permissions"
        );
    }
    report.resource(
        ResourceKind::Redis,
        Disposition::ExternallySupplied,
        vec![],
        projects,
        "cluster Redis verified for history merged search; events not required",
    );
    Ok(())
}
