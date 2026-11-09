use super::*;
use crate::process::Invocation;
use instrukt_core::fsx::atomic_write_with_permissions;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Definition {
    pub name: String,
    pub unit: String,
    pub scheduler: String,
    pub files: BTreeMap<PathBuf, String>,
    #[serde(default)]
    pub registered: bool,
}
impl Definition {
    fn label(&self) -> String {
        let unit = if self.scheduler == "cron" {
            "# instrukt-tick"
        } else {
            &self.unit
        };
        format!("{} ({})", unit, self.scheduler)
    }
}
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Owned {
    schema_version: u32,
    resources: BTreeMap<String, Definition>,
    #[serde(default)]
    assets: BTreeMap<PathBuf, String>,
}
impl Owned {
    pub fn load(home: &Home) -> anyhow::Result<Self> {
        match std::fs::read(home.state_dir().join("installation/owned.json")) {
            Ok(bytes) => {
                let owned: Self = serde_json::from_slice(&bytes)?;
                anyhow::ensure!(
                    owned.schema_version == 1,
                    "unsupported installation ownership version; upgrade instrukt"
                );
                Ok(owned)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                schema_version: 1,
                ..Self::default()
            }),
            Err(e) => Err(e.into()),
        }
    }
    fn save(&self, home: &Home) -> anyhow::Result<()> {
        atomic_write_with_permissions(
            &home.state_dir().join("installation/owned.json"),
            &serde_json::to_vec_pretty(self)?,
            std::fs::Permissions::from_mode(0o600),
        )?;
        Ok(())
    }
    pub fn retained_file(
        &mut self,
        request: &Request<'_>,
        file: &Path,
        content: &str,
    ) -> anyhow::Result<()> {
        match read(file)? {
            Some(current) => {
                anyhow::ensure!(
                    self.assets.get(file) == Some(&current),
                    "ownership conflict at {}; restore recorded owned configuration before repair",
                    file.display()
                );
                anyhow::ensure!(
                    current == content,
                    "owned Redis configuration differs; resolve the configuration/credential change before rerunning install"
                );
                anyhow::ensure!(
                    std::fs::metadata(file)?.permissions().mode() & 0o077 == 0,
                    "private Redis file permissions must be owner-only at {}",
                    file.display()
                );
            }
            None => {
                anyhow::ensure!(
                    !request.check,
                    "missing private Redis file {}; run instrukt install",
                    file.display()
                );
                self.assets.insert(file.into(), content.into());
                self.save(request.home)?;
                atomic_write_with_permissions(
                    file,
                    content.as_bytes(),
                    std::fs::Permissions::from_mode(0o600),
                )?;
            }
        }
        Ok(())
    }
    pub fn reconcile(
        &mut self,
        request: &Request<'_>,
        desired: &Definition,
        required: bool,
        complete: bool,
        projects: &[String],
        report: &mut Report,
    ) -> anyhow::Result<()> {
        if !required {
            return self.withdraw_named(request, &desired.name, complete, report);
        }
        let previous = self.resources.get(&desired.name);
        if let Some(previous) = previous {
            anyhow::ensure!(
                previous.scheduler == desired.scheduler,
                "{} manager changed; restore the recorded manager before reconciling",
                desired.name
            );
        }
        let mut changed = false;
        for (file, content) in &desired.files {
            match read(file)? {
                None => changed = true,
                Some(current) => {
                    let recorded = previous.and_then(|p| p.files.get(file));
                    anyhow::ensure!(
                        recorded == Some(&current),
                        "ownership conflict at {}; restore the recorded owned definition or move the foreign file, then rerun instrukt install",
                        file.display()
                    );
                    changed |= current != *content;
                }
            }
        }
        if desired.scheduler == "cron" {
            let table = Invocation::program("crontab", &["-l"]).run(Duration::from_secs(30))?;
            anyhow::ensure!(
                table.success || table.stderr.to_ascii_lowercase().contains("no crontab for"),
                "cannot inspect crontab; restore access before install"
            );
            let marked = table
                .stdout
                .lines()
                .filter(|line| line.trim_end().ends_with("# instrukt-tick"))
                .collect::<Vec<_>>();
            anyhow::ensure!(
                marked.len() <= 1
                    && marked
                        .first()
                        .is_none_or(|line| previous.is_some_and(|old| old.unit == *line)),
                "ownership conflict in crontab; restore the recorded instrukt line or move the foreign marker before install"
            );
            changed |= marked.first().copied() != Some(desired.unit.as_str());
        }
        let registered = previous.is_some_and(|p| p.registered);
        let active = previous.is_some() && running(desired)?;
        if previous.is_none() {
            anyhow::ensure!(
                !running(desired)?,
                "ownership conflict: {} is already loaded without an installation record; move the foreign unit before running instrukt install",
                desired.unit
            );
        }
        if request.check {
            if changed || !active || !registered {
                report.finding(&desired.name,"missing/broken registration; run instrukt install after resolving prerequisites");
            }
            report.resource(
                &desired.name,
                if changed || !active || !registered {
                    "unmet"
                } else {
                    "reused"
                },
                desired.files.keys().cloned().collect(),
                projects,
                format!(
                    "{}: checked definitions, recorded ownership and service-manager state",
                    desired.label()
                ),
            );
            return Ok(());
        }
        if !changed && active {
            if !registered {
                if let Some(record) = self.resources.get_mut(&desired.name) {
                    record.registered = true;
                }
                self.save(request.home)?;
            }
            report.resource(
                &desired.name,
                "reused",
                desired.files.keys().cloned().collect(),
                projects,
                format!("{}: working registration unchanged", desired.label()),
            );
            return Ok(());
        }
        // Keep receiver socket custody when the process is restarted. A port change
        // crosses the operator's edge route, so require alignment before replacing it.
        if desired.name == "receiver" && active && changed {
            anyhow::bail!(
                "receiver definition/port changed; align the edge route and stop the owned receiver socket before rerunning instrukt install"
            );
        }
        let repaired = previous.is_some();
        self.resources.insert(desired.name.clone(), desired.clone());
        self.save(request.home)?;
        if desired.name == "wake-up" {
            crate::install::install(request.scheduler, &request.wake)?;
        } else {
            for (file, content) in &desired.files {
                atomic_write_with_permissions(
                    file,
                    content.as_bytes(),
                    std::fs::Permissions::from_mode(0o600),
                )?;
            }
            start(desired)?;
        }
        if let Some(record) = self.resources.get_mut(&desired.name) {
            record.registered = true;
        }
        self.save(request.home)?;
        report.resource(
            &desired.name,
            if repaired { "repaired" } else { "installed" },
            desired.files.keys().cloned().collect(),
            projects,
            format!("{}: registered with the service manager", desired.label()),
        );
        Ok(())
    }
    pub fn withdraw_named(
        &mut self,
        request: &Request<'_>,
        name: &str,
        complete: bool,
        report: &mut Report,
    ) -> anyhow::Result<()> {
        let Some(record) = self.resources.get(name) else {
            return Ok(());
        };
        if !record.registered {
            return Ok(());
        }
        anyhow::ensure!(
            complete,
            "incomplete project demand; restore unreadable projects before withdrawing {name}"
        );
        for (file, expected) in &record.files {
            if let Some(actual) = read(file)? {
                anyhow::ensure!(
                    actual == *expected,
                    "ownership conflict at {}; restore the recorded definition before withdrawal",
                    file.display()
                );
            }
        }
        if request.check {
            report.finding(
                name,
                "obsolete owned registration; run instrukt install to withdraw it",
            );
            return Ok(());
        }
        stop(record)?;
        for file in record.files.keys() {
            if file.is_file() {
                std::fs::remove_file(file)?;
            }
        }
        report.resource(
            name,
            "withdrawn",
            record.files.keys().cloned().collect(),
            &[],
            format!(
                "{}: retained data, credentials and shared executables unchanged",
                record.label()
            ),
        );
        if record.scheduler == "systemd" {
            required(&["systemctl", "--user", "daemon-reload"])?;
        }
        if let Some(record) = self.resources.get_mut(name) {
            record.registered = false;
        }
        self.save(request.home)?;
        Ok(())
    }
}
fn read(file: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::symlink_metadata(file) {
        Ok(meta) => anyhow::ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "ownership conflict at {}; expected a regular owned file",
            file.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    Ok(Some(std::fs::read_to_string(file)?))
}
fn command(argv: &[&str]) -> anyhow::Result<bool> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("missing service manager"))?;
    Ok(Invocation::program(program, args)
        .run(Duration::from_secs(30))?
        .success)
}
fn required(argv: &[&str]) -> anyhow::Result<()> {
    anyhow::ensure!(
        command(argv)?,
        "service manager rejected {}; inspect the unit and rerun instrukt install",
        argv.join(" ")
    );
    Ok(())
}
fn running(d: &Definition) -> anyhow::Result<bool> {
    match d.scheduler.as_str() {
        "launchd" => command(&["launchctl", "list", &d.unit]),
        "systemd" => Ok(
            command(&["systemctl", "--user", "is-active", "--quiet", &d.unit])?
                && command(&["systemctl", "--user", "is-enabled", "--quiet", &d.unit])?,
        ),
        "cron" => {
            let result = Invocation::program("crontab", &["-l"]).run(Duration::from_secs(30))?;
            Ok(result.success && result.stdout.lines().any(|line| line == d.unit))
        }
        _ => anyhow::bail!("unknown recorded service manager; restore its configuration"),
    }
}
fn start(d: &Definition) -> anyhow::Result<()> {
    if d.scheduler == "launchd" {
        let file = d
            .files
            .keys()
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing plist"))?;
        required(&["launchctl", "load", "-w", &file.display().to_string()])
    } else {
        required(&["systemctl", "--user", "daemon-reload"])?;
        required(&["systemctl", "--user", "enable", "--now", &d.unit])
    }
}
fn systemd_loaded(unit: &str) -> anyhow::Result<bool> {
    let result = Invocation::program(
        "systemctl",
        &["--user", "show", unit, "--property=LoadState", "--value"],
    )
    .run(Duration::from_secs(30))?;
    anyhow::ensure!(
        result.success,
        "cannot inspect systemd unit {unit}; restore user-manager access before withdrawal"
    );
    match result.stdout.trim() {
        "loaded" => Ok(true),
        "not-found" => Ok(false),
        _ => anyhow::bail!(
            "systemd unit {unit} has an unsafe load state; repair its definition before withdrawal"
        ),
    }
}
fn stop(d: &Definition) -> anyhow::Result<()> {
    match d.scheduler.as_str() {
        "launchd" => {
            let file = d
                .files
                .keys()
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing plist"))?;
            if running(d)? {
                required(&["launchctl", "unload", &file.display().to_string()])?;
            }
        }
        "systemd" => {
            if systemd_loaded(&d.unit)? {
                required(&["systemctl", "--user", "disable", "--now", &d.unit])?;
            }
            if d.name == "receiver" && systemd_loaded("instrukt-webhooks.service")? {
                required(&["systemctl", "--user", "stop", "instrukt-webhooks.service"])?;
            }
        }
        "cron" => {
            let result = Invocation::program("crontab", &["-l"]).run(Duration::from_secs(30))?;
            anyhow::ensure!(
                result.success,
                "cannot read crontab; restore access before withdrawal"
            );
            let text = result
                .stdout
                .lines()
                .filter(|line| !line.trim_end().ends_with("# instrukt-tick"))
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
            let written = Invocation::program("crontab", &["-"])
                .input(text.as_bytes())
                .run(Duration::from_secs(30))?;
            anyhow::ensure!(written.success, "cannot withdraw owned crontab line");
        }
        _ => anyhow::bail!("unsupported recorded scheduler"),
    }
    Ok(())
}
pub(super) fn tick(request: &Request<'_>) -> Definition {
    let files = crate::install::registration_files(request.scheduler, &request.wake)
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect();
    Definition {
        name: "wake-up".into(),
        unit: match request.scheduler {
            Scheduler::Launchd => crate::install::LABEL.into(),
            Scheduler::SystemdUser => "instrukt-tick.timer".into(),
            Scheduler::Crontab => crate::install::crontab_with_wakeup("", &request.wake)
                .trim_end()
                .into(),
        },
        scheduler: request.scheduler.name().into(),
        files,
        registered: false,
    }
}
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn quote(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    )
}
fn native(request: &Request<'_>, name: &str, args: &[String], socket: Option<u16>) -> Definition {
    let label = format!("ai.instrukt.{name}");
    let mut files = BTreeMap::new();
    let unit;
    if request.scheduler == Scheduler::Launchd {
        unit = label.clone();
        let program = args
            .iter()
            .map(|s| format!("<string>{}</string>", xml(s)))
            .collect::<String>();
        let sockets=socket.map_or("<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>".into(),|port|format!("<key>Sockets</key><dict><key>instrukt</key><dict><key>SockNodeName</key><string>127.0.0.1</string><key>SockServiceName</key><string>{port}</string><key>SockType</key><string>stream</string></dict></dict>"));
        files.insert(request.home.user_home().join(format!("Library/LaunchAgents/{label}.plist")),format!("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Label</key><string>{label}</string><key>ProgramArguments</key><array>{program}</array><key>EnvironmentVariables</key><dict><key>HOME</key><string>{}</string><key>INSTRUKT_HOME</key><string>{}</string><key>INSTRUKT_DEVELOPMENT</key><string>{}</string><key>PATH</key><string>{}</string></dict><key>Umask</key><integer>63</integer>{sockets}</dict></plist>\n",xml(&request.home.machine_user_home().display().to_string()),xml(&request.home.root().display().to_string()),u8::from(request.home.is_development()),xml(request.wake.path_env)));
    } else {
        let stem = format!("instrukt-{name}");
        let dir = request
            .wake
            .config_home
            .map(Path::to_path_buf)
            .unwrap_or_else(|| request.home.user_home().join(".config"))
            .join("systemd/user");
        let extra = if socket.is_some() {
            format!("Requires={stem}.socket\nAfter={stem}.socket\n")
        } else {
            String::new()
        };
        files.insert(dir.join(format!("{stem}.service")),format!("[Unit]\nDescription=instrukt {name}\n{extra}\n[Service]\nExecStart={}\nEnvironment={}\nEnvironment={}\nEnvironment={}\nEnvironment={}\nUMask=0077\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n",args.iter().map(|s|quote(s)).collect::<Vec<_>>().join(" "),quote(&format!("HOME={}",request.home.machine_user_home().display())),quote(&format!("PATH={}",request.wake.path_env)),quote(&format!("INSTRUKT_HOME={}",request.home.root().display())),quote(&format!("INSTRUKT_DEVELOPMENT={}",u8::from(request.home.is_development())))));
        unit = if let Some(port) = socket {
            files.insert(dir.join(format!("{stem}.socket")),format!("[Socket]\nListenStream=127.0.0.1:{port}\nFileDescriptorName=instrukt\n\n[Install]\nWantedBy=sockets.target\n"));
            format!("{stem}.socket")
        } else {
            format!("{stem}.service")
        };
    }
    Definition {
        name: if name == "webhooks" { "receiver" } else { name }.into(),
        unit,
        scheduler: request.scheduler.name().into(),
        files,
        registered: false,
    }
}
pub(super) fn receiver(request: &Request<'_>, port: u16) -> Definition {
    native(
        request,
        "webhooks",
        &[
            request.wake.executable.display().to_string(),
            "webhooks".into(),
            "serve".into(),
            "--socket-activation".into(),
        ],
        Some(port),
    )
}
pub(super) fn redis(request: &Request<'_>, executable: &Path, config: &Path) -> Definition {
    native(
        request,
        "redis",
        &[
            executable.display().to_string(),
            config.display().to_string(),
        ],
        None,
    )
}
