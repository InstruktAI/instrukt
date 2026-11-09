//! The generic wake-up (`task-timing`, "Wake-up installation and
//! visibility"): `instrukt install` registers `instrukt tick` every minute
//! with the machine's scheduler — launchd on macOS, a systemd user timer
//! where `systemctl` is available, else the user's crontab. There is no
//! per-task registration.
//!
//! The registration file is written into the user's home; the scheduler's
//! own command (`launchctl`, `systemctl`, `crontab`), found on `PATH`, then
//! loads it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::process::{Invocation, ProcessError};
use crate::state::{self, StateError};

/// The launchd label and systemd unit stem of the wake-up.
pub const LABEL: &str = "ai.instrukt.tick";
const SYSTEMD_UNIT: &str = "instrukt-tick";
/// Marks instrukt's line in a crontab.
const CRON_MARKER: &str = "# instrukt-tick";

/// How long the scheduler's command may take.
const REGISTRAR_TIMEOUT: Duration = Duration::from_secs(30);

/// The scheduler a wake-up is registered with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Scheduler {
    #[serde(rename = "launchd")]
    Launchd,
    #[serde(rename = "systemd")]
    SystemdUser,
    #[serde(rename = "cron")]
    Crontab,
}

impl Scheduler {
    pub fn name(self) -> &'static str {
        match self {
            Scheduler::Launchd => "launchd",
            Scheduler::SystemdUser => "systemd",
            Scheduler::Crontab => "cron",
        }
    }

    /// The machine's native scheduler, or its fallback.
    pub fn detect(path_env: Option<&str>) -> Self {
        if cfg!(target_os = "macos") && on_path("launchctl", path_env) {
            Scheduler::Launchd
        } else if cfg!(target_os = "linux") && on_path("systemctl", path_env) {
            Scheduler::SystemdUser
        } else {
            Scheduler::Crontab
        }
    }
}

/// What the wake-up runs: this binary's `tick`, in this environment.
#[derive(Debug, Clone)]
pub struct WakeUp<'a> {
    pub home: &'a instrukt_core::home::Home,
    /// `$XDG_CONFIG_HOME`, when set.
    pub config_home: Option<&'a Path>,
    pub executable: &'a Path,
    /// The `PATH` the tick runs with, so it finds the runtime's commands.
    pub path_env: &'a str,
}

/// A file the registration writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationFile {
    pub path: PathBuf,
    pub contents: String,
}

/// An installation failed.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("`{command}` failed: {detail}")]
    Registrar { command: String, detail: String },
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error(transparent)]
    State(#[from] StateError),
}

/// What an installation registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub scheduler: Scheduler,
    /// The registration file; `None` for the crontab.
    pub file: Option<PathBuf>,
}

/// The files a scheduler's registration consists of.
pub fn registration_files(scheduler: Scheduler, wake: &WakeUp<'_>) -> Vec<RegistrationFile> {
    match scheduler {
        Scheduler::Launchd => vec![RegistrationFile {
            path: launch_agent(wake.home.user_home()),
            contents: plist(wake),
        }],
        Scheduler::SystemdUser => {
            let dir = systemd_dir(wake);
            vec![
                RegistrationFile {
                    path: dir.join(format!("{SYSTEMD_UNIT}.service")),
                    contents: format!(
                        "[Unit]\nDescription=instrukt runner wake-up\n\n[Service]\nType=oneshot\nEnvironment=PATH={}\nEnvironment={}\nEnvironment={}\nEnvironment={}\nExecStart={} tick\n",
                        wake.path_env,
                        systemd_quote(&format!("HOME={}",wake.home.machine_user_home().display())),
                        systemd_quote(&format!("INSTRUKT_HOME={}",wake.home.root().display())),
                        systemd_quote(&format!("INSTRUKT_DEVELOPMENT={}",u8::from(wake.home.is_development()))),
                        systemd_quote(&wake.executable.display().to_string())
                    ),
                },
                RegistrationFile {
                    path: dir.join(format!("{SYSTEMD_UNIT}.timer")),
                    contents: "[Unit]\nDescription=instrukt runner wake-up every minute\n\n[Timer]\nOnCalendar=*-*-* *:*:00\nAccuracySec=1s\n\n[Install]\nWantedBy=timers.target\n".to_owned(),
                },
            ]
        }
        Scheduler::Crontab => Vec::new(),
    }
}

