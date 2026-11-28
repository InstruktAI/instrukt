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

use super::_connectors::DeclaredConnector;
use super::_error::{ConfigError, Finding, FindingKind};
use super::_layer::{Layer, inspected, read, read_layer};
use super::_module::Module;
use super::_secrets::SecretSnapshot;
use super::_validate::{Given, findings_given};
use super::{Agents, Configuration, SecretSource};
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
    /// The retention window admitted: `None` while it is off or withheld.
    thread_retention: Option<jiff::SignedDuration>,
    /// Whether global `backup` findings withheld home backup admission.
    backup_withheld: bool,
    /// Whether global `llm` findings withheld screening admission.
    llm_withheld: bool,
}

/// A company home admitted with its generation.
#[derive(Debug, Clone)]
pub struct AdmittedCompany {
    pub name: ProjectSlug,
    pub config: Configuration,
    /// The configuration of a caller in the company's home directory: the
    /// machine layer with the home's own project layer, if it has one, as
    /// admitted. The company's settings are never part of it: they are
    /// read as [`AdmittedCompany::config`].
    pub home_config: Configuration,
    /// The company's home directory.
    pub dir: PathBuf,
    /// The connectors it declared, as its document declares them.
    pub connectors: Vec<DeclaredConnector>,
    /// Each `${NAME}` reference its connectors make, resolved from its own
    /// secrets file by the read admission validated.
    pub secrets: AdmittedSecrets,
}

/// The secrets a company's declared connectors reference, by `${NAME}`
/// reference: present and nonblank when its generation was admitted.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct AdmittedSecrets(std::collections::BTreeMap<String, String>);

impl AdmittedSecrets {
    /// The value admitted for `reference`, if the company's connectors make
    /// that reference.
    pub fn resolve(&self, reference: &str) -> Option<&str> {
        self.0.get(reference).map(String::as_str)
    }
}

