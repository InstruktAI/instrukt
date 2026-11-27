//! instrukt's own services on this computer, by name. A unit, definition
//! file or crontab line under instrukt's name for this home is instrukt's:
//! it is written, reloaded or withdrawn by that name, under whichever service
//! manager present holds it. Nothing records what was written, and anything
//! under another name is never read or touched
//! (`project/instrukt/spec/feature/runner/module-installation`).
use super::{Disposition, Report, Request, ResourceKind};
use crate::install::{Scheduler, systemd_assignments};
use crate::process::Invocation;
use instrukt_core::fsx::atomic_write_with_permissions;
use instrukt_core::home::Home;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

/// One of instrukt's services as this home names it under one manager.
#[derive(Clone)]
pub(super) struct Definition {
    pub name: ResourceKind,
    pub unit: String,
    pub scheduler: Scheduler,
    pub locations: crate::install::SchedulerLocations,
    /// The service's definition files: written by installation and removed
    /// when it is withdrawn.
    pub files: BTreeMap<PathBuf, String>,
    /// Files written with the definition and kept when it is withdrawn: the
    /// private Redis's configuration and ACL.
    pub kept: BTreeMap<PathBuf, String>,
    probe: Option<(u16, String)>,
}
impl Definition {
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
        format!("{} ({})", self.unit, self.scheduler.name())
    }
    /// Whether a file or the crontab line of this definition is missing or
    /// differs from what it should hold.
    fn changed(&self) -> anyhow::Result<bool> {
        for (file, content) in self.files.iter().chain(&self.kept) {
            if read(file)?.as_deref() != Some(content.as_str()) {
                return Ok(true);
            }
        }
        if self.scheduler == Scheduler::Crontab {
            return Ok(marked_line(
                &crate::install::read_crontab(self.locations.crontab_file.as_deref())?,
                marker(&self.unit),
            )
            .as_deref()
                != Some(self.unit.as_str()));
        }
        Ok(false)
    }
    /// Writes every file of the definition, owner-only. A definition file
    /// whose environment names another instrukt home is that home's: it is
    /// left as it is, and its own home's install moves it to its own name.
    fn write(&self, home: &Home) -> anyhow::Result<()> {
        match self.owner(home)? {
            Owner::This => {}
            Owner::Other(other) => anyhow::bail!(
                "{} runs the instrukt home {other}; nothing was changed. Install from that home moves it to that home's own name",
                self.label()
            ),
            Owner::Unknown(why) => anyhow::bail!(
                "{} is left in place: {why}, so whose it is cannot be established; nothing was changed",
                self.label()
            ),
        }
        for (file, content) in self.files.iter().chain(&self.kept) {
            read(file)?;
            atomic_write_with_permissions(
                file,
                content.as_bytes(),
                std::fs::Permissions::from_mode(0o600),
            )?;
        }
        Ok(())
    }
    /// Whether any file or the crontab line of this definition exists, or
    /// its manager reports it registered.
    fn present(&self) -> anyhow::Result<bool> {
        if self.scheduler == Scheduler::Crontab {
            return Ok(marked_line(
                &crate::install::read_crontab(self.locations.crontab_file.as_deref())?,
                marker(&self.unit),
            )
            .is_some());
        }
        let listed = listed(self.scheduler)?;
        Ok(self.files.keys().any(|file| file.exists())
            || self
                .registrations()
                .iter()
                .any(|unit| listed.contains(unit)))
    }
    /// The names its manager registers it under: the unit, and for a
    /// systemd receiver the service its socket starts.
    fn registrations(&self) -> Vec<String> {
        let mut names = vec![self.unit.clone()];
        if self.name == ResourceKind::Receiver && self.scheduler == Scheduler::SystemdUser {
            names.push(format!("{}.service", self.unit.trim_end_matches(".socket")));
        }
        names
    }
    /// The instrukt home this definition's environment names: `INSTRUKT_HOME`
    /// in its launchd plist, systemd unit or crontab line, or, its files gone,
    /// in the environment its manager retains for its registration.
    fn named_home(&self) -> anyhow::Result<Belongs> {
        if self.scheduler == Scheduler::Crontab {
            return Ok(marked_line(
                &crate::install::read_crontab(self.locations.crontab_file.as_deref())?,
                marker(&self.unit),
            )
            .and_then(|line| named_home(&line))
            .map_or(Belongs::Unnamed, Belongs::Home));
        }
        let mut files = false;
        for file in self.files.keys() {
            if let Some(content) = read(file)? {
                files = true;
                if let Some(home) = named_home(&content) {
                    return Ok(Belongs::Home(home));
                }
            }
        }
        if files {
            return Ok(Belongs::Unnamed);
        }
        let listed = listed(self.scheduler)?;
        for unit in self.registrations() {
            if !listed.contains(&unit) {
                continue;
            }
            match registered_home(self.scheduler, &unit)? {
                Belongs::Unnamed => {}
                named => return Ok(named),
            }
        }
        Ok(Belongs::Unnamed)
    }
    /// Whether this definition is `home`'s: it names `home`, or no home; or
    /// another home's; or unknown, its environment unreadable.
    fn owner(&self, home: &Home) -> anyhow::Result<Owner> {
        Ok(match self.named_home()? {
            Belongs::Unnamed => Owner::This,
            Belongs::Home(named) if same_home(&named, home) => Owner::This,
            Belongs::Home(named) => Owner::Other(named),
            Belongs::Unknown(why) => Owner::Unknown(why),
        })
    }
    /// Stops and unregisters the service and removes its definition files;
    /// what `kept` names stays.
    fn withdraw(&self) -> anyhow::Result<()> {
        stop(self)?;
        for file in self.files.keys() {
            if read(file)?.is_some() {
                instrukt_core::fsx::remove_file(file)?;
            }
        }
        if self.scheduler == Scheduler::SystemdUser {
            required(&["systemctl", "--user", "daemon-reload"])?;
        }
        Ok(())
    }
}

