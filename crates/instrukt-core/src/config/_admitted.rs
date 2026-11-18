//! The admitted configuration generation: what a long-lived runner uses in
//! place of reading files. The machine layer, the agent profiles, each
//! company home with its connector secrets resolved, and each published
//! project's layer are loaded and validated together. A runner builds one
//! when it starts and again on every wake; a complete valid one replaces the
//! current generation for every consumer at once, and an invalid one leaves
//! the current generation in force while its findings are reported. A hand
//! edit made while the runner runs is therefore used whole or not at all.
//!
//! Each document is read once per generation: what is validated is what is
//! admitted. A runner serves its current generation ([`serve`]): in its
//! process the configuration and profile loaders answer the admitted values
//! instead of the files, so no part of it reads them on its own.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde_yaml_ng::Value;

use super::_error::{ConfigError, Finding, FindingKind};
use super::_layer::{Layer, inspected, read};
use super::_module::Module;
use super::_secrets::SecretSnapshot;
use super::_validate::{Given, findings_given};
use super::{Agents, Configuration, SecretSource, SlackConnector};
use crate::home::Home;
use crate::ids::ProjectSlug;
use crate::publication::PublishedRecord;

/// Why no generation could be built: the machine layer or the agent
/// profiles are invalid, or the published projects cannot be listed.
#[derive(Debug, thiserror::Error)]
pub enum AdmissionError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Publication(#[from] crate::publication::PublicationStateError),
}

/// One admitted generation.
#[derive(Debug, Clone)]
pub struct Admitted {
    global: Configuration,
    /// The machine layer as admitted, from which project layers combine.
    global_layer: Option<Value>,
    agents: Arc<Agents>,
    companies: Vec<AdmittedCompany>,
    projects: Vec<AdmittedProject>,
    /// Some project is published here, admitted or not.
    published: bool,
}

/// A company home admitted with its generation.
#[derive(Debug, Clone)]
pub struct AdmittedCompany {
    pub name: ProjectSlug,
    pub config: Configuration,
    /// The configuration of a caller in the company's home directory: the
    /// machine layer with the company's document as that directory's layer,
    /// as admitted.
    pub home_config: Configuration,
    /// The company's home directory.
    pub dir: PathBuf,
    /// Its declared Slack connector with the secrets it references resolved.
    pub slack: Option<AdmittedSlack>,
}

/// A declared Slack connector whose referenced secrets were present and
/// nonblank when its generation was admitted.
#[derive(Clone)]
pub struct AdmittedSlack {
    pub connector: SlackConnector,
    pub app_token: String,
    pub bot_token: String,
}

impl std::fmt::Debug for AdmittedSlack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedSlack")
            .field("connector", &self.connector)
            .finish_non_exhaustive()
    }
}

/// A published project the generation considers, with what is admitted for
/// it on this computer.
#[derive(Debug, Clone)]
pub struct AdmittedProject {
    pub slug: ProjectSlug,
    pub checkout: PathBuf,
    pub config: Configuration,
    /// Todos or event responses switched on, admitted or not: the runner
    /// considers the project (its `main` is kept current) whatever a finding
    /// withholds, since a finding withholds only its module's own work.
    pub switched: bool,
    /// Todos switched on and admitted: no finding against `modules.todos`.
    pub todos: bool,
    /// Event responses switched on and admitted: never true while the
    /// project's layer has a finding against `modules.events`, as it does
    /// without the event transport.
    pub events: bool,
    /// History switched on and admitted: no finding against
    /// `modules.history`, and the layer validates otherwise.
    pub history: bool,
}

/// A generation as built from the files, with what was not admitted.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub generation: Admitted,
    /// Why a company, a project or a project's events were not admitted.
    pub findings: Vec<Finding>,
    /// Every company home and every published project's layer was admitted.
    /// Only a complete candidate replaces a running generation; a finding
    /// that withholds a module of a project never blocks one.
    pub complete: bool,
}

