//! Layer documents on disk: locating, reading, revisions, version checks,
//! migration, initialization and the locked, validated, atomic patch.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_yaml_ng::{Mapping, Value};
use sha2::{Digest, Sha256};

use super::_company_document::{self, CompanyDocument};
use super::_error::{ConfigError, FindingKind, Findings};
use super::_merge::merge_patch;
use super::_schema::LayerKind;
use super::_validate::{findings_for, todos_on};
use crate::fsx::{LockGuard, atomic_write};
use crate::git;
use crate::home::Home;
use crate::ids::ProjectSlug;

/// How long a layer write waits for another writer of the same document;
/// writes hold the lock for one read-modify-write only.
pub(super) const WRITE_LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The file name of both configuration layers (and, in a home made before
/// `company.yaml`, of its company's settings document).
pub const CONFIG_FILE_NAME: &str = "instrukt.yaml";

/// One addressable layer document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layer {
    /// The machine's `~/.instrukt/instrukt.yaml`.
    Global,
    /// The `instrukt.yaml` at the root of the repository at this path.
    Project(PathBuf),
    /// The `company.yaml` of the company home of this name
    /// (`~/.instrukt/companies/<name>/`); a document of its own, no layer.
    Company(ProjectSlug),
}

impl Layer {
    /// The project layer of the repository containing `dir`.
    pub fn project_containing(dir: &Path) -> Result<Self, ConfigError> {
        match git::worktree_root(dir)? {
            Some(root) => Ok(Layer::Project(root)),
            None => Err(ConfigError::NotInProject {
                dir: dir.to_path_buf(),
            }),
        }
    }

    pub fn kind(&self) -> LayerKind {
        match self {
            Layer::Global => LayerKind::Global,
            Layer::Project(_) => LayerKind::Project,
            Layer::Company(_) => LayerKind::Company,
        }
    }

    /// The document's canonical path: for a company, its `company.yaml`,
    /// which a home made before keeps as a root `instrukt.yaml` until
    /// converted. Reads go through [`read_layer`]; this path keys the
    /// document's write lock and names it where it is absent.
    pub fn path(&self, home: &Home) -> PathBuf {
        match self {
            Layer::Global => home.global_config(),
            Layer::Project(root) => root.join(CONFIG_FILE_NAME),
            Layer::Company(name) => home
                .company_dir(name)
                .join(_company_document::COMPANY_DOCUMENT),
        }
    }

    /// The path a finding or error names for the document: where it stands
    /// (a company's legacy `instrukt.yaml` until converted, `company.yaml`
    /// of two present), otherwise [`Layer::path`]. Nothing is converted.
    pub fn located(&self, home: &Home) -> PathBuf {
        if let Layer::Company(name) = self {
            match _company_document::resolve(home, name) {
                Ok(CompanyDocument::Present(path) | CompanyDocument::Legacy(path)) => return path,
                Ok(CompanyDocument::Both { company, .. }) => return company,
                Ok(CompanyDocument::Absent) | Err(_) => {}
            }
        }
        self.path(home)
    }

    pub(crate) fn init_door(&self) -> String {
        match self {
            Layer::Global => "instrukt config init --global".into(),
            Layer::Project(_) => "instrukt config init".into(),
            Layer::Company(name) => format!("instrukt config init --company {name}"),
        }
    }
}

/// An opaque digest of a layer document's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision(String);

