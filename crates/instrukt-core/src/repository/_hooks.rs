//! `instrukt hooks install|remove|status`: instrukt's section in each hook
//! file of the hooks directory, and nothing else.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::{self, Configuration, Module};
use crate::fsx;
use crate::home::Home;
use crate::ids::ProjectSlug;

use super::_error::RepositoryError;
use super::_git::{self, HooksDir};
use super::_hook::Hook;
use super::_run::{self, HookRunRecord};
use super::_section::{self, Installed, SectionState};

/// The mode of a hook file instrukt creates.
const NEW_HOOK_MODE: u32 = 0o755;

/// What an install or removal did to one hook file's section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionChange {
    /// The section was added (the file was created when it did not exist).
    Written,
    /// The section's content was replaced.
    Updated,
    /// The section was already current; nothing changed.
    AlreadyCurrent,
    /// The section was removed.
    Removed,
    /// There was no section to remove.
    Absent,
    /// The file is a script for another interpreter, which cannot run the
    /// section; nothing was written.
    NotShell { interpreter: String },
}

/// One hook file and what happened to its section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookChange {
    pub hook: Hook,
    pub file: PathBuf,
    pub change: SectionChange,
    /// The 1-based line running `exec` the section was inserted before.
    pub before_exec_line: Option<usize>,
    /// The file exists without execute permission, so git does not run it;
    /// instrukt leaves its permissions as they are.
    pub not_executable: bool,
}

/// What `hooks install` or `hooks remove` did, per hook file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HooksReport {
    pub dir: HooksDir,
    pub hooks: Vec<HookChange>,
}

/// Where a hook file's section stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionStatus {
    Current,
    Outdated,
    Absent,
    /// The file's markers do not delimit one section.
    Damaged {
        reason: String,
    },
}

/// One hook in use: its section and its last run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookStatus {
    pub hook: Hook,
    pub file: PathBuf,
    pub section: SectionStatus,
    pub not_executable: bool,
    /// The last recorded run; `None` before the first, or without a project
    /// slug to record it under.
    pub last_run: Option<HookRunRecord>,
}

/// `hooks status`: the hooks directory and every hook in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HooksStatus {
    pub dir: HooksDir,
    pub project: Option<ProjectSlug>,
    pub hooks: Vec<HookStatus>,
}

/// Writes instrukt's section into each hook file it uses in the repository
/// containing `cwd`, and removes the pre-push section while blueprint is
/// off. Every edit is decided before any file is written: a file whose
/// markers are damaged refuses the whole install.
pub fn install_hooks(home: &Home, cwd: &Path) -> Result<HooksReport, RepositoryError> {
    let root = _git::root(cwd)?;
    install_at(home, &root)
}

/// [`install_hooks`] for the working tree rooted at `root`.
pub(crate) fn install_at(home: &Home, root: &Path) -> Result<HooksReport, RepositoryError> {
    let started = Instant::now();
    let dir = _git::hooks_dir(root)?;
    let blueprint = Configuration::load(home, Some(root))?.module_enabled(Module::Blueprint)?;
    let mut planned = Vec::new();
    for hook in Hook::ALL {
        let file = HookFile::read(&dir.path, hook)?;
        let wanted = !hook.needs_blueprint() || blueprint;
        let plan = if wanted {
            plan_install(&file, hook)?
        } else {
            match plan_removal(&file)? {
                Some(plan) => plan,
                None => continue,
            }
        };
        planned.push((file, plan));
    }
    let mut hooks = Vec::new();
    for (file, plan) in planned {
        if let Some(text) = &plan.text {
            file.write(text)?;
        }
        hooks.push(HookChange {
            hook: file.hook,
            file: file.path.clone(),
            change: plan.change,
            before_exec_line: plan.before_exec_line,
            not_executable: file.not_executable(),
        });
    }
    tracing::debug!(
        event = "hooks_installed",
        hooks_dir = %dir.path.display(),
        changed = hooks.iter().filter(|h| matches!(h.change, SectionChange::Written | SectionChange::Updated | SectionChange::Removed)).count(),
        duration_ms = started.elapsed().as_millis() as u64,
    );
    Ok(HooksReport { dir, hooks })
}

/// Removes instrukt's section from every hook file holding one; every other
/// line, and the files themselves, remain.
pub fn remove_hooks(cwd: &Path) -> Result<HooksReport, RepositoryError> {
    let started = Instant::now();
    let root = _git::root(cwd)?;
    let dir = _git::hooks_dir(&root)?;
    let mut planned = Vec::new();
    for hook in Hook::ALL {
        let file = HookFile::read(&dir.path, hook)?;
        let plan = plan_removal(&file)?.unwrap_or(Plan {
            text: None,
            change: SectionChange::Absent,
            before_exec_line: None,
        });
        planned.push((file, plan));
    }
    let mut hooks = Vec::new();
    for (file, plan) in planned {
        if let Some(text) = &plan.text {
            file.write(text)?;
        }
        hooks.push(HookChange {
            hook: file.hook,
            file: file.path.clone(),
            change: plan.change,
            before_exec_line: None,
            not_executable: file.not_executable(),
        });
    }
    tracing::debug!(
        event = "hooks_removed",
        hooks_dir = %dir.path.display(),
        removed = hooks.iter().filter(|h| h.change == SectionChange::Removed).count(),
        duration_ms = started.elapsed().as_millis() as u64,
    );
    Ok(HooksReport { dir, hooks })
}