impl Admitted {
    /// Builds a generation from the files, reading each once. An invalid
    /// machine layer or `agents.yaml` refuses it: nothing can run on them. A
    /// company home that does not validate, or a project whose layer cannot
    /// be read, is left out with its findings and makes the candidate
    /// incomplete; a finding against a project's module withholds that module.
    pub fn load(home: &Home) -> Result<Candidate, AdmissionError> {
        // Admission reads and never writes: older documents are converted in
        // memory, and the profiles are the shipped ones when there is no file.
        let global_path = home.global_config();
        let raw_global = read(&global_path)?.map(|doc| doc.value);
        let global_layer = raw_global
            .clone()
            .map(|raw| inspected(home, &Layer::Global, &global_path, raw))
            .transpose()?;
        let agents_path = home.agents_file();
        let agents_bytes = match std::fs::read(&agents_path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(ConfigError::Read {
                    path: agents_path,
                    source,
                }
                .into());
            }
        };
        let agents = Arc::new(Agents::from_documents(
            home,
            agents_bytes,
            raw_global.as_ref(),
        )?);
        let records: Vec<PublishedRecord> = PublishedRecord::load_all(home)?;
        let published = !records.is_empty();
        let mut layers = Vec::new();
        for record in &records {
            if !record.checkout.is_dir() {
                tracing::warn!(event = "project_checkout_missing", project = %record.project, checkout = %record.checkout.display());
                continue;
            }
            let layer = Layer::Project(record.checkout.clone());
            let document = record.checkout.join(super::CONFIG_FILE_NAME);
            let value = read(&document)
                .and_then(|doc| {
                    doc.map(|doc| inspected(home, &layer, &document, doc.value))
                        .transpose()
                })
                .map_err(|error| unreadable(document.clone(), &error));
            layers.push((record, value));
        }
        let read_layers: Vec<(PathBuf, Option<Value>)> = layers
            .iter()
            .filter_map(|(record, value)| {
                value
                    .as_ref()
                    .ok()
                    .map(|value| (record.checkout.clone(), value.clone()))
            })
            .collect();
        let given = Given {
            global: Some(global_layer.as_ref()),
            agents: Some(&agents),
            projects: Some(&read_layers),
            secrets: None,
        };
        if let Some(value) = &global_layer {
            let found = findings_given(home, &Layer::Global, &global_path, value, given)?;
            if !found.is_empty() {
                return Err(ConfigError::Invalid(super::Findings(found)).into());
            }
        }
        let global = Configuration::from_layers(home, global_layer.clone(), None)?.pinned(&agents);
        let mut findings = Vec::new();
        let mut complete = true;
        let mut companies = Vec::new();
        for (name, unreadable_folder) in super::company_folders(home)? {
            let admitted = match unreadable_folder {
                Some(error) => Err(vec![unreadable(home.company_dir(&name), &error)]),
                None => admit_company(home, &name, &agents, given),
            };
            match admitted {
                Ok(Some(company)) => companies.push(company),
                Ok(None) => {}
                Err(found) => {
                    complete = false;
                    findings.extend(found);
                }
            }
        }
        let mut projects = Vec::new();
        for (record, value) in layers {
            match value.and_then(|value| admit_project(home, record, value, &agents, given)) {
                Ok((project, found)) => {
                    findings.extend(found);
                    projects.push(project);
                }
                Err(finding) => {
                    complete = false;
                    findings.push(finding);
                }
            }
        }
        Ok(Candidate {
            generation: Self {
                global,
                global_layer,
                agents,
                companies,
                projects,
                published,
            },
            findings,
            complete,
        })
    }

    /// The machine layer as admitted, before any project layer combines
    /// with it.
    pub(super) fn global_layer(&self) -> Option<&Value> {
        self.global_layer.as_ref()
    }

    /// The admitted configuration for a caller in the directory `repo_root`:
    /// the machine layer for `None`, a published project's for its checkout,
    /// an admitted company's for its home directory, none for any other.
    pub(super) fn layer(&self, repo_root: Option<&std::path::Path>) -> Option<&Configuration> {
        let Some(root) = repo_root else {
            return Some(&self.global);
        };
        let same = |covered: &std::path::Path| {
            covered == root
                || matches!(
                    (covered.canonicalize(), root.canonicalize()),
                    (Ok(a), Ok(b)) if a == b
                )
        };
        self.projects
            .iter()
            .find(|project| same(&project.checkout))
            .map(|project| &project.config)
            .or_else(|| {
                self.companies
                    .iter()
                    .find(|company| same(&company.dir))
                    .map(|company| &company.home_config)
            })
    }

    /// The machine layer.
    pub fn global(&self) -> &Configuration {
        &self.global
    }

    /// The agent profiles.
    pub fn agents(&self) -> &Agents {
        &self.agents
    }

    /// The agent profiles, shared with what is prepared in this generation.
    pub fn shared_agents(&self) -> Arc<Agents> {
        Arc::clone(&self.agents)
    }

    /// The admitted company homes.
    pub fn companies(&self) -> &[AdmittedCompany] {
        &self.companies
    }

    /// The admitted company home `name`, if it is one.
    pub fn company(&self, name: &ProjectSlug) -> Option<&AdmittedCompany> {
        self.companies.iter().find(|company| &company.name == name)
    }

    /// The published projects whose configuration was admitted.
    pub fn projects(&self) -> &[AdmittedProject] {
        &self.projects
    }

    /// Whether history extraction is on: the machine's switch while no
    /// project is published here, otherwise any admitted project's; a
    /// published project not admitted for history enables nothing.
    pub fn history_enabled(&self) -> Result<bool, ConfigError> {
        if !self.published {
            return self.global.module_enabled(Module::History);
        }
        Ok(self.projects.iter().any(|project| project.history))
    }

    /// Whether `module` is on for the machine or any considered project.
    pub fn module_anywhere(&self, module: Module) -> Result<bool, ConfigError> {
        Ok(self.global.module_enabled(module)?
            || self.projects.iter().any(|project| match module {
                Module::Todos => project.todos,
                Module::Events => project.events,
                Module::History => project.history,
                _ => false,
            }))
    }
}