impl Revision {
    /// The revision of document content `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        let digest = Sha256::digest(bytes);
        Self(hex::encode(&digest[..8]))
    }

    /// A revision as a caller hands it back.
    pub fn from_text(text: &str) -> Self {
        Self(text.trim().to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A layer document as read from disk.
pub(crate) struct Document {
    pub(crate) bytes: Vec<u8>,
    pub(crate) value: Value,
}

/// Reads the document at `path`; `None` when it does not exist.
pub(crate) fn read(path: &Path) -> Result<Option<Document>, ConfigError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let value = parse(path, &bytes)?;
    Ok(Some(Document { bytes, value }))
}

/// Reads `layer`'s document where it stands, with the path it was read
/// from: a company's resolved document (its legacy `instrukt.yaml` until
/// converted; both present refused), the project layer unless its file
/// holds a company's settings, the machine's layer. `None` when there is
/// none. Nothing is converted; the paths that persist a conversion run
/// [`converted`] first.
pub(crate) fn read_layer(
    home: &Home,
    layer: &Layer,
) -> Result<Option<(PathBuf, Document)>, ConfigError> {
    let path = match layer {
        Layer::Company(name) => match _company_document::resolve(home, name)? {
            CompanyDocument::Present(path) | CompanyDocument::Legacy(path) => path,
            CompanyDocument::Both { company, legacy } => {
                return Err(ConfigError::BothCompanyDocuments { company, legacy });
            }
            CompanyDocument::Absent => return Ok(None),
        },
        Layer::Global | Layer::Project(_) => layer.path(home),
    };
    Ok(read(&path)?
        .filter(|document| {
            !(matches!(layer, Layer::Project(_))
                && _company_document::declares_company(&document.value))
        })
        .map(|document| (path, document)))
}

/// Converts a company's legacy document to `company.yaml` where reading
/// `layer` through `home` persists conversion; nothing otherwise.
fn converted(home: &Home, layer: &Layer) -> Result<(), ConfigError> {
    match layer {
        Layer::Company(name) if persists_conversion(home, layer) => {
            _company_document::convert(home, name)
        }
        _ => Ok(()),
    }
}

fn parse(path: &Path, bytes: &[u8]) -> Result<Value, ConfigError> {
    let value: Value = serde_yaml_ng::from_slice(bytes).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    match value {
        Value::Null => Ok(Value::Mapping(Mapping::new())),
        Value::Mapping(_) => Ok(value),
        _ => Err(ConfigError::NotAMapping {
            path: path.to_path_buf(),
        }),
    }
}

/// How a document's `schema_version` compares with its kind's current one.
enum Version {
    Current,
    Older(u64),
    Newer(u64),
    Unknown,
}

fn version(value: &Value, kind: LayerKind) -> Version {
    let current = kind.current_schema_version();
    match value.get("schema_version").and_then(Value::as_u64) {
        Some(v) if v == current => Version::Current,
        Some(v) if (1..current).contains(&v) => Version::Older(v),
        Some(v) if v > current => Version::Newer(v),
        _ => Version::Unknown,
    }
}

/// Refuses a document a command other than `init` and publication cannot
/// use: one of a newer schema (upgrade instrukt) or an older one (migrate
/// it first). A document without a usable version is left to validation.
pub(crate) fn require_usable(layer: &Layer, path: &Path, value: &Value) -> Result<(), ConfigError> {
    let current = layer.kind().current_schema_version();
    match version(value, layer.kind()) {
        Version::Current | Version::Unknown => Ok(()),
        Version::Newer(found) => Err(ConfigError::NewerSchema {
            path: path.to_path_buf(),
            found,
            current,
        }),
        Version::Older(found) => Err(ConfigError::OlderSchema {
            path: path.to_path_buf(),
            found,
            current,
            door: layer.init_door(),
        }),
    }
}

/// Whether reading `layer` through `home` persists its conversion: only the
/// documents the machine owns (its layer and its company homes) outside
/// inspection. A project layer is committed to its
/// repository, so ordinary paths convert it in memory and leave the file as
/// committed; [`migrate`] (the `config` writing verbs and the pre-commit
/// hook) and [`patch`] are the paths that write it
/// (`docs/project/spec/feature/configuration/owned-files.md`).
fn persists_conversion(home: &Home, layer: &Layer) -> bool {
    !home.is_inspection() && matches!(layer, Layer::Global | Layer::Company(_))
}

/// Reads a layer for a read command: absent is `None`; an unusable schema
/// version is refused.
pub(crate) fn read_usable(home: &Home, layer: &Layer) -> Result<Option<Value>, ConfigError> {
    if !persists_conversion(home, layer) {
        return Ok(inspect_layer(home, layer)?.value);
    }
    migrate_if_needed(home, layer)?;
    match read_layer(home, layer)? {
        Some((path, doc)) => {
            require_usable(layer, &path, &doc.value)?;
            Ok(Some(doc.value))
        }
        None => Ok(None),
    }
}

pub(crate) fn migrate_if_needed(home: &Home, layer: &Layer) -> Result<(), ConfigError> {
    if !persists_conversion(home, layer) {
        return Ok(());
    }
    converted(home, layer)?;
    if read_layer(home, layer)?
        .is_some_and(|(_, doc)| matches!(version(&doc.value, layer.kind()), Version::Older(_)))
    {
        migrate(home, layer)?;
    }
    Ok(())
}

#[derive(Debug, serde::Serialize)]
pub struct LayerInspection {
    pub path: PathBuf,
    pub present: bool,
    pub schema_version: Option<u64>,
    pub migration: Option<String>,
    #[serde(skip)]
    pub(crate) value: Option<Value>,
}
impl LayerInspection {
    /// The document as a read command uses it: converted to its kind's
    /// current version in memory; `None` when absent.
    pub fn value(&self) -> Option<&Value> {
        self.value.as_ref()
    }

    pub fn module_setting(&self, module: super::Module) -> Result<Option<bool>, ConfigError> {
        match self
            .value
            .as_ref()
            .and_then(|v| v.get("modules"))
            .and_then(|v| v.get(module.key()))
        {
            None => Ok(None),
            Some(Value::Bool(value)) => Ok(Some(*value)),
            Some(_) => Err(ConfigError::InvalidValue {
                key: format!("modules.{}", module.key()),
                message: "expected boolean".into(),
            }),
        }
    }
    pub fn project(&self) -> Result<Option<ProjectSlug>, ConfigError> {
        let Some(value) = self.value.as_ref().and_then(|v| v.get("project")) else {
            return Ok(None);
        };
        let text = value.as_str().unwrap_or_default();
        ProjectSlug::parse(text)
            .map(Some)
            .map_err(|source| ConfigError::Slug {
                path: self.path.clone(),
                source,
            })
    }
}
/// Why the machine layer cannot be loaded at all: it is unreadable, not
/// YAML, not a mapping, or its `schema_version` is missing or newer than this
/// instrukt knows. `None` when it is absent or loads; what validation finds
/// in a layer that loads stays with `instrukt config validate`.
pub fn unloadable_machine_layer(home: &Home) -> Option<ConfigError> {
    let path = Layer::Global.path(home);
    let document = match read(&path) {
        Ok(Some(document)) => document,
        Ok(None) => return None,
        Err(error) => return Some(error),
    };
    match version(&document.value, LayerKind::Global) {
        Version::Newer(found) => Some(ConfigError::NewerSchema {
            path,
            found,
            current: LayerKind::Global.current_schema_version(),
        }),
        Version::Unknown => Some(ConfigError::UnknownSchema { path }),
        Version::Current | Version::Older(_) => None,
    }
}

/// A current in-memory view, retaining the on-disk version and migration notice.
pub fn inspect_layer(home: &Home, layer: &Layer) -> Result<LayerInspection, ConfigError> {
    let (path, document) = match read_layer(home, layer)? {
        Some((path, document)) => (path, Some(document)),
        None => (layer.path(home), None),
    };
    let mut view = LayerInspection {
        path: path.clone(),
        present: document.is_some(),
        schema_version: None,
        migration: None,
        value: None,
    };
    if let Some(document) = document {
        let value = document.value;
        view.schema_version = value.get("schema_version").and_then(Value::as_u64);
        if let Version::Older(from) = version(&value, layer.kind()) {
            view.migration = Some(format!(
                "{}: schema_version {from} inspected as {}; file migrates on the next writing command",
                path.display(),
                layer.kind().current_schema_version()
            ));
        }
        view.value = Some(inspected(home, layer, &path, value)?);
    }
    Ok(view)
}

/// `raw`, a document of `layer` as read from `path`, as a read command uses
/// it: an older schema converted in memory, an unusable one refused. Nothing
/// is read or written.
pub(crate) fn inspected(
    home: &Home,
    layer: &Layer,
    path: &Path,
    mut value: Value,
) -> Result<Value, ConfigError> {
    if let Version::Older(from) = version(&value, layer.kind()) {
        super::_migrate::selection(home, layer, &mut value, false)?;
        value = super::_migrate::upcycle(value, from, layer.kind());
    }
    require_usable(layer, path, &value)?;
    Ok(value)
}

/// The current revision of `layer`.
pub fn revision(home: &Home, layer: &Layer) -> Result<Revision, ConfigError> {
    let persists = persists_conversion(home, layer);
    if persists {
        migrate_if_needed(home, layer)?;
    } else {
        inspect_layer(home, layer)?;
    }
    match read_layer(home, layer)? {
        Some((path, doc)) => {
            if persists {
                require_usable(layer, &path, &doc.value)?;
            }
            Ok(Revision::of(&doc.bytes))
        }
        None => Err(ConfigError::NoLayer {
            path: layer.path(home),
            door: layer.init_door(),
        }),
    }
}

/// Parses patch text: YAML that must be a mapping.
pub fn parse_patch(text: &str) -> Result<Value, ConfigError> {
    let value: Value = serde_yaml_ng::from_str(text).map_err(ConfigError::PatchParse)?;
    if value.is_mapping() {
        Ok(value)
    } else {
        Err(ConfigError::PatchNotAMapping)
    }
}

/// Applies merge `patch` to `layer`, only when the document is still at
/// `expected` and the whole result is valid; writes atomically and returns
/// the new revision. Of two patches from one revision, one succeeds and
/// the other is refused.
pub fn patch(
    home: &Home,
    layer: &Layer,
    expected: &Revision,
    patch: &Value,
) -> Result<Revision, ConfigError> {
    if !patch.is_mapping() {
        return Err(ConfigError::PatchNotAMapping);
    }
    if matches!(layer, Layer::Project(_)) {
        migrate_if_needed(home, &Layer::Global)?;
    }
    converted(home, layer)?;
    let started = Instant::now();
    let _lock = LockGuard::acquire(&lock_path(home, &layer.path(home)), WRITE_LOCK_TIMEOUT)?;
    let Some((path, mut doc)) = read_layer(home, layer)? else {
        return Err(ConfigError::NoLayer {
            path: layer.path(home),
            door: layer.init_door(),
        });
    };
    let inspected_revision = Revision::of(&doc.bytes);
    // The machine's documents keep their conversion even when the patch is
    // refused; a project layer is converted only by the write that applies
    // the patch.
    if matches!(layer, Layer::Global | Layer::Company(_))
        && let Version::Older(from) = version(&doc.value, layer.kind())
    {
        super::_migrate::selection(home, layer, &mut doc.value, true)?;
        doc.value = super::_migrate::upcycle(doc.value, from, layer.kind());
        write(&path, &doc.bytes, &doc.value)?;
        doc = read(&path)?.ok_or_else(|| ConfigError::NoLayer {
            path: path.clone(),
            door: layer.init_door(),
        })?;
    }
    let (before, next) = patched(layer, &path, &doc, patch)?;
    let current = Revision::of(&doc.bytes);
    if &inspected_revision != expected {
        return Err(ConfigError::RevisionMismatch {
            expected: expected.to_string(),
            current: current.to_string(),
        });
    }
    let findings = refused_findings(home, layer, &path, &before, &next)?;
    if !findings.is_empty() {
        return Err(ConfigError::Invalid(Findings(findings)));
    }
    let revision = write(&path, &doc.bytes, &next)?;
    tracing::debug!(
        event = "config_patched",
        path = %path.display(),
        from_revision = %expected,
        revision = %revision,
        duration_ms = elapsed_ms(started),
    );
    Ok(revision)
}

/// The findings for which a patch from `before` to `next` of `layer` at
/// `path` is refused: those of `next`, less the ones that already stood on
/// `before`. Todos already on stay a validation finding; only a write that
/// switches them on is refused for it. Event responses that already lacked
/// their transport stay a finding too; a write that switches them on
/// without it, or removes it from under them, is refused. Automatic review
/// already on stays a finding; only a write that switches it on is refused
/// for one.
fn refused_findings(
    home: &Home,
    layer: &Layer,
    path: &Path,
    before: &Value,
    next: &Value,
) -> Result<Vec<super::_error::Finding>, ConfigError> {
    let mut findings = findings_for(&home.for_inspection(), layer, path, next)?;
    if layer.kind() == LayerKind::Project {
        let global = read_usable(home, &Layer::Global)?;
        if todos_on(before, global.as_ref())? {
            findings.retain(|f| f.kind != FindingKind::TodosNeedGitHub);
        }
        if super::_validate::needs_transport(&super::_effective::effective_value(
            global.as_ref(),
            Some(before),
        )?) {
            findings.retain(|f| f.kind != FindingKind::EventsNeedTransport);
        }
        if super::_review::review_on(before) {
            findings.retain(|f| f.key != super::_review::REVIEW_AUTOMATIC_KEY);
        }
    }
    if layer.kind() == LayerKind::Global {
        let before = super::_effective::effective_value(Some(before), None)?;
        if super::_validate::needs_transport(&before) {
            findings.retain(|f| f.kind != FindingKind::EventsNeedTransport);
        }
        if super::_validate::thread_retention_unavailable(&before).is_some() {
            findings.retain(|f| !super::withholds_thread_retention(f));
        }
    }
    Ok(findings)
}

/// The document `doc` of `layer` at `path` as it stands, converted to its
/// kind's current version in memory when older, and the document `patch` makes of
/// it.
pub(super) fn patched(
    layer: &Layer,
    path: &Path,
    doc: &Document,
    patch: &Value,
) -> Result<(Value, Value), ConfigError> {
    let mut before = doc.value.clone();
    if let Version::Older(from) = version(&before, layer.kind()) {
        before = super::_migrate::upcycle(before, from, layer.kind());
    }
    require_usable(layer, path, &before)?;
    let mut next = before.clone();
    merge_patch(&mut next, patch);
    Ok((before, next))
}

/// The repository whose two identities must be ready before `patch`
/// applies to `layer` at `expected`: only for a patch that switches
/// automatic review on and that nothing on this computer already refuses.
/// Read without the document's lock, so GitHub is asked before it is
/// taken; the patch's own revision check binds what was judged to what is
/// written.
pub fn review_switch(
    home: &Home,
    layer: &Layer,
    expected: &Revision,
    patch: &Value,
) -> Result<Option<String>, ConfigError> {
    let Layer::Project(root) = layer else {
        return Ok(None);
    };
    let Some((path, doc)) = read_layer(home, layer)? else {
        return Ok(None);
    };
    if &Revision::of(&doc.bytes) != expected {
        return Ok(None);
    }
    let (before, next) = patched(layer, &path, &doc, patch)?;
    if !super::_review::switches_on(&before, &next)
        || !refused_findings(home, layer, &path, &before, &next)?.is_empty()
    {
        return Ok(None);
    }
    let global = read_usable(home, &Layer::Global)?;
    super::_review::repository(root, &next, global.as_ref())
}

/// The repository whose two identities must be ready for the project layer
/// `layer` as stored: none while it does not switch automatic review on.
pub fn review_target(home: &Home, layer: &Layer) -> Result<Option<String>, ConfigError> {
    let Layer::Project(root) = layer else {
        return Ok(None);
    };
    let Some(value) = read_usable(home, layer)? else {
        return Ok(None);
    };
    if !super::_review::review_on(&value) {
        return Ok(None);
    }
    let global = read_usable(home, &Layer::Global)?;
    super::_review::repository(root, &value, global.as_ref())
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// What [`init`] did to a layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitOutcome {
    Created,
    AlreadyCurrent,
    Migrated { from: u64 },
}

/// What [`migrate`] did to a layer document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// There is no document.
    Absent,
    /// The document is at its kind's current version; untouched.
    AlreadyCurrent,
    /// The document was rewritten from version `from`.
    Migrated { from: u64 },
}

