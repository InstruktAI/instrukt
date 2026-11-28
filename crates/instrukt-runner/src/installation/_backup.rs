//! Owned daily OS backup timer definition for launchd, systemd user timers,
//! and crontab (`project/instrukt/spec/feature/runner/module-installation`,
//! `project/instrukt/spec/feature/runner/home-backup`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use instrukt_core::config::BackupTime;

use super::_services::Definition;
use super::{Request, ResourceKind};
use crate::install::{
    Scheduler, backup_cron_marker, backup_label, backup_systemd_stem, backup_timer,
    escape_crontab_percent, systemd_escape_percent, systemd_quote, validate_no_control_chars,
    xml_escape,
};

/// Builds the desired daily backup timer definition for `time`.
/// Rejects control characters (`\n`, `\r`, `\0`) in executable path or
/// environment variables before rendering scheduler files/lines.
pub(super) fn definition(request: &Request<'_>, time: BackupTime) -> anyhow::Result<Definition> {
    definition_for(request.home, request.scheduler, &request.wake, time)
}

fn definition_for(
    home: &instrukt_core::home::Home,
    scheduler: Scheduler,
    wake: &crate::install::WakeUp<'_>,
    time: BackupTime,
) -> anyhow::Result<Definition> {
    let exe = wake.executable.display().to_string();
    validate_no_control_chars("executable", &exe)?;
    let env_map = wake.environment();
    for (k, v) in &env_map {
        validate_no_control_chars("environment key", k)?;
        validate_no_control_chars(k, v)?;
    }

    let mut files = BTreeMap::new();
    let unit = match scheduler {
        Scheduler::Launchd => {
            let label = backup_label(home);
            let path = wake.locations.launch_agents.join(format!("{label}.plist"));
            let contents = launchd_plist(&label, &exe, &env_map, time);
            files.insert(path, contents);
            label
        }
        Scheduler::SystemdUser => {
            let stem = backup_systemd_stem(home);
            let dir = &wake.locations.systemd;
            files.insert(
                dir.join(format!("{stem}.service")),
                systemd_service(&exe, &env_map),
            );
            files.insert(dir.join(format!("{stem}.timer")), systemd_timer(time));
            backup_timer(home)
        }
        Scheduler::Crontab => crontab_line(home, &exe, &env_map, time),
    };

    Ok(Definition {
        name: ResourceKind::BackupTimer,
        unit,
        scheduler,
        locations: wake.locations.clone(),
        files,
        kept: BTreeMap::new(),
        probe: None,
    })
}

/// Schedule-free identity constructor used by `_services::discover_under` so
/// withdrawal, legacy cleanup, and unknown-registration inspection work even
/// when `backup` is unconfigured or withheld.
pub(super) fn identity(request: &Request<'_>) -> Definition {
    let mut files = BTreeMap::new();
    let unit = match request.scheduler {
        Scheduler::Launchd => {
            let label = backup_label(request.home);
            files.insert(
                request
                    .wake
                    .locations
                    .launch_agents
                    .join(format!("{label}.plist")),
                String::new(),
            );
            label
        }
        Scheduler::SystemdUser => {
            let stem = backup_systemd_stem(request.home);
            let dir = &request.wake.locations.systemd;
            files.insert(dir.join(format!("{stem}.service")), String::new());
            files.insert(dir.join(format!("{stem}.timer")), String::new());
            backup_timer(request.home)
        }
        Scheduler::Crontab => backup_cron_marker(request.home),
    };
    Definition {
        name: ResourceKind::BackupTimer,
        unit,
        scheduler: request.scheduler,
        locations: request.wake.locations.clone(),
        files,
        kept: BTreeMap::new(),
        probe: None,
    }
}