/// The crontab with instrukt's line replaced by one for `wake`.
pub fn crontab_with_wakeup(current: &str, wake: &WakeUp<'_>) -> String {
    let mut lines: Vec<String> = current
        .lines()
        .filter(|line| !line.trim_end().ends_with(CRON_MARKER))
        .map(str::to_owned)
        .collect();
    lines.push(format!(
        "* * * * * PATH={} HOME={} INSTRUKT_HOME={} INSTRUKT_DEVELOPMENT={} {} tick {CRON_MARKER}",
        crate::process::shell_quote(wake.path_env),
        crate::process::shell_quote(&wake.home.machine_user_home().display().to_string()),
        crate::process::shell_quote(&wake.home.root().display().to_string()),
        u8::from(wake.home.is_development()),
        crate::process::shell_quote(&wake.executable.display().to_string())
    ));
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Registers the wake-up with `scheduler`.
pub fn install(scheduler: Scheduler, wake: &WakeUp<'_>) -> Result<Installed, InstallError> {
    let files = registration_files(scheduler, wake);
    for file in &files {
        state::write_bytes(&file.path, file.contents.as_bytes())?;
    }
    register(scheduler, wake)
}

/// Register definitions already written and ownership-checked by installation.
pub(crate) fn register(scheduler: Scheduler, wake: &WakeUp<'_>) -> Result<Installed, InstallError> {
    let files = registration_files(scheduler, wake);
    match scheduler {
        Scheduler::Launchd => {
            let plist = launch_agent(wake.home.user_home()).display().to_string();
            let unloaded =
                Invocation::program("launchctl", &["unload", &plist]).run(REGISTRAR_TIMEOUT)?;
            if !unloaded.success {
                tracing::debug!(event = "wakeup_not_previously_loaded", stderr = %unloaded.stderr.trim());
            }
            registrar(&["launchctl", "load", "-w", &plist], None)?;
        }
        Scheduler::SystemdUser => {
            registrar(&["systemctl", "--user", "daemon-reload"], None)?;
            registrar(
                &[
                    "systemctl",
                    "--user",
                    "enable",
                    "--now",
                    &format!("{SYSTEMD_UNIT}.timer"),
                ],
                None,
            )?;
        }
        Scheduler::Crontab => {
            let listed = Invocation::program("crontab", &["-l"]).run(REGISTRAR_TIMEOUT)?;
            let current = current_crontab(listed.success, &listed.stdout, &listed.stderr)?;
            let updated = crontab_with_wakeup(&current, wake);
            registrar(&["crontab", "-"], Some(updated.as_bytes()))?;
        }
    }
    tracing::info!(event = "wakeup_installed", scheduler = scheduler.name());
    Ok(Installed {
        scheduler,
        file: files.first().map(|file| file.path.clone()),
    })
}

/// The user's current crontab from `crontab -l`. Only cron's own answer
/// that the user has no crontab reads as empty; any other failure keeps
/// the table untouched and fails the installation.
fn current_crontab(success: bool, stdout: &str, stderr: &str) -> Result<String, InstallError> {
    if success {
        Ok(stdout.to_owned())
    } else if stderr.to_ascii_lowercase().contains("no crontab for") {
        Ok(String::new())
    } else {
        Err(InstallError::Registrar {
            command: "crontab -l".into(),
            detail: stderr.trim().to_owned(),
        })
    }
}

fn registrar(argv: &[&str], input: Option<&[u8]>) -> Result<(), InstallError> {
    let (program, args) = argv.split_first().ok_or_else(|| InstallError::Registrar {
        command: String::new(),
        detail: "no command".into(),
    })?;
    let mut invocation = Invocation::program(program, args);
    if let Some(input) = input {
        invocation = invocation.input(input);
    }
    let done = invocation.run(REGISTRAR_TIMEOUT)?;
    if done.success {
        Ok(())
    } else {
        Err(InstallError::Registrar {
            command: argv.join(" "),
            detail: done.stderr.trim().to_owned(),
        })
    }
}

fn launch_agent(user_home: &Path) -> PathBuf {
    user_home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

fn systemd_dir(wake: &WakeUp<'_>) -> PathBuf {
    wake.config_home
        .map_or_else(|| wake.home.user_home().join(".config"), Path::to_path_buf)
        .join("systemd")
        .join("user")
}

fn plist(wake: &WakeUp<'_>) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
    <key>Label</key>\n\
    <string>{LABEL}</string>\n\
    <key>ProgramArguments</key>\n\
    <array>\n\
        <string>{}</string>\n\
        <string>tick</string>\n\
    </array>\n\
    <key>EnvironmentVariables</key>\n\
    <dict>\n\
        <key>PATH</key>\n\
        <string>{}</string>\n\
        <key>HOME</key>\n\
        <string>{}</string>\n\
        <key>INSTRUKT_HOME</key><string>{}</string>\n\
        <key>INSTRUKT_DEVELOPMENT</key><string>{}</string>\n\
    </dict>\n\
    <key>StartInterval</key>\n\
    <integer>60</integer>\n\
    <key>RunAtLoad</key>\n\
    <true/>\n\
</dict>\n\
</plist>\n",
        xml_escape(&wake.executable.display().to_string()),
        xml_escape(wake.path_env),
        xml_escape(&wake.home.machine_user_home().display().to_string()),
        xml_escape(&wake.home.root().display().to_string()),
        u8::from(wake.home.is_development()),
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn systemd_quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn on_path(program: &str, path_env: Option<&str>) -> bool {
    path_env.is_some_and(|path| {
        std::env::split_paths(path).any(|dir| {
            let candidate = dir.join(program);
            candidate.is_file()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crontab_keeps_other_lines_and_one_instrukt_line() {
        let home = instrukt_core::home::Home::at_user_home(PathBuf::from("/home/dev"));
        let exe = PathBuf::from("/opt/instrukt");
        let wake = WakeUp {
            home: &home,
            config_home: None,
            executable: &exe,
            path_env: "/usr/bin",
        };
        let other = "0 3 * * * backup";
        let once = crontab_with_wakeup(&format!("{other}\n"), &wake);
        let twice = crontab_with_wakeup(&once, &wake);
        assert_eq!(once, twice);
        assert_eq!(once.lines().count(), 2);
        assert_eq!(once.lines().next(), Some(other));
    }

    #[test]
    fn only_an_absent_crontab_reads_as_empty() {
        let kept = "0 3 * * * backup\n";
        assert_eq!(current_crontab(true, kept, "").unwrap(), kept);
        assert!(
            current_crontab(false, "", "crontab: no crontab for dev\n")
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            current_crontab(false, "", "crontab: permission denied\n"),
            Err(InstallError::Registrar { .. })
        ));
    }

    #[test]
    fn registration_files_live_in_the_users_home() {
        let home = instrukt_core::home::Home::at_user_home(PathBuf::from("/home/dev"));
        let config = PathBuf::from("/home/dev/.cfg");
        let exe = PathBuf::from("/opt/instrukt");
        let wake = WakeUp {
            home: &home,
            config_home: Some(&config),
            executable: &exe,
            path_env: "/usr/bin",
        };
        for scheduler in [Scheduler::Launchd, Scheduler::SystemdUser] {
            for file in registration_files(scheduler, &wake) {
                assert!(
                    file.path.starts_with(home.user_home()),
                    "{}",
                    file.path.display()
                );
            }
        }
        assert!(registration_files(Scheduler::Crontab, &wake).is_empty());
    }
}