/// Brings `layer` to its kind's current schema version for a path that is not an
/// `instrukt config` writing verb — publication, installation, ticks: the
/// machine's layer is migrated on disk ([`migrate`]); a project layer is
/// checked and converted in memory only, its file left as committed. Either
/// way a newer or unknown version is refused.
pub fn bring_current(home: &Home, layer: &Layer) -> Result<(), ConfigError> {
    if persists_conversion(home, layer) {
        migrate(home, layer)?;
        return Ok(());
    }
    let Some((path, doc)) = read_layer(home, layer)? else {
        return Ok(());
    };
    match version(&doc.value, layer.kind()) {
        Version::Current => Ok(()),
        Version::Older(_) => inspected(home, layer, &path, doc.value).map(drop),
        Version::Newer(found) => Err(ConfigError::NewerSchema {
            path,
            found,
            current: layer.kind().current_schema_version(),
        }),
        Version::Unknown => Err(ConfigError::UnknownSchema { path }),
    }
}

/// The project layer document `bytes` of the repository rooted at `root`,
/// converted to the project layer's current schema version and rendered as [`migrate`]
/// writes it; `None` when it is already
/// current. A newer or unknown version is refused. The pre-commit hook
/// converts the version a commit is being made from with this.
pub fn converted_project_layer(root: &Path, bytes: &[u8]) -> Result<Option<Vec<u8>>, ConfigError> {
    let path = root.join(CONFIG_FILE_NAME);
    let value = parse(&path, bytes)?;
    match version(&value, LayerKind::Project) {
        Version::Current => Ok(None),
        Version::Newer(found) => Err(ConfigError::NewerSchema {
            path,
            found,
            current: LayerKind::Project.current_schema_version(),
        }),
        Version::Unknown => Err(ConfigError::UnknownSchema { path }),
        Version::Older(from) => {
            let value = super::_migrate::upcycle(value, from, LayerKind::Project);
            render(&path, bytes, &value).map(Some)
        }
    }
}

