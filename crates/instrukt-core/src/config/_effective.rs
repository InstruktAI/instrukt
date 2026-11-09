//! The effective configuration: defaults, the global layer and the project
//! layer applied over it, read by dotted path with interpolation and
//! through typed accessors for the keys other contexts read.

use std::path::{Path, PathBuf};

use serde_yaml_ng::Value;

use super::_error::ConfigError;
use super::_features::{self, Feature, Unavailable};
use super::_interpolate::{Interpolator, Mode};
use super::_layer::{Layer, read_usable};
use super::_merge::overlay;
use super::_module::Module;
use super::_runtime::{Env, Runtime};
use super::_schema::{self, DEFAULT_GITHUB_API_URL, LayerKind};
use crate::home::Home;
use crate::ids::{DomainName, ProjectSlug};

/// One model chain tier in a role or task preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Light,
    Standard,
    Strong,
    Premium,
}

impl Tier {
    /// `strong`, `standard` or `light`.
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Strong => "strong",
            Tier::Premium => "premium",
            Tier::Standard => "standard",
            Tier::Light => "light",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Tier::Strong, Tier::Standard, Tier::Light, Tier::Premium]
            .into_iter()
            .find(|mode| mode.as_str() == text)
    }
}

/// A gate whose approver the project configures (`approvals.<gate>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalGate {
    Prd,
    Task,
}

/// Who approves a gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approver {
    Human,
    Peer,
}

/// A ref the runner watches (`events.watch`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedRef {
    pub remote: String,
    pub git_ref: String,
}

/// How the runner reports anomalies (`runner.notify`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notify {
    /// `off`: no notification.
    Off,
    /// A configured command template, unexpanded.
    Command(String),
    /// Unset: the selected runtime's own notification capability.
    RuntimeDefault,
}

/// The configuration a caller reads: the global layer with the project
/// layer applied over it inside a project, the global layer alone
/// elsewhere. Keys a layer does not permit are not read from it; unset
/// keys answer their defaults.
#[derive(Debug, Clone)]
pub struct Configuration {
    value: Value,
    project_root: Option<PathBuf>,
    project_value: Option<Value>,
    home: Home,
}

impl Configuration {
    /// The effective configuration for a caller in the repository rooted at
    /// `repo_root` (the global layer alone when `None`, or when that
    /// repository has no project layer).
    pub fn load(home: &Home, repo_root: Option<&Path>) -> Result<Self, ConfigError> {
        let global = read_usable(home, &Layer::Global)?;
        let project = match repo_root {
            Some(root) => read_usable(home, &Layer::Project(root.to_path_buf()))?,
            None => None,
        };
        let project_root = project.as_ref().and(repo_root).map(Path::to_path_buf);
        let mut config = Self::from_value(
            effective_value(global.as_ref(), project.as_ref())?,
            project_root,
            home.clone(),
        );
        config.project_value = project;
        Ok(config)
    }

    /// The effective configuration for a caller whose working directory is
    /// `dir`: the project layer of the repository containing it, if any.
    pub fn load_at(home: &Home, dir: &Path) -> Result<Self, ConfigError> {
        let root = crate::git::worktree_root(dir)?;
        Self::load(home, root.as_deref())
    }

    /// The global layer alone (`--global`).
    pub fn load_global(home: &Home) -> Result<Self, ConfigError> {
        Self::load(home, None)
    }

    fn from_value(value: Value, project_root: Option<PathBuf>, home: Home) -> Self {
        Self {
            value,
            project_root,
            project_value: None,
            home,
        }
    }

    /// The root of the repository whose project layer takes part, if any.
    pub fn project_root(&self) -> Option<&Path> {
        self.project_root.as_deref()
    }

    /// The value at dotted `path` (the whole configuration when `None`),
    /// interpolated in `env`. Transport credentials are masked before expansion;
    /// typed accessors resolve them for I/O. `runtime.session_id` uses the selected adapter.
    pub fn get(&self, path: Option<&str>, env: &Env) -> Result<Value, ConfigError> {
        let mut value = self.value.clone();
        super::_redis::redact(&mut value, "");
        super::_webhooks::redact(&mut value);
        Interpolator::new(&value, Mode::Expand(env)).get(path)
    }

    /// Read machine-owned Redis settings, resolving credential references for I/O.
    pub fn redis(&self, env: &Env) -> Result<super::RedisSettings, ConfigError> {
        super::_redis::settings(self, env)
    }

