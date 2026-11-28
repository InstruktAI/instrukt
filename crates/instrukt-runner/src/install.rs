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

/// The launchd label and systemd unit stem of the default home's wake-up.
pub const LABEL: &str = "ai.instrukt.tick";
const SYSTEMD_UNIT: &str = "instrukt-tick";
/// Marks the default home's wake-up line in a crontab.
pub(crate) const CRON_MARKER: &str = "# instrukt-tick";

/// The launchd label, systemd unit stem, and crontab marker of the default
/// home's daily encrypted backup timer.
pub const BACKUP_LABEL: &str = "ai.instrukt.backup";
pub(crate) const BACKUP_SYSTEMD_UNIT: &str = "instrukt-backup";
pub(crate) const BACKUP_CRON_MARKER: &str = "# instrukt-backup";

/// What a home other than `~/.instrukt` adds to instrukt's service names:
/// the start of its canonical path's digest. `None` for the default home,
/// which keeps the names without it. A service's name is its identity, so a
/// second home never writes, reloads or withdraws the first one's units.
pub(crate) fn home_suffix(home: &instrukt_core::home::Home) -> Option<String> {
    use sha2::{Digest as _, Sha256};
    let canonical =
        |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let root = canonical(home.root());
    if root == canonical(&home.machine_user_home().join(".instrukt")) {
        return None;
    }
    let digest = hex::encode(Sha256::digest(root.as_os_str().as_encoded_bytes()));
    Some(digest[..8].to_owned())
}

/// A launchd label for `home`: `base`, with the home's suffix after a dot.
pub(crate) fn launchd_label(base: &str, home: &instrukt_core::home::Home) -> String {
    home_suffix(home).map_or_else(|| base.to_owned(), |suffix| format!("{base}.{suffix}"))
}

/// A systemd unit stem for `home`: `base`, with the home's suffix after a dash.
pub(crate) fn systemd_stem(base: &str, home: &instrukt_core::home::Home) -> String {
    home_suffix(home).map_or_else(|| base.to_owned(), |suffix| format!("{base}-{suffix}"))
}

/// The marker of `home`'s wake-up line in a crontab.
pub(crate) fn cron_marker(home: &instrukt_core::home::Home) -> String {
    home_suffix(home).map_or_else(
        || CRON_MARKER.to_owned(),
        |suffix| format!("{CRON_MARKER}-{suffix}"),
    )
}

/// The wake-up's launchd label for `home`.
pub(crate) fn label(home: &instrukt_core::home::Home) -> String {
    launchd_label(LABEL, home)
}

/// The wake-up's systemd timer for `home`.
pub(crate) fn timer(home: &instrukt_core::home::Home) -> String {
    format!("{}.timer", systemd_stem(SYSTEMD_UNIT, home))
}

/// The daily backup timer's launchd label for `home`.
pub(crate) fn backup_label(home: &instrukt_core::home::Home) -> String {
    launchd_label(BACKUP_LABEL, home)
}

/// The daily backup timer's systemd unit stem for `home`.
pub(crate) fn backup_systemd_stem(home: &instrukt_core::home::Home) -> String {
    systemd_stem(BACKUP_SYSTEMD_UNIT, home)
}

/// The daily backup timer's systemd timer unit name for `home`.
pub(crate) fn backup_timer(home: &instrukt_core::home::Home) -> String {
    format!("{}.timer", backup_systemd_stem(home))
}

/// The marker of `home`'s daily backup line in a crontab.
pub(crate) fn backup_cron_marker(home: &instrukt_core::home::Home) -> String {
    home_suffix(home).map_or_else(
        || BACKUP_CRON_MARKER.to_owned(),
        |suffix| format!("{BACKUP_CRON_MARKER}-{suffix}"),
    )
}

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

