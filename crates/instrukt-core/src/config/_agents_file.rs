//! The agents file of an instrukt home (`Home::agents_file`) on disk: what
//! the next writing command does to it, its read and the revision of its
//! stored bytes, and every write to it under its one lock — the entry's
//! convergence, and the revision-checked patch that keeps the file to its
//! overrides of the shipped defaults
//! (`project/instrukt/spec/feature/configuration/configuration-editing`,
//! Agents file).
use super::_agents::{Agents, VERSION, overrides_only};
use super::_merge::merge_patch;
use super::{ConfigError, Layer, Revision};
use crate::{
    fsx::{LockGuard, atomic_write},
    home::Home,
};
use serde::Deserialize;
use serde_yaml_ng::{Mapping, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const DEPLOYMENT_VERSION: u64 = 1;

/// How long a writer of the agents file waits for another.
const LOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// The lock every writer of the agents file holds for its read-modify-write.
pub(super) fn agents_lock(home: &Home) -> PathBuf {
    home.locks_dir().join("agents.lock")
}

/// The revision of the agents file whose stored bytes are `bytes`; `None`,
/// no file, has the absent file's revision, the same on every read.
fn revision_of(bytes: Option<&[u8]>) -> Revision {
    Revision::of(bytes.unwrap_or_default())
}

/// The revision of the agents file's stored bytes; without a file, the
/// absent file's revision, whatever conversion or migration is pending.
/// Nothing is locked or written.
pub fn agents_revision(home: &Home) -> Result<Revision, ConfigError> {
    Ok(revision_of(read_existing(&home.agents_file())?.as_deref()))
}

/// The value at dotted `path` of the effective agent selection
/// ([`agents_document`]), or the whole selection without a path, literally:
/// nothing is interpolated. A path that does not exist fails, naming the
/// missing segment. Nothing is locked or written.
pub fn agents_value(home: &Home, path: Option<&str>) -> Result<Value, ConfigError> {
    let document = agents_document(home)?;
    let Some(path) = path else {
        return Ok(document);
    };
    let segments: Vec<&str> = path.split('.').collect();
    let (_, value) = super::_interpolate::at_path(&document, &segments, path)?;
    Ok(value.clone())
}

/// The effective agent selection as every reading command sees the file —
/// an older file converted and an older global layer's selection carried
/// over in memory, merged over the shipped defaults — validated, as literal
/// data. Nothing is locked or written.
fn agents_document(home: &Home) -> Result<Value, ConfigError> {
    let stored = read_existing(&home.agents_file())?;
    let global = super::_layer::read(&home.global_config())?.map(|document| document.value);
    let text = Agents::selection_text(stored, global.as_ref())?;
    let document = Agents::effective_document(&text)?;
    Agents::validated(document.clone())?;
    Ok(document)
}

/// Applies merge `patch` to the agents file's overrides, only while its
/// stored bytes still have revision `expected` and the resulting effective
/// selection is valid, and stores only what differs from the shipped
/// defaults: the file is written atomically, or removed when no override
/// remains. Returns the new revision. A patch that changes the file makes
/// every other patch from `expected` stale; one that leaves the stored bytes
/// as they were (such as an empty patch on an absent file) leaves `expected`
/// current.
///
/// As a layer patch does with the machine's layer, the patch first persists,
/// under the same locks, what the next writing command does to the file —
/// its shape conversion or recognised-copy removal, and an older global
/// layer's selection carried into it — and keeps that when it refuses.
pub fn patch_agents(
    home: &Home,
    expected: &Revision,
    patch: &Value,
) -> Result<Revision, ConfigError> {
    if !patch.is_mapping() {
        return Err(ConfigError::PatchNotAMapping);
    }
    let started = Instant::now();
    // Lock order: the global layer's lock, then the agents file's.
    let global = super::_layer::lock_if_older(home, &Layer::Global)?;
    let agents = LockGuard::acquire(&agents_lock(home), LOCK_TIMEOUT)?;
    let path = home.agents_file();
    let inspected = revision_of(read_existing(&path)?.as_deref());
    converge_holding(home, &agents)?;
    if let Some(ref global_lock) = global {
        super::_layer::migrate_holding(home, &Layer::Global, global_lock, Some(&agents))?;
    }
    let before = read_existing(&path)?;
    if &inspected != expected {
        return Err(ConfigError::RevisionMismatch {
            expected: expected.to_string(),
            current: revision_of(before.as_deref()).to_string(),
        });
    }
    let (revision, removed) = match stored_overrides(before.as_deref(), patch, &path)? {
        None => {
            match crate::fsx::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(crate::fsx::FsError::Write { path, source }.into());
                }
            }
            (revision_of(None), true)
        }
        Some(reduced) => (
            super::_layer::write(&path, before.as_deref().unwrap_or_default(), &reduced)?,
            false,
        ),
    };
    tracing::debug!(
        event = "agents_patched",
        from_revision = %expected,
        revision = %revision,
        removed,
        duration_ms = super::_layer::elapsed_ms(started),
    );
    Ok(revision)
}