/// The company home `name` as admitted, `None` when its folder holds no
/// document, or its findings. Its document and its secrets file are each
/// read once: what is judged is what is admitted.
fn admit_company(
    home: &Home,
    name: &ProjectSlug,
    agents: &Arc<Agents>,
    given: Given<'_>,
) -> Result<Option<AdmittedCompany>, Vec<Finding>> {
    let layer = Layer::Company(name.clone());
    let document = layer.path(home);
    let value = read(&document)
        .and_then(|doc| {
            doc.map(|doc| inspected(home, &layer, &document, doc.value))
                .transpose()
        })
        .map_err(|error| vec![unreadable(document.clone(), &error)])?;
    let Some(value) = value else {
        return Ok(None);
    };
    let secrets = SecretSnapshot::read(home, &SecretSource::Company(name.clone()));
    let given = Given {
        secrets: Some(&secrets),
        ..given
    };
    // Every finding the document has is reported together, the secrets it
    // references among them.
    let found = findings_given(home, &layer, &document, &value, given)
        .map_err(|error| vec![unreadable(document.clone(), &error)])?;
    if !found.is_empty() {
        return Err(found);
    }
    let config = Configuration::company_from_value(home, name, &value)
        .map_err(|error| vec![unreadable(document.clone(), &error)])?
        .pinned(agents);
    let dir = home.company_dir(name);
    let home_config = Configuration::from_layers(
        home,
        given.global.flatten().cloned(),
        Some((dir.clone(), value.clone())),
    )
    .map_err(|error| vec![unreadable(document.clone(), &error)])?
    .pinned(agents);
    let slack = match config.slack_connector() {
        Ok(Some(connector)) => {
            match (
                secrets.resolve(&connector.app_token),
                secrets.resolve(&connector.bot_token),
            ) {
                (Ok(app_token), Ok(bot_token)) => Some(AdmittedSlack {
                    connector,
                    app_token,
                    bot_token,
                }),
                (Err(error), _) | (_, Err(error)) => {
                    return Err(vec![unreadable(document, &error)]);
                }
            }
        }
        Ok(None) => None,
        Err(error) => return Err(vec![unreadable(document, &error)]),
    };
    Ok(Some(AdmittedCompany {
        name: name.clone(),
        config,
        home_config,
        dir,
        slack,
    }))
}