/// Brings `layer` to its kind's current schema version with its settings kept:
/// an older document is rewritten through the same atomic write as a
/// patch; a current one is left untouched; a newer one is refused.
/// `init`, the `config` writing verbs, the pre-commit hook and, for the
/// machine's layer, [`bring_current`] run this.
pub fn migrate(home: &Home, layer: &Layer) -> Result<MigrationOutcome, ConfigError> {
    converted(home, layer)?;
    let started = Instant::now();
    let _lock = LockGuard::acquire(&lock_path(home, &layer.path(home)), WRITE_LOCK_TIMEOUT)?;
    let Some((path, doc)) = read_layer(home, layer)? else {
        return Ok(MigrationOutcome::Absent);
    };
    let current = layer.kind().current_schema_version();
    match version(&doc.value, layer.kind()) {
        Version::Current => Ok(MigrationOutcome::AlreadyCurrent),
        Version::Newer(found) => Err(ConfigError::NewerSchema {
            path,
            found,
            current,
        }),
        Version::Unknown => Err(ConfigError::UnknownSchema { path }),
        Version::Older(from) => {
            let mut migrated = doc.value;
            super::_migrate::selection(home, layer, &mut migrated, true)?;
            let migrated = super::_migrate::upcycle(migrated, from, layer.kind());
            let revision = write(&path, &doc.bytes, &migrated)?;
            tracing::debug!(
                event = "config_migrated",
                path = %path.display(),
                from_version = from,
                to_version = current,
                revision = %revision,
                duration_ms = elapsed_ms(started),
            );
            Ok(MigrationOutcome::Migrated { from })
        }
    }
}