    /// Whether this project explicitly refers to a local Redis endpoint, independently
    /// of inherited defaults or module enablement. Installation retains such demand.
    pub fn project_local_redis(&self) -> Result<Option<String>, ConfigError> {
        let Some(value) = &self.project_value else {
            return Ok(None);
        };
        value
            .get("redis")
            .and_then(|r| r.get("local"))
            .and_then(|r| r.get("url"))
            .and_then(Value::as_str)
            .map(|text| self.secret_template(text))
            .transpose()
    }

    fn recorded_path(home: &Home, project: &ProjectSlug) -> PathBuf {
        home.state_dir()
            .join("configuration")
            .join(format!("{project}.yaml"))
    }
    /// Retain desired project settings for inspection without reopening its checkout.
    pub fn record_project(
        &self,
        project: &ProjectSlug,
        repository: Option<&str>,
    ) -> Result<(), ConfigError> {
        let path = Self::recorded_path(&self.home, project);
        let value = self
            .project_value
            .as_ref()
            .ok_or_else(|| invalid("project", "no project layer to retain"))?
            .clone();
        let mut value = value;
        if let Some(repository) = repository {
            value["github"]["repository"] = Value::String(repository.into());
        }
        let text = super::render_yaml(&value).map_err(|source| ConfigError::Render {
            path: path.clone(),
            source,
        })?;
        use std::os::unix::fs::PermissionsExt;
        crate::fsx::atomic_write_with_permissions(
            &path,
            text.as_bytes(),
            std::fs::Permissions::from_mode(0o600),
        )?;
        Ok(())
    }
    pub fn load_recorded_project(
        home: &Home,
        project: &ProjectSlug,
        checkout: &Path,
    ) -> Result<Self, ConfigError> {
        let path = Self::recorded_path(home, project);
        let Some(document) = super::_layer::read(&path)? else {
            return Err(ConfigError::InvalidValue {
                key: project.to_string(),
                message: format!(
                    "{} has no retained desired configuration; run instrukt install to record it",
                    path.display()
                ),
            });
        };
        let value = super::_layer::current_view(
            home,
            &Layer::Project(checkout.into()),
            &path,
            document.value,
        )?;
        let global = read_usable(home, &Layer::Global)?;
        let mut config = Self::from_value(
            effective_value(global.as_ref(), Some(&value))?,
            Some(checkout.into()),
            home.clone(),
        );
        config.project_value = Some(value);
        Ok(config)
    }

    pub fn webhooks(&self) -> Result<super::WebhookSources, ConfigError> {
        super::_webhooks::parse(self.value.get("webhooks"))
    }

    pub fn ingress(&self) -> Result<super::Ingress, ConfigError> {
        super::_webhooks::ingress(self.value.get("ingress"))
    }

    pub(super) fn redis_raw(&self) -> Option<&Value> {
        self.value.get("redis")
    }

    /// The value at dotted `path` as a string, interpolated in `env`; a
    /// mapping or list is refused.
    pub fn get_str(&self, path: &str, env: &Env) -> Result<String, ConfigError> {
        match self.get(Some(path), env)? {
            Value::String(text) => Ok(text),
            Value::Bool(b) => Ok(b.to_string()),
            Value::Number(n) => Ok(n.to_string()),
            _ => Err(ConfigError::NotScalar {
                path: path.to_owned(),
            }),
        }
    }

    /// Expands the references of a template read from this configuration
    /// (an action or a notification command) in `env`.
    pub fn expand(&self, template: &str, env: &Env) -> Result<String, ConfigError> {
        Interpolator::new(&self.value, Mode::Expand(env)).expand_text(template, None)
    }

    /// The selected compiled runtime. Installed tools and caller context never select one.
    pub fn runner_runtime(&self) -> Result<Option<Runtime>, ConfigError> {
        self.str_at(&["runner", "runtime"])?
            .map(|name| {
                Runtime::named(name)
                    .ok_or_else(|| invalid("runner.runtime", "unknown compiled runtime adapter"))
            })
            .transpose()
    }

    pub fn caller(&self, env: &Env) -> Result<Option<crate::ids::SessionRef>, ConfigError> {
        Ok(self
            .runner_runtime()?
            .and_then(|runtime| runtime.caller(env)))
    }