/// The published project of `record`, whose layer read `value` (`None`:
/// it has none), as admitted with the findings that withhold a module of
/// it, or why it is not admitted at all.
fn admit_project(
    home: &Home,
    record: &PublishedRecord,
    value: Option<Value>,
    agents: &Arc<Agents>,
    given: Given<'_>,
) -> Result<(AdmittedProject, Vec<Finding>), Finding> {
    let layer = Layer::Project(record.checkout.clone());
    let document = record.checkout.join(super::CONFIG_FILE_NAME);
    // The layer is judged as `config validate` judges it.
    let findings = match &value {
        Some(value) => findings_given(home, &layer, &document, value, given)
            .map_err(|error| unreadable(document.clone(), &error))?,
        None => Vec::new(),
    };
    let config = Configuration::from_layers(
        home,
        given.global.flatten().cloned(),
        value.map(|value| (record.checkout.clone(), value)),
    )
    .map_err(|error| unreadable(document.clone(), &error))?
    .pinned(agents);
    let on = |module| {
        config
            .module_enabled(module)
            .map_err(|error| unreadable(document.clone(), &error))
    };
    let switched = Modules {
        todos: on(Module::Todos)?,
        events: on(Module::Events)?,
        history: on(Module::History)?,
    };
    let Modules {
        todos,
        events,
        history,
    } = switched.admitted(&findings);
    Ok((
        AdmittedProject {
            slug: record.project.clone(),
            checkout: record.checkout.clone(),
            config,
            switched: switched.todos || switched.events,
            todos,
            events,
            history,
        },
        findings,
    ))
}

/// The modules a project switches on, or has admitted on this computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Modules {
    todos: bool,
    events: bool,
    history: bool,
}

impl Modules {
    /// What is admitted of these switches, given the layer's findings: a
    /// finding against a module (`modules.<module>`) withholds that module
    /// alone; a layer that does not validate for any other reason withholds
    /// history, which reads every adopted project, and nothing else.
    fn admitted(self, findings: &[Finding]) -> Self {
        let against = |module: &str| {
            let key = format!("modules.{module}");
            findings
                .iter()
                .any(|f| f.key == key || f.key.starts_with(&format!("{key}.")))
        };
        let invalid = findings.iter().any(|f| !f.key.starts_with("modules."));
        Self {
            todos: self.todos && !against("todos"),
            events: self.events && !against("events"),
            history: self.history && !against("history") && !invalid,
        }
    }
}

fn unreadable(document: PathBuf, error: &ConfigError) -> Finding {
    Finding {
        document,
        key: String::new(),
        kind: FindingKind::InvalidValue,
        message: error.to_string(),
    }
}

/// The generation every consumer of one runner takes. Replaced only whole.
#[derive(Debug, Clone)]
pub struct Current(Swap<Admitted>);

/// What offering a candidate to [`Current`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// The candidate is now the current generation.
    Replaced,
    /// The candidate was incomplete; the current generation stays in force.
    Kept,
}

impl Current {
    pub fn new(generation: Admitted) -> Self {
        Self(Swap::new(generation))
    }

    /// The generation in force now.
    pub fn get(&self) -> Arc<Admitted> {
        self.0.get()
    }