/// What a definition's environment says of the instrukt home it runs.
enum Belongs {
    /// It names this `INSTRUKT_HOME`.
    Home(String),
    /// It names none: a valid environment without `INSTRUKT_HOME`.
    Unnamed,
    /// Its manager's report could not be read or decoded, for this reason.
    Unknown(String),
}

/// Whose a definition is, relative to one home.
enum Owner {
    This,
    Other(String),
    Unknown(String),
}

/// Whether `named` is `home`'s root, compared by canonical path.
fn same_home(named: &str, home: &Home) -> bool {
    let canonical =
        |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical(Path::new(named)) == canonical(home.root())
}

/// Every service manager present on the computer, the installing one first:
/// those whose command is on `PATH`.
fn managers(request: &Request<'_>) -> Vec<Scheduler> {
    let program = |scheduler| match scheduler {
        Scheduler::Launchd => "launchctl",
        Scheduler::SystemdUser => "systemctl",
        Scheduler::Crontab => "crontab",
    };
    let mut found = vec![request.scheduler];
    found.extend(
        [
            Scheduler::Launchd,
            Scheduler::SystemdUser,
            Scheduler::Crontab,
        ]
        .into_iter()
        .filter(|scheduler| *scheduler != request.scheduler),
    );
    found.retain(|scheduler| {
        (*scheduler == Scheduler::Crontab && request.wake.locations.crontab_file.is_some())
            || on_path(program(*scheduler), request.wake.path_env)
    });
    found
}
fn on_path(program: &str, path_env: &str) -> bool {
    std::env::split_paths(path_env).any(|dir| dir.join(program).is_file())
}

/// What `scheduler` reports registered now under any name: launchd's job
/// labels, systemd's units. Cron is read for its marked line instead.
fn listed(scheduler: Scheduler) -> anyhow::Result<std::collections::BTreeSet<String>> {
    let (program, args, column): (&str, &[&str], usize) = match scheduler {
        Scheduler::Launchd => ("launchctl", &["list"], 2),
        Scheduler::SystemdUser => (
            "systemctl",
            &[
                "--user",
                "list-units",
                "--all",
                "--plain",
                "--no-legend",
                "--full",
            ],
            0,
        ),
        Scheduler::Crontab => return Ok(Default::default()),
    };
    let result = Invocation::program(program, args).run(Duration::from_secs(10))?;
    anyhow::ensure!(
        result.success,
        "cannot list {} registrations; restore access to it",
        scheduler.name()
    );
    Ok(result
        .stdout
        .lines()
        .filter_map(|line| line.split_whitespace().nth(column))
        .map(str::to_owned)
        .collect())
}

/// The `INSTRUKT_HOME` a manager retains for the registration `unit`, read
/// from the manager itself: what `launchctl print` shows of the job's
/// environment, or systemd's `Environment` property of the unit.
fn registered_home(scheduler: Scheduler, unit: &str) -> anyhow::Result<Belongs> {
    let output = match scheduler {
        Scheduler::Launchd => {
            let uid = Invocation::program("id", &["-u"]).run(Duration::from_secs(10))?;
            anyhow::ensure!(uid.success, "cannot identify the launchd user domain");
            let target = format!("gui/{}/{unit}", uid.stdout.trim());
            Invocation::program("launchctl", &["print", &target]).run(Duration::from_secs(10))?
        }
        Scheduler::SystemdUser => Invocation::program(
            "systemctl",
            &["--user", "show", unit, "--property=Environment", "--value"],
        )
        .run(Duration::from_secs(10))?,
        Scheduler::Crontab => return Ok(Belongs::Unnamed),
    };
    if !output.success {
        return Ok(Belongs::Unknown(format!(
            "{} does not report the environment of {unit}",
            scheduler.name()
        )));
    }
    if scheduler == Scheduler::Launchd {
        return Ok(output
            .stdout
            .lines()
            .find_map(|line| line.trim().strip_prefix("INSTRUKT_HOME => "))
            .map_or(Belongs::Unnamed, |home| Belongs::Home(home.to_owned())));
    }
    for line in output.stdout.lines() {
        match systemd_assignments(line.trim()) {
            Err(why) => {
                return Ok(Belongs::Unknown(format!(
                    "systemd's environment of {unit} cannot be decoded: {why}"
                )));
            }
            Ok(words) => {
                if let Some(home) = words
                    .iter()
                    .find_map(|word| word.strip_prefix("INSTRUKT_HOME="))
                {
                    return Ok(Belongs::Home(home.to_owned()));
                }
            }
        }
    }
    Ok(Belongs::Unnamed)
}