/// Creates `layer` with its kind's current schema version (and, for a project,
/// its slug) when absent; migrates an existing one. The project slug is
/// `slug` when given, otherwise the primary worktree's folder name
/// normalized; an empty normalization is refused.
pub fn init(
    home: &Home,
    layer: &Layer,
    slug: Option<&ProjectSlug>,
) -> Result<InitOutcome, ConfigError> {
    let started = Instant::now();
    let path = layer.path(home);
    if let Layer::Company(name) = layer {
        let dir = home.company_dir(name);
        crate::fsx::create_dir_all(&dir).map_err(|source| ConfigError::CreateHome {
            path: dir.clone(),
            source,
        })?;
        _company_document::ignore_local_material(&dir)?;
        _company_document::convert(home, name)?;
        return init_company(home, layer, name, started);
    }
    if path.exists() {
        return match migrate(home, layer)? {
            MigrationOutcome::Migrated { from } => Ok(InitOutcome::Migrated { from }),
            MigrationOutcome::AlreadyCurrent | MigrationOutcome::Absent => {
                Ok(InitOutcome::AlreadyCurrent)
            }
        };
    }
    let mut doc = Mapping::new();
    doc.insert(
        Value::String("schema_version".to_owned()),
        Value::Number(layer.kind().current_schema_version().into()),
    );
    if let Layer::Project(root) = layer {
        let slug = match slug {
            Some(slug) => slug.clone(),
            None => derived_slug(root)?,
        };
        doc.insert(
            Value::String("project".to_owned()),
            Value::String(slug.to_string()),
        );
    }
    let _lock = LockGuard::acquire(&lock_path(home, &path), WRITE_LOCK_TIMEOUT)?;
    if path.exists() {
        return Ok(InitOutcome::AlreadyCurrent);
    }
    let revision = write(&path, &[], &Value::Mapping(doc))?;
    tracing::debug!(
        event = "config_initialized",
        path = %path.display(),
        schema_version = layer.kind().current_schema_version(),
        revision = %revision,
        duration_ms = elapsed_ms(started),
    );
    Ok(InitOutcome::Created)
}