impl std::fmt::Debug for AdmittedSecrets {
    /// The references, never their values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
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
    /// Automatic review switched on with event responses admitted and no
    /// finding at `review.automatic` that needs no GitHub. Whether the two
    /// identities are ready is asked where review is done.
    pub review: bool,
    /// Worktree reclamation of the declared regenerable output admitted: no
    /// finding stands at `regenerable`.
    pub reclamation: bool,
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
    /// machine layer or `agents.yaml` refuses it: nothing can run on them;
    /// a machine layer whose only findings withhold thread retention or home
    /// backup is admitted with those capabilities withheld and its findings
    /// reported. A company home that does not validate, or a project whose
    /// layer cannot be read, is left out with its findings and makes the
    /// candidate incomplete; a finding against a project's module withholds
    /// that module.
    pub fn load(home: &Home) -> Result<Candidate, AdmissionError> {
        // Admission reads and never writes: older documents are converted in
        // memory, and the profiles are the shipped ones when there is no file.
        let global_path = home.global_config();
        let raw_global = read(&global_path)?.map(|doc| doc.value);
        let global_layer = raw_global
            .clone()
            .map(|raw| inspected(&Layer::Global, &global_path, raw))
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
        let agents = Arc::new(Agents::from_documents(agents_bytes, raw_global.as_ref())?);
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
                    doc.map(|doc| inspected(&layer, &document, doc.value))
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
        let mut findings = Vec::new();
        let mut thread_retention = super::_thread_retention::admitted(global_layer.as_ref());
        let mut backup_withheld = false;
        let mut llm_withheld = false;
        if let Some(value) = &global_layer {
            let found = findings_given(home, &Layer::Global, &global_path, value, given)?;
            match admit_machine_layer(found) {
                MachineLayer::Admitted => {}
                MachineLayer::Withheld {
                    findings: found,
                    thread_retention: withhold_retention,
                    backup: withhold_backup,
                    llm: withhold_llm,
                } => {
                    if withhold_retention {
                        thread_retention = None;
                    }
                    backup_withheld = withhold_backup;
                    llm_withheld = withhold_llm;
                    findings.extend(found);
                }
                MachineLayer::Refused(found) => {
                    return Err(ConfigError::Invalid(super::Findings(found)).into());
                }
            }
        }
        let global = Configuration::from_layers(home, global_layer.clone(), None)?.pinned(&agents);
        let mut complete = true;
        let mut companies = Vec::new();
        for (name, unreadable_folder) in super::company_folders(home)? {
            let admitted = match unreadable_folder {
                Some(error) => Err(vec![unreadable(home.company_dir(&name), &error)]),
                None => admit_company(home, &name, &agents, given),
            };
            match admitted {
                Ok(Some(mut company)) => {
                    let mut retained = Vec::with_capacity(company.connectors.len());
                    for connector in company.connectors {
                        if let Some(finding) = super::_validate::connector_role_procedure_finding(
                            home,
                            &name,
                            &connector.key,
                            connector.role.as_str(),
                            Some(&agents),
                        ) {
                            findings.push(finding);
                        } else {
                            retained.push(connector);
                        }
                    }
                    company.connectors = retained;
                    companies.push(company);
                }
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
                thread_retention,
                backup_withheld,
                llm_withheld,
            },
            findings,
            complete,
        })
    }

    /// Whether global `backup` findings withheld home backup admission in this
    /// generation while leaving the rest of the global layer admitted.
    pub fn backup_withheld(&self) -> bool {
        self.backup_withheld
    }

    /// Whether global `llm` findings withheld screening admission in this
    /// generation while leaving the rest of the global layer admitted.
    pub fn llm_withheld(&self) -> bool {
        self.llm_withheld
    }

    /// The computer's retention window as admitted: `None` while it is off,
    /// or withheld because the selected runtime cannot archive threads. The
    /// one read of the window for consumers.
    pub fn thread_retention(&self) -> Option<jiff::SignedDuration> {
        self.thread_retention
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

/// What the machine layer's findings admit.
#[derive(Debug, PartialEq)]
enum MachineLayer {
    /// No finding: the layer is admitted whole.
    Admitted,
    /// Only findings that withhold thread retention, home backup, and/or LLM
    /// screening: the layer is admitted with those capabilities withheld, and
    /// these are reported.
    Withheld {
        findings: Vec<Finding>,
        thread_retention: bool,
        backup: bool,
        llm: bool,
    },
    /// Any other finding: nothing can run on the layer.
    Refused(Vec<Finding>),
}

/// The admission of a machine layer with findings `found`
/// (`project/instrukt/spec/feature/configuration/layered-configuration`).
fn admit_machine_layer(found: Vec<Finding>) -> MachineLayer {
    if found.is_empty() {
        MachineLayer::Admitted
    } else if found.iter().all(|f| {
        super::withholds_thread_retention(f)
            || super::withholds_backup(f)
            || super::withholds_llm(f)
    }) {
        let thread_retention = found.iter().any(super::withholds_thread_retention);
        let backup = found.iter().any(super::withholds_backup);
        let llm = found.iter().any(super::withholds_llm);
        MachineLayer::Withheld {
            findings: found,
            thread_retention,
            backup,
            llm,
        }
    } else {
        MachineLayer::Refused(found)
    }
}

/// The company home `name` as admitted, `None` when its folder holds no
/// document, or its findings. Its document is read where it stands, a
/// legacy one unconverted, and both present is a finding naming both.
/// Deciding which file holds the settings reads a legacy document before
/// it is taken; the settings judged are those of one read of the chosen
/// file, and they are what is admitted. The secrets file is read once, and
/// that read is what validation and the admitted connectors resolve from.
fn admit_company(
    home: &Home,
    name: &ProjectSlug,
    agents: &Arc<Agents>,
    given: Given<'_>,
) -> Result<Option<AdmittedCompany>, Vec<Finding>> {
    let layer = Layer::Company(name.clone());
    let document = layer.located(home);
    let value = read_layer(home, &layer)
        .and_then(|found| {
            found
                .map(|(path, doc)| inspected(&layer, &path, doc.value))
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
    let project_layer = Layer::Project(dir.clone());
    let project = read_layer(home, &project_layer)
        .and_then(|found| {
            found
                .map(|(path, doc)| inspected(&project_layer, &path, doc.value))
                .transpose()
        })
        .map_err(|error| vec![unreadable(project_layer.path(home), &error)])?;
    let home_config = Configuration::from_layers(
        home,
        given.global.flatten().cloned(),
        project.map(|value| (dir.clone(), value)),
    )
    .map_err(|error| vec![unreadable(project_layer.path(home), &error)])?
    .pinned(agents);
    // Each connector as declared, and each secret reference its entry
    // makes, resolved from the same read of the secrets file validation used.
    let connectors = config
        .connectors()
        .map_err(|error| vec![unreadable(document.clone(), &error)])?;
    let mut resolved = std::collections::BTreeMap::new();
    for declared in &connectors {
        for field in declared
            .entry
            .as_mapping()
            .into_iter()
            .flat_map(|e| e.values())
        {
            if let Some(reference) = field.as_str().filter(|t| super::is_secret_reference(t)) {
                let secret = secrets
                    .resolve(reference)
                    .map_err(|error| vec![unreadable(document.clone(), &error)])?;
                resolved.insert(reference.to_owned(), secret);
            }
        }
    }
    Ok(Some(AdmittedCompany {
        name: name.clone(),
        config,
        home_config,
        dir,
        connectors,
        secrets: AdmittedSecrets(resolved),
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
    let review = events
        && config
            .review_automatic()
            .map_err(|error| unreadable(document.clone(), &error))?
        && !findings
            .iter()
            .any(|f| f.key == super::_review::REVIEW_AUTOMATIC_KEY);
    let reclamation = !findings
        .iter()
        .any(super::_regenerable::withholds_reclamation);
    Ok((
        AdmittedProject {
            slug: record.project.clone(),
            checkout: record.checkout.clone(),
            config,
            switched: switched.todos || switched.events,
            todos,
            events,
            history,
            review,
            reclamation,
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
    /// alone; a finding at `review.*` withholds automatic review alone, and
    /// one at `regenerable` withholds worktree reclamation alone; a layer
    /// that does not validate for any other reason withholds history, which
    /// reads every adopted project, and nothing else.
    fn admitted(self, findings: &[Finding]) -> Self {
        let against = |module: &str| {
            let key = format!("modules.{module}");
            findings
                .iter()
                .any(|f| f.key == key || f.key.starts_with(&format!("{key}.")))
        };
        let invalid = findings.iter().any(|f| {
            !f.key.starts_with("modules.")
                && !f.key.starts_with("review.")
                && !super::_regenerable::withholds_reclamation(f)
        });
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
        // A finding at automatic review withholds no module.
        assert_eq!(switched.admitted(&[finding("review.automatic")]), switched);
        assert_eq!(
            switched.admitted(&[finding("modules.todos")]),
            Modules {
                todos: false,
                events: true,
                history: true,
            }
        );
    }

    #[test]
    fn an_old_shape_company_layer_loads_with_its_event_responses_withheld() {
        // A company layer of the shape written before the company identity:
        // schema 6, a company repository, no `company_name`.
        let old: Value = serde_yaml_ng::from_str(&format!(
            "schema_version: {}\nproject: company-acme\ncompany_repository: true\nmodules: {{todos: false, events: true}}\nreview: {{automatic: true}}\n",
            super::super::PROJECT_SCHEMA_VERSION
        ))
        .unwrap();
        let findings: Vec<Finding> =
            super::super::_validate::judge(&old, super::super::LayerKind::Project, None, None)
                .into_iter()
                .filter(|f| f.kind != FindingKind::EventsNeedTransport)
                .map(|f| Finding {
                    document: PathBuf::from("instrukt.yaml"),
                    key: f.key,
                    kind: f.kind,
                    message: f.message,
                })
                .collect();
        assert_eq!(
            findings
                .iter()
                .map(|f| (f.key.as_str(), f.kind))
                .collect::<Vec<_>>(),
            vec![("modules.events", FindingKind::NoCompanyIdentity)]
        );
        let switched = Modules {
            todos: false,
            events: true,
            history: true,
        };
        assert_eq!(
            switched.admitted(&findings),
            Modules {
                todos: false,
                events: false,
                history: true,
            }
        );
    }

    /// A finding that the selected runtime cannot carry the retention window.
    fn retention_capability() -> Finding {
        Finding {
            kind: FindingKind::RuntimeCapabilityMissing,
            ..finding("runner.archive_finished_after")
        }
    }

    #[test]
    fn a_machine_layer_without_findings_is_admitted() {
        assert_eq!(admit_machine_layer(Vec::new()), MachineLayer::Admitted);
    }

    #[test]
    fn a_machine_layer_whose_only_finding_withholds_retention_is_admitted_with_it_withheld() {
        assert_eq!(
            admit_machine_layer(vec![retention_capability()]),
            MachineLayer::Withheld {
                findings: vec![retention_capability()],
                thread_retention: true,
                backup: false,
                llm: false,
            }
        );
    }

    #[test]
    fn a_machine_layer_with_backup_findings_only_withholds_backup_alone() {
        let backup_finding = finding("backup.endpoint");
        assert_eq!(
            admit_machine_layer(vec![backup_finding.clone()]),
            MachineLayer::Withheld {
                findings: vec![backup_finding.clone()],
                thread_retention: false,
                backup: true,
                llm: false,
            }
        );
        assert_eq!(
            admit_machine_layer(vec![retention_capability(), backup_finding.clone()]),
            MachineLayer::Withheld {
                findings: vec![retention_capability(), backup_finding],
                thread_retention: true,
                backup: true,
                llm: false,
            }
        );
    }

    #[test]
    fn a_machine_layer_with_llm_findings_only_withholds_llm_alone() {
        let llm_finding = finding("llm.screening.url");
        assert_eq!(
            admit_machine_layer(vec![llm_finding.clone()]),
            MachineLayer::Withheld {
                findings: vec![llm_finding.clone()],
                thread_retention: false,
                backup: false,
                llm: true,
            }
        );
        let other_finding = finding("ticks");
        assert_eq!(
            admit_machine_layer(vec![llm_finding.clone(), other_finding.clone()]),
            MachineLayer::Refused(vec![llm_finding, other_finding])
        );
    }

    #[test]
    fn a_machine_layer_with_any_other_finding_beside_it_is_refused() {
        let shape = finding("runner.archive_finished_after");
        assert_eq!(
            admit_machine_layer(vec![retention_capability(), shape.clone()]),
            MachineLayer::Refused(vec![retention_capability(), shape])
        );
    }

    #[test]
    fn a_finding_at_the_regenerable_declaration_withholds_no_module() {
        let switched = Modules {
            todos: true,
            events: true,
            history: true,
        };
        assert_eq!(
            switched.admitted(&[finding(super::super::REGENERABLE_KEY)]),
            switched
        );
    }

    /// A generation with nothing in it but whether a project is published.
    fn generation(published: bool) -> Admitted {
        let home = Home::at_user_home(PathBuf::from("/nonexistent"));
        let agents = Arc::new(Agents::from_documents(None, None).unwrap());
        Admitted {
            global: Configuration::from_layers(&home, None, None).unwrap(),
            global_layer: None,
            agents,
            companies: Vec::new(),
            projects: Vec::new(),
            published,
            thread_retention: None,
            backup_withheld: false,
            llm_withheld: false,
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
            super::super::GLOBAL_SCHEMA_VERSION
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
        let schema = super::super::GLOBAL_SCHEMA_VERSION;
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
    fn a_home_made_before_company_yaml_is_admitted_with_its_settings_unconverted() {
        let dir = scratch("legacy-company");
        let home = Home::at_user_home(dir.clone());
        let company = ProjectSlug::parse("acme").unwrap();
        let company_dir = home.company_dir(&company);
        std::fs::create_dir_all(&company_dir).unwrap();
        let legacy = format!(
            "schema_version: {}\ncompany: acme\nprofiles: [antigravity]\n",
            super::super::COMPANY_SCHEMA_VERSION
        );
        std::fs::write(company_dir.join(super::super::CONFIG_FILE_NAME), &legacy).unwrap();
        let candidate = Admitted::load(&home).unwrap();
        assert!(candidate.complete, "{:?}", candidate.findings);
        let admitted = candidate.generation.company(&company).unwrap();
        assert_eq!(
            admitted
                .config
                .get(Some("profiles"), &super::super::Env::default())
                .unwrap(),
            serde_yaml_ng::from_str::<Value>("[antigravity]").unwrap()
        );
        // The company's settings are no layer of a caller in its home.
        assert!(
            admitted
                .home_config
                .get(Some("profiles"), &super::super::Env::default())
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(company_dir.join(super::super::CONFIG_FILE_NAME)).unwrap(),
            legacy
        );
        assert!(!company_dir.join(super::super::COMPANY_DOCUMENT).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_home_holding_both_settings_documents_is_a_finding_naming_both() {
        let dir = scratch("both-company");
        let home = Home::at_user_home(dir.clone());
        let company = ProjectSlug::parse("acme").unwrap();
        let company_dir = home.company_dir(&company);
        std::fs::create_dir_all(&company_dir).unwrap();
        let document = format!(
            "schema_version: {}\ncompany: acme\n",
            super::super::COMPANY_SCHEMA_VERSION
        );
        std::fs::write(company_dir.join(super::super::CONFIG_FILE_NAME), &document).unwrap();
        std::fs::write(company_dir.join(super::super::COMPANY_DOCUMENT), &document).unwrap();
        let candidate = Admitted::load(&home).unwrap();
        assert!(!candidate.complete);
        assert!(candidate.generation.company(&company).is_none());
        let finding = candidate
            .findings
            .iter()
            .find(|finding| finding.document == company_dir.join(super::super::COMPANY_DOCUMENT))
            .unwrap_or_else(|| panic!("{:?}", candidate.findings));
        insta::assert_snapshot!(
            "both_settings_documents",
            finding
                .message
                .replace(&company_dir.display().to_string(), "[HOME]")
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
                super::super::COMPANY_SCHEMA_VERSION
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
