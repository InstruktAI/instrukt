//! An owned effective receiver invocation, separate from native socket custody.
use crate::{install::Scheduler, process::Invocation};
use instrukt_core::home::Home;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf};
use std::{
    io::{Read, Write},
    time::Duration,
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReceiverLaunch {
    schema_version: u32,
    pub executable: PathBuf,
    pub environment: BTreeMap<String, String>,
    port: u16,
}
impl ReceiverLaunch {
    pub fn file(home: &Home, port: u16) -> PathBuf {
        home.state_dir()
            .join("receiver")
            .join(format!("{port}.json"))
    }
    pub fn desired(request: &super::Request<'_>, port: u16) -> Self {
        Self {
            schema_version: 1,
            executable: request.wake.executable.into(),
            port,
            environment: BTreeMap::from([
                (
                    "HOME".into(),
                    request.home.machine_user_home().display().to_string(),
                ),
                (
                    "INSTRUKT_HOME".into(),
                    request.home.root().display().to_string(),
                ),
                (
                    "INSTRUKT_DEVELOPMENT".into(),
                    u8::from(request.home.is_development()).to_string(),
                ),
                ("PATH".into(), request.wake.path_env.into()),
            ]),
        }
    }
    pub fn encode(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
    pub fn parse(bytes: &[u8], port: u16) -> anyhow::Result<Self> {
        let value: Self = serde_json::from_slice(bytes)?;
        anyhow::ensure!(
            value.schema_version == 1 && value.port == port && value.executable.is_absolute(),
            "invalid owned receiver invocation; rerun instrukt install"
        );
        anyhow::ensure!(
            value.environment.keys().all(|key| matches!(
                key.as_str(),
                "HOME" | "INSTRUKT_HOME" | "INSTRUKT_DEVELOPMENT" | "PATH"
            )),
            "invalid receiver environment shape"
        );
        Ok(value)
    }
    pub fn applied(&self, digest: &str) -> bool {
        std::env::var("INSTRUKT_RECEIVER_APPLIED").ok().as_deref() == Some(digest)
            && std::env::current_exe().ok().as_deref() == Some(self.executable.as_path())
            && self
                .environment
                .iter()
                .all(|(key, value)| std::env::var(key).ok().as_ref() == Some(value))
    }
}
pub(crate) fn owned(home: &Home, port: u16) -> anyhow::Result<Option<(ReceiverLaunch, String)>> {
    let file = ReceiverLaunch::file(home, port);
    let Some(contents) = super::_services::owned_receiver_file(home, &file)? else {
        return Ok(None);
    };
    let digest = hex::encode(Sha256::digest(contents.as_bytes()));
    Ok(Some((
        ReceiverLaunch::parse(contents.as_bytes(), port)?,
        digest,
    )))
}

#[derive(Serialize, Deserialize)]
struct Execution {
    pid: u32,
    invocation: String,
}

pub(crate) const PROBE_PATH: &str = "/_instrukt/receiver";

/// Fingerprint only this running process and its machine-local invocation settings.
pub(crate) fn running(port: u16) -> anyhow::Result<String> {
    let environment = ["HOME", "INSTRUKT_HOME", "INSTRUKT_DEVELOPMENT", "PATH"]
        .into_iter()
        .map(|key| Ok((key.into(), std::env::var(key)?)))
        .collect::<Result<BTreeMap<String, String>, std::env::VarError>>()?;
    let invocation = ReceiverLaunch {
        schema_version: 1,
        executable: std::env::current_exe()?,
        environment,
        port,
    };
    Ok(serde_json::to_string(&Execution {
        pid: std::process::id(),
        invocation: hex::encode(Sha256::digest(invocation.encode()?.as_bytes())),
    })?)
}

/// A dedicated machine-local probe never evaluates project webhook declarations.
pub(crate) fn verify(home: &Home, port: u16) -> anyhow::Result<()> {
    let (_, desired) = owned(home, port)?.ok_or_else(|| {
        anyhow::anyhow!("receiver execution has no owned invocation; run instrukt install")
    })?;
    let pid = live_process(home, port)?;
    let timeout = Duration::from_secs(3);
    let mut stream = std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        timeout,
    )?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(
        format!("GET {PROBE_PATH} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    )?;
    let mut response = String::new();
    stream.take(4096).read_to_string(&mut response)?;
    let (headers, body) = response.split_once("\r\n\r\n").ok_or_else(|| {
        anyhow::anyhow!("receiver execution proof is missing; run instrukt install")
    })?;
    anyhow::ensure!(
        headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            == Some("200"),
        "running receiver does not prove the desired executable/environment; run instrukt install"
    );
    let observed: Execution = serde_json::from_str(body)?;
    anyhow::ensure!(
        observed.pid == pid && observed.invocation == desired,
        "running receiver does not prove the desired executable/environment; run instrukt install"
    );
    Ok(())
}

/// launchctl's documented override affects one invocation; the receiver arms the next
/// start before serving so a cached pre-invocation launcher cannot return on restart.
pub(crate) fn arm(home: &Home, port: u16) -> anyhow::Result<()> {
    let Some((Scheduler::Launchd, unit)) = super::_services::receiver_job(home, port)? else {
        return Ok(());
    };
    let (desired, _) =
        owned(home, port)?.ok_or_else(|| anyhow::anyhow!("missing owned receiver invocation"))?;
    let uid = Invocation::program("id", &["-u"]).run(Duration::from_secs(10))?;
    anyhow::ensure!(uid.success, "cannot identify the launchd user domain");
    let uid = uid.stdout.trim().parse::<u32>()?;
    let executable = desired.executable.display().to_string();
    let mut args = vec![
        "debug".to_owned(),
        format!("gui/{uid}/{unit}"),
        "--program".into(),
        executable.clone(),
        "--environment".into(),
    ];
    args.extend(
        desired
            .environment
            .iter()
            .map(|(key, value)| format!("{key}={value}")),
    );
    args.extend([
        "--".into(),
        executable,
        "webhooks".into(),
        "serve".into(),
        "--socket-activation".into(),
    ]);
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let result = Invocation::program("launchctl", &refs).run(Duration::from_secs(30))?;
    anyhow::ensure!(
        result.success,
        "launchd cannot apply the desired receiver invocation; restore launchctl debug access and rerun instrukt install"
    );
    Ok(())
}

// Check manager process state before connecting: a read must not activate a cold service.
fn live_process(home: &Home, port: u16) -> anyhow::Result<u32> {
    let (scheduler, unit) = super::_services::receiver_job(home, port)?
        .ok_or_else(|| anyhow::anyhow!("receiver has no owned service identity"))?;
    let pid = match scheduler {
        Scheduler::Launchd => {
            let result =
                Invocation::program("launchctl", &["list"]).run(Duration::from_secs(10))?;
            anyhow::ensure!(result.success, "cannot inspect launchd receiver process");
            result.stdout.lines().find_map(|line| {
                let mut fields = line.split_whitespace();
                let pid = fields.next()?;
                fields.next()?;
                (fields.next()? == unit)
                    .then(|| pid.parse::<u32>().ok())
                    .flatten()
            })
        }
        Scheduler::SystemdUser => {
            let service = format!("{}.service", unit.trim_end_matches(".socket"));
            let result = Invocation::program(
                "systemctl",
                &["--user", "show", &service, "--property=MainPID", "--value"],
            )
            .run(Duration::from_secs(10))?;
            anyhow::ensure!(result.success, "cannot inspect systemd receiver process");
            result.stdout.trim().parse::<u32>().ok()
        }
        Scheduler::Crontab => None,
    };
    pid.filter(|pid| *pid != 0)
        .ok_or_else(|| anyhow::anyhow!("receiver process is not running; run instrukt install"))
}