/// [`init`] of the company home `name`, its folder made and its legacy
/// document converted: an existing document is migrated, both present
/// refused, an absent one written as `company.yaml`.
fn init_company(
    home: &Home,
    layer: &Layer,
    name: &ProjectSlug,
    started: Instant,
) -> Result<InitOutcome, ConfigError> {
    match _company_document::resolve(home, name)? {
        CompanyDocument::Both { company, legacy } => {
            return Err(ConfigError::BothCompanyDocuments { company, legacy });
        }
        CompanyDocument::Present(_) | CompanyDocument::Legacy(_) => {
            return match migrate(home, layer)? {
                MigrationOutcome::Migrated { from } => Ok(InitOutcome::Migrated { from }),
                MigrationOutcome::AlreadyCurrent | MigrationOutcome::Absent => {
                    Ok(InitOutcome::AlreadyCurrent)
                }
            };
        }
        CompanyDocument::Absent => {}
    }
    let path = layer.path(home);
    let _lock = LockGuard::acquire(&lock_path(home, &path), WRITE_LOCK_TIMEOUT)?;
    if !_company_document::write_company_settings(&home.company_dir(name), name)? {
        return Ok(InitOutcome::AlreadyCurrent);
    }
    tracing::debug!(
        event = "config_initialized",
        path = %path.display(),
        schema_version = layer.kind().current_schema_version(),
        duration_ms = elapsed_ms(started),
    );
    Ok(InitOutcome::Created)
}