/// The notice for a registration whose home cannot be established: what
/// was left running, and the two ways to settle it.
fn unknown_notice(definition: &Definition) -> anyhow::Result<String> {
    let removal = match definition.scheduler {
        Scheduler::Launchd => {
            let uid = Invocation::program("id", &["-u"]).run(Duration::from_secs(10))?;
            anyhow::ensure!(uid.success, "cannot identify the launchd user domain");
            format!(
                "launchctl bootout gui/{}/{}",
                uid.stdout.trim(),
                definition.unit
            )
        }
        Scheduler::SystemdUser => format!("systemctl --user disable --now {}", definition.unit),
        Scheduler::Crontab => format!(
            "crontab -e, deleting the line ending `{}`",
            marker(&definition.unit)
        ),
    };
    Ok(format!(
        "{} ({}): instrukt cannot tell which instrukt home this registration belongs to, so it was left running. Run `instrukt install` from the home it belongs to, or remove it yourself with `{removal}`",
        definition.unit,
        definition.scheduler.name()
    ))
}

/// Each registration under this home's names whose home cannot be
/// established from its manager's report, reported once as left in place.
pub(super) fn report_unknown(request: &Request<'_>, report: &mut Report) -> anyhow::Result<()> {
    for kind in [
        ResourceKind::WakeUp,
        ResourceKind::Runner,
        ResourceKind::Redis,
        ResourceKind::Receiver,
    ] {
        // A crontab line names its home itself; only a manager's report of a
        // registration can be unreadable.
        let native = managers(request)
            .into_iter()
            .filter(|scheduler| *scheduler != Scheduler::Crontab)
            .collect::<Vec<_>>();
        for definition in discover_under(request, kind, &native)? {
            if let Owner::Unknown(_) = definition.owner(request.home)? {
                report.notices.push(unknown_notice(&definition)?);
            }
        }
    }
    Ok(())
}

/// This home's definitions of `kind` present under every manager on the
/// computer, a receiver's at every port: by file, crontab line or what the
/// manager reports registered. A definition whose environment names another
/// instrukt home is that home's, and is not this home's to touch.
fn discover(request: &Request<'_>, kind: ResourceKind) -> anyhow::Result<Vec<Definition>> {
    let mut found = Vec::new();
    for definition in discover_named(request, kind)? {
        if matches!(definition.owner(request.home)?, Owner::This) {
            found.push(definition);
        }
    }
    Ok(found)
}

/// Every definition of `kind` present under the names `request.home` gives
/// them, whichever home its environment names.
fn discover_named(request: &Request<'_>, kind: ResourceKind) -> anyhow::Result<Vec<Definition>> {
    discover_under(request, kind, &managers(request))
}

/// [`discover_named`] under the managers `under` only.
fn discover_under(
    request: &Request<'_>,
    kind: ResourceKind,
    under: &[Scheduler],
) -> anyhow::Result<Vec<Definition>> {
    let mut found = Vec::new();
    for &scheduler in under {
        // Cron keeps only the wake-up.
        if scheduler == Scheduler::Crontab && kind != ResourceKind::WakeUp {
            continue;
        }
        let request = Request {
            scheduler,
            ..request.clone()
        };
        let candidates = match kind {
            ResourceKind::WakeUp => vec![tick(&request)],
            ResourceKind::Runner => vec![runner(&request)],
            ResourceKind::Redis => vec![native(&request, "redis", &[], None)],
            ResourceKind::Receiver => receiver_ports(&request)?
                .into_iter()
                .map(|port| receiver_unit(&request, port))
                .collect(),
            // A runtime project is no service of any scheduler.
            ResourceKind::LocalRedis | ResourceKind::Graphify | ResourceKind::RuntimeProject => {
                vec![]
            }
        };
        for candidate in candidates {
            if candidate.present()? {
                found.push(candidate);
            }
        }
    }
    Ok(found)
}

