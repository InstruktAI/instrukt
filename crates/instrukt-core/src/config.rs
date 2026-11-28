//! The configuration context: `instrukt.yaml` in a global and a project
//! layer (`docs/project/spec/domain/configuration.md`).
//!
//! One capability: answer, validate, migrate and safely change layered
//! configuration. Readers take a [`Configuration`] (the effective
//! configuration, or the global layer alone) and read keys by dotted path
//! with interpolation, or through typed accessors. Writers address one
//! [`Layer`]: [`init`], [`migrate`], [`patch`] against a [`revision`], and
//! [`validate`].

mod _admitted;
mod _agents;
mod _agents_file;
mod _agents_migrate;
mod _backup;
mod _company;
mod _company_document;
mod _connectors;
mod _edit;
mod _effective;
mod _error;
mod _features;
mod _generation;
mod _github_org;
mod _identity;
mod _interpolate;
mod _layer;
mod _mcp;
mod _merge;
mod _migrate;
mod _module;
mod _redis;
mod _regenerable;
mod _render;
mod _review;
mod _root;
mod _runtime;
mod _schema;
mod _secrets;
mod _thread_retention;
mod _validate;
mod _webhooks;

use std::path::Path;

pub use _agents::{
    AgentSelection, Agents, COMPANY_AGENT_ROLE, Effort, HarnessKind, Profile,
    RETIRED_ORCHESTRATOR_ROLES, RoleSelection, SHIPPED_AGENTS, TODO_ORCHESTRATOR_ROLE,
    current_role,
};
pub use _agents_file::{agents_revision, agents_value, patch_agents};

pub use _admitted::{
    AdmissionError, Admitted, AdmittedCompany, AdmittedProject, AdmittedSecrets, Candidate,
    Current, Offer, serve,
};
pub use _backup::{
    BACKUP_KEY, BackupAdmission, BackupConfig, BackupTime, DEFAULT_BACKUP_KEEP,
    DEFAULT_BACKUP_TIME, SecretValue, inspect_global_backup, withholds_backup,
};
pub use _company::{Member, Members};
pub use _company_document::{COMPANY_DOCUMENT, convert_company_document, write_company_settings};
pub use _connectors::{
    ConnectorAdmit, DeclaredConnector, ReplyMode, SessionScope, connector_declaration,
    install_connector_declarations,
};
pub use _effective::{ApprovalGate, Approver, BoardRef, Configuration, Notify, Tier, WatchedRef};
pub use _error::{ConfigError, Finding, FindingKind, Findings};
pub use _features::{Feature, Unavailable, availability as feature_availability};
pub use _generation::{
    CreationFlag, Generation, GenerationFacts, GenerationRefusal, GenerationRequest, admit_home,
    company_repository, decide as decide_generation, skeleton_layer, write_skeleton,
};
pub use _github_org::{GITHUB_ORG_KEY, admit_github_org};
pub use _identity::{
    COMPANY_NAME_KEY, CompanyIdentity, CompanyName, CompanyNameError, IdentityFinding,
    company_identity,
};
pub use _layer::{
    CONFIG_FILE_NAME, InitOutcome, Layer, LayerInspection, MigrationOutcome, Revision,
    bring_current, converted_project_layer, document_lock_path, init, inspect_layer, migrate,
    parse_patch, patch, review_switch, review_target, revision, unloadable_machine_layer,
};
pub use _mcp::{
    CompanyIssuerConfig, DEFAULT_ISSUER_SIGNING_KEY_NAME, McpClient, McpServer, McpSignIn,
    SignInClient, SignInIdentity, SignInMode, SpeakerLogin,
    admitted_address as mcp_address_admitted,
};
pub use _module::Module;
pub use _redis::{REDIS_BLOCK_SECONDS, RedisEndpoint, RedisSettings};
pub use _regenerable::{EntryRefusal, REGENERABLE_KEY, RegenerablePath, withholds_reclamation};
pub use _render::render_yaml;
pub use _review::{REVIEW_AUTOMATIC_KEY, REVIEW_NEEDED_EVENT, REVIEW_PROCEDURE, REVIEW_ROLE};
pub use _root::{RootDocument, classify_root};
pub use _runtime::WorkflowCapability;
pub use _runtime::{Env, NativeCapabilities, Runtime};
pub use _schema::{
    COMPANY_SCHEMA_VERSION, GLOBAL_SCHEMA_VERSION, LayerKind, PROJECT_SCHEMA_VERSION,
    SHIPPED_BLUEPRINT_ENGINES, describe_schema, editor_schema, layer_json_schema,
};
pub use _thread_retention::withholds_thread_retention;
pub use _validate::{
    connector_procedure_finding, connector_role_procedure_finding, validate, validate_local,
};
pub use _webhooks::{
    Ingress, SigningSecrets, WebhookProtocol, WebhookSource, WebhookSources, ingress_url_admitted,
    webhook_domain, webhook_name,
};