    /// Replaces the current generation with `candidate` when it is complete;
    /// leaves it otherwise.
    pub fn offer(&self, candidate: Candidate) -> Offer {
        self.0.offer(candidate.generation, candidate.complete)
    }
}

/// The generation a runner serves in its process, for its home.
static SERVED: RwLock<Option<(PathBuf, Current)>> = RwLock::new(None);

/// Serves `current` to every configuration and profile loader in this
/// process for `home`: from now on they answer its admitted values instead
/// of reading the files. A runner calls this once its first generation is
/// admitted.
pub fn serve(home: &Home, current: &Current) {
    let entry = Some((home.root().to_path_buf(), current.clone()));
    match SERVED.write() {
        Ok(mut served) => *served = entry,
        Err(poisoned) => *poisoned.into_inner() = entry,
    }
}

/// The generation served for `home` in this process, if any.
pub(super) fn served(home: &Home) -> Option<Arc<Admitted>> {
    let served = match SERVED.read() {
        Ok(served) => served,
        Err(poisoned) => poisoned.into_inner(),
    };
    served
        .as_ref()
        .filter(|(root, _)| root == home.root())
        .map(|(_, current)| current.get())
}

/// A value shared by every holder, replaced only whole and only by a
/// complete successor.
#[derive(Debug)]
struct Swap<T>(Arc<RwLock<Arc<T>>>);

impl<T> Clone for Swap<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> Swap<T> {
    fn new(value: T) -> Self {
        Self(Arc::new(RwLock::new(Arc::new(value))))
    }

    fn get(&self) -> Arc<T> {
        match self.0.read() {
            Ok(current) => Arc::clone(&current),
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        }
    }