/// Optional native application roots and exact scheduler locations carried to children.
pub const LOCATION_VARIABLES: [&str; 11] = [
    "INSTRUKT_LOG_DIR",
    "T3CODE_HOME",
    "CODEX_HOME",
    "CLAUDE_CONFIG_DIR",
    "OPENCODE_CONFIG_DIR",
    "FX_HOME",
    "KIRO_HOME",
    "PI_CODING_AGENT_DIR",
    "SYSTEMD_UNIT_PATH",
    "INSTRUKT_LAUNCH_AGENTS_DIR",
    "INSTRUKT_CRONTAB_FILE",
];

/// Resolved once at invocation entry; every scheduler consumer uses these locations.
#[derive(Debug, Clone)]
pub struct SchedulerLocations {
    pub systemd: PathBuf,
    pub launch_agents: PathBuf,
    pub crontab_file: Option<PathBuf>,
    pub environment: std::collections::BTreeMap<String, String>,
}
impl SchedulerLocations {
    pub fn resolve(
        home: &instrukt_core::home::Home,
        env: &instrukt_core::config::Env,
    ) -> Result<Self, InstallError> {
        let absolute = |setting: &str, value: &str| -> Result<PathBuf, InstallError> {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                Ok(path)
            } else {
                Err(InstallError::Location {
                    setting: setting.into(),
                    reason: "must be an absolute path".into(),
                })
            }
        };
        let systemd = if let Some(value) = env.get("SYSTEMD_UNIT_PATH") {
            let entries: Vec<_> = value.split(':').collect();
            let mut selected = None;
            for (n, entry) in entries.iter().enumerate() {
                if entry.is_empty() && n > 0 && n == entries.len() - 1 {
                    continue;
                }
                let path = absolute("SYSTEMD_UNIT_PATH", entry)?;
                selected.get_or_insert(path);
            }
            selected.ok_or_else(|| InstallError::Location {
                setting: "SYSTEMD_UNIT_PATH".into(),
                reason: "no explicit absolute directory".into(),
            })?
        } else {
            // XDG's native default ignores nonabsolute base-directory values.
            env.get("XDG_CONFIG_HOME")
                .filter(|value| Path::new(value).is_absolute())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.machine_user_home().join(".config"))
                .join("systemd/user")
        };
        Ok(Self {
            systemd,
            launch_agents: env
                .get("INSTRUKT_LAUNCH_AGENTS_DIR")
                .map(|v| absolute("INSTRUKT_LAUNCH_AGENTS_DIR", v))
                .transpose()?
                .unwrap_or_else(|| home.machine_user_home().join("Library/LaunchAgents")),
            crontab_file: env
                .get("INSTRUKT_CRONTAB_FILE")
                .map(|v| absolute("INSTRUKT_CRONTAB_FILE", v))
                .transpose()?,
            environment: LOCATION_VARIABLES
                .iter()
                .filter_map(|name| env.get(name).map(|v| ((*name).into(), v.into())))
                .collect(),
        })
    }
    pub(crate) fn verify_systemd(&self) -> Result<(), InstallError> {
        let done = Invocation::program(
            "systemctl",
            &["--user", "show", "--property=UnitPath", "--value"],
        )
        .run(REGISTRAR_TIMEOUT)?;
        if !done.success {
            return Err(InstallError::Registrar {
                command: "systemctl --user show UnitPath".into(),
                detail: done.stderr,
            });
        }
        let paths =
            systemd_assignments(done.stdout.trim()).map_err(|reason| InstallError::Location {
                setting: "manager UnitPath".into(),
                reason,
            })?;
        if !paths.iter().any(|p| Path::new(p) == self.systemd) {
            return Err(InstallError::Location {
                setting: "manager UnitPath".into(),
                reason: format!(
                    "running manager does not include {}; installer environment does not reconfigure it",
                    self.systemd.display()
                ),
            });
        }
        Ok(())
    }
}

