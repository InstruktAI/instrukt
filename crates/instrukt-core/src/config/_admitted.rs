//! The admitted configuration generation: what a long-lived runner uses in
//! place of reading files. The machine layer, the agent profiles, each
//! company home with its connector secrets resolved, and each published
//! project's layer are loaded and validated together. A runner builds one
//! when it starts and again on every wake; a complete valid one replaces the
//! current generation for every consumer at once, and an invalid one leaves
//! the current generation in force while its findings are reported. A hand
//! edit made while the runner runs is therefore used whole or not at all.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use super::_error::{ConfigError, Finding, FindingKind};
use super::_layer::{Layer, read_usable};
use super::_module::Module;
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
    agents: Arc<Agents>,
    companies: Vec<AdmittedCompany>,
    projects: Vec<AdmittedProject>,
}

/// A company home admitted with its generation.
#[derive(Debug, Clone)]
pub struct AdmittedCompany {
    pub name: ProjectSlug,
    pub config: Configuration,
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
    pub todos: bool,
    /// Event responses switched on and admitted: never true while the
    /// project's effective configuration lacks the event transport.
    pub events: bool,
    /// History switched on for the project.
    pub history: bool,
}

/// A generation as built from the files, with what was not admitted.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub generation: Admitted,
    /// Why a company, a project or a project's events were not admitted.
    pub findings: Vec<Finding>,
    /// Every company home was admitted. Only a complete candidate replaces a
    /// running generation; a project's own findings never block one.
    pub complete: bool,
}

impl Admitted {
    /// Builds a generation from the files. An invalid machine layer or
    /// `agents.yaml` refuses it: nothing can run on them. A company home or a
    /// project that does not validate is left out with its findings.
    pub fn load(home: &Home) -> Result<Candidate, AdmissionError> {
        let global_layer = Layer::Global;
        let global_value = read_usable(home, &global_layer)?;
        if let Some(value) = &global_value {
            let found = super::_validate::findings_for(
                home,
                &global_layer,
                &global_layer.path(home),
                value,
            )?;
            if !found.is_empty() {
                return Err(ConfigError::Invalid(super::Findings(found)).into());
            }
        }
        // Admission reads and never writes: the profiles as they are, or the
        // shipped ones when there is no file, without deploying them.
        let agents = Arc::new(Agents::load(&home.for_inspection())?);
        let global = Configuration::load_global(home)?.pinned(&agents);
        let mut findings = Vec::new();
        let mut complete = true;
        let mut companies = Vec::new();
        for (name, loaded) in super::company_homes(home)? {
            match admit_company(home, &name, loaded, &agents) {
                Ok(Some(company)) => companies.push(company),
                Ok(None) => {}
                Err(found) => {
                    complete = false;
                    findings.extend(found);
                }
            }
        }
        let mut projects = Vec::new();
        for record in PublishedRecord::load_all(home)? {
            if !record.checkout.is_dir() {
                tracing::warn!(event = "project_checkout_missing", project = %record.project, checkout = %record.checkout.display());
                continue;
            }
            match admit_project(home, &record, &agents) {
                Ok((project, found)) => {
                    findings.extend(found);
                    projects.push(project);
                }
                Err(finding) => findings.push(finding),
            }
        }
        Ok(Candidate {
            generation: Self {
                global,
                agents,
                companies,
                projects,
            },
            findings,
            complete,
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
    /// project is published here, otherwise any admitted project's.
    pub fn history_enabled(&self) -> Result<bool, ConfigError> {
        if self.projects.is_empty() {
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

/// The company home `name` as admitted, `None` when its folder is not a
/// company, or its findings.
fn admit_company(
    home: &Home,
    name: &ProjectSlug,
    loaded: Result<Configuration, ConfigError>,
    agents: &Arc<Agents>,
) -> Result<Option<AdmittedCompany>, Vec<Finding>> {
    let layer = Layer::Company(name.clone());
    let document = layer.path(home);
    // The document is judged whenever it can be read, so every finding it
    // has is reported together, the secrets it references among them.
    let value = match read_usable(home, &layer) {
        Ok(Some(value)) => value,
        Ok(None) => return Ok(None),
        Err(error) => return Err(vec![unreadable(document, &error)]),
    };
    let found = super::_validate::findings_for(home, &layer, &document, &value)
        .map_err(|error| vec![unreadable(document.clone(), &error)])?;
    if !found.is_empty() {
        return Err(found);
    }
    let config = match loaded {
        Ok(config) => config.pinned(agents),
        Err(ConfigError::NoLayer { .. }) => return Ok(None),
        Err(error) => return Err(vec![unreadable(document, &error)]),
    };
    let slack = match config.slack_connector() {
        Ok(Some(connector)) => {
            let source = SecretSource::Company(name.clone());
            let resolve = |reference: &str| super::resolve_secret_from(home, &source, reference);
            match (resolve(&connector.app_token), resolve(&connector.bot_token)) {
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
        slack,
    }))
}

/// The published project of `record` as admitted, with the findings that
/// left something out of it, or why it is not admitted at all.
fn admit_project(
    home: &Home,
    record: &PublishedRecord,
    agents: &Arc<Agents>,
) -> Result<(AdmittedProject, Vec<Finding>), Finding> {
    let document = record.checkout.join(super::CONFIG_FILE_NAME);
    let config = Configuration::load(home, Some(&record.checkout))
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
    let transport = config.cluster_redis_url().ok().flatten().is_some();
    let Modules {
        todos,
        events,
        history,
    } = switched.admitted(transport);
    let mut findings = Vec::new();
    if switched.events && !events {
        findings.push(Finding {
            document,
            key: "modules.events".into(),
            kind: FindingKind::EventsNeedTransport,
            message: format!(
                "event responses of project `{}` are not admitted on this computer: the event transport has no `redis.cluster.url`; set it in the global layer (`instrukt config patch --global`)",
                record.project
            ),
        });
    }
    Ok((
        AdmittedProject {
            slug: record.project.clone(),
            checkout: record.checkout.clone(),
            config,
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
    /// What is admitted of these switches: event responses only with the
    /// event transport they are carried on; todos and history as switched.
    fn admitted(self, transport: bool) -> Self {
        Self {
            events: self.events && transport,
            ..self
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

    #[test]
    fn event_responses_need_the_transport_and_todos_and_history_do_not() {
        let switched = Modules {
            todos: true,
            events: true,
            history: true,
        };
        assert_eq!(
            switched.admitted(false),
            Modules {
                todos: true,
                events: false,
                history: true,
            }
        );
        assert_eq!(switched.admitted(true), switched);
    }

    #[test]
    fn an_incomplete_candidate_leaves_the_current_generation_for_every_holder() {
        let current = Swap::new(1);
        let holder = current.clone();
        assert_eq!(current.offer(2, false), Offer::Kept);
        assert_eq!(*holder.get(), 1);
        assert_eq!(current.offer(3, true), Offer::Replaced);
        assert_eq!(*holder.get(), 3);
    }
}
