//! The configuration context: `instrukt.yaml` in a global and a project
//! layer (`docs/project/spec/domain/configuration.md`).
//!
//! One capability: answer, validate, migrate and safely change layered
//! configuration. Readers take a [`Configuration`] (the effective
//! configuration, or the global layer alone) and read keys by dotted path
//! with interpolation, or through typed accessors. Writers address one
//! [`Layer`]: [`init`], [`migrate`], [`patch`] against a [`revision`], and
//! [`validate`].

mod _agents;
mod _agents_migrate;
mod _effective;
mod _error;
mod _features;
mod _interpolate;
mod _layer;
mod _merge;
mod _migrate;
mod _module;
mod _redis;
mod _render;
mod _runtime;
mod _schema;
mod _secrets;
mod _validate;
mod _webhooks;

use std::path::Path;

pub use _agents::{
    AgentSelection, Agents, Effort, HarnessKind, Profile, RoleSelection, SHIPPED_AGENTS,
};

pub use _effective::{ApprovalGate, Approver, BoardRef, Configuration, Notify, Tier, WatchedRef};
pub use _error::{ConfigError, Finding, FindingKind, Findings};
pub use _features::{Feature, Unavailable, availability as feature_availability};
pub use _layer::{
    CONFIG_FILE_NAME, InitOutcome, Layer, LayerInspection, MigrationOutcome, Revision, init,
    inspect_layer, migrate, parse_patch, patch, revision,
};
pub use _module::Module;
pub use _redis::{REDIS_BLOCK_SECONDS, RedisEndpoint, RedisSettings};
pub use _render::render_yaml;
pub use _runtime::WorkflowCapability;
pub use _runtime::{Env, NativeCapabilities, Runtime};
pub use _schema::{CURRENT_SCHEMA_VERSION, LayerKind, SHIPPED_BLUEPRINT_ENGINES, editor_schema};
pub use _validate::{validate, validate_local};
pub use _webhooks::{
    Ingress, SigningSecrets, WebhookProtocol, WebhookSource, WebhookSources, webhook_domain,
    webhook_name,
};

use crate::home::Home;
use crate::ids::ProjectSlug;

pub use _secrets::{generate_secret, resolve_secret, set_secret};

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

/// Whether text is an exact protected machine-local secret reference.
pub fn is_secret_reference(text: &str) -> bool {
    _secrets::reference(text).is_some()
}