fn launchd_plist(
    label: &str,
    exe: &str,
    env_map: &BTreeMap<String, String>,
    time: BackupTime,
) -> String {
    let mut env_entries = String::new();
    for (k, v) in env_map {
        // Keep `<key>INSTRUKT_HOME</key><string>...</string>` contiguous on one
        // line so `_services::named_home` parses ownership reliably.
        env_entries.push_str(&format!(
            "        <key>{}</key><string>{}</string>\n",
            xml_escape(k),
            xml_escape(v),
        ));
    }
    [
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_owned(),
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">".to_owned(),
        "<plist version=\"1.0\">".to_owned(),
        "<dict>".to_owned(),
        "    <key>Label</key>".to_owned(),
        format!("    <string>{}</string>", xml_escape(label)),
        "    <key>ProgramArguments</key>".to_owned(),
        "    <array>".to_owned(),
        format!("        <string>{}</string>", xml_escape(exe)),
        "        <string>backup</string>".to_owned(),
        "        <string>run</string>".to_owned(),
        "    </array>".to_owned(),
        "    <key>EnvironmentVariables</key>".to_owned(),
        "    <dict>".to_owned(),
        env_entries.trim_end_matches('\n').to_owned(),
        "    </dict>".to_owned(),
        "    <key>StartCalendarInterval</key>".to_owned(),
        "    <dict>".to_owned(),
        "        <key>Hour</key>".to_owned(),
        format!("        <integer>{}</integer>", time.hour()),
        "        <key>Minute</key>".to_owned(),
        format!("        <integer>{}</integer>", time.minute()),
        "    </dict>".to_owned(),
        "    <key>Umask</key>".to_owned(),
        "    <integer>63</integer>".to_owned(),
        "</dict>".to_owned(),
        "</plist>".to_owned(),
        String::new(),
    ]
    .join("\n")
}

fn systemd_service(exe: &str, env_map: &BTreeMap<String, String>) -> String {
    let mut out = String::from(
        "[Unit]\nDescription=instrukt encrypted home backup\n\n[Service]\nType=oneshot\nUMask=0077\n",
    );
    for (k, v) in env_map {
        let escaped = systemd_escape_percent(&format!("{k}={v}"));
        out.push_str(&format!("Environment={}\n", systemd_quote(&escaped)));
    }
    let exec = systemd_quote(&systemd_escape_percent(exe));
    out.push_str(&format!("ExecStart={exec} backup run\n"));
    out
}

fn systemd_timer(time: BackupTime) -> String {
    format!(
        "[Unit]\nDescription=instrukt daily home backup timer\n\n[Timer]\nOnCalendar=*-*-* {:02}:{:02}:00\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n",
        time.hour(),
        time.minute(),
    )
}