    pub fn caller_identity(
        &self,
        env: &Env,
    ) -> Result<Option<crate::interfaces::CallerIdentity>, ConfigError> {
        Ok(self
            .runner_runtime()?
            .and_then(|runtime| env.caller_identity(runtime.name())))
    }

    /// Whether `features` are available on the chosen runtime; `None` when
    /// they are.
    pub fn unavailable(&self, features: &[Feature]) -> Result<Option<Unavailable>, ConfigError> {
        Ok(_features::availability(self.runner_runtime()?, features))
    }

    /// Whether `module` is switched on (off by default).
    pub fn module_enabled(&self, module: Module) -> Result<bool, ConfigError> {
        Ok(self.bool_at(&["modules", module.key()])?.unwrap_or(false))
    }

    /// The project slug of the project layer, if one takes part.
    pub fn project(&self) -> Result<Option<ProjectSlug>, ConfigError> {
        self.str_at(&["project"])?
            .map(|raw| ProjectSlug::parse(raw).map_err(|err| invalid("project", &err.to_string())))
            .transpose()
    }

    /// `agents.<name>.git_author`, when configured.
    pub fn agent_git_author(&self, agent: &str) -> Result<Option<&str>, ConfigError> {
        self.str_at(&["agents", agent, "git_author"])
    }

    /// The machine-local role/profile configuration.
    pub fn agents(&self) -> Result<super::Agents, ConfigError> {
        super::Agents::load(&self.home)
    }

    pub(super) fn secret_template(&self, template: &str) -> Result<String, ConfigError> {
        Interpolator::new(&self.value, Mode::Secrets(&self.home)).expand_text(template, None)
    }

    pub(super) fn secret(&self, reference: &str) -> Result<String, ConfigError> {
        super::resolve_secret(&self.home, reference)
    }

    /// `github.api_url` (default `https://api.github.com`).
    pub fn github_api_url(&self) -> Result<&str, ConfigError> {
        Ok(self
            .str_at(&["github", "api_url"])?
            .unwrap_or(DEFAULT_GITHUB_API_URL))
    }

    /// `github.repository` (`owner/repo`), when the project sets it; the
    /// default is derived from the remote the default branch tracks.
    pub fn github_repository(&self) -> Result<Option<&str>, ConfigError> {
        self.str_at(&["github", "repository"])
    }

    /// `github.approvers`: the logins whose inbound approvals the work
    /// record accepts (empty by default).
    pub fn github_approvers(&self) -> Result<Vec<&str>, ConfigError> {
        let Some(list) = self.at(&["github", "approvers"]) else {
            return Ok(Vec::new());
        };
        list.as_sequence()
            .ok_or_else(|| invalid("github.approvers", "is not a list"))?
            .iter()
            .map(|item| {
                item.as_str()
                    .ok_or_else(|| invalid("github.approvers", "holds a non-string login"))
            })
            .collect()
    }

    /// `workflow.domain` (default `sdlc`).
    pub fn workflow_domain(&self) -> Result<DomainName, ConfigError> {
        let raw = self.str_at(&["workflow", "domain"])?.unwrap_or("sdlc");
        DomainName::parse(raw).map_err(|err| invalid("workflow.domain", &err.to_string()))
    }

    /// `approvals.<gate>` (default human).
    pub fn approver(&self, gate: ApprovalGate) -> Result<Approver, ConfigError> {
        let key = match gate {
            ApprovalGate::Prd => "prd",
            ApprovalGate::Task => "task",
        };
        match self.str_at(&["approvals", key])? {
            None | Some("human") => Ok(Approver::Human),
            Some("peer") => Ok(Approver::Peer),
            Some(_) => Err(invalid(
                &format!("approvals.{key}"),
                "is not `human` or `peer`",
            )),
        }
    }

    /// `events.watch` (empty by default).
    pub fn events_watch(&self) -> Result<Vec<WatchedRef>, ConfigError> {
        let Some(list) = self.at(&["events", "watch"]) else {
            return Ok(Vec::new());
        };
        let items = list
            .as_sequence()
            .ok_or_else(|| invalid("events.watch", "is not a list"))?;
        items
            .iter()
            .map(|item| {
                let field = |name: &str| item.get(name).and_then(Value::as_str).map(str::to_owned);
                match (field("remote"), field("ref")) {
                    (Some(remote), Some(git_ref)) => Ok(WatchedRef { remote, git_ref }),
                    _ => Err(invalid(
                        "events.watch",
                        "holds an entry without `remote` and `ref`",
                    )),
                }
            })
            .collect()
    }