use crate::home::Home;
use crate::ids::ProjectSlug;

pub use _secrets::{
    SecretSource, generate_secret, generate_secret_in, resolve_secret, resolve_secret_from,
    set_secret, set_secret_in,
};

/// The project slug the repository rooted at `repo_root` declares in its
/// project layer; `None` when it has no project layer or declares none.
///
/// Docs, publication, repository and specs read the slug through this
/// signature.
pub fn project_slug(repo_root: &Path) -> Result<Option<ProjectSlug>, ConfigError> {
    let layer = Layer::Project(repo_root.to_path_buf());
    let home = Home::at_user_home(repo_root.into()).for_inspection();
    let path = repo_root.join(CONFIG_FILE_NAME);
    let Some(value) = _layer::inspect_layer(&home, &layer)?.value else {
        return Ok(None);
    };
    match value.get("project") {
        None | Some(serde_yaml_ng::Value::Null) => Ok(None),
        Some(raw) => {
            let text = raw.as_str().unwrap_or_default();
            ProjectSlug::parse(text)
                .map(Some)
                .map_err(|source| ConfigError::Slug { path, source })
        }
    }
}

/// Whether `module` is switched on in the effective configuration: the
/// project layer of `repo_root` (when given) over the global layer of
/// `home`; off when neither sets it.
///
/// Commands of optional modules and the CLI's wiring read switches through
/// this signature.
pub fn module_enabled(
    home: &Home,
    repo_root: Option<&Path>,
    module: Module,
) -> Result<bool, ConfigError> {
    Configuration::load(home, repo_root)?.module_enabled(module)
}

/// One folder below `~/.instrukt/companies/`: its name and its document as
/// read, or why it is not a company.
pub type CompanyHome = (ProjectSlug, Result<Configuration, ConfigError>);

/// The company homes on this computer: every folder below
/// `~/.instrukt/companies/` whose name is a valid company name, with its
/// document read as [`Configuration::load_company`] reads it. A folder
/// without a valid settings document is listed with the reason it is not a
/// company; nothing is served or run for it.
pub fn company_homes(home: &Home) -> Result<Vec<CompanyHome>, ConfigError> {
    Ok(company_folders(home)?
        .into_iter()
        .map(|(name, unreadable)| {
            let config = match unreadable {
                Some(error) => Err(error),
                None => Configuration::load_company(home, &name),
            };
            (name, config)
        })
        .collect())
}

/// The folders below `~/.instrukt/companies/` whose name is a valid company
/// name, sorted, each with why it cannot be inspected; nothing in them is
/// read.
pub(crate) fn company_folders(
    home: &Home,
) -> Result<Vec<(ProjectSlug, Option<ConfigError>)>, ConfigError> {
    let dir = home.companies_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(ConfigError::Read { path: dir, source }),
    };
    let mut folders = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ConfigError::Read {
            path: dir.clone(),
            source,
        })?;
        let folder = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let unreadable = match std::fs::metadata(&path) {
            Ok(metadata) if !metadata.is_dir() => continue,
            Ok(_) => None,
            Err(source) => Some(ConfigError::Read { path, source }),
        };
        let Ok(name) = ProjectSlug::parse(&folder) else {
            tracing::warn!(event = "company_home_invalid", folder = %folder, "not a valid company name");
            continue;
        };
        folders.push((name, unreadable));
    }
    folders.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    Ok(folders)
}

/// Whether text is an exact protected machine-local secret reference.
pub fn is_secret_reference(text: &str) -> bool {
    _secrets::reference(text).is_some()
}