fn crontab_line(
    home: &instrukt_core::home::Home,
    exe: &str,
    env_map: &BTreeMap<String, String>,
    time: BackupTime,
) -> String {
    let marker = backup_cron_marker(home);
    let env_prefix = env_map
        .iter()
        .map(|(k, v)| {
            format!(
                "{k}={}",
                escape_crontab_percent(&crate::process::shell_quote(v))
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let quoted_exe = escape_crontab_percent(&crate::process::shell_quote(exe));
    format!(
        "{:02} {:02} * * * {env_prefix} {quoted_exe} backup run {marker}",
        time.minute(),
        time.hour(),
    )
}

/// Returns the expected file paths for `BackupTimer` under the active scheduler
/// (used when reporting withheld/unmet findings without mutating files).
fn expected_paths(request: &Request<'_>) -> Vec<PathBuf> {
    let id = identity(request);
    if request.scheduler == Scheduler::Crontab {
        request
            .wake
            .locations
            .crontab_file
            .iter()
            .cloned()
            .collect()
    } else {
        id.files.keys().cloned().collect()
    }
}

/// Converges, checks, or withdraws the machine's owned daily home backup timer
/// directly from global `backup` admission (`module-installation`, "Daily home
/// backup timer"). Unconfigured homes withdraw an existing timer; withheld
/// backup configuration reports findings and leaves any existing timer in place.
pub(super) fn converge(request: &Request<'_>, report: &mut super::Report) {
    use instrukt_core::config::{BackupAdmission, inspect_global_backup};
    match inspect_global_backup(request.home) {
        Ok(BackupAdmission::Admitted(cfg)) => match definition(request, cfg.time) {
            Ok(desired) => {
                if let Err(error) =
                    super::_services::reconcile(request, &desired, true, true, &[], report)
                {
                    report.finding(ResourceKind::BackupTimer, error);
                }
            }
            Err(error) => {
                let cause = error.to_string();
                report.finding(ResourceKind::BackupTimer, &cause);
                report.resource(
                    ResourceKind::BackupTimer,
                    super::Disposition::Unmet,
                    expected_paths(request),
                    &[],
                    cause,
                );
            }
        },
        Ok(BackupAdmission::Withheld(findings)) => {
            for finding in &findings {
                report.finding(
                    ResourceKind::BackupTimer,
                    format!(
                        "`{}`: {}; daily home backup timer is withheld without withdrawing any existing timer",
                        finding.key, finding.message
                    ),
                );
            }
            report.resource(
                ResourceKind::BackupTimer,
                super::Disposition::Unmet,
                expected_paths(request),
                &[],
                "backup configuration is withheld; fix findings in ~/.instrukt/instrukt.yaml and rerun `instrukt install`",
            );
        }
        Ok(BackupAdmission::Unconfigured) => {
            if let Err(error) =
                super::_services::withdraw_named(request, ResourceKind::BackupTimer, true, report)
            {
                report.finding(ResourceKind::BackupTimer, error);
            }
        }
        Err(error) => {
            let cause = error.to_string();
            report.finding(ResourceKind::BackupTimer, &cause);
            report.resource(
                ResourceKind::BackupTimer,
                super::Disposition::Unmet,
                expected_paths(request),
                &[],
                cause,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::{SchedulerLocations, WakeUp};
    use instrukt_core::config::Env;
    use instrukt_core::home::Home;

    #[test]
    fn renders_launchd_systemd_and_crontab_with_escaping_and_umask() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        let env = Env::from_pairs([("INSTRUKT_LOG_DIR", "/tmp/log%dir")]);
        let exe = PathBuf::from("/opt/bin/instrukt");
        let locations = SchedulerLocations::resolve(&home, &env).unwrap();
        let wake = WakeUp {
            home: &home,
            locations: locations.clone(),
            executable: &exe,
            path_env: "/usr/bin:/bin",
        };
        let time = BackupTime::parse("04:30").unwrap();

        // Launchd
        let def_l = definition_for(&home, Scheduler::Launchd, &wake, time).unwrap();
        let plist = def_l.files.values().next().unwrap();
        assert!(plist.contains("<key>StartCalendarInterval</key>"));
        assert!(plist.contains("<key>Hour</key>\n        <integer>4</integer>"));
        assert!(plist.contains("<key>Minute</key>\n        <integer>30</integer>"));
        assert!(plist.contains("<key>Umask</key>\n    <integer>63</integer>"));
        assert!(plist.contains("<string>backup</string>"));
        assert!(plist.contains("<string>run</string>"));
        assert!(plist.contains(&format!(
            "<key>INSTRUKT_HOME</key><string>{}</string>",
            home.root().display()
        )));

        // Systemd
        let def_s = definition_for(&home, Scheduler::SystemdUser, &wake, time).unwrap();
        let svc = def_s
            .files
            .iter()
            .find(|(p, _)| p.extension().is_some_and(|e| e == "service"))
            .unwrap()
            .1;
        let tmr = def_s
            .files
            .iter()
            .find(|(p, _)| p.extension().is_some_and(|e| e == "timer"))
            .unwrap()
            .1;
        assert!(svc.contains("Type=oneshot"));
        assert!(svc.contains("UMask=0077"));
        assert!(svc.contains("INSTRUKT_LOG_DIR=/tmp/log%%dir"));
        assert!(svc.contains("backup run"));
        assert!(tmr.contains("OnCalendar=*-*-* 04:30:00"));
        assert!(tmr.contains("Persistent=true"));

        // Crontab
        let def_c = definition_for(&home, Scheduler::Crontab, &wake, time).unwrap();
        assert!(def_c.unit.starts_with("30 04 * * * "));
        assert!(def_c.unit.contains("/tmp/log\\%dir"));
        assert!(def_c.unit.ends_with("backup run # instrukt-backup"));

        // Control char rejection
        let bad_exe = PathBuf::from("/opt/bin/ins\ntrukt");
        let bad_wake = WakeUp {
            home: &home,
            locations,
            executable: &bad_exe,
            path_env: "/usr/bin:/bin",
        };
        assert!(definition_for(&home, Scheduler::Crontab, &bad_wake, time).is_err());
    }
}