    /// `runner.autostart` for todos of `kind` (default off; a map omitting
    /// the kind is off).
    pub fn runner_autostart(&self, kind: &str) -> Result<bool, ConfigError> {
        match self.at(&["runner", "autostart"]) {
            None => Ok(false),
            Some(Value::Bool(on)) => Ok(*on),
            Some(Value::Mapping(map)) => match map.get(kind) {
                None => Ok(false),
                Some(Value::Bool(on)) => Ok(*on),
                Some(_) => Err(invalid("runner.autostart", "maps a kind to a non-boolean")),
            },
            Some(_) => Err(invalid("runner.autostart", "is not a boolean or a mapping")),
        }
    }

    /// `runner.notify`.
    pub fn runner_notify(&self) -> Result<Notify, ConfigError> {
        Ok(match self.str_at(&["runner", "notify"])? {
            None => Notify::RuntimeDefault,
            Some("off") => Notify::Off,
            Some(command) => Notify::Command(command.to_owned()),
        })
    }

    /// `blueprint.engine`; `None` selects the shipped adapter.
    pub fn blueprint_engine(&self) -> Result<Option<&str>, ConfigError> {
        self.str_at(&["blueprint", "engine"])
    }

    /// Whether the machine's wake-up is installed (`ticks`, default on).
    pub fn ticks_enabled(&self) -> Result<bool, ConfigError> {
        match self.str_at(&["ticks"])? {
            None | Some("on") => Ok(true),
            Some("off") => Ok(false),
            Some(_) => Err(invalid("ticks", "is not `on` or `off`")),
        }
    }

    /// The GitHub project todos are reflected to (`board`: `false`, the
    /// default, or `<owner>/<number>`). `true` names no project and is
    /// refused, asking for the reference.
    pub fn board(&self) -> Result<Option<BoardRef>, ConfigError> {
        match self.at(&["board"]) {
            None | Some(Value::Null) | Some(Value::Bool(false)) => Ok(None),
            Some(Value::Bool(true)) => Err(invalid(
                "board",
                "names no project; set it to the GitHub project `<owner>/<number>`, or `false`",
            )),
            Some(Value::String(text)) => board_ref(text)
                .map(Some)
                .ok_or_else(|| invalid("board", "is not `<owner>/<number>`")),
            Some(_) => Err(invalid(
                "board",
                "is not `false` or a GitHub project `<owner>/<number>`",
            )),
        }
    }

    /// The modules the project's flavor requires (`requires`).
    pub fn requires(&self) -> Result<Vec<Module>, ConfigError> {
        let Some(list) = self.at(&["requires"]) else {
            return Ok(Vec::new());
        };
        list.as_sequence()
            .ok_or_else(|| invalid("requires", "is not a list"))?
            .iter()
            .map(|item| {
                item.as_str()
                    .and_then(Module::from_key)
                    .ok_or_else(|| invalid("requires", "names an unknown module"))
            })
            .collect()
    }

    fn at(&self, path: &[&str]) -> Option<&Value> {
        path.iter()
            .try_fold(&self.value, |value, key| value.get(*key))
    }

    fn str_at(&self, path: &[&str]) -> Result<Option<&str>, ConfigError> {
        match self.at(path) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            Some(_) => Err(invalid(&path.join("."), "is not a string")),
        }
    }

    fn bool_at(&self, path: &[&str]) -> Result<Option<bool>, ConfigError> {
        match self.at(path) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(invalid(&path.join("."), "is not true or false")),
        }
    }
}

/// A GitHub project (Projects v2): its owner's login and its number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardRef {
    pub owner: String,
    pub number: u64,
}

/// `<owner>/<number>` read as a project reference.
pub(crate) fn board_ref(text: &str) -> Option<BoardRef> {
    let (owner, number) = text.split_once('/')?;
    let owner_ok =
        !owner.is_empty() && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let number: u64 = number.parse().ok().filter(|n| *n > 0)?;
    owner_ok.then(|| BoardRef {
        owner: owner.to_owned(),
        number,
    })
}

fn invalid(key: &str, message: &str) -> ConfigError {
    ConfigError::InvalidValue {
        key: key.to_owned(),
        message: message.to_owned(),
    }
}