/// The primary worktree's folder name, normalized to a slug.
fn derived_slug(root: &Path) -> Result<ProjectSlug, ConfigError> {
    let listing = git::run(root, ["worktree", "list", "--porcelain"])?;
    let primary = listing
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .unwrap_or_else(|| root.to_path_buf());
    let folder = primary
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let normalized = normalize_slug(&folder);
    ProjectSlug::parse(&normalized).map_err(|_| ConfigError::EmptySlug { folder })
}

/// Lowercases, turns every run of characters other than ASCII letters and
/// digits into one `-`, and trims `-` from both ends.
pub(crate) fn normalize_slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

/// Writes `value` at `path`, keeping the editor-schema modeline its
/// previous bytes `before` open with, if any
/// (`docs/project/spec/feature/publication/editor-schemas.md`).
pub(super) fn write(path: &Path, before: &[u8], value: &Value) -> Result<Revision, ConfigError> {
    let bytes = render(path, before, value)?;
    atomic_write(path, &bytes)?;
    Ok(Revision::of(&bytes))
}

/// The bytes [`write`] stores for `value` at `path` whose previous bytes
/// were `before`: `before` edited in place where only the changed top-level
/// keys need new text, so comments and formatting elsewhere stay; otherwise
/// the whole value rendered anew below the previous modeline.
fn render(path: &Path, before: &[u8], value: &Value) -> Result<Vec<u8>, ConfigError> {
    if let Some(bytes) = super::_edit::edit_in_place(before, value) {
        return Ok(bytes);
    }
    if !before.is_empty() {
        tracing::warn!(
            event = "config_rendered_whole",
            path = %path.display(),
            "the document is not a block-style mapping of plain keys; it is rendered anew and its comments are not kept"
        );
    }
    let rendered = super::_render::render_yaml(value).map_err(|source| ConfigError::Render {
        path: path.to_path_buf(),
        source,
    })?;
    let mut bytes = match crate::modeline::leading(before) {
        Some(line) => [line, b"\n"].concat(),
        None => Vec::new(),
    };
    bytes.extend_from_slice(rendered.as_bytes());
    Ok(bytes)
}