/// The hooks directory in use and, per hook instrukt uses, whether its
/// section is current, outdated or absent, with its last run.
pub fn hooks_status(home: &Home, cwd: &Path) -> Result<HooksStatus, RepositoryError> {
    let root = _git::root(cwd)?;
    let dir = _git::hooks_dir(&root)?;
    let project = config::project_slug(&root)?;
    let blueprint = Configuration::load(home, Some(&root))?.module_enabled(Module::Blueprint)?;
    let mut hooks = Vec::new();
    for hook in Hook::ALL {
        if hook.needs_blueprint() && !blueprint {
            continue;
        }
        let file = HookFile::read(&dir.path, hook)?;
        let section = match file.text.as_deref() {
            None => SectionStatus::Absent,
            Some(text) => match _section::state(text, hook) {
                Ok(SectionState::Current) => SectionStatus::Current,
                Ok(SectionState::Outdated) => SectionStatus::Outdated,
                Ok(SectionState::Absent) => SectionStatus::Absent,
                Err(err) => SectionStatus::Damaged {
                    reason: err.to_string(),
                },
            },
        };
        let last_run = match &project {
            Some(project) => _run::last_run(home, project, hook)?,
            None => None,
        };
        hooks.push(HookStatus {
            hook,
            file: file.path.clone(),
            section,
            not_executable: file.not_executable(),
            last_run,
        });
    }
    Ok(HooksStatus {
        dir,
        project,
        hooks,
    })
}

/// A decided edit of one hook file; `text` is `None` when nothing is
/// written.
struct Plan {
    text: Option<String>,
    change: SectionChange,
    before_exec_line: Option<usize>,
}

fn plan_install(file: &HookFile, hook: Hook) -> Result<Plan, RepositoryError> {
    let Some(text) = file.text.as_deref() else {
        return Ok(Plan {
            text: Some(_section::new_file(hook)),
            change: SectionChange::Written,
            before_exec_line: None,
        });
    };
    let installed = _section::install(text, hook).map_err(|source| file.damaged(source))?;
    if matches!(
        installed,
        Installed::Appended(_) | Installed::BeforeExec { .. }
    ) && let Some(interpreter) = _section::foreign_interpreter(text)
    {
        return Ok(Plan {
            text: None,
            change: SectionChange::NotShell { interpreter },
            before_exec_line: None,
        });
    }
    Ok(match installed {
        Installed::Unchanged => Plan {
            text: None,
            change: SectionChange::AlreadyCurrent,
            before_exec_line: None,
        },
        Installed::Replaced(text) => Plan {
            text: Some(text),
            change: SectionChange::Updated,
            before_exec_line: None,
        },
        Installed::Appended(text) => Plan {
            text: Some(text),
            change: SectionChange::Written,
            before_exec_line: None,
        },
        Installed::BeforeExec { text, line } => Plan {
            text: Some(text),
            change: SectionChange::Written,
            before_exec_line: Some(line),
        },
    })
}

/// The removal of the file's section; `None` when it holds none.
fn plan_removal(file: &HookFile) -> Result<Option<Plan>, RepositoryError> {
    let Some(text) = file.text.as_deref() else {
        return Ok(None);
    };
    let removed = _section::remove(text).map_err(|source| file.damaged(source))?;
    Ok(removed.map(|text| Plan {
        text: Some(text),
        change: SectionChange::Removed,
        before_exec_line: None,
    }))
}

/// One hook file as found: its path in the hooks directory, the file its
/// writes go to (a symlink's target), its text and permissions.
struct HookFile {
    hook: Hook,
    path: PathBuf,
    target: PathBuf,
    text: Option<String>,
    permissions: Option<std::fs::Permissions>,
}

impl HookFile {
    fn read(dir: &Path, hook: Hook) -> Result<Self, RepositoryError> {
        let path = dir.join(hook.name());
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    hook,
                    target: path.clone(),
                    path,
                    text: None,
                    permissions: None,
                });
            }
            Err(source) => return Err(RepositoryError::Read { path, source }),
        };
        let read_error = |source| RepositoryError::Read {
            path: path.clone(),
            source,
        };
        let target = std::fs::canonicalize(&path).map_err(read_error)?;
        let permissions = std::fs::metadata(&target)
            .map_err(read_error)?
            .permissions();
        let text = String::from_utf8(bytes)
            .map_err(|_| RepositoryError::NotText { path: path.clone() })?;
        Ok(Self {
            hook,
            path,
            target,
            text: Some(text),
            permissions: Some(permissions),
        })
    }

    fn not_executable(&self) -> bool {
        self.permissions
            .as_ref()
            .is_some_and(|permissions| permissions.mode() & 0o111 == 0)
    }

    fn damaged(&self, source: _section::SectionError) -> RepositoryError {
        RepositoryError::Section {
            path: self.path.clone(),
            source,
        }
    }

    fn write(&self, text: &str) -> Result<(), RepositoryError> {
        let permissions = self
            .permissions
            .clone()
            .unwrap_or_else(|| std::fs::Permissions::from_mode(NEW_HOOK_MODE));
        fsx::atomic_write_with_permissions(&self.target, text.as_bytes(), permissions)?;
        Ok(())
    }
}