/// Applies `patch` to `before` (or a fresh `{schema_version: VERSION}` document
/// when absent), validates the resulting effective selection, and reduces to
/// overrides only; returns `None` when nothing beyond `schema_version` remains.
fn stored_overrides(
    before: Option<&[u8]>,
    patch: &Value,
    path: &Path,
) -> Result<Option<Value>, ConfigError> {
    let mut overrides = match before {
        Some(bytes) => {
            let parsed: Value =
                serde_yaml_ng::from_slice(bytes).map_err(|source| ConfigError::Parse {
                    path: path.to_path_buf(),
                    source,
                })?;
            match parsed {
                Value::Null => Value::Mapping(Mapping::new()),
                Value::Mapping(_) => parsed,
                _ => {
                    return Err(ConfigError::NotAMapping {
                        path: path.to_path_buf(),
                    });
                }
            }
        }
        None => {
            let mut doc = Mapping::new();
            doc.insert("schema_version".into(), VERSION.into());
            Value::Mapping(doc)
        }
    };
    merge_patch(&mut overrides, patch);
    let text = super::render_yaml(&overrides).map_err(|source| ConfigError::Render {
        path: path.to_path_buf(),
        source,
    })?;
    let _: Agents = Agents::parse(&text)?;
    let reduced = overrides_only(overrides)?;
    let empty = reduced
        .as_mapping()
        .is_some_and(|fields| fields.keys().all(|k| k.as_str() == Some("schema_version")));
    Ok((!empty).then_some(reduced))
}

fn converge_holding(home: &Home, _agents: &LockGuard) -> Result<(), ConfigError> {
    let path = home.agents_file();
    let state = home.agents_deployment_file();
    let Some(current) = read_existing(&path)? else {
        return Ok(());
    };
    match next_write(&current, Deployment::load(&state).as_ref())? {
        NextWrite::Keep => {}
        NextWrite::Remove => remove_copy(&path, &state)?,
        NextWrite::Convert(converted) => {
            atomic_write(&path, &converted.bytes)?;
            tracing::info!(
                event = "agents_shape_migrated",
                from_version = converted.from,
                to_version = VERSION
            );
        }
    }
    Ok(())
}

impl Agents {
    /// What a writing command does to `agents.yaml` before reading it. A full
    /// copy of shipped defaults the deployment record recognises as unedited
    /// is removed, and then the record; any other file is kept and has a due
    /// shape conversion persisted. Removal comes first, so a recognised copy
    /// is never converted and kept.
    pub fn converge_file(home: &Home) -> Result<(), ConfigError> {
        let path = home.agents_file();
        let state = home.agents_deployment_file();
        let Some(before) = read_existing(&path)? else {
            return Ok(());
        };
        if next_write(&before, Deployment::load(&state).as_ref())? == NextWrite::Keep {
            return Ok(());
        }
        let lock = LockGuard::acquire(&agents_lock(home), LOCK_TIMEOUT)?;
        converge_holding(home, &lock)
    }

    /// What a reading command reports about `agents.yaml`: the change the
    /// next writing command makes to it, if any, without making it.
    pub fn inspection_notice(home: &Home) -> Result<Option<String>, ConfigError> {
        let path = home.agents_file();
        let Some(bytes) = read_existing(&path)? else {
            return Ok(None);
        };
        let state = home.agents_deployment_file();
        Ok(
            match next_write(&bytes, Deployment::load(&state).as_ref())? {
                NextWrite::Keep => None,
                NextWrite::Remove => Some(format!(
                    "{}: an unedited copy of earlier shipped defaults; the next writing command removes it and the shipped defaults apply",
                    path.display()
                )),
                NextWrite::Convert(_) => Some(format!(
                    "{}: selection configuration inspected in memory; file migrates on the next writing command",
                    path.display()
                )),
            },
        )
    }
}