/// The ports of this home's receiver definitions under the request's manager.
fn receiver_ports(request: &Request<'_>) -> anyhow::Result<Vec<u16>> {
    let (dir, prefix, end) = match request.scheduler {
        Scheduler::Launchd => (
            request.wake.locations.launch_agents.clone(),
            "ai.instrukt.webhooks-",
            match crate::install::home_suffix(request.home) {
                Some(suffix) => format!(".{suffix}.plist"),
                None => ".plist".to_owned(),
            },
        ),
        Scheduler::SystemdUser => (
            request.wake.locations.systemd.clone(),
            "instrukt-webhooks-",
            match crate::install::home_suffix(request.home) {
                Some(suffix) => format!("-{suffix}.socket"),
                None => ".socket".to_owned(),
            },
        ),
        Scheduler::Crontab => return Ok(vec![]),
    };
    let mut names = Vec::new();
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            for entry in entries {
                if let Some(name) = entry?.file_name().to_str() {
                    names.push(name.to_owned());
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    // A registration the manager still reports, its file gone: a launchd
    // label, which carries no file extension, or a systemd receiver's socket
    // or the service it starts.
    let registered_ends = match request.scheduler {
        Scheduler::Launchd => vec![end.trim_end_matches(".plist").to_owned()],
        _ => vec![end.clone(), end.replace(".socket", ".service")],
    };
    let registered = listed(request.scheduler)?;
    let mut ports = names
        .iter()
        .filter_map(|name| name.strip_prefix(prefix)?.strip_suffix(end.as_str()))
        .chain(registered.iter().flat_map(|name| {
            registered_ends
                .iter()
                .filter_map(move |ending| name.strip_prefix(prefix)?.strip_suffix(ending.as_str()))
        }))
        .filter_map(|port| port.parse::<u16>().ok())
        .collect::<Vec<_>>();
    ports.sort_unstable();
    ports.dedup();
    Ok(ports)
}

/// Whether a definition of `kind` under this home's name is present.
pub(super) fn registered(request: &Request<'_>, kind: ResourceKind) -> bool {
    discover(request, kind).is_ok_and(|found| !found.is_empty())
}

/// The definitions of `desired`'s kind under this home's name that another
/// manager holds, or, for a receiver, at another port.
fn withdraw_others(request: &Request<'_>, desired: &Definition) -> anyhow::Result<Vec<Definition>> {
    let others = discover(request, desired.name)?
        .into_iter()
        .filter(|found| found.scheduler != desired.scheduler || found.unit != desired.unit)
        .collect::<Vec<_>>();
    Ok(others)
}

/// Withdraws the definitions `desired` replaces: those another manager
/// holds or, for a receiver, at another port. An incomplete demand
/// withdraws nothing; they stay, and that is reported.
fn replace_others(
    others: &[Definition],
    complete: bool,
    report: &mut Report,
) -> anyhow::Result<()> {
    for other in others {
        if complete {
            other.withdraw()?;
        } else {
            report.finding(
                other.name,
                format!(
                    "{} stays while project demand is incomplete; restore unreadable projects and rerun instrukt install",
                    other.label()
                ),
            );
        }
    }
    Ok(())
}

/// Reconciles one service: its definition files are written and the service
/// registered or restarted when a file or its registration is missing or
/// differs. `--check` reports what an installation would repair.
pub(super) fn reconcile(
    request: &Request<'_>,
    desired: &Definition,
    required: bool,
    complete: bool,
    projects: &[String],
    report: &mut Report,
) -> anyhow::Result<()> {
    if !required {
        return withdraw_named(request, desired.name, complete, report);
    }
    if desired.name == ResourceKind::Receiver {
        return receiver_resource(request, desired, complete, projects, report);
    }
    let others = withdraw_others(request, desired)?;
    let changed = desired.changed()?;
    let active = running(desired)?;
    let paths = desired.files.keys().cloned().collect::<Vec<_>>();
    if desired.scheduler == Scheduler::Crontab && desired.locations.crontab_file.is_some() {
        if !request.check {
            crate::install::register(request.scheduler, &request.wake)?;
        }
        report.finding(desired.name, "file_only: cron table is materialized but not registered; configure native scheduling for wake-up");
        report.resource(
            desired.name,
            Disposition::Unmet,
            desired.locations.crontab_file.iter().cloned().collect(),
            projects,
            "file_only: not registered",
        );
        return Ok(());
    }
    if request.check {
        let unmet = changed || !active || !others.is_empty();
        if unmet {
            report.finding(
                desired.name,
                "missing/broken registration; run instrukt install after resolving prerequisites",
            );
        }
        report.resource(
            desired.name,
            if unmet {
                Disposition::Unmet
            } else {
                Disposition::Reused
            },
            paths,
            projects,
            format!(
                "{}: checked definitions and service-manager state",
                desired.label()
            ),
        );
        return Ok(());
    }
    replace_others(&others, complete, report)?;
    if !changed && active {
        report.resource(
            desired.name,
            Disposition::Reused,
            paths,
            projects,
            format!("{}: working registration unchanged", desired.label()),
        );
        return Ok(());
    }
    let existed = desired.present()?;
    desired.write(request.home)?;
    if desired.name == ResourceKind::WakeUp {
        crate::install::register(request.scheduler, &request.wake)?;
    } else {
        // A changed Redis or runner service restarts, so the written
        // configuration is the one running.
        if active {
            stop(desired)?;
        }
        start(desired)?;
    }
    report.resource(
        desired.name,
        if existed {
            Disposition::Repaired
        } else {
            Disposition::Installed
        },
        paths,
        projects,
        format!("{}: registered with the service manager", desired.label()),
    );
    Ok(())
}

/// The receiver's definition at `port`: its socket and service, and the
/// invocation the receiver reads at its boundary.
pub(super) fn receiver_definition(
    request: &Request<'_>,
    port: u16,
    route: String,
) -> anyhow::Result<Definition> {
    let mut definition = receiver_unit(request, port);
    let launch = super::_launch::ReceiverLaunch::desired(request, port);
    definition.files.insert(
        super::_launch::ReceiverLaunch::file(request.home, port),
        launch.encode()?,
    );
    definition.probe = Some((port, route));
    Ok(definition)
}
fn receiver_unit(request: &Request<'_>, port: u16) -> Definition {
    let mut definition = native(
        request,
        &format!("webhooks-{port}"),
        &[
            request.wake.executable.display().to_string(),
            "webhooks".into(),
            "serve".into(),
            "--socket-activation".into(),
        ],
        Some(port),
    );
    // The invocation is part of the receiver: withdrawn with it, and written
    // with its content by `receiver_definition`.
    definition.files.insert(
        super::_launch::ReceiverLaunch::file(request.home, port),
        String::new(),
    );
    definition
}

fn receiver_resource(
    request: &Request<'_>,
    desired: &Definition,
    complete: bool,
    projects: &[String],
    report: &mut Report,
) -> anyhow::Result<()> {
    let others = withdraw_others(request, desired)?;
    let changed = desired.changed()?;
    let active = running(desired)?;
    let port = desired
        .port()
        .ok_or_else(|| anyhow::anyhow!("receiver listener has no port"))?;
    let execution = super::_launch::verify(request.home, desired.scheduler, &desired.unit, port);
    let effective = execution.is_ok();
    let unmet = changed || !active || !effective || !others.is_empty();
    if request.check {
        if unmet {
            report.finding(
                "receiver",
                format!(
                    "missing, stale or incomplete receiver; run instrukt install{}",
                    execution
                        .err()
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default()
                ),
            );
        }
        report.resource(
            ResourceKind::Receiver,
            if unmet {
                Disposition::Unmet
            } else {
                Disposition::Reused
            },
            desired.files.keys().cloned().collect(),
            projects,
            format!("{}: checked listener", desired.label()),
        );
        return Ok(());
    }
    if !others.is_empty() {
        anyhow::ensure!(
            complete,
            "complete project demand is required before changing receiver placement"
        );
    }
    // Every installation writes the receiver's invocation and restarts it,
    // keeping the listening socket, so the installed executable is the one
    // serving.
    let existed = desired.present()?;
    desired.write(request.home)?;
    if !active {
        start(desired)?;
    }
    restart_receiver(request.home, desired)?;
    super::_launch::verify(request.home, desired.scheduler, &desired.unit, port)?;
    if let Some((port, route)) = &desired.probe {
        super::receiver_health(*port, route)?;
    }
    // The replacement listens and answers before a listener it replaces is
    // withdrawn.
    for old in &others {
        old.withdraw()?;
        report.notices.push(format!(
            "receiver edge route changed from port {} to {port}; align the operator's external route",
            old.port().map(|p| p.to_string()).unwrap_or_else(|| "unknown".into()),
        ));
    }
    report.resource(
        ResourceKind::Receiver,
        if !existed {
            Disposition::Installed
        } else if unmet {
            Disposition::Repaired
        } else {
            Disposition::Reused
        },
        desired.files.keys().cloned().collect(),
        projects,
        format!("{}: receiver ready with socket custody", desired.label()),
    );
    Ok(())
}

/// Reconcile the runner service: its definition and registration, and a
/// live service process. Installing writes the definition and restarts the
/// service every time, so the installed executable and configuration are
/// the ones running; nothing records what the process executes. `--check`
/// reports a missing, stopped or unloaded service. Returns why the service
/// is unmet, `None` when it is loaded and alive.
pub(super) fn runner_service(
    request: &Request<'_>,
    desired: &Definition,
    complete: bool,
    projects: &[String],
    report: &mut Report,
) -> anyhow::Result<Option<String>> {
    let others = withdraw_others(request, desired)?;
    let changed = desired.changed()?;
    let process = service_process(desired)?;
    let paths = desired.files.keys().cloned().collect::<Vec<_>>();
    let cause = if changed {
        Some("its definition is missing or stale".to_owned())
    } else if !others.is_empty() {
        Some("another service manager holds it".to_owned())
    } else {
        not_alive(&process)
    };
    if request.check {
        report.resource(
            desired.name,
            if cause.is_some() {
                Disposition::Unmet
            } else {
                Disposition::Reused
            },
            paths,
            projects,
            format!(
                "{}: checked definition, registration and its live process",
                desired.label()
            ),
        );
        if let Some(cause) = &cause {
            report.finding(desired.name, format!("{cause}; run instrukt install"));
        }
        return Ok(cause);
    }
    replace_others(&others, complete, report)?;
    let existed = desired.present()?;
    desired.write(request.home)?;
    // An existing process is stopped whether or not the unit is enabled, so
    // the start runs the desired executable and configuration.
    if process.loaded || process.pid.is_some() {
        stop(desired)?;
    }
    start(desired)?;
    let started = std::time::Instant::now();
    let mut unmet = restarted(&service_process(desired)?, process.pid);
    while unmet.is_some() && started.elapsed() < RUNNER_START_WAIT {
        std::thread::sleep(Duration::from_millis(250));
        unmet = restarted(&service_process(desired)?, process.pid);
    }
    if let Some(unmet) = unmet {
        anyhow::bail!(
            "the restarted runner service is not running: {unmet}; inspect the service manager's log and rerun instrukt install"
        );
    }
    report.resource(
        desired.name,
        if !existed {
            Disposition::Installed
        } else if cause.is_some() {
            Disposition::Repaired
        } else {
            Disposition::Reused
        },
        paths,
        projects,
        format!(
            "{}: started and running the installed executable",
            desired.label()
        ),
    );
    Ok(None)
}

/// Withdraws this home's definitions of `name` under every manager present.
/// An incomplete demand withdraws nothing: an unreadable project may still
/// demand it.
pub(super) fn withdraw_named(
    request: &Request<'_>,
    name: ResourceKind,
    complete: bool,
    report: &mut Report,
) -> anyhow::Result<()> {
    let definitions = discover(request, name)?;
    if definitions.is_empty() {
        return Ok(());
    }
    anyhow::ensure!(
        complete,
        "incomplete project demand; restore unreadable projects before withdrawing {name}"
    );
    if request.check {
        report.finding(
            name,
            "obsolete owned registration; run instrukt install to withdraw it",
        );
        return Ok(());
    }
    for definition in &definitions {
        definition.withdraw()?;
    }
    report.resource(
        name,
        Disposition::Withdrawn,
        definitions.iter().flat_map(|d| d.files.keys().cloned()).collect(),
        &[],
        "owned registrations withdrawn; retained data, credentials and shared executables unchanged",
    );
    Ok(())
}

/// For a home other than `~/.instrukt`: the services an earlier release
/// registered under the default names while naming this home in their
/// environment are withdrawn, so this home's own names replace them. The
/// default home never matches a unit naming another home. An incomplete
/// demand withdraws nothing.
pub(super) fn withdraw_legacy(
    request: &Request<'_>,
    complete: bool,
    report: &mut Report,
) -> anyhow::Result<()> {
    if crate::install::home_suffix(request.home).is_none() {
        return Ok(());
    }
    let default = Home::at_user_home(request.home.machine_user_home().to_path_buf());
    let legacy = Request {
        home: &default,
        wake: crate::install::WakeUp {
            home: &default,
            ..request.wake.clone()
        },
        ..request.clone()
    };
    for kind in [
        ResourceKind::WakeUp,
        ResourceKind::Runner,
        ResourceKind::Redis,
        ResourceKind::Receiver,
    ] {
        for definition in discover_named(&legacy, kind)? {
            // Only one that names this home; one naming no home, or the
            // default home, is the default home's, and one whose home cannot
            // be read is left in place.
            match definition.named_home()? {
                Belongs::Home(named) if same_home(&named, request.home) => {}
                // One whose home its manager's report does not establish is
                // left running and reported once; this home's own pass never
                // meets a name without the suffix.
                Belongs::Unknown(_) => {
                    report.notices.push(unknown_notice(&definition)?);
                    continue;
                }
                _ => continue,
            }
            if request.check || !complete {
                report.finding(
                    kind,
                    format!(
                        "{} names this home under the default name; {}",
                        definition.label(),
                        if complete {
                            "run instrukt install to withdraw it"
                        } else {
                            "it stays while project demand is incomplete; restore unreadable projects and rerun instrukt install"
                        }
                    ),
                );
                continue;
            }
            definition.withdraw()?;
            report.resource(
                kind,
                Disposition::Withdrawn,
                definition.files.keys().cloned().collect(),
                &[],
                format!(
                    "{}: an earlier registration of this home under the default name",
                    definition.label()
                ),
            );
        }
    }
    Ok(())
}

/// The value of `INSTRUKT_HOME` that a launchd plist, a systemd unit or a
/// crontab line written by instrukt sets, unescaped.
fn named_home(text: &str) -> Option<String> {
    if let Some((_, rest)) = text.split_once("<key>INSTRUKT_HOME</key><string>") {
        let value = rest.split_once("</string>")?.0;
        return Some(
            value
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&amp;", "&"),
        );
    }
    if let Some((_, rest)) = text.split_once("\"INSTRUKT_HOME=") {
        let mut value = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => value.push(chars.next()?),
                '"' => return Some(value),
                c => value.push(c),
            }
        }
        return None;
    }
    if let Some((_, rest)) = text.split_once("INSTRUKT_HOME='") {
        let mut value = String::new();
        let mut rest = rest;
        loop {
            let (part, tail) = rest.split_once('\'')?;
            value.push_str(part);
            match tail.strip_prefix("\\''") {
                Some(more) => {
                    value.push('\'');
                    rest = more;
                }
                None => return Some(value),
            }
        }
    }
    None
}