/// What the wake-up runs: this binary's `tick`, in this environment.
#[derive(Debug, Clone)]
pub struct WakeUp<'a> {
    pub home: &'a instrukt_core::home::Home,
    pub locations: SchedulerLocations,
    pub executable: &'a Path,
    /// The `PATH` the tick runs with, so it finds the runtime's commands.
    pub path_env: &'a str,
}

impl WakeUp<'_> {
    pub(crate) fn environment(&self) -> std::collections::BTreeMap<String, String> {
        let mut values = self.locations.environment.clone();
        values.extend([
            (
                "HOME".into(),
                self.home.machine_user_home().display().to_string(),
            ),
            (
                "INSTRUKT_HOME".into(),
                self.home.root().display().to_string(),
            ),
            (
                "INSTRUKT_DEVELOPMENT".into(),
                u8::from(self.home.is_development()).to_string(),
            ),
            ("PATH".into(), self.path_env.into()),
        ]);
        values
    }
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
    #[error("invalid {setting}: {reason}")]
    Location { setting: String, reason: String },
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
    /// File-only cron materialization is not an active registration.
    pub registered: bool,
    /// The registration file; `None` for the crontab.
    pub file: Option<PathBuf>,
}

/// The files a scheduler's registration consists of.
pub fn registration_files(scheduler: Scheduler, wake: &WakeUp<'_>) -> Vec<RegistrationFile> {
    let mut files = match scheduler {
        Scheduler::Launchd => vec![RegistrationFile {
            path: launch_agent(wake),
            contents: plist(wake),
        }],
        Scheduler::SystemdUser => {
            let dir = &wake.locations.systemd;
            let stem = systemd_stem(SYSTEMD_UNIT, wake.home);
            vec![
                RegistrationFile {
                    path: dir.join(format!("{stem}.service")),
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
                    path: dir.join(format!("{stem}.timer")),
                    contents: "[Unit]\nDescription=instrukt runner wake-up every minute\n\n[Timer]\nOnCalendar=*-*-* *:*:00\nAccuracySec=1s\n\n[Install]\nWantedBy=timers.target\n".to_owned(),
                },
            ]
        }
        Scheduler::Crontab => Vec::new(),
    };
    for file in &mut files {
        if scheduler == Scheduler::Launchd {
            let extra = wake
                .locations
                .environment
                .iter()
                .map(|(k, v)| {
                    format!(
                        "<key>{}</key><string>{}</string>",
                        xml_escape(k),
                        xml_escape(v)
                    )
                })
                .collect::<String>();
            file.contents = file
                .contents
                .replacen("</dict>", &format!("{extra}</dict>"), 1);
        } else if file.path.extension().is_some_and(|e| e == "service") {
            let extra = wake
                .locations
                .environment
                .iter()
                .map(|(k, v)| format!("Environment={}\n", systemd_quote(&format!("{k}={v}"))))
                .collect::<String>();
            file.contents =
                file.contents
                    .replacen("\nExecStart=", &format!("\n{extra}ExecStart="), 1);
        }
    }
    files
}