/// Defaults, then the global layer, then the project layer — each layer
/// contributing only the keys it may hold.
pub(crate) fn effective_value(
    global: Option<&Value>,
    project: Option<&Value>,
) -> Result<Value, ConfigError> {
    let mut value = _schema::defaults(LayerKind::Global);
    let shipped: Value =
        serde_yaml_ng::from_str(include_str!("shipped/t3.yaml")).map_err(|source| {
            ConfigError::Parse {
                path: "shipped/t3.yaml".into(),
                source,
            }
        })?;
    overlay(&mut value, &shipped);
    if let Some(global) = global {
        overlay(&mut value, &_schema::permitted(global, LayerKind::Global));
    }
    if let Some(project) = project {
        overlay(&mut value, &_schema::defaults(LayerKind::Project));
        overlay(&mut value, &_schema::permitted(project, LayerKind::Project));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(text: &str) -> Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    fn config(global: &str, project: Option<&str>) -> Configuration {
        let project = project.map(yaml);
        Configuration::from_value(
            effective_value(Some(&yaml(global)), project.as_ref()).unwrap(),
            None,
            Home::at_user_home("/unused".into()),
        )
    }

    const GLOBAL: &str = "schema_version: 1\nmodules: {todos: true, blueprint: true}\n\
        runner: {autostart: true, runtime: t3, notify: 'notify-me'}\n\
        github: {api_url: 'http://github.local'}\n\
        runtimes: [{name: a, session_id_env: A}, {name: b, session_id_env: B, agent: claude}]\n\
        agents: {order: [claude], tier: strong, claude: {git_author: 'C <c@x>'}}\n";

    #[test]
    fn the_project_overrides_where_permitted_and_never_the_machines_keys() {
        let project = "schema_version: 1\nproject: demo\nmodules: {todos: false}\n\
            runner: {autostart: {build: true}, runtime: a}\nruntimes: []\nticks: 'off'\n\
            approvals: {task: peer}\nagents: {order: [other]}\n";
        let cfg = config(GLOBAL, Some(project));
        assert!(!cfg.module_enabled(Module::Todos).unwrap());
        assert!(cfg.module_enabled(Module::Blueprint).unwrap());
        assert!(cfg.runner_autostart("build").unwrap());
        assert!(!cfg.runner_autostart("bug").unwrap());
        let global = config(GLOBAL, None);
        assert_eq!(
            cfg.runner_runtime().unwrap(),
            global.runner_runtime().unwrap()
        );
        assert_eq!(
            cfg.ticks_enabled().unwrap(),
            global.ticks_enabled().unwrap()
        );
        assert_eq!(cfg.approver(ApprovalGate::Task).unwrap(), Approver::Peer);
    }

    #[test]
    fn a_malformed_project_container_does_not_hide_the_machines_settings() {
        let global = config(GLOBAL, None);
        for project in [
            "schema_version: 1\nproject: p\nrunner: text\ngithub: ~\nagents: ~\n",
            "schema_version: 1\nproject: p\nrunner: [1]\ngithub: 3\nagents: text\n",
        ] {
            let cfg = config(GLOBAL, Some(project));
            assert_eq!(
                cfg.runner_notify().unwrap(),
                global.runner_notify().unwrap()
            );
            assert_eq!(
                cfg.github_api_url().unwrap(),
                global.github_api_url().unwrap()
            );
            assert_eq!(
                cfg.agent_git_author("claude").unwrap(),
                global.agent_git_author("claude").unwrap()
            );
            assert_eq!(
                cfg.runner_runtime().unwrap(),
                global.runner_runtime().unwrap()
            );
        }
    }

    #[test]
    fn requires_decodes_every_module_by_its_key() {
        let names: Vec<&str> = Module::ALL.iter().map(|m| m.key()).collect();
        let cfg = config(
            "schema_version: 1\n",
            Some(&format!(
                "schema_version: 1\nrequires: [{}]\n",
                names.join(", ")
            )),
        );
        assert_eq!(cfg.requires().unwrap(), Module::ALL);
    }

    #[test]
    fn the_global_layer_alone_reads_no_project_keys() {
        let cfg = config("schema_version: 1\n", None);
        assert!(matches!(
            cfg.get(Some("board"), &Env::default()),
            Err(ConfigError::MissingKey { .. })
        ));
        assert!(cfg.runner_runtime().unwrap().is_none());
    }
}