/// What the next writing command does to `agents.yaml`.
#[derive(Debug, PartialEq, Eq)]
enum NextWrite {
    /// Nothing for this operation: the file is not a recognised copy and no
    /// shape conversion applies. Whether it is valid is for [`Agents::parse`]
    /// to decide.
    Keep,
    /// Remove the file and then the deployment record: the record recognises
    /// the file as a copy an earlier release wrote and nobody edited.
    Remove,
    /// Persist the file's own bytes at the current shape.
    Convert(super::_agents_migrate::Converted),
}

/// The decision for `file` beside `record`, the readable deployment record
/// (`None` when there is none, or it is unreadable). Removal comes first, so
/// a recognised copy is never converted and kept.
fn next_write(file: &[u8], record: Option<&Deployment>) -> Result<NextWrite, ConfigError> {
    if record.is_some_and(|record| record.recognises(file)) {
        return Ok(NextWrite::Remove);
    }
    Ok(match super::_agents_migrate::current(file)? {
        Some(converted) => NextWrite::Convert(converted),
        None => NextWrite::Keep,
    })
}

/// Removes a recognised copy at `path`, then its record at `state`.
fn remove_copy(path: &Path, state: &Path) -> Result<(), ConfigError> {
    for removed in [path, state] {
        match crate::fsx::remove_file(removed) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(crate::fsx::FsError::Write {
                    path: removed.to_path_buf(),
                    source,
                }
                .into());
            }
        }
    }
    tracing::info!(event = "agents_defaults_copy_removed");
    Ok(())
}

/// `agents.yaml`'s bytes, or `None` when there is no file.
pub(super) fn read_existing(path: &Path) -> Result<Option<Vec<u8>>, ConfigError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ConfigError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// What an ignored deployment record leaves, and how an operator returns to
/// the shipped defaults: told whether the record could not be read or could
/// not be understood.
const IGNORED_RECORD_RECOVERY: &str = "the record is ignored: agents.yaml stays as the user's overrides, still converted to the current shape; to return to the shipped defaults, remove agents.yaml together with this record once you have confirmed it holds no edits you want";

#[derive(Deserialize)]
struct Deployment {
    schema_version: u64,
    digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<String>,
}