/// Replaces any line in `current` ending with `marker` with `line`, keeping
/// all other lines intact.
pub(crate) fn crontab_with_marked_line(current: &str, marker: &str, line: &str) -> String {
    let mut lines: Vec<String> = current
        .lines()
        .filter(|existing| !existing.trim_end().ends_with(marker))
        .map(str::to_owned)
        .collect();
    lines.push(line.to_owned());
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Removes any line in `current` ending with `marker`, keeping all other lines
/// intact.
pub(crate) fn remove_marked_line(current: &str, marker: &str) -> String {
    let kept: Vec<&str> = current
        .lines()
        .filter(|existing| !existing.trim_end().ends_with(marker))
        .collect();
    let mut text = kept.join("\n");
    text.push('\n');
    text
}

/// Registers one marked crontab line, preserving every other line in the table.
pub(crate) fn register_crontab_line(
    file: Option<&Path>,
    marker: &str,
    line: &str,
) -> Result<(), InstallError> {
    let current = read_crontab(file)?;
    let updated = crontab_with_marked_line(&current, marker, line);
    write_crontab(file, &updated)
}

/// Escapes `%` as `\%` for Vixie/BSD cron, where unescaped `%` terminates the
/// command line and sends subsequent bytes as stdin.
pub(crate) fn escape_crontab_percent(text: &str) -> String {
    text.replace('%', "\\%")
}

/// Rejects control characters that could inject lines or split scheduler
/// definitions (`\n`, `\r`, `\0`).
pub(crate) fn validate_no_control_chars(label: &str, value: &str) -> Result<(), InstallError> {
    if value.contains(['\n', '\r', '\0']) {
        return Err(InstallError::Location {
            setting: label.to_owned(),
            reason: "must not contain newline, carriage return, or NUL bytes".into(),
        });
    }
    Ok(())
}

/// The crontab with this home's line replaced by one for `wake`. Lines
/// marked for another home, and every unmarked line, are kept.
pub fn crontab_with_wakeup(current: &str, wake: &WakeUp<'_>) -> String {
    let marker = cron_marker(wake.home);
    let mut line = format!(
        "* * * * * PATH={} HOME={} INSTRUKT_HOME={} INSTRUKT_DEVELOPMENT={} {} tick {marker}",
        escape_crontab_percent(&crate::process::shell_quote(wake.path_env)),
        escape_crontab_percent(&crate::process::shell_quote(
            &wake.home.machine_user_home().display().to_string()
        )),
        escape_crontab_percent(&crate::process::shell_quote(
            &wake.home.root().display().to_string()
        )),
        u8::from(wake.home.is_development()),
        escape_crontab_percent(&crate::process::shell_quote(
            &wake.executable.display().to_string()
        ))
    );
    if !wake.locations.environment.is_empty() {
        let extra = wake
            .locations
            .environment
            .iter()
            .map(|(k, v)| {
                format!(
                    "{k}={}",
                    escape_crontab_percent(&crate::process::shell_quote(v))
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        line = line.replacen("* * * * * ", &format!("* * * * * {extra} "), 1);
    }
    crontab_with_marked_line(current, &marker, &line)
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
            let plist = launch_agent(wake).display().to_string();
            let unloaded =
                Invocation::program("launchctl", &["unload", &plist]).run(REGISTRAR_TIMEOUT)?;
            if !unloaded.success {
                tracing::debug!(event = "wakeup_not_previously_loaded", stderr = %unloaded.stderr.trim());
            }
            registrar(&["launchctl", "load", "-w", &plist], None)?;
        }
        Scheduler::SystemdUser => {
            wake.locations.verify_systemd()?;
            registrar(&["systemctl", "--user", "daemon-reload"], None)?;
            registrar(
                &["systemctl", "--user", "enable", "--now", &timer(wake.home)],
                None,
            )?;
        }
        Scheduler::Crontab => {
            let current = read_crontab(wake.locations.crontab_file.as_deref())?;
            let updated = crontab_with_wakeup(&current, wake);
            write_crontab(wake.locations.crontab_file.as_deref(), &updated)?;
        }
    }
    tracing::info!(event = "wakeup_installed", scheduler = scheduler.name());
    Ok(Installed {
        scheduler,
        registered: scheduler != Scheduler::Crontab || wake.locations.crontab_file.is_none(),
        file: wake
            .locations
            .crontab_file
            .clone()
            .filter(|_| scheduler == Scheduler::Crontab)
            .or_else(|| files.first().map(|file| file.path.clone())),
    })
}

pub(crate) fn read_crontab(file: Option<&Path>) -> Result<String, InstallError> {
    if let Some(path) = file {
        return match std::fs::read_to_string(path) {
            Ok(text) => Ok(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(InstallError::Location {
                setting: "INSTRUKT_CRONTAB_FILE".into(),
                reason: e.to_string(),
            }),
        };
    }
    let listed = Invocation::program("crontab", &["-l"]).run(REGISTRAR_TIMEOUT)?;
    current_crontab(listed.success, &listed.stdout, &listed.stderr)
}
pub(crate) fn write_crontab(file: Option<&Path>, text: &str) -> Result<(), InstallError> {
    if let Some(path) = file {
        state::write_bytes(path, text.as_bytes())?;
        Ok(())
    } else {
        registrar(&["crontab", "-"], Some(text.as_bytes()))
    }
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

/// The assignments of systemd's `Environment` property as `systemctl show`
/// prints them: separated by spaces, each one that needs it in double quotes
/// with `"`, `\\`, `` ` `` and `$` escaped by a backslash and control
/// characters as C escapes (systemd's `shell_maybe_quote`). Output that form
/// never takes is refused: a quote that is not closed or not at a word's
/// edge, a backslash ending the text, or an escape it does not write.
pub(crate) fn systemd_assignments(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word: Vec<u8> = Vec::new();
    let mut started = false;
    let mut quoted = false;
    let mut bytes = line.bytes().peekable();
    while let Some(byte) = bytes.next() {
        match byte {
            b'"' if quoted => {
                quoted = false;
                if bytes
                    .peek()
                    .is_some_and(|next| !matches!(next, b' ' | b'\t'))
                {
                    return Err("text follows a closing quote".into());
                }
            }
            b'"' => {
                if started {
                    return Err("a quote inside an assignment".into());
                }
                quoted = true;
                started = true;
            }
            b'\\' if quoted => {
                let next = bytes.next().ok_or("a backslash ends the text")?;
                match next {
                    b'a' => word.push(0x07),
                    b'b' => word.push(0x08),
                    b'f' => word.push(0x0c),
                    b'n' => word.push(b'\n'),
                    b'r' => word.push(b'\r'),
                    b't' => word.push(b'\t'),
                    b'v' => word.push(0x0b),
                    b'"' | b'\\' | b'`' | b'$' => word.push(next),
                    b'0'..=b'3' => {
                        let mut value = next - b'0';
                        for _ in 0..2 {
                            match bytes.next() {
                                Some(digit @ b'0'..=b'7') => value = value * 8 + (digit - b'0'),
                                _ => return Err("an octal escape without three digits".into()),
                            }
                        }
                        word.push(value);
                    }
                    other => {
                        return Err(format!("the escape \\{}", char::from(other)));
                    }
                }
            }
            b' ' | b'\t' if !quoted => {
                if started {
                    words.push(String::from_utf8_lossy(&word).into_owned());
                    word.clear();
                    started = false;
                }
            }
            other => {
                word.push(other);
                started = true;
            }
        }
    }
    if quoted {
        return Err("a quote is not closed".into());
    }
    if started {
        words.push(String::from_utf8_lossy(&word).into_owned());
    }
    Ok(words)
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

fn launch_agent(wake: &WakeUp<'_>) -> PathBuf {
    wake.locations
        .launch_agents
        .join(format!("{}.plist", label(wake.home)))
}

fn plist(wake: &WakeUp<'_>) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
    <key>Label</key>\n\
    <string>{}</string>\n\
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
        label(wake.home),
        xml_escape(&wake.executable.display().to_string()),
        xml_escape(wake.path_env),
        xml_escape(&wake.home.machine_user_home().display().to_string()),
        xml_escape(&wake.home.root().display().to_string()),
        u8::from(wake.home.is_development()),
    )
}

pub(crate) fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(crate) fn systemd_quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(crate) fn systemd_escape_percent(text: &str) -> String {
    text.replace('%', "%%")
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
            locations: SchedulerLocations::resolve(&home, &instrukt_core::config::Env::default())
                .unwrap(),
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
            locations: SchedulerLocations::resolve(
                &home,
                &instrukt_core::config::Env::from_pairs([(
                    "XDG_CONFIG_HOME",
                    config.to_str().unwrap(),
                )]),
            )
            .unwrap(),
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
