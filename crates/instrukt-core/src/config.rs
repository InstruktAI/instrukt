//! The configuration context: `instrukt.yaml` in a global and a project
//! layer (`docs/local/spec/domain/configuration.md`).
//!
//! One capability: answer, validate, migrate and safely change layered
//! configuration. Readers take a [`Configuration`] (the effective
//! configuration, or the global layer alone) and read keys by dotted path
//! with interpolation, or through typed accessors. Writers address one
//! [`Layer`]: [`init`], [`migrate`], [`patch`] against a [`revision`], and
//! [`validate`].

mod _effective;
mod _error;
mod _interpolate;
mod _layer;
mod _merge;
mod _module;
mod _render;
mod _runtime;
mod _schema;
mod _validate;

use std::path::Path;

pub use _effective::{
    ApprovalGate, Approver, Configuration, Notify, PublishedDomain, ThinkingMode, WatchedRef,
};
pub use _error::{ConfigError, Finding, FindingKind, Findings};
pub use _layer::{
    CONFIG_FILE_NAME, InitOutcome, Layer, MigrationOutcome, Revision, init, migrate, parse_patch,
    patch, revision,
};
pub use _module::Module;
pub use _render::render_yaml;
pub use _runtime::{Env, RuntimeEntry};
pub use _schema::{CURRENT_SCHEMA_VERSION, LayerKind, SHIPPED_BLUEPRINT_ENGINES};
pub use _validate::validate;

use crate::home::Home;
use crate::ids::ProjectSlug;

/// The project slug the repository rooted at `repo_root` declares in its
/// project layer; `None` when it has no project layer or declares none.
///
/// Docs, publication, repository and specs read the slug through this
/// signature.
pub fn project_slug(repo_root: &Path) -> Result<Option<ProjectSlug>, ConfigError> {
    let layer = Layer::Project(repo_root.to_path_buf());
    let path = repo_root.join(CONFIG_FILE_NAME);
    let Some(value) = _layer::read(&path)?.map(|doc| doc.value) else {
        return Ok(None);
    };
    _layer::require_usable(&layer, &path, &value)?;
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