impl Deployment {
    /// The record an earlier release kept of the defaults it deployed; `None`
    /// when there is none. A record that cannot be read or is in an unknown
    /// shape recognises nothing: the file beside it is kept and the command
    /// proceeds, because the record is only bookkeeping about instrukt's own
    /// earlier write and the person's file is never removed on a guess.
    fn load(state: &Path) -> Option<Self> {
        let bytes = match std::fs::read(state) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                tracing::warn!(
                    event = "agents_deployment_record_unreadable",
                    record = %state.display(),
                    %error,
                    "{IGNORED_RECORD_RECOVERY}"
                );
                return None;
            }
        };
        match Self::parse(&bytes) {
            Ok(record) => Some(record),
            Err(cause) => {
                tracing::warn!(
                    event = "agents_deployment_record_invalid",
                    record = %state.display(),
                    %cause,
                    "{IGNORED_RECORD_RECOVERY}"
                );
                None
            }
        }
    }

    /// A deployment record from its bytes, or why it recognises nothing.
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let record: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if record.schema_version != DEPLOYMENT_VERSION {
            return Err(format!(
                "schema_version {} is not the supported {DEPLOYMENT_VERSION}",
                record.schema_version
            ));
        }
        Ok(record)
    }

    /// Whether `bytes` are what an earlier release itself wrote: the deployed
    /// defaults, or a write it recorded before making it.
    fn recognises(&self, bytes: &[u8]) -> bool {
        let found = hex::encode(Sha256::digest(bytes));
        self.digest == found || self.pending.as_deref() == Some(found.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{_agents::VERSION, SHIPPED_AGENTS};
    use serde_yaml_ng::Value;
    use std::path::Path;

    /// The deployment record an earlier release kept of `bytes`.
    fn record_of(bytes: &str) -> super::Deployment {
        use sha2::Digest as _;
        let record = serde_json::json!({
            "schema_version": 1,
            "digest": hex::encode(sha2::Sha256::digest(bytes.as_bytes())),
        });
        super::Deployment::parse(record.to_string().as_bytes()).unwrap()
    }

    /// The shipped defaults in the shape an earlier release wrote: the split
    /// orchestrator role the current shape retires.
    fn older_shape(text: &str) -> String {
        text.replacen("  todo-orchestrator:", "  work-orchestrator:", 1)
    }

    #[test]
    fn a_recognised_older_copy_is_removed_rather_than_converted() {
        let older = older_shape(SHIPPED_AGENTS);
        assert_eq!(
            super::next_write(older.as_bytes(), Some(&record_of(&older))).unwrap(),
            super::NextWrite::Remove
        );
    }

    #[test]
    fn an_edited_older_file_is_converted_and_kept() {
        let edited = format!("{}# my own note\n", older_shape(SHIPPED_AGENTS));
        let decided = super::next_write(
            edited.as_bytes(),
            Some(&record_of(&older_shape(SHIPPED_AGENTS))),
        )
        .unwrap();
        assert!(matches!(decided, super::NextWrite::Convert(_)));
    }

    #[test]
    fn an_edited_current_file_is_kept_as_it_is() {
        let edited = format!("{SHIPPED_AGENTS}# my own note\n");
        assert_eq!(
            super::next_write(edited.as_bytes(), Some(&record_of(SHIPPED_AGENTS))).unwrap(),
            super::NextWrite::Keep
        );
    }

    #[test]
    fn a_record_in_an_unknown_shape_recognises_nothing() {
        use sha2::Digest as _;
        let record = serde_json::json!({
            "schema_version": 2,
            "digest": hex::encode(sha2::Sha256::digest(SHIPPED_AGENTS.as_bytes())),
        });
        assert!(super::Deployment::parse(record.to_string().as_bytes()).is_err());
        assert!(super::Deployment::parse(b"{not a record").is_err());
    }

    fn yaml(text: &str) -> Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    #[test]
    fn a_patch_removing_the_last_override_leaves_nothing_to_store() {
        let before = format!("schema_version: {VERSION}\nroles: {{planner: {{order: [codex]}}}}\n");
        let patch = yaml("roles: {planner: {order: ~}}");
        let stored =
            super::stored_overrides(Some(before.as_bytes()), &patch, Path::new("agents.yaml"))
                .unwrap();
        assert_eq!(stored, None);
    }

    #[test]
    fn a_patch_on_no_file_stores_only_what_differs_from_the_shipped_defaults() {
        let shipped = yaml(SHIPPED_AGENTS);
        let builder = &shipped["roles"]["builder"];
        let mut reversed = builder["order"].as_sequence().unwrap().clone();
        reversed.reverse();
        let mut patch = yaml("roles: {builder: {}}");
        patch["roles"]["builder"]["tier"] = builder["tier"].clone();
        patch["roles"]["builder"]["order"] = Value::Sequence(reversed.clone());
        let mut expected = yaml(&format!(
            "schema_version: {VERSION}\nroles: {{builder: {{}}}}"
        ));
        expected["roles"]["builder"]["order"] = Value::Sequence(reversed);
        let stored = super::stored_overrides(None, &patch, Path::new("agents.yaml")).unwrap();
        assert_eq!(stored, Some(expected));
    }

    fn temp_home() -> (tempfile::TempDir, crate::home::Home) {
        let dir = tempfile::tempdir().unwrap();
        let home = crate::home::Home::at_user_home(dir.path().to_path_buf());
        std::fs::create_dir_all(home.root()).unwrap();
        (dir, home)
    }

    #[test]
    fn absent_agents_revision_stays_stable_when_an_older_global_layer_carries_a_selection() {
        let (_dir, home) = temp_home();
        let absent = super::agents_revision(&home).unwrap();
        std::fs::write(
            home.global_config(),
            "schema_version: 1\nagents:\n  order: [codex, claude]\n",
        )
        .unwrap();
        assert_eq!(super::agents_revision(&home).unwrap(), absent);
        assert!(!home.agents_file().exists());
    }

    #[test]
    fn patching_from_a_stale_revision_is_refused_and_leaves_the_file_unchanged() {
        let (_dir, home) = temp_home();
        let absent = super::agents_revision(&home).unwrap();
        let current =
            super::patch_agents(&home, &absent, &yaml("roles: {builder: {tier: light}}")).unwrap();
        let stored = std::fs::read(home.agents_file()).unwrap();

        let refused =
            super::patch_agents(&home, &absent, &yaml("roles: {builder: {tier: strong}}"));
        assert!(
            matches!(
                &refused,
                Err(super::ConfigError::RevisionMismatch { expected, current: found })
                    if expected == absent.as_str() && found == current.as_str()
            ),
            "{refused:?}"
        );
        assert_eq!(std::fs::read(home.agents_file()).unwrap(), stored);
    }

    #[test]
    fn patching_with_an_invalid_profile_is_refused_and_writes_no_file() {
        let (_dir, home) = temp_home();
        let absent = super::agents_revision(&home).unwrap();
        let refused = super::patch_agents(
            &home,
            &absent,
            &yaml("roles: {builder: {order: [missing]}}"),
        );
        assert!(
            matches!(refused, Err(super::ConfigError::InvalidValue { .. })),
            "{refused:?}"
        );
        assert!(!home.agents_file().exists());
        assert_eq!(super::agents_revision(&home).unwrap(), absent);
    }

    #[test]
    fn of_two_concurrent_patches_from_the_absent_revision_over_an_older_global_layer_exactly_one_lands()
     {
        let (_dir, home) = temp_home();
        std::fs::write(
            home.global_config(),
            "schema_version: 1\nagents:\n  order: [codex, claude]\n",
        )
        .unwrap();
        let rev = super::agents_revision(&home).unwrap();
        let outcomes: Vec<Result<super::Revision, super::ConfigError>> =
            std::thread::scope(|scope| {
                let handles: Vec<_> = ["builder", "planner"]
                    .into_iter()
                    .map(|role| {
                        let (home, rev) = (&home, &rev);
                        scope.spawn(move || {
                            super::patch_agents(
                                home,
                                rev,
                                &yaml(&format!("roles: {{{role}: {{tier: light}}}}")),
                            )
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });
        assert_eq!(
            outcomes.iter().filter(|r| r.is_ok()).count(),
            1,
            "{outcomes:?}"
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|r| matches!(r, Err(super::ConfigError::RevisionMismatch { .. })))
                .count(),
            1,
            "{outcomes:?}"
        );
        let global = yaml(&std::fs::read_to_string(home.global_config()).unwrap());
        assert_eq!(
            global["schema_version"],
            Value::from(super::super::GLOBAL_SCHEMA_VERSION)
        );
        assert_eq!(global.get("agents"), Some(&yaml("{}")));
        let stored = yaml(&std::fs::read_to_string(home.agents_file()).unwrap());
        assert_eq!(stored["schema_version"], Value::from(VERSION));
        assert_eq!(stored["default"], yaml("{order: [codex, claude]}"));
        assert_eq!(stored["roles"].as_mapping().map(|m| m.len()), Some(1));
    }

    #[test]
    fn patching_a_v3_full_copy_converts_it_and_prunes_to_overrides() {
        let (_dir, home) = temp_home();
        let mut full = yaml(SHIPPED_AGENTS);
        full["schema_version"] = Value::from(3_u64);
        let (other_role, reversed_order) = full["roles"]
            .as_mapping()
            .unwrap()
            .iter()
            .find_map(|(name, entry)| {
                let role = name.as_str()?;
                if role == "builder" {
                    return None;
                }
                let order = entry.get("order")?.as_sequence()?;
                if order.len() < 2 {
                    return None;
                }
                let mut reversed = order.clone();
                reversed.reverse();
                Some((role.to_owned(), reversed))
            })
            .unwrap();
        full["roles"][other_role.as_str()]["order"] = Value::Sequence(reversed_order.clone());
        std::fs::write(home.agents_file(), serde_yaml_ng::to_string(&full).unwrap()).unwrap();

        let rev = super::agents_revision(&home).unwrap();
        super::patch_agents(
            &home,
            &rev,
            &yaml("roles: {builder: {order: [antigravity]}}"),
        )
        .unwrap();

        let mut expected = yaml(&format!(
            "schema_version: {VERSION}\nroles: {{builder: {{order: [antigravity]}}}}\n"
        ));
        expected["roles"][other_role.as_str()] = yaml("{}");
        expected["roles"][other_role.as_str()]["order"] = Value::Sequence(reversed_order);
        assert_eq!(
            yaml(&std::fs::read_to_string(home.agents_file()).unwrap()),
            expected
        );
    }

    #[test]
    fn agents_value_reads_dotted_paths_literally_without_interpolating_references() {
        let (_dir, home) = temp_home();
        let rev = super::agents_revision(&home).unwrap();
        super::patch_agents(
            &home,
            &rev,
            &yaml("profiles: {codex: {standard: [{model: 'gpt-{project}', effort: high}]}}"),
        )
        .unwrap();

        assert_eq!(
            super::agents_value(&home, Some("profiles.codex.standard.0.model")).unwrap(),
            Value::String("gpt-{project}".into())
        );
        assert!(matches!(
            super::agents_value(&home, Some("roles.missing")),
            Err(super::ConfigError::MissingKey { segment, .. }) if segment == "missing"
        ));
    }
}
