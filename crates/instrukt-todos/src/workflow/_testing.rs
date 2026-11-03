//! Test fixtures shared by the engine's colocated tests.

use std::path::PathBuf;

use instrukt_core::ids::{CommitSha, ProjectSlug, TodoSlug};
use tempfile::TempDir;

use super::_derive::Subject;
use super::_facts::{FactValue, Facts};
use super::_render::ConfigLookupError;

/// The sdlc flavor's authoring source in this repository.
pub(crate) fn sdlc_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/global/sdlc/workflow")
}

/// A flavor directory holding `files` (relative path, content).
pub(crate) fn flavor_dir(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, content) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    dir
}

pub(crate) fn subject(facts: Facts) -> Subject {
    Subject {
        project: ProjectSlug::parse("demo").unwrap(),
        slug: TodoSlug::parse("add-login").unwrap(),
        facts,
    }
}

pub(crate) fn facts(entries: &[(&str, FactValue)]) -> Facts {
    let mut facts = Facts::new();
    for (name, value) in entries {
        facts.insert(name, value.clone()).unwrap();
    }
    facts
}

pub(crate) fn text(value: &str) -> FactValue {
    FactValue::Text(value.to_owned())
}

pub(crate) fn sha(fill: char) -> CommitSha {
    CommitSha::parse(&fill.to_string().repeat(40)).unwrap()
}

/// Configuration with nothing in it.
pub(crate) fn no_config(_: &str) -> Result<Option<String>, ConfigLookupError> {
    Ok(None)
}

/// Configuration inside a runtime session `sess-1`.
pub(crate) fn session_config(path: &str) -> Result<Option<String>, ConfigLookupError> {
    Ok((path == "runtime.session_id").then(|| "sess-1".to_owned()))
}