/// The marker a crontab definition's line ends with: this home's
/// `# instrukt-tick` marker.
fn marker(unit: &str) -> &str {
    unit.rfind(crate::install::CRON_MARKER)
        .map_or(unit, |at| &unit[at..])
}
/// The line of `table` marked with `marker`.
fn marked_line(table: &str, marker: &str) -> Option<String> {
    table
        .lines()
        .find(|line| line.trim_end().ends_with(marker))
        .map(str::to_owned)
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
        definition.locations.verify_systemd()?;
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
            d.locations.verify_systemd()?;
            Ok(
                command(&["systemctl", "--user", "is-active", "--quiet", &d.unit])?
                    && command(&["systemctl", "--user", "is-enabled", "--quiet", &d.unit])?,
            )
        }
        Scheduler::Crontab => {
            if d.locations.crontab_file.is_some() {
                return Ok(false);
            }
            let text = crate::install::read_crontab(None)?;
            Ok(text.lines().any(|line| line == d.unit))
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
        d.locations.verify_systemd()?;
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
    match d.scheduler {
        Scheduler::Launchd => {
            let file = d
                .files
                .keys()
                .find(|file| file.extension().is_some_and(|ext| ext == "plist"))
                .ok_or_else(|| anyhow::anyhow!("missing plist"))?;
            if running(d)? {
                if file.exists() {
                    required(&["launchctl", "unload", &file.display().to_string()])?;
                } else {
                    // Registered with its definition file gone: by its label.
                    required(&["launchctl", "remove", &d.unit])?;
                }
            }
        }
        Scheduler::SystemdUser => {
            if systemd_loaded(&d.unit)? {
                required(&["systemctl", "--user", "disable", "--now", &d.unit])?;
            } else if listed(Scheduler::SystemdUser)?.contains(&d.unit) {
                // Still running with its unit file gone: by its name.
                required(&["systemctl", "--user", "stop", &d.unit])?;
            }
            // The service a receiver's socket starts, loaded or still listed
            // with its unit file gone.
            let service = format!("{}.service", d.unit.trim_end_matches(".socket"));
            if d.name == ResourceKind::Receiver
                && (systemd_loaded(&service)? || listed(Scheduler::SystemdUser)?.contains(&service))
            {
                required(&["systemctl", "--user", "stop", &service])?;
            }
        }
        Scheduler::Crontab => {
            let current = crate::install::read_crontab(d.locations.crontab_file.as_deref())?;
            let marker = marker(&d.unit);
            let text = current
                .lines()
                .filter(|line| !line.trim_end().ends_with(marker))
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
            crate::install::write_crontab(d.locations.crontab_file.as_deref(), &text)?;
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
            Scheduler::Launchd => crate::install::label(request.home),
            Scheduler::SystemdUser => crate::install::timer(request.home),
            Scheduler::Crontab => crate::install::crontab_with_wakeup("", &request.wake)
                .trim_end()
                .into(),
        },
        scheduler: request.scheduler,
        locations: request.wake.locations.clone(),
        files,
        kept: BTreeMap::new(),
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
    let label = crate::install::launchd_label(&format!("ai.instrukt.{name}"), request.home);
    let mut files = BTreeMap::new();
    let unit;
    if request.scheduler == Scheduler::Launchd {
        unit = label.clone();
        let program = args
            .iter()
            .map(|s| format!("<string>{}</string>", xml(s)))
            .collect::<String>();
        // The runner is started again after every exit, at most once per
        // RETRY_SECONDS; launchd never gives up on a KeepAlive job.
        let keep = if name == RUNNER {
            format!(
                "<key>RunAtLoad</key><true/><key>KeepAlive</key><true/><key>ThrottleInterval</key><integer>{RETRY_SECONDS}</integer>"
            )
        } else {
            "<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>".into()
        };
        let sockets=socket.map_or(keep,|port|format!("<key>Sockets</key><dict><key>instrukt</key><dict><key>SockNodeName</key><string>127.0.0.1</string><key>SockServiceName</key><string>{port}</string><key>SockType</key><string>stream</string></dict></dict>"));
        files.insert(request.wake.locations.launch_agents.join(format!("{label}.plist")),format!("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Label</key><string>{label}</string><key>ProgramArguments</key><array>{program}</array><key>EnvironmentVariables</key><dict><key>HOME</key><string>{}</string><key>INSTRUKT_HOME</key><string>{}</string><key>INSTRUKT_DEVELOPMENT</key><string>{}</string><key>PATH</key><string>{}</string></dict><key>Umask</key><integer>63</integer>{sockets}</dict></plist>\n",xml(&request.home.machine_user_home().display().to_string()),xml(&request.home.root().display().to_string()),u8::from(request.home.is_development()),xml(request.wake.path_env)));
    } else {
        let stem = crate::install::systemd_stem(&format!("instrukt-{name}"), request.home);
        let dir = &request.wake.locations.systemd;
        let extra = if socket.is_some() {
            format!("Requires={stem}.socket\nAfter={stem}.socket\n")
        } else if name == RUNNER {
            // No start limit: the runner is retried for as long as it takes.
            "StartLimitIntervalSec=0\n".into()
        } else {
            String::new()
        };
        // The runner is started again after every exit, RETRY_SECONDS
        // apart; other services restart after a failure.
        let restart = if name == RUNNER {
            format!("always\nRestartSec={RETRY_SECONDS}")
        } else {
            "on-failure".into()
        };
        files.insert(dir.join(format!("{stem}.service")),format!("[Unit]\nDescription=instrukt {name}\n{extra}\n[Service]\nExecStart={}\nEnvironment={}\nEnvironment={}\nEnvironment={}\nEnvironment={}\nUMask=0077\nRestart={restart}\n\n[Install]\nWantedBy=default.target\n",args.iter().map(|s|quote(s)).collect::<Vec<_>>().join(" "),quote(&format!("HOME={}",request.home.machine_user_home().display())),quote(&format!("PATH={}",request.wake.path_env)),quote(&format!("INSTRUKT_HOME={}",request.home.root().display())),quote(&format!("INSTRUKT_DEVELOPMENT={}",u8::from(request.home.is_development())))));
        unit = if let Some(port) = socket {
            files.insert(dir.join(format!("{stem}.socket")),format!("[Socket]\nListenStream=127.0.0.1:{port}\nFileDescriptorName=instrukt\n\n[Install]\nWantedBy=sockets.target\n"));
            format!("{stem}.socket")
        } else {
            format!("{stem}.service")
        };
    }
    for (path, content) in &mut files {
        if request.scheduler == Scheduler::Launchd {
            let extra = request
                .wake
                .locations
                .environment
                .iter()
                .map(|(k, v)| format!("<key>{}</key><string>{}</string>", xml(k), xml(v)))
                .collect::<String>();
            *content =
                content.replacen("</dict><key>Umask", &format!("{extra}</dict><key>Umask"), 1);
        } else if path.extension().is_some_and(|e| e == "service") {
            let extra = request
                .wake
                .locations
                .environment
                .iter()
                .map(|(k, v)| format!("Environment={}\n", quote(&format!("{k}={v}"))))
                .collect::<String>();
            *content = content.replacen("\nUMask=", &format!("\n{extra}UMask="), 1);
        }
    }
    Definition {
        name: if name.starts_with("webhooks") {
            ResourceKind::Receiver
        } else if name == RUNNER {
            ResourceKind::Runner
        } else {
            ResourceKind::Redis
        },
        unit,
        scheduler: request.scheduler,
        locations: request.wake.locations.clone(),
        files,
        kept: BTreeMap::new(),
        probe: None,
    }
}
/// The runner service's name in its unit: `ai.instrukt.runner` under
/// launchd, `instrukt-runner.service` under systemd, with the home's suffix
/// for a home other than `~/.instrukt`.
const RUNNER: &str = "runner";

/// The seconds between two starts of a runner service that exited.
const RETRY_SECONDS: u32 = 5;

/// How long installation waits for a started runner service to have a new
/// live process.
const RUNNER_START_WAIT: Duration = Duration::from_secs(30);

/// What the service manager knows of a service: whether it has the unit
/// loaded, and the id of its live process. Each is a separate fact.
struct ServiceProcess {
    loaded: bool,
    pid: Option<u32>,
}

fn service_process(d: &Definition) -> anyhow::Result<ServiceProcess> {
    match d.scheduler {
        Scheduler::Launchd => {
            let result =
                Invocation::program("launchctl", &["list"]).run(Duration::from_secs(10))?;
            anyhow::ensure!(result.success, "cannot inspect launchd jobs");
            let row = result.stdout.lines().find_map(|line| {
                let mut fields = line.split_whitespace();
                let pid = fields.next()?;
                fields.next()?;
                (fields.next()? == d.unit).then(|| pid.parse::<u32>().ok())
            });
            // A loaded user agent is the one launchd starts at login.
            Ok(ServiceProcess {
                loaded: row.is_some(),
                pid: row.flatten(),
            })
        }
        Scheduler::SystemdUser => {
            let result = Invocation::program(
                "systemctl",
                &["--user", "show", &d.unit, "--property=MainPID", "--value"],
            )
            .run(Duration::from_secs(10))?;
            anyhow::ensure!(result.success, "cannot inspect systemd unit {}", d.unit);
            Ok(ServiceProcess {
                loaded: systemd_loaded(&d.unit)?,
                pid: result
                    .stdout
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|pid| *pid != 0),
            })
        }
        Scheduler::Crontab => Ok(ServiceProcess {
            loaded: false,
            pid: None,
        }),
    }
}

