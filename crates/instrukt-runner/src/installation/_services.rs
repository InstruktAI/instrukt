use super::{Disposition, Report, Request, ResourceKind};
use crate::install::Scheduler;
use crate::process::Invocation;
use instrukt_core::fsx::atomic_write_with_permissions;
use instrukt_core::home::Home;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Definition {
    pub name: ResourceKind,
    pub unit: String,
    pub scheduler: Scheduler,
    pub files: BTreeMap<PathBuf, String>,
    #[serde(default)]
    pub registered: bool,
    #[serde(skip)]
    probe: Option<(u16, String)>,
}
impl Definition {
    fn stem(&self) -> String {
        if self.scheduler == Scheduler::Launchd {
            self.unit.trim_start_matches("ai.instrukt.").into()
        } else {
            self.unit
                .trim_start_matches("instrukt-")
                .trim_end_matches(".socket")
                .into()
        }
    }
    fn port(&self) -> Option<u16> {
        for contents in self.files.values() {
            if let Some((_, tail)) = contents.split_once("ListenStream=127.0.0.1:") {
                return tail.lines().next()?.parse().ok();
            }
            if let Some((_, tail)) = contents.split_once("<key>SockServiceName</key><string>") {
                return tail.split_once("</string>")?.0.parse().ok();
            }
        }
        None
    }
    fn label(&self) -> String {
        let unit = if self.scheduler == Scheduler::Crontab {
            "# instrukt-tick"
        } else {
            &self.unit
        };
        format!("{} ({})", unit, self.scheduler.name())
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Transition {
    before: Vec<Definition>,
    after: Definition,
}
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Owned {
    schema_version: u32,
    resources: BTreeMap<ResourceKind, Definition>,
    #[serde(default)]
    assets: BTreeMap<PathBuf, String>,
    #[serde(default)]
    pending_receiver: Option<Definition>,
    #[serde(default)]
    updates: BTreeMap<ResourceKind, Transition>,
    #[serde(skip)]
    migrated_from: Option<u32>,
}
impl Owned {
    pub fn load(home: &Home) -> anyhow::Result<Self> {
        match std::fs::read(home.state_dir().join("installation/owned.json")) {
            Ok(bytes) => {
                let mut owned: Self = serde_json::from_slice(&bytes)?;
                anyhow::ensure!(
                    matches!(owned.schema_version, 1..=3),
                    "unsupported installation ownership version; upgrade instrukt"
                );
                owned.migrated_from = (owned.schema_version < 3).then_some(owned.schema_version);
                owned.schema_version = 3;
                Ok(owned)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                schema_version: 3,
                ..Self::default()
            }),
            Err(e) => Err(e.into()),
        }
    }
    pub fn persist_migration(&mut self, home: &Home) -> anyhow::Result<()> {
        if self.migrated_from.is_some() {
            self.save(home)?;
            self.migrated_from = None;
        }
        Ok(())
    }
    fn save(&self, home: &Home) -> anyhow::Result<()> {
        atomic_write_with_permissions(
            &home.state_dir().join("installation/owned.json"),
            &serde_json::to_vec_pretty(self)?,
            std::fs::Permissions::from_mode(0o600),
        )?;
        Ok(())
    }
    fn proofs(&self, kind: ResourceKind) -> Vec<&Definition> {
        let mut definitions = self.resources.get(&kind).into_iter().collect::<Vec<_>>();
        if let Some(update) = self.updates.get(&kind) {
            definitions.extend(update.before.iter());
            definitions.push(&update.after);
        }
        if kind == ResourceKind::Receiver
            && let Some(pending) = &self.pending_receiver
        {
            definitions.push(pending);
        }
        definitions
    }
    fn proven(&self, kind: ResourceKind, file: &Path, actual: &str) -> bool {
        self.proofs(kind).into_iter().any(|definition| {
            definition
                .files
                .get(file)
                .is_some_and(|value| value == actual)
        })
    }
    fn begin(&mut self, home: &Home, desired: &Definition) -> anyhow::Result<()> {
        let before = self.proofs(desired.name).into_iter().cloned().collect();
        self.updates.insert(
            desired.name,
            Transition {
                before,
                after: desired.clone(),
            },
        );
        self.save(home)
    }
    fn write_files(&self, desired: &Definition) -> anyhow::Result<()> {
        for (file, content) in &desired.files {
            if let Some(actual) = read(file)? {
                anyhow::ensure!(
                    self.proven(desired.name, file, &actual),
                    "ownership conflict at {}; restore a recorded definition",
                    file.display()
                );
            }
            atomic_write_with_permissions(
                file,
                content.as_bytes(),
                std::fs::Permissions::from_mode(0o600),
            )?;
        }
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
            return self.withdraw_named(request, desired.name, complete, report);
        }
        if desired.name == ResourceKind::Receiver {
            return self.receiver_resource(request, desired, complete, projects, report);
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
                    let recorded = self.proven(desired.name, file, &current);
                    anyhow::ensure!(
                        recorded,
                        "ownership conflict at {}; restore the recorded owned definition or move the foreign file, then rerun instrukt install",
                        file.display()
                    );
                    changed |= current != *content;
                }
            }
        }
        if desired.scheduler == Scheduler::Crontab {
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
                    && marked.first().is_none_or(|line| self
                        .proofs(desired.name)
                        .iter()
                        .any(|old| old.unit == *line)),
                "ownership conflict in crontab; restore the recorded instrukt line or move the foreign marker before install"
            );
            changed |= marked.first().copied() != Some(desired.unit.as_str());
        }
        let registered =
            previous.is_some_and(|p| p.registered) && !self.updates.contains_key(&desired.name);
        let identified = self
            .proofs(desired.name)
            .into_iter()
            .any(|record| record.scheduler == desired.scheduler && record.unit == desired.unit);
        let active = identified && running(desired)?;
        if !identified {
            anyhow::ensure!(
                !running(desired)?,
                "ownership conflict: {} is already loaded without an installation record; move the foreign unit before running instrukt install",
                desired.unit
            );
        }
        if request.check {
            if changed || !active || !registered {
                report.finding(desired.name,"missing/broken registration; run instrukt install after resolving prerequisites");
            }
            report.resource(
                desired.name,
                if changed || !active || !registered {
                    Disposition::Unmet
                } else {
                    Disposition::Reused
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
        if !changed && active && !self.updates.contains_key(&desired.name) {
            if !registered {
                if let Some(record) = self.resources.get_mut(&desired.name) {
                    record.registered = true;
                }
                self.save(request.home)?;
            }
            report.resource(
                desired.name,
                Disposition::Reused,
                desired.files.keys().cloned().collect(),
                projects,
                format!("{}: working registration unchanged", desired.label()),
            );
            return Ok(());
        }
        let repaired = previous.is_some();
        self.begin(request.home, desired)?;
        self.write_files(desired)?;
        if desired.name == ResourceKind::WakeUp {
            crate::install::register(request.scheduler, &request.wake)?;
        } else {
            start(desired)?;
        }
        let mut recorded = desired.clone();
        recorded.registered = true;
        self.resources.insert(desired.name, recorded);
        self.updates.remove(&desired.name);
        self.save(request.home)?;
        report.resource(
            desired.name,
            if repaired {
                Disposition::Repaired
            } else {
                Disposition::Installed
            },
            desired.files.keys().cloned().collect(),
            projects,
            format!("{}: registered with the service manager", desired.label()),
        );
        Ok(())
    }
    pub fn receiver_definition(
        &self,
        request: &Request<'_>,
        port: u16,
        route: String,
    ) -> anyhow::Result<Definition> {
        // Keep an existing listener identity when its address is unchanged.
        let stem = self
            .resources
            .get(&ResourceKind::Receiver)
            .filter(|d| d.port() == Some(port))
            .map(|d| d.stem())
            .unwrap_or_else(|| format!("webhooks-{port}"));
        let mut definition = native(
            request,
            &stem,
            &[
                request.wake.executable.display().to_string(),
                "webhooks".into(),
                "serve".into(),
                "--socket-activation".into(),
            ],
            Some(port),
        );
        let launch = super::_launch::ReceiverLaunch::desired(request, port);
        definition.files.insert(
            super::_launch::ReceiverLaunch::file(request.home, port),
            launch.encode()?,
        );
        definition.probe = Some((port, route));
        Ok(definition)
    }
    fn receiver_resource(
        &mut self,
        request: &Request<'_>,
        desired: &Definition,
        complete: bool,
        projects: &[String],
        report: &mut Report,
    ) -> anyhow::Result<()> {
        let previous = self.resources.get(&ResourceKind::Receiver).cloned();
        if let Some(old) = &previous {
            anyhow::ensure!(
                old.scheduler == desired.scheduler,
                "receiver manager changed; restore its recorded manager before installation"
            );
        }
        if self
            .pending_receiver
            .as_ref()
            .is_some_and(|pending| pending.unit != desired.unit)
        {
            anyhow::ensure!(
                !request.check,
                "incomplete receiver transition; run instrukt install to reconcile it"
            );
            anyhow::ensure!(
                complete,
                "complete project demand is required before withdrawing a pending receiver"
            );
            let pending = self
                .pending_receiver
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing pending receiver identity"))?;
            self.validate_proofs(ResourceKind::Receiver)?;
            stop(pending)?;
            self.remove_proven(pending)?;
            self.pending_receiver = None;
            self.save(request.home)?;
        }
        let proof = self
            .pending_receiver
            .as_ref()
            .filter(|d| d.unit == desired.unit)
            .or(previous.as_ref().filter(|d| d.unit == desired.unit));
        let mut changed = false;
        for (file, content) in &desired.files {
            match read(file)? {
                None => changed = true,
                Some(actual) => {
                    anyhow::ensure!(
                        self.proven(ResourceKind::Receiver, file, &actual),
                        "receiver ownership conflict at {}; restore the recorded definition before installation",
                        file.display()
                    );
                    changed |= actual != *content;
                }
            }
        }
        let same_unit = previous.as_ref().is_some_and(|d| d.unit == desired.unit);
        let active = running(desired)?;
        anyhow::ensure!(
            !active || proof.is_some(),
            "receiver {} is already registered without matching ownership",
            desired.unit
        );
        let incomplete =
            self.pending_receiver.is_some() || self.updates.contains_key(&ResourceKind::Receiver);
        let port = desired
            .port()
            .ok_or_else(|| anyhow::anyhow!("receiver listener has no port"))?;
        let execution = super::_launch::verify(request.home, port);
        let effective = execution.is_ok();
        if request.check {
            if changed || !active || !same_unit || incomplete || !effective {
                report.finding(
                    "receiver",
                    format!(
                        "missing, stale or incomplete owned receiver; run instrukt install{}",
                        execution
                            .err()
                            .map(|e| format!(": {e}"))
                            .unwrap_or_default()
                    ),
                );
            }
            report.resource(
                ResourceKind::Receiver,
                if changed || !active || !same_unit || incomplete || !effective {
                    Disposition::Unmet
                } else {
                    Disposition::Reused
                },
                desired.files.keys().cloned().collect(),
                projects,
                format!("{}: checked owned listener", desired.label()),
            );
            return Ok(());
        }
        if !changed && active && same_unit && !incomplete && effective {
            report.resource(
                ResourceKind::Receiver,
                Disposition::Reused,
                desired.files.keys().cloned().collect(),
                projects,
                format!("{}: listener retained", desired.label()),
            );
            return Ok(());
        }
        if !same_unit {
            anyhow::ensure!(
                complete,
                "complete project demand is required before changing receiver placement"
            );
        }
        if previous.is_some() && !same_unit {
            self.validate_proofs(ResourceKind::Receiver)?;
        }
        self.begin(request.home, desired)?;
        self.pending_receiver = Some(desired.clone());
        self.save(request.home)?;
        self.write_files(desired)?;
        if !active {
            start(desired)?;
        }
        restart_receiver(request.home, desired)?;
        super::_launch::verify(request.home, port)?;
        if let Some((port, route)) = &desired.probe {
            super::receiver_health(*port, route)?;
        }
        if let Some(old) = &previous
            && !same_unit
        {
            self.validate_proofs(ResourceKind::Receiver)?;
            stop(old)?;
            self.remove_proven(old)?;
            report.notices.push(format!("receiver edge route changed from port {} to {}; align the operator's external route",old.port().map(|p|p.to_string()).unwrap_or_else(||"unknown".into()),desired.port().map(|p|p.to_string()).unwrap_or_else(||"unknown".into())));
        }
        let mut recorded = desired.clone();
        recorded.registered = true;
        self.resources.insert(ResourceKind::Receiver, recorded);
        self.pending_receiver = None;
        self.updates.remove(&ResourceKind::Receiver);
        self.save(request.home)?;
        report.resource(
            ResourceKind::Receiver,
            if previous.is_some() {
                Disposition::Repaired
            } else {
                Disposition::Installed
            },
            desired.files.keys().cloned().collect(),
            projects,
            format!("{}: receiver ready with socket custody", desired.label()),
        );
        Ok(())
    }
    fn validate_proofs(&self, kind: ResourceKind) -> anyhow::Result<()> {
        for definition in self.proofs(kind) {
            for file in definition.files.keys() {
                if let Some(actual) = read(file)? {
                    anyhow::ensure!(
                        self.proven(kind, file, &actual),
                        "ownership conflict at {}; restore a recorded definition before withdrawal",
                        file.display()
                    );
                }
            }
        }
        Ok(())
    }
    fn remove_proven(&self, definition: &Definition) -> anyhow::Result<()> {
        for file in definition.files.keys() {
            if let Some(actual) = read(file)? {
                anyhow::ensure!(
                    self.proven(definition.name, file, &actual),
                    "ownership conflict at {}; restore a recorded definition",
                    file.display()
                );
                std::fs::remove_file(file)?;
            }
        }
        Ok(())
    }
    pub fn withdraw_named(
        &mut self,
        request: &Request<'_>,
        name: ResourceKind,
        complete: bool,
        report: &mut Report,
    ) -> anyhow::Result<()> {
        let definitions = self
            .proofs(name)
            .into_iter()
            .rev()
            .cloned()
            .collect::<Vec<_>>();
        if definitions.is_empty()
            || (!self.updates.contains_key(&name)
                && (name != ResourceKind::Receiver || self.pending_receiver.is_none())
                && !definitions.iter().any(|d| d.registered))
        {
            return Ok(());
        }
        anyhow::ensure!(
            complete,
            "incomplete project demand; restore unreadable projects before withdrawing {name}"
        );
        self.validate_proofs(name)?;
        if definitions
            .iter()
            .any(|d| d.scheduler == Scheduler::Crontab)
        {
            verify_cron(&definitions)?;
        }
        if request.check {
            report.finding(
                name,
                "obsolete owned registration; run instrukt install to withdraw it",
            );
            return Ok(());
        }
        let mut stopped = std::collections::BTreeSet::new();
        for definition in &definitions {
            if stopped.insert(&definition.unit) {
                stop(definition)?;
            }
            self.remove_proven(definition)?;
            if definition.scheduler == Scheduler::SystemdUser {
                required(&["systemctl", "--user", "daemon-reload"])?;
            }
        }
        report.resource(name, Disposition::Withdrawn,
            definitions.iter().flat_map(|d| d.files.keys().cloned()).collect(), &[],
            "owned registrations withdrawn; retained data, credentials and shared executables unchanged");
        if let Some(record) = self.resources.get_mut(&name) {
            record.registered = false;
        }
        if name == ResourceKind::Receiver {
            self.pending_receiver = None;
        }
        self.updates.remove(&name);
        self.save(request.home)?;
        Ok(())
    }
}
fn restart_receiver(home: &Home, definition: &Definition) -> anyhow::Result<()> {
    if definition.scheduler == Scheduler::Launchd {
        super::_launch::arm(
            home,
            definition
                .port()
                .ok_or_else(|| anyhow::anyhow!("missing receiver port"))?,
        )?;
        let uid = Invocation::program("id", &["-u"]).run(Duration::from_secs(10))?;
        anyhow::ensure!(uid.success, "cannot identify the launchd user domain");
        let uid = uid.stdout.trim().parse::<u32>()?;
        required(&[
            "launchctl",
            "kickstart",
            "-k",
            &format!("gui/{uid}/{}", definition.unit),
        ])
    } else {
        required(&["systemctl", "--user", "daemon-reload"])?;
        required(&[
            "systemctl",
            "--user",
            "restart",
            &format!("{}.service", definition.unit.trim_end_matches(".socket")),
        ])
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
    match d.scheduler {
        Scheduler::Launchd => command(&["launchctl", "list", &d.unit]),
        Scheduler::SystemdUser => {
            Ok(
                command(&["systemctl", "--user", "is-active", "--quiet", &d.unit])?
                    && command(&["systemctl", "--user", "is-enabled", "--quiet", &d.unit])?,
            )
        }
        Scheduler::Crontab => {
            let result = Invocation::program("crontab", &["-l"]).run(Duration::from_secs(30))?;
            Ok(result.success && result.stdout.lines().any(|line| line == d.unit))
        }
    }
}
fn start(d: &Definition) -> anyhow::Result<()> {
    if d.scheduler == Scheduler::Launchd {
        let file = d
            .files
            .keys()
            .find(|file| file.extension().is_some_and(|ext| ext == "plist"))
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
fn verify_cron(definitions: &[Definition]) -> anyhow::Result<()> {
    let table = Invocation::program("crontab", &["-l"]).run(Duration::from_secs(30))?;
    anyhow::ensure!(
        table.success,
        "cannot inspect crontab; restore access before withdrawal"
    );
    let marked = table
        .stdout
        .lines()
        .filter(|line| line.trim_end().ends_with("# instrukt-tick"))
        .collect::<Vec<_>>();
    anyhow::ensure!(
        marked
            .iter()
            .all(|line| definitions.iter().any(|d| *line == d.unit))
            && marked.len() <= 1,
        "crontab ownership conflict; restore the recorded instrukt line before withdrawal"
    );
    Ok(())
}
fn stop(d: &Definition) -> anyhow::Result<()> {
    match d.scheduler {
        Scheduler::Launchd => {
            let file = d
                .files
                .keys()
                .find(|file| file.extension().is_some_and(|ext| ext == "plist"))
                .ok_or_else(|| anyhow::anyhow!("missing plist"))?;
            if running(d)? {
                required(&["launchctl", "unload", &file.display().to_string()])?;
            }
        }
        Scheduler::SystemdUser => {
            if systemd_loaded(&d.unit)? {
                required(&["systemctl", "--user", "disable", "--now", &d.unit])?;
            }
            if d.name == ResourceKind::Receiver
                && systemd_loaded(&format!("{}.service", d.unit.trim_end_matches(".socket")))?
            {
                required(&[
                    "systemctl",
                    "--user",
                    "stop",
                    &format!("{}.service", d.unit.trim_end_matches(".socket")),
                ])?;
            }
        }
        Scheduler::Crontab => {
            let result = Invocation::program("crontab", &["-l"]).run(Duration::from_secs(30))?;
            anyhow::ensure!(
                result.success,
                "cannot read crontab; restore access before withdrawal"
            );
            let text = result
                .stdout
                .lines()
                .filter(|line| *line != d.unit)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
            let written = Invocation::program("crontab", &["-"])
                .input(text.as_bytes())
                .run(Duration::from_secs(30))?;
            anyhow::ensure!(written.success, "cannot withdraw owned crontab line");
        }
    }
    Ok(())
}
pub(super) fn tick(request: &Request<'_>) -> Definition {
    let files = crate::install::registration_files(request.scheduler, &request.wake)
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect();
    Definition {
        name: ResourceKind::WakeUp,
        unit: match request.scheduler {
            Scheduler::Launchd => crate::install::LABEL.into(),
            Scheduler::SystemdUser => "instrukt-tick.timer".into(),
            Scheduler::Crontab => crate::install::crontab_with_wakeup("", &request.wake)
                .trim_end()
                .into(),
        },
        scheduler: request.scheduler,
        files,
        registered: false,
        probe: None,
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
        name: if name.starts_with("webhooks") {
            ResourceKind::Receiver
        } else {
            ResourceKind::Redis
        },
        unit,
        scheduler: request.scheduler,
        files,
        registered: false,
        probe: None,
    }
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

pub(super) fn owned_receiver_file(home: &Home, file: &Path) -> anyhow::Result<Option<String>> {
    let Some(contents) = read(file)? else {
        return Ok(None);
    };
    anyhow::ensure!(
        std::fs::metadata(file)?.permissions().mode() & 0o077 == 0,
        "receiver invocation must be owner-only"
    );
    let owned = Owned::load(home)?;
    anyhow::ensure!(
        owned.proven(ResourceKind::Receiver, file, &contents),
        "receiver invocation has no matching ownership proof; restore its recorded definition"
    );
    Ok(Some(contents))
}

pub(super) fn receiver_job(home: &Home, port: u16) -> anyhow::Result<Option<(Scheduler, String)>> {
    let owned = Owned::load(home)?;
    Ok(owned
        .proofs(ResourceKind::Receiver)
        .into_iter()
        .find(|d| d.port() == Some(port))
        .map(|d| (d.scheduler, d.unit.clone())))
}