/// The lock serializing writers of the document at `path`, keyed by its
/// folder and its own file name, so a company's `company.yaml` and the
/// project layer beside it never share one. It lives in the home, never
/// beside a project's document.
pub(super) fn lock_path(home: &Home, path: &Path) -> PathBuf {
    let canonical = path
        .parent()
        .and_then(|dir| dir.canonicalize().ok())
        .zip(path.file_name())
        .map(|(dir, name)| dir.join(name))
        .unwrap_or_else(|| path.to_path_buf());
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    home.locks_dir()
        .join(format!("config-{}.lock", hex::encode(&digest[..8])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GLOBAL_SCHEMA_VERSION, PROJECT_SCHEMA_VERSION};

    #[test]
    fn folder_names_normalize_to_valid_slugs() {
        for folder in ["My Project_v2", "--Ünïcode--", "a  b", "UPPER.case"] {
            let slug = normalize_slug(folder);
            assert!(ProjectSlug::parse(&slug).is_ok(), "{folder}");
            assert_eq!(normalize_slug(&slug), slug, "normalizing is idempotent");
        }
        assert!(normalize_slug("___").is_empty());
    }

    fn yaml(text: &str) -> Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    #[test]
    fn only_a_document_of_the_current_or_an_unknown_version_is_usable() {
        let layer = Layer::Global;
        let path = Path::new("instrukt.yaml");
        let newer = format!("schema_version: {}\n", GLOBAL_SCHEMA_VERSION + 1);
        assert!(matches!(
            require_usable(&layer, path, &yaml(&newer)),
            Err(ConfigError::NewerSchema { .. })
        ));
        let current = format!("schema_version: {GLOBAL_SCHEMA_VERSION}\n");
        assert!(require_usable(&layer, path, &yaml(&current)).is_ok());
        assert!(require_usable(&layer, path, &yaml("project: p\n")).is_ok());
    }

    #[test]
    fn an_older_committed_project_layer_converts_with_its_modeline_and_a_current_one_does_not() {
        let root = Path::new("repo");
        let modeline = "# yaml-language-server: $schema=.instrukt/schemas/instrukt.schema.json";
        let older = format!("{modeline}\nschema_version: 2\nproject: p\nactions: {{test: t}}\n");
        let converted = converted_project_layer(root, older.as_bytes())
            .unwrap()
            .unwrap();
        assert_eq!(
            crate::modeline::leading(&converted),
            crate::modeline::leading(older.as_bytes()),
            "the modeline is kept"
        );
        let value = yaml(&String::from_utf8(converted.clone()).unwrap());
        assert_eq!(
            value["schema_version"].as_u64(),
            Some(PROJECT_SCHEMA_VERSION)
        );
        assert!(value.get("actions").is_none());
        assert_eq!(converted_project_layer(root, &converted).unwrap(), None);
        let newer = format!("schema_version: {}\n", PROJECT_SCHEMA_VERSION + 1);
        assert!(matches!(
            converted_project_layer(root, newer.as_bytes()),
            Err(ConfigError::NewerSchema { .. })
        ));
    }

    #[test]
    fn revisions_follow_content_only() {
        assert_eq!(Revision::of(b"a: 1\n"), Revision::of(b"a: 1\n"));
        assert_ne!(Revision::of(b"a: 1\n"), Revision::of(b"a: 2\n"));
    }

    #[test]
    fn a_patch_leaves_automatic_review_on_while_a_finding_at_it_stands() {
        let dir = std::env::temp_dir().join(format!(
            "instrukt-layer-review-standing-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let home = Home::at_user_home(dir.join("user"));
        std::fs::create_dir_all(home.root()).unwrap();
        std::fs::write(
            home.root().join(CONFIG_FILE_NAME),
            format!("schema_version: {GLOBAL_SCHEMA_VERSION}\n"),
        )
        .unwrap();
        let root = dir.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        // Events are off and nothing is published: findings stand at the
        // setting, which is already on.
        std::fs::write(
            root.join(CONFIG_FILE_NAME),
            format!(
                "schema_version: {PROJECT_SCHEMA_VERSION}\nproject: demo\ngithub:\n  repository: acme/demo\nreview:\n  automatic: true\n"
            ),
        )
        .unwrap();
        let layer = Layer::Project(root.clone());
        let expected = revision(&home, &layer).unwrap();
        let patched = patch(&home, &layer, &expected, &yaml("board: acme/1"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(patched.is_ok(), "{patched:?}");
    }

    #[test]
    fn switching_automatic_review_on_asks_for_readiness_while_a_finding_that_already_stood_remains()
    {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().join("user"));
        std::fs::create_dir_all(home.root().join("state")).unwrap();
        // No event transport on this computer.
        std::fs::write(
            home.root().join(CONFIG_FILE_NAME),
            format!("schema_version: {GLOBAL_SCHEMA_VERSION}\n"),
        )
        .unwrap();
        std::fs::write(
            home.index_file(),
            format!(
                "schema_version: 2\nsnippets:\n- id: {id}\n  description: d\n  type: procedure\n  scope: global\n  domain: sdlc\n  path: {id}.md\n  source_project: instrukt\n  role: {role}\n  subscriptions: [{event}]\n  subscription_home: project\n",
                id = super::super::_review::REVIEW_PROCEDURE,
                role = super::super::_review::REVIEW_ROLE,
                event = super::super::_review::REVIEW_NEEDED_EVENT,
            ),
        )
        .unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        // Event responses already lack their transport: that finding stood
        // before the patch, and the patch does not refuse for it.
        std::fs::write(
            root.join(CONFIG_FILE_NAME),
            format!(
                "schema_version: {PROJECT_SCHEMA_VERSION}\nproject: demo\ngithub:\n  repository: acme/demo\nmodules:\n  events: true\n"
            ),
        )
        .unwrap();
        let layer = Layer::Project(root);
        let expected = revision(&home, &layer).unwrap();
        let on = yaml("review: {automatic: true}");
        assert_eq!(
            review_switch(&home, &layer, &expected, &on).unwrap(),
            Some("acme/demo".to_owned()),
            "the identities' readiness is asked before the write"
        );
    }

    #[test]
    fn a_patch_must_be_a_mapping() {
        assert!(parse_patch("a: 1").is_ok());
        for bad in ["[1]", "3", ""] {
            assert!(matches!(
                parse_patch(bad),
                Err(ConfigError::PatchNotAMapping)
            ));
        }
    }
}