/// Why the runner service is not running: not loaded, or no live process.
/// `None` when the manager has it loaded with a live process, which may be
/// waiting for the runner lock.
fn not_alive(process: &ServiceProcess) -> Option<String> {
    if !process.loaded {
        return Some("the service manager does not have it loaded".into());
    }
    if process.pid.is_none() {
        return Some("no live service process".into());
    }
    None
}

/// Why a service just started is not yet running: `not_alive`, or a live
/// process that is still the one the restart stopped.
fn restarted(process: &ServiceProcess, stopped: Option<u32>) -> Option<String> {
    not_alive(process).or_else(|| {
        (process.pid == stopped).then(|| "the stopped process is still its live process".into())
    })
}

/// The owned service that runs the installed binary's `instrukt run` as the
/// runner service.
pub(super) fn runner(request: &Request<'_>) -> Definition {
    native(
        request,
        RUNNER,
        &[
            request.wake.executable.display().to_string(),
            "run".into(),
            crate::runner::SERVICE_FLAG.into(),
        ],
        None,
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

/// The receiver's invocation file as the receiver reads it at its boundary:
/// a regular, owner-only file, or `None` when there is none.
pub(super) fn receiver_file(file: &Path) -> anyhow::Result<Option<String>> {
    let Some(contents) = read(file)? else {
        return Ok(None);
    };
    anyhow::ensure!(
        std::fs::metadata(file)?.permissions().mode() & 0o077 == 0,
        "receiver invocation must be owner-only"
    );
    Ok(Some(contents))
}

/// The launchd label of this home's receiver at `port`, when its definition
/// is present under launchd.
pub(super) fn launchd_receiver(
    home: &Home,
    locations: &crate::install::SchedulerLocations,
    port: u16,
) -> Option<String> {
    let label = crate::install::launchd_label(&format!("ai.instrukt.webhooks-{port}"), home);
    locations
        .launch_agents
        .join(format!("{label}.plist"))
        .is_file()
        .then_some(label)
}

#[cfg(test)]
mod tests {
    use super::systemd_assignments;

    #[test]
    fn systemd_environment_assignments_are_decoded_whole() {
        let printed =
            r#"HOME=/home/dev "INSTRUKT_HOME=/tmp/another home" "X=a\"b\$c\\d\te\001" PATH=/a:/b"#;
        assert_eq!(
            systemd_assignments(printed).unwrap(),
            vec![
                "HOME=/home/dev".to_owned(),
                "INSTRUKT_HOME=/tmp/another home".to_owned(),
                "X=a\"b$c\\d\te\u{1}".to_owned(),
                "PATH=/a:/b".to_owned(),
            ]
        );
    }

    #[test]
    fn malformed_systemd_environment_is_refused() {
        for printed in [
            // An unmatched quote ending the text.
            r#"INSTRUKT_HOME=/home/dev/.instrukt""#,
            // A quote never closed, swallowing the assignments after it.
            r#"HOME="/home/fixture INSTRUKT_HOME=/tmp/another home"#,
            // A backslash ending the text.
            r#""INSTRUKT_HOME=/tmp/x\"#,
            // An escape systemd does not write.
            r#""INSTRUKT_HOME=/tmp/\q""#,
        ] {
            assert!(systemd_assignments(printed).is_err(), "{printed}");
        }
    }
}
