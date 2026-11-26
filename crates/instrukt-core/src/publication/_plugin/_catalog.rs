//! The plugin's entry in the repository's plugin catalog, which is the
//! plugin's manifest: reading the catalog, the version the entry holds (from
//! an earlier rendering's manifest where the entry holds none), and the
//! catalog with the entry set, or created where the repository holds none.

use std::path::PathBuf;

use serde_json::Value as Json;

use super::super::_render::PlannedFile;
use super::super::_source::Committed;
use super::super::PublicationError;
use super::{CATALOG, CompanyName, PluginFinding, PluginVersion, folder, plugin_description};
use crate::ids::ProjectSlug;

/// The manifest an earlier rendering of the plugin of `key` wrote, from the
/// repository's root: read for its version alone, never written or removed.
pub(in crate::publication) fn earlier_manifest(key: &ProjectSlug) -> String {
    format!("plugins/{key}/.claude-plugin/plugin.json")
}

/// The repository's catalog, as far as it can be read.
pub(super) enum Catalog<'t> {
    /// The repository holds none.
    Missing,
    /// The catalog as read, with its bytes.
    Read(Json, &'t [u8]),
    /// A finding or a link leaves it unread.
    Unknown,
}

/// The version the catalog `document`'s entry for `key` holds: `None` when
/// it lists no entry of that name, or the finding that the entry's version
/// is not three numbers. An entry an earlier rendering wrote holds no
/// version: the version of the manifest that rendering left in `tree`
/// ([`earlier_manifest`]) is the one held, and without one that is three
/// numbers the entry's missing version is the finding.
pub(super) fn held_version(
    tree: &Committed,
    document: &Json,
    key: &ProjectSlug,
) -> Result<Option<PluginVersion>, PluginFinding> {
    let Some(entry) = document["plugins"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry.get("name").and_then(Json::as_str) == Some(key.as_str()))
    else {
        return Ok(None);
    };
    match entry.get("version") {
        Some(Json::String(text)) => {
            PluginVersion::parse(text)
                .map(Some)
                .ok_or_else(|| PluginFinding::EntryVersion {
                    version: Some(text.clone()),
                })
        }
        Some(other) => Err(PluginFinding::EntryVersion {
            version: Some(other.to_string()),
        }),
        None => tree
            .file(&earlier_manifest(key))
            .and_then(|bytes| serde_json::from_slice::<Json>(bytes).ok())
            .and_then(|manifest| manifest.get("version")?.as_str().map(PluginVersion::parse))
            .flatten()
            .map(Some)
            .ok_or(PluginFinding::EntryVersion { version: None }),
    }
}

/// `value` as one JSON document: indented by two spaces, ending in one
/// newline.
fn json_document(value: &Json, file: &str) -> Result<String, PublicationError> {
    serde_json::to_string_pretty(value)
        .map(|text| format!("{text}\n"))
        .map_err(|err| PublicationError::Rendering {
            file: file.to_owned(),
            reason: err.to_string(),
        })
}

/// The catalog the view holds, as read, with its bytes; `None` when it
/// holds none, or the finding that it is no JSON object holding a list of
/// plugins.
pub(super) fn read_catalog(tree: &Committed) -> Result<Option<(Json, &[u8])>, PluginFinding> {
    let Some(bytes) = tree.file(CATALOG) else {
        return Ok(None);
    };
    let unreadable = |reason: &str| PluginFinding::CatalogUnreadable {
        reason: reason.to_owned(),
    };
    let document = match serde_json::from_slice::<Json>(bytes) {
        Ok(document @ Json::Object(_)) => document,
        Ok(_) => return Err(unreadable("it is not a JSON object")),
        Err(err) => return Err(unreadable(&format!("it is not JSON: {err}"))),
    };
    if !matches!(document.get("plugins"), Some(Json::Array(_))) {
        return Err(unreadable("it holds no `plugins` list"));
    }
    Ok(Some((document, bytes)))
}

/// The catalog a repository without one receives: named by the company's
/// key, the company as its owner, listing no plugin yet.
pub(super) fn new_catalog(key: &ProjectSlug, name: &CompanyName) -> Json {
    serde_json::json!({
        "name": key.as_str(),
        "owner": { "name": name.as_str() },
        "plugins": [],
    })
}

/// The plugin's entry, which is its manifest: its name, the repository's
/// root as its source, its one skill, its version, its description and the
/// company as its author.
fn wanted_entry(
    key: &ProjectSlug,
    name: &CompanyName,
    version: &PluginVersion,
) -> [(&'static str, Json); 7] {
    [
        ("name", Json::from(key.as_str())),
        // The catalog's own repository, its root, as Claude's catalog
        // format names it.
        ("source", Json::from("./")),
        ("strict", Json::from(false)),
        ("skills", serde_json::json!([format!("./{}", folder(key))])),
        ("version", Json::from(version.to_string())),
        ("description", Json::from(plugin_description(name))),
        ("author", serde_json::json!({ "name": name.as_str() })),
    ]
}

/// The catalog `document`, read from `bytes` (`None`: the repository holds
/// none), with the plugin's entry set at `version`, as the file to write;
/// `None` when the entry already holds its values.
pub(super) fn catalog_entry(
    mut document: Json,
    bytes: Option<&[u8]>,
    key: &ProjectSlug,
    name: &CompanyName,
    version: &PluginVersion,
) -> Result<Option<PlannedFile>, PublicationError> {
    let Some(Json::Array(plugins)) = document.get_mut("plugins") else {
        return Ok(None);
    };
    let wanted = wanted_entry(key, name, version);
    let listed = plugins
        .iter_mut()
        .filter_map(Json::as_object_mut)
        .find(|entry| entry.get("name").and_then(Json::as_str) == Some(key.as_str()));
    match listed {
        Some(entry)
            if bytes.is_some()
                && wanted
                    .iter()
                    .all(|(member, value)| entry.get(*member) == Some(value)) =>
        {
            return Ok(None);
        }
        Some(entry) => {
            for (member, value) in wanted {
                entry.insert(member.to_owned(), value);
            }
        }
        None => plugins.push(Json::Object(
            wanted
                .into_iter()
                .map(|(member, value)| (member.to_owned(), value))
                .collect(),
        )),
    }
    Ok(Some(PlannedFile {
        path: PathBuf::from(CATALOG),
        text: json_document(&document, CATALOG)?,
        current: bytes.map(<[u8]>::to_vec),
    }))
}