    fn offer(&self, next: T, complete: bool) -> Offer {
        if !complete {
            return Offer::Kept;
        }
        let next = Arc::new(next);
        match self.0.write() {
            Ok(mut current) => *current = next,
            Err(poisoned) => *poisoned.into_inner() = next,
        }
        Offer::Replaced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(key: &str) -> Finding {
        Finding {
            document: PathBuf::from("instrukt.yaml"),
            key: key.into(),
            kind: FindingKind::InvalidValue,
            message: String::new(),
        }
    }

    #[test]
    fn a_finding_withholds_its_module_and_an_invalid_layer_withholds_history() {
        let switched = Modules {
            todos: true,
            events: true,
            history: true,
        };
        assert_eq!(switched.admitted(&[]), switched);
        // The event transport's finding withholds event responses alone.
        assert_eq!(
            switched.admitted(&[finding("modules.events")]),
            Modules {
                todos: true,
                events: false,
                history: true,
            }
        );
        // A layer that does not validate otherwise withholds history alone.
        assert_eq!(
            switched.admitted(&[finding("board")]),
            Modules {
                todos: true,
                events: true,
                history: false,
            }
        );
        assert_eq!(
            switched.admitted(&[finding("modules.todos")]),
            Modules {
                todos: false,
                events: true,
                history: true,
            }
        );
    }

    /// A generation with nothing in it but whether a project is published.
    fn generation(published: bool) -> Admitted {
        let home = Home::at_user_home(PathBuf::from("/nonexistent"));
        let agents = Arc::new(Agents::from_documents(&home, None, None).unwrap());
        Admitted {
            global: Configuration::from_layers(&home, None, None).unwrap(),
            global_layer: None,
            agents,
            companies: Vec::new(),
            projects: Vec::new(),
            published,
        }
    }

    #[test]
    fn an_incomplete_candidate_leaves_the_current_generation_for_every_holder() {
        let current = Current::new(generation(false));
        let holder = current.clone();
        let incomplete = Candidate {
            generation: generation(true),
            findings: Vec::new(),
            complete: false,
        };
        assert_eq!(current.offer(incomplete), Offer::Kept);
        assert!(!holder.get().published);
        let complete = Candidate {
            generation: generation(true),
            findings: Vec::new(),
            complete: true,
        };
        assert_eq!(current.offer(complete), Offer::Replaced);
        assert!(holder.get().published);
    }

    #[test]
    fn the_machines_history_switch_counts_only_while_no_project_is_published() {
        let home = Home::at_user_home(PathBuf::from("/nonexistent"));
        let machine: Value = serde_yaml_ng::from_str(&format!(
            "schema_version: {}\nmodules: {{history: true}}\n",
            super::super::CURRENT_SCHEMA_VERSION
        ))
        .unwrap();
        let with_switch = |published| Admitted {
            global: Configuration::from_layers(&home, Some(machine.clone()), None).unwrap(),
            global_layer: Some(machine.clone()),
            ..generation(published)
        };
        assert!(with_switch(false).history_enabled().unwrap());
        // A published project left out of the generation enables nothing.
        assert!(!with_switch(true).history_enabled().unwrap());
    }

    /// A directory of its own for one test, emptied.
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("instrukt-admitted-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_machine_layer_admitted_is_the_one_validated_though_its_file_changes_meanwhile() {
        use std::io::Write as _;
        let dir = scratch("read-once");
        let home = Home::at_user_home(dir.clone()).for_inspection();
        let machine = home.global_config();
        std::fs::create_dir_all(machine.parent().unwrap()).unwrap();
        let schema = super::super::CURRENT_SCHEMA_VERSION;
        std::fs::write(
            &machine,
            format!("schema_version: {schema}\nrunner: {{runtime: t3}}\n"),
        )
        .unwrap();
        // agents.yaml is a FIFO: admission, having read the machine layer,
        // waits on it until the writer below has replaced the machine layer
        // with an invalid one.
        let agents = home.agents_file();
        let made = std::process::Command::new("mkfifo")
            .arg(&agents)
            .status()
            .unwrap();
        assert!(made.success());
        let writer = {
            let (machine, agents) = (machine.clone(), agents.clone());
            std::thread::spawn(move || {
                let mut fifo = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&agents)
                    .unwrap();
                let next = machine.with_extension("next");
                std::fs::write(
                    &next,
                    format!("schema_version: {schema}\nrunner: {{runtime: unknown-runtime}}\n"),
                )
                .unwrap();
                std::fs::rename(&next, &machine).unwrap();
                fifo.write_all(super::super::SHIPPED_AGENTS.as_bytes())
                    .unwrap();
            })
        };
        let candidate = Admitted::load(&home).unwrap();
        writer.join().unwrap();
        assert!(candidate.complete);
        assert!(candidate.findings.is_empty());
        assert_eq!(
            candidate.generation.global().runner_runtime().unwrap(),
            super::super::Runtime::COMPILED
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_company_home_directory_is_served_its_admitted_configuration() {
        let dir = scratch("company-home");
        let home = Home::at_user_home(dir.clone());
        let company = ProjectSlug::parse("acme").unwrap();
        let company_dir = home.company_dir(&company);
        std::fs::create_dir_all(&company_dir).unwrap();
        let document = company_dir.join(super::super::CONFIG_FILE_NAME);
        std::fs::write(
            &document,
            format!(
                "schema_version: {}\ncompany: acme\nprofiles: [antigravity]\n",
                super::super::CURRENT_SCHEMA_VERSION
            ),
        )
        .unwrap();
        let candidate = Admitted::load(&home).unwrap();
        assert!(candidate.complete);
        let current = Current::new(candidate.generation);
        serve(&home, &current);
        // The company's document becomes unparseable after admission: a
        // caller in its home directory, as a start prepared there without
        // profiles, is answered from the admitted generation.
        std::fs::write(&document, "schema_version: [\n").unwrap();
        let served = Configuration::load(&home, Some(&company_dir));
        assert!(served.is_ok_and(|config| config.agents().is_ok()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
