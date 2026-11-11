//! Layer documents on disk: locating, reading, revisions, version checks,
//! migration, initialization and the locked, validated, atomic patch.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_yaml_ng::{Mapping, Value};
use sha2::{Digest, Sha256};

use super::_error::{ConfigError, FindingKind, Findings};
use super::_merge::merge_patch;
use super::_schema::{CURRENT_SCHEMA_VERSION, LayerKind};
use super::_validate::{findings_for, todos_on};
use crate::fsx::{LockGuard, atomic_write};
use crate::git;
use crate::home::Home;
use crate::ids::ProjectSlug;

/// How long a layer write waits for another writer of the same document;
/// writes hold the lock for one read-modify-write only.
const WRITE_LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The file name of both configuration layers.
pub const CONFIG_FILE_NAME: &str = "instrukt.yaml";

/// One addressable layer document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layer {
    /// The machine's `~/.instrukt/instrukt.yaml`.
    Global,
    /// The `instrukt.yaml` at the root of the repository at this path.
    Project(PathBuf),
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
        }
    }

    /// The document's path.
    pub fn path(&self, home: &Home) -> PathBuf {
        match self {
            Layer::Global => home.global_config(),
            Layer::Project(root) => root.join(CONFIG_FILE_NAME),
        }
    }

    pub(crate) fn init_door(&self) -> &'static str {
        match self {
            Layer::Global => "instrukt config init --global",
            Layer::Project(_) => "instrukt config init",
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

/// How a document's `schema_version` compares with the current one.
enum Version {
    Current,
    Older(u64),
    Newer(u64),
    Unknown,
}

fn version(value: &Value) -> Version {
    match value.get("schema_version").and_then(Value::as_u64) {
        Some(v) if v == CURRENT_SCHEMA_VERSION => Version::Current,
        Some(v) if (1..CURRENT_SCHEMA_VERSION).contains(&v) => Version::Older(v),
        Some(v) if v > CURRENT_SCHEMA_VERSION => Version::Newer(v),
        _ => Version::Unknown,
    }
}

/// Refuses a document a command other than `init` and publication cannot
/// use: one of a newer schema (upgrade instrukt) or an older one (migrate
/// it first). A document without a usable version is left to validation.
pub(crate) fn require_usable(layer: &Layer, path: &Path, value: &Value) -> Result<(), ConfigError> {
    match version(value) {
        Version::Current | Version::Unknown => Ok(()),
        Version::Newer(found) => Err(ConfigError::NewerSchema {
            path: path.to_path_buf(),
            found,
            current: CURRENT_SCHEMA_VERSION,
        }),
        Version::Older(found) => Err(ConfigError::OlderSchema {
            path: path.to_path_buf(),
            found,
            current: CURRENT_SCHEMA_VERSION,
            door: layer.init_door(),
        }),
    }
}

/// Whether reading `layer` through `home` persists its conversion: only the
/// machine's layer outside inspection. A project layer is committed to its
/// repository, so ordinary paths convert it in memory and leave the file as
/// committed; [`migrate`] (the `config` writing verbs and the pre-commit
/// hook) and [`patch`] are the paths that write it
/// (`docs/project/spec/feature/configuration/owned-files.md`).
fn persists_conversion(home: &Home, layer: &Layer) -> bool {
    !home.is_inspection() && matches!(layer, Layer::Global)
}

/// Reads a layer for a read command: absent is `None`; an unusable schema
/// version is refused.
pub(crate) fn read_usable(home: &Home, layer: &Layer) -> Result<Option<Value>, ConfigError> {
    if !persists_conversion(home, layer) {
        return Ok(inspect_layer(home, layer)?.value);
    }
    migrate_if_needed(home, layer)?;
    let path = layer.path(home);
    match read(&path)? {
        Some(doc) => {
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
    if read(&layer.path(home))?.is_some_and(|doc| matches!(version(&doc.value), Version::Older(_)))
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
pub(super) fn current_view(
    home: &Home,
    layer: &Layer,
    path: &Path,
    mut value: Value,
) -> Result<Value, ConfigError> {
    if let Version::Older(from) = version(&value) {
        super::_migrate::selection(home, layer, &mut value, false)?;
        value = migrate_value(value, from, layer.kind());
    }
    require_usable(layer, path, &value)?;
    Ok(value)
}
/// A current in-memory view, retaining the on-disk version and migration notice.
pub fn inspect_layer(home: &Home, layer: &Layer) -> Result<LayerInspection, ConfigError> {
    let path = layer.path(home);
    let document = read(&path)?;
    let mut view = LayerInspection {
        path: path.clone(),
        present: document.is_some(),
        schema_version: None,
        migration: None,
        value: None,
    };
    if let Some(document) = document {
        let mut value = document.value;
        view.schema_version = value.get("schema_version").and_then(Value::as_u64);
        if let Version::Older(from) = version(&value) {
            super::_migrate::selection(home, layer, &mut value, false)?;
            value = migrate_value(value, from, layer.kind());
            view.migration = Some(format!(
                "{}: schema_version {from} inspected as {}; file migrates on the next writing command",
                path.display(),
                CURRENT_SCHEMA_VERSION
            ));
        }
        require_usable(layer, &path, &value)?;
        view.value = Some(value);
    }
    Ok(view)
}

/// The current revision of `layer`.
pub fn revision(home: &Home, layer: &Layer) -> Result<Revision, ConfigError> {
    let persists = persists_conversion(home, layer);
    if persists {
        migrate_if_needed(home, layer)?;
    } else {
        inspect_layer(home, layer)?;
    }
    let path = layer.path(home);
    match read(&path)? {
        Some(doc) => {
            if persists {
                require_usable(layer, &path, &doc.value)?;
            }
            Ok(Revision::of(&doc.bytes))
        }
        None => Err(ConfigError::NoLayer {
            path,
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
    let started = Instant::now();
    let path = layer.path(home);
    let _lock = LockGuard::acquire(&lock_path(home, &path), WRITE_LOCK_TIMEOUT)?;
    let Some(mut doc) = read(&path)? else {
        return Err(ConfigError::NoLayer {
            path,
            door: layer.init_door(),
        });
    };
    let inspected_revision = Revision::of(&doc.bytes);
    if let Version::Older(from) = version(&doc.value) {
        super::_migrate::selection(home, layer, &mut doc.value, true)?;
        doc.value = migrate_value(doc.value, from, layer.kind());
        // The machine's layer keeps its conversion even when the patch is
        // refused; a project layer is converted only by the write that
        // applies the patch.
        if matches!(layer, Layer::Global) {
            write(&path, &doc.bytes, &doc.value)?;
            doc = read(&path)?.ok_or_else(|| ConfigError::NoLayer {
                path: path.clone(),
                door: layer.init_door(),
            })?;
        }
    }
    require_usable(layer, &path, &doc.value)?;
    let current = Revision::of(&doc.bytes);
    if &inspected_revision != expected {
        return Err(ConfigError::RevisionMismatch {
            expected: expected.to_string(),
            current: current.to_string(),
        });
    }
    let before = doc.value;
    let mut next = before.clone();
    merge_patch(&mut next, patch);
    let mut findings = findings_for(&home.for_inspection(), layer, &path, &next)?;
    // Todos already on stay a validation finding; only a write that
    // switches them on is refused for it.
    if layer.kind() == LayerKind::Project {
        let global = read_usable(home, &Layer::Global)?;
        if todos_on(&before, global.as_ref())? {
            findings.retain(|f| f.kind != FindingKind::TodosNeedGitHub);
        }
    }
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
    /// The document is at the current version; untouched.
    AlreadyCurrent,
    /// The document was rewritten from version `from`.
    Migrated { from: u64 },
}

/// Brings `layer` to the current schema version for a path that is not an
/// `instrukt config` writing verb — publication, installation, ticks: the
/// machine's layer is migrated on disk ([`migrate`]); a project layer is
/// checked and converted in memory only, its file left as committed. Either
/// way a newer or unknown version is refused.
pub fn bring_current(home: &Home, layer: &Layer) -> Result<(), ConfigError> {
    if persists_conversion(home, layer) {
        migrate(home, layer)?;
        return Ok(());
    }
    let path = layer.path(home);
    let Some(doc) = read(&path)? else {
        return Ok(());
    };
    match version(&doc.value) {
        Version::Current => Ok(()),
        Version::Older(_) => current_view(home, layer, &path, doc.value).map(drop),
        Version::Newer(found) => Err(ConfigError::NewerSchema {
            path,
            found,
            current: CURRENT_SCHEMA_VERSION,
        }),
        Version::Unknown => Err(ConfigError::UnknownSchema { path }),
    }
}

/// The project layer document `bytes` of the repository rooted at `root`,
/// converted to the current schema version and rendered as [`migrate`]
/// writes it; `None` when it is already
/// current. A newer or unknown version is refused. The pre-commit hook
/// converts the version a commit is being made from with this.
pub fn converted_project_layer(root: &Path, bytes: &[u8]) -> Result<Option<Vec<u8>>, ConfigError> {
    let path = root.join(CONFIG_FILE_NAME);
    let value = parse(&path, bytes)?;
    match version(&value) {
        Version::Current => Ok(None),
        Version::Newer(found) => Err(ConfigError::NewerSchema {
            path,
            found,
            current: CURRENT_SCHEMA_VERSION,
        }),
        Version::Unknown => Err(ConfigError::UnknownSchema { path }),
        Version::Older(from) => {
            let value = migrate_value(value, from, LayerKind::Project);
            render(&path, bytes, &value).map(Some)
        }
    }
}

/// Brings `layer` to the current schema version with its settings kept:
/// an older document is rewritten through the same atomic write as a
/// patch; a current one is left untouched; a newer one is refused.
/// `init`, the `config` writing verbs, the pre-commit hook and, for the
/// machine's layer, [`bring_current`] run this.
pub fn migrate(home: &Home, layer: &Layer) -> Result<MigrationOutcome, ConfigError> {
    let started = Instant::now();
    let path = layer.path(home);
    let _lock = LockGuard::acquire(&lock_path(home, &path), WRITE_LOCK_TIMEOUT)?;
    let Some(doc) = read(&path)? else {
        return Ok(MigrationOutcome::Absent);
    };
    match version(&doc.value) {
        Version::Current => Ok(MigrationOutcome::AlreadyCurrent),
        Version::Newer(found) => Err(ConfigError::NewerSchema {
            path,
            found,
            current: CURRENT_SCHEMA_VERSION,
        }),
        Version::Unknown => Err(ConfigError::UnknownSchema { path }),
        Version::Older(from) => {
            let mut migrated = doc.value;
            super::_migrate::selection(home, layer, &mut migrated, true)?;
            let migrated = migrate_value(migrated, from, layer.kind());
            let revision = write(&path, &doc.bytes, &migrated)?;
            tracing::debug!(
                event = "config_migrated",
                path = %path.display(),
                from_version = from,
                to_version = CURRENT_SCHEMA_VERSION,
                revision = %revision,
                duration_ms = elapsed_ms(started),
            );
            Ok(MigrationOutcome::Migrated { from })
        }
    }
}

/// Remove retired project settings and stamp the current owned schema version.
fn migrate_value(mut value: Value, from: u64, layer: LayerKind) -> Value {
    if let Value::Mapping(map) = &mut value {
        if from < 3 && layer == LayerKind::Project {
            map.remove("actions");
        }
        map.insert(
            Value::String("schema_version".to_owned()),
            Value::Number(CURRENT_SCHEMA_VERSION.into()),
        );
    }
    value
}

/// Creates `layer` with the current schema version (and, for a project,
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
        Value::Number(CURRENT_SCHEMA_VERSION.into()),
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
        schema_version = CURRENT_SCHEMA_VERSION,
        revision = %revision,
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
fn write(path: &Path, before: &[u8], value: &Value) -> Result<Revision, ConfigError> {
    let bytes = render(path, before, value)?;
    atomic_write(path, &bytes)?;
    Ok(Revision::of(&bytes))
}

/// The bytes [`write`] stores for `value` at `path` whose previous bytes
/// were `before`.
fn render(path: &Path, before: &[u8], value: &Value) -> Result<Vec<u8>, ConfigError> {
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

/// The lock serializing writers of the document at `path`. It lives in the
/// home, never beside a project's document.
fn lock_path(home: &Home, path: &Path) -> PathBuf {
    let canonical = path
        .parent()
        .and_then(|dir| dir.canonicalize().ok())
        .map(|dir| dir.join(CONFIG_FILE_NAME))
        .unwrap_or_else(|| path.to_path_buf());
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    home.locks_dir()
        .join(format!("config-{}.lock", hex::encode(&digest[..8])))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let newer = format!("schema_version: {}\n", CURRENT_SCHEMA_VERSION + 1);
        assert!(matches!(
            require_usable(&layer, path, &yaml(&newer)),
            Err(ConfigError::NewerSchema { .. })
        ));
        let current = format!("schema_version: {CURRENT_SCHEMA_VERSION}\n");
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
        let text = String::from_utf8(converted.clone()).unwrap();
        assert_eq!(text.lines().next(), Some(modeline));
        let value = yaml(&text);
        assert_eq!(
            value["schema_version"].as_u64(),
            Some(CURRENT_SCHEMA_VERSION)
        );
        assert!(value.get("actions").is_none());
        assert_eq!(converted_project_layer(root, &converted).unwrap(), None);
        let newer = format!("schema_version: {}\n", CURRENT_SCHEMA_VERSION + 1);
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
