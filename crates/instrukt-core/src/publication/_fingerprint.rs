//! The eval fingerprint: the digest of the doctrine in force, derived from a
//! prose digest over every published snippet that declares `eval: true` and
//! every always-on source, and a units digest over the always-on sources'
//! current compressed units, recorded with both per published commit
//! (`project/instrukt/spec/feature/publication/eval-fingerprint`).

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::_units::{self, UnitFinding};
use super::PublicationError;
use crate::docs::{
    self, Corpus, GENERAL, INDEX_FILE, Lens, Position, PositionIndex, RefTarget, SourceFile, View,
    published_origin,
};
use crate::home::Home;
use crate::ids::{CommitSha, DomainName, ProjectSlug, SnippetId};
use crate::index::{GlobalIndex, IndexScope};

/// The name of the record of the current fingerprint.
const CURRENT: &str = "current.yaml";

/// What the fingerprint and each digest start from, so an empty corpus has a
/// defined fingerprint of its own and no other scheme collides with this one.
const SCHEME: &[u8] = b"instrukt-eval-fingerprint-v2\n";

/// What the units digest holds for an always-on source with no current
/// unit.
const NO_UNIT: &[u8] = b"none";

/// The eval fingerprint one publishing sync computed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalFingerprint {
    /// `sha256:<hex>`.
    pub fingerprint: String,
    /// How many `eval: true` snippets it covers.
    pub snippets: usize,
    /// The prose digest, `sha256:<hex>`; `None` in a record written before
    /// the fingerprint kept it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prose: Option<String>,
    /// The units digest, `sha256:<hex>`; `None` in a record written before
    /// the fingerprint kept it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<String>,
    /// The project whose sync computed it, and the commit it published.
    pub project: ProjectSlug,
    pub commit: CommitSha,
    /// When the publishing sync recorded it, by the invocation's clock;
    /// `None` in a record written before the time was kept, whose place
    /// in the order of records is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<jiff::Timestamp>,
}

impl EvalFingerprint {
    /// The fingerprint of the doctrine in force: the one the latest
    /// publishing sync recorded. `None` before the first sync.
    pub fn current(home: &Home) -> Result<Option<Self>, PublicationError> {
        read(home.eval_fingerprint_dir().join(CURRENT))
    }

    /// The fingerprint recorded when `commit` was published.
    pub fn for_commit(home: &Home, commit: &CommitSha) -> Result<Option<Self>, PublicationError> {
        read(record_path(home, commit))
    }

    /// Every per-commit record: the fingerprint each published commit was
    /// recorded with, in commit order of their names. None before the first
    /// publishing sync.
    pub fn recorded(home: &Home) -> Result<Vec<Self>, PublicationError> {
        let dir = home.eval_fingerprint_dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(PublicationError::Io { path: dir, source }),
        };
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| PublicationError::Io {
                path: dir.clone(),
                source,
            })?;
            let path = entry.path();
            let is_commit = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".yaml"))
                .is_some_and(|stem| CommitSha::parse(stem).is_ok());
            if is_commit {
                paths.push(path);
            }
        }
        paths.sort();
        let mut out = Vec::new();
        for path in paths {
            if let Some(record) = read(path)? {
                out.push(record);
            }
        }
        Ok(out)
    }
}

fn record_path(home: &Home, commit: &CommitSha) -> PathBuf {
    home.eval_fingerprint_dir()
        .join(format!("{}.yaml", commit.as_str()))
}

fn read(path: PathBuf) -> Result<Option<EvalFingerprint>, PublicationError> {
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(PublicationError::Io { path, source }),
    };
    serde_yaml_ng::from_str(&text)
        .map(Some)
        .map_err(|source| PublicationError::Parse { path, source })
}

/// Which record a sync decides about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Record {
    /// The record of the commit the sync publishes.
    PerCommit,
    /// The record of the fingerprint in force.
    Current,
}

/// The record a sync writes in place of `existing`, given the fingerprint
/// it `computed` (which carries no time); `None` when `existing` already
/// holds that content and stays as it is, its time included. The per-commit
/// record holds the content when its fingerprint and snippet count are the
/// computed ones; the current record when its fingerprint is, whichever
/// project and commit it names; either only when its two digests are the
/// computed ones too. A record written takes the sync's time.
fn decide(
    record: Record,
    existing: Option<&EvalFingerprint>,
    computed: &EvalFingerprint,
    now: jiff::Timestamp,
) -> Option<EvalFingerprint> {
    let current = existing.is_some_and(|existing| {
        let digests = existing.fingerprint == computed.fingerprint
            && existing.prose == computed.prose
            && existing.units == computed.units;
        match record {
            Record::PerCommit => digests && existing.snippets == computed.snippets,
            Record::Current => digests,
        }
    });
    (!current).then(|| EvalFingerprint {
        recorded_at: Some(now),
        ..computed.clone()
    })
}

/// The records a sync writes, as `(path, contents)`: of the per-commit one
/// and the current one, each that does not already hold what was computed.
/// A record that is a symbolic link holds nothing of publication's: it is
/// written, and the write replaces the link.
pub(super) fn records(
    home: &Home,
    computed: &EvalFingerprint,
    now: jiff::Timestamp,
) -> Result<Vec<(PathBuf, Vec<u8>)>, PublicationError> {
    let [per_commit, current] = record_paths(home, &computed.commit);
    let mut writes = Vec::new();
    for (record, path) in [(Record::PerCommit, per_commit), (Record::Current, current)] {
        let existing = if is_link(&path)? {
            None
        } else {
            read(path.clone())?
        };
        if let Some(next) = decide(record, existing.as_ref(), computed, now) {
            let text = serde_yaml_ng::to_string(&next).map_err(PublicationError::Render)?;
            writes.push((path, text.into_bytes()));
        }
    }
    Ok(writes)
}

/// Whether `path` is itself a symbolic link.
fn is_link(path: &std::path::Path) -> Result<bool, PublicationError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => Ok(meta.file_type().is_symlink()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(PublicationError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// The records a sync publishing `commit` writes: the per-commit one and
/// the current one.
pub(super) fn record_paths(home: &Home, commit: &CommitSha) -> [PathBuf; 2] {
    [
        record_path(home, commit),
        home.eval_fingerprint_dir().join(CURRENT),
    ]
}

/// Computes the fingerprint over the published corpus of the home as
/// `index` describes it, from the home's published bytes alone: every
/// global entry's file read from the home, the evaluated ones those whose
/// frontmatter declares `eval: true`, the always-on sources those the
/// published `docs/general/index.yaml` selects with their closure, and each
/// always-on source's unit from its position's `_compressed/`, current by the
/// source digest the index records for the source. A unit
/// unreadable or behind a link counts as none and is returned as a finding.
/// It carries no time: [`records`] decides whether a record is written, and
/// when.
pub(super) fn compute(
    home: &Home,
    index: &GlobalIndex,
    project: &ProjectSlug,
    commit: &CommitSha,
) -> Result<(EvalFingerprint, Vec<UnitFinding>), PublicationError> {
    let docs = home.docs_dir();
    let mut sources = Vec::new();
    for entry in index
        .snippets
        .iter()
        .filter(|e| e.scope == IndexScope::Global)
    {
        let path = docs.join(&entry.path);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            // Another project's entry whose file is gone is not part of the
            // published corpus; its owner's next sync drops the entry.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(
                    event = "eval_fingerprint_entry_missing",
                    id = %entry.id,
                    path = %path.display(),
                );
                continue;
            }
            Err(source) => return Err(PublicationError::Io { path, source }),
        };
        // The corpus reads the file at the repository path it was published
        // from, which is where its identity follows from.
        let Some(origin) = published_origin(&entry.path.to_string_lossy()) else {
            continue;
        };
        sources.push(SourceFile { path: origin, text });
    }
    let corpus = Corpus::from_sources(None, Some(home), sources.iter().cloned());
    let texts: std::collections::BTreeMap<&str, &str> = sources
        .iter()
        .map(|s| (s.path.as_str(), s.text.as_str()))
        .collect();
    let raw = |s: &docs::Snippet| texts.get(s.path().to_str()?).map(|t| t.as_bytes());
    let evaluated: BTreeSet<&SnippetId> = corpus
        .snippets()
        .iter()
        .filter(|s| s.current().frontmatter().eval && raw(s).is_some())
        .map(docs::Snippet::id)
        .collect();
    let always_on = always_on(home, &corpus)?;
    let prose: Vec<(&SnippetId, &[u8])> = evaluated
        .iter()
        .copied()
        .chain(always_on.iter())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|id| Some((id, raw(corpus.snippet(id)?)?)))
        .collect();
    let mut degraded = Vec::new();
    let mut bodies: Vec<(SnippetId, Vec<u8>)> = Vec::new();
    for id in &always_on {
        let (Some(domain), Some((_, rel))) = (id.domain(), id.as_str().split_once('/')) else {
            continue;
        };
        // A unit is current by the source digest the source's owner recorded
        // in the index when it published it, never by digesting the prose
        // again.
        let recorded = index
            .snippets
            .iter()
            .find(|entry| &entry.id == id)
            .and_then(|entry| entry.content_sha256.as_deref());
        let read = _units::read_home(home, &domain, &format!("{rel}.md"));
        let held = match read {
            Some(Err(finding)) => {
                tracing::warn!(
                    event = "eval_fingerprint_unit_degraded",
                    unit = %finding.path,
                    finding = finding.kind.code(),
                );
                degraded.push(finding);
                NO_UNIT.to_vec()
            }
            Some(Ok(unit)) if Some(unit.source_sha256.as_str()) == recorded => {
                unit.body.into_bytes()
            }
            Some(Ok(_)) | None => NO_UNIT.to_vec(),
        };
        bodies.push((id.clone(), held));
    }
    let units: Vec<(&SnippetId, &[u8])> = bodies
        .iter()
        .map(|(id, body)| (id, body.as_slice()))
        .collect();
    let (prose, units) = (digest(b"prose\n", &prose), digest(b"units\n", &units));
    Ok((
        EvalFingerprint {
            fingerprint: fingerprint(&prose, &units),
            snippets: evaluated.len(),
            prose: Some(prose),
            units: Some(units),
            project: project.clone(),
            commit: commit.clone(),
            recorded_at: None,
        },
        degraded,
    ))
}

/// The always-on sources the home publishes: every published snippet the
/// published `docs/general/index.yaml` selects — its `expanded` ids and the
/// ids of every `agents` list — with their transitive required reads, a
/// bare-domain read as that domain's published `expanded` set; delivered
/// only.
fn always_on(home: &Home, corpus: &Corpus) -> Result<BTreeSet<SnippetId>, PublicationError> {
    let index =
        |position: &Position, dir: PathBuf| -> Result<Option<PositionIndex>, PublicationError> {
            let path = dir.join(INDEX_FILE);
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(source) => return Err(PublicationError::Io { path, source }),
            };
            PositionIndex::parse(position, &text)
                .map(Some)
                .map_err(|source| PublicationError::Parse { path, source })
        };
    let mut seen = BTreeSet::new();
    let Some(PositionIndex::General(general)) =
        index(&Position::General, home.docs_dir().join(GENERAL))?
    else {
        return Ok(seen);
    };
    let mut pending: Vec<SnippetId> = general
        .expanded
        .iter()
        .chain(general.agents.values().flatten())
        .rev()
        .cloned()
        .collect();
    let mut domains: BTreeSet<DomainName> = BTreeSet::new();
    let mut found = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(snippet) = corpus.snippet(&id) else {
            continue;
        };
        let View::Visible { version, .. } = snippet.view(Lens::Current) else {
            continue;
        };
        found.insert(id.clone());
        for read in docs::lens_required_reads(version.body(), Lens::Current, None) {
            match read.target {
                Some(RefTarget::Snippet(nested)) => pending.push(nested),
                Some(RefTarget::Domain(domain)) if domains.insert(domain.clone()) => {
                    if let Some(index) =
                        index(&Position::of_domain(&domain), home.domain_dir(&domain))?
                    {
                        pending.extend(index.expanded().iter().cloned());
                    }
                }
                _ => {}
            }
        }
    }
    Ok(found)
}

/// The fingerprint of a prose digest and a units digest.
fn fingerprint(prose: &str, units: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(SCHEME);
    hasher.update(prose.as_bytes());
    hasher.update(b"\n");
    hasher.update(units.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// The digest of `(id, raw content)` pairs under `label`, independent of
/// their order.
fn digest(label: &[u8], snippets: &[(&SnippetId, &[u8])]) -> String {
    let mut sorted: Vec<&(&SnippetId, &[u8])> = snippets.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let mut hasher = Sha256::new();
    hasher.update(SCHEME);
    hasher.update(label);
    for (id, content) in sorted {
        hasher.update(id.as_str().as_bytes());
        hasher.update(b"\0");
        hasher.update(content.len().to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(content);
    }
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> SnippetId {
        SnippetId::parse(s).unwrap()
    }

    #[test]
    fn the_digest_ignores_order_and_follows_content() {
        let (a, b) = (id("sdlc/policy/a"), id("sdlc/policy/b"));
        let one = digest(b"prose\n", &[(&a, b"x"), (&b, b"y")]);
        assert_eq!(one, digest(b"prose\n", &[(&b, b"y"), (&a, b"x")]));
        assert_ne!(one, digest(b"prose\n", &[(&a, b"x"), (&b, b"z")]));
        assert_ne!(one, digest(b"prose\n", &[(&a, b"x")]));
    }

    #[test]
    fn an_empty_corpus_has_its_own_digest() {
        let a = id("sdlc/policy/a");
        assert_ne!(digest(b"prose\n", &[]), digest(b"prose\n", &[(&a, b"")]));
        let (empty, _) = computed(&Published::default());
        let (one, _) = computed(&Published {
            care: Some(CARE.to_owned()),
            ..Published::default()
        });
        assert_ne!(empty.fingerprint, one.fingerprint);
    }

    /// A per-commit record as syncs wrote it before the time was kept.
    const PRE_TIME_RECORD: &str = "\
fingerprint: sha256:00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff
snippets: 2
project: flows
commit: 0123456789abcdef0123456789abcdef01234567
";

    fn fingerprint(digest: &str, project: &str, at: Option<&str>) -> EvalFingerprint {
        EvalFingerprint {
            fingerprint: digest.to_owned(),
            snippets: 2,
            prose: None,
            units: None,
            project: ProjectSlug::parse(project).unwrap(),
            commit: CommitSha::parse("0123456789abcdef0123456789abcdef01234567").unwrap(),
            recorded_at: at.map(|at| at.parse().unwrap()),
        }
    }

    #[test]
    fn a_record_holding_what_was_computed_is_left_with_its_time_and_any_other_is_written_now() {
        let now: jiff::Timestamp = "2026-11-18T12:00:00Z".parse().unwrap();
        let computed = fingerprint("sha256:aa", "flows", None);
        let older = fingerprint("sha256:aa", "flows", Some("2026-11-01T08:00:00Z"));
        assert_eq!(
            decide(Record::PerCommit, Some(&older), &computed, now),
            None
        );
        assert_eq!(decide(Record::Current, Some(&older), &computed, now), None);
        let changed = fingerprint("sha256:bb", "flows", Some("2026-11-01T08:00:00Z"));
        let written = fingerprint("sha256:aa", "flows", Some("2026-11-18T12:00:00Z"));
        assert_eq!(
            decide(Record::PerCommit, Some(&changed), &computed, now),
            Some(written.clone())
        );
        assert_eq!(decide(Record::Current, None, &computed, now), Some(written));
        let another_project = fingerprint("sha256:aa", "other", Some("2026-11-01T08:00:00Z"));
        assert_eq!(
            decide(Record::Current, Some(&another_project), &computed, now),
            None
        );
    }

    #[test]
    fn a_record_written_before_the_time_was_kept_loads_with_its_time_unknown() {
        let record: EvalFingerprint = serde_yaml_ng::from_str(PRE_TIME_RECORD).unwrap();
        assert_eq!(record.recorded_at, None);
        assert_eq!(record.snippets, 2);
        assert_eq!((record.prose, record.units), (None, None));
    }

    const CARE: &str = "---\ndescription: Care.\n---\n\n# Care\n\nTake care.\n";

    /// What a home publishes: `general/policy/care`, always on, with an
    /// optional unit, linked or not, and a project snippet.
    #[derive(Default, Clone)]
    struct Published {
        care: Option<String>,
        unit: Option<String>,
        unit_linked: bool,
        project: Option<String>,
        /// The source digest the index records for `care`; its prose's
        /// digest, as its owner's sync records it, when `None`.
        recorded: Option<String>,
    }

    /// A unit of `source` declaring its digest when `current`, with `body`.
    fn unit(source: &str, current: bool, body: &str) -> String {
        let prose = docs::split_frontmatter(source).1;
        let digest = _units::source_digest(if current { prose } else { "older" });
        format!("---\nsource_sha256: {digest}\nmodel: m\n---\n{body}")
    }

    /// The fingerprint a sync computes over a home publishing `published`.
    fn computed(published: &Published) -> (EvalFingerprint, Vec<UnitFinding>) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        let general = home.docs_dir().join(GENERAL);
        std::fs::create_dir_all(general.join("policy")).unwrap();
        let mut entries = String::new();
        if let Some(care) = &published.care {
            std::fs::write(
                general.join(INDEX_FILE),
                "expanded:\n  - general/policy/care\n",
            )
            .unwrap();
            std::fs::write(general.join("policy/care.md"), care).unwrap();
            let recorded = published
                .recorded
                .clone()
                .unwrap_or_else(|| _units::source_digest(docs::split_frontmatter(care).1));
            entries.push_str(&format!("- id: general/policy/care\n  description: Care.\n  type: policy\n  scope: global\n  domain: general\n  path: general/policy/care.md\n  source_project: demo\n  content_sha256: {recorded}\n"));
        }
        if let Some(unit) = &published.unit {
            let place = if published.unit_linked {
                dir.path().join("elsewhere")
            } else {
                general.join(_units::RESERVED)
            };
            std::fs::create_dir_all(place.join("policy")).unwrap();
            std::fs::write(place.join("policy/care.md"), unit).unwrap();
            if published.unit_linked {
                std::os::unix::fs::symlink(&place, general.join(_units::RESERVED)).unwrap();
            }
        }
        if let Some(text) = &published.project {
            let dir = home.docs_dir().join("project/demo/policy");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("house.md"), text).unwrap();
            entries.push_str("- id: project/demo/policy/house\n  description: House.\n  type: policy\n  scope: local\n  domain: project\n  path: project/demo/policy/house.md\n  source_project: demo\n");
        }
        let index: GlobalIndex = serde_yaml_ng::from_str(&format!(
            "schema_version: 3\nsnippets:{}\n{entries}",
            if entries.is_empty() { " []" } else { "" }
        ))
        .unwrap();
        compute(
            &home,
            &index,
            &ProjectSlug::parse("demo").unwrap(),
            &CommitSha::parse("0123456789abcdef0123456789abcdef01234567").unwrap(),
        )
        .unwrap()
    }

    fn with_unit(body: &str) -> Published {
        Published {
            care: Some(CARE.to_owned()),
            unit: Some(unit(CARE, true, body)),
            ..Published::default()
        }
    }

    #[test]
    fn a_unit_only_change_moves_only_the_units_digest() {
        let (before, _) = computed(&with_unit("- take care\n"));
        let (after, _) = computed(&with_unit("- take good care\n"));
        assert_eq!(before.prose, after.prose);
        assert_ne!(before.units, after.units);
        assert_ne!(before.fingerprint, after.fingerprint);
    }

    #[test]
    fn a_prose_change_moves_the_prose_digest() {
        let (before, _) = computed(&Published {
            care: Some(CARE.to_owned()),
            ..Published::default()
        });
        let (after, _) = computed(&Published {
            care: Some(CARE.replace("Take care.", "Take great care.")),
            ..Published::default()
        });
        assert_ne!(before.prose, after.prose);
        assert_eq!(before.units, after.units);
    }

    #[test]
    fn a_unit_going_stale_moves_the_units_digest_as_its_absence_would() {
        let (current, _) = computed(&with_unit("- take care\n"));
        let (stale, _) = computed(&Published {
            unit: Some(unit(CARE, false, "- take care\n")),
            ..with_unit("")
        });
        let (none, _) = computed(&Published {
            care: Some(CARE.to_owned()),
            ..Published::default()
        });
        assert_ne!(current.units, stale.units);
        assert_eq!(stale.units, none.units);
    }

    #[test]
    fn a_project_snippet_moves_neither_digest() {
        let (without, _) = computed(&with_unit("- take care\n"));
        let (with, _) = computed(&Published {
            project: Some("---\ndescription: House.\n---\n\n# House\n".to_owned()),
            ..with_unit("- take care\n")
        });
        assert_eq!(
            (without.prose, without.units, without.fingerprint),
            (with.prose, with.units, with.fingerprint)
        );
    }

    #[test]
    fn a_unit_is_current_by_the_digest_the_index_records_not_by_digesting_the_prose() {
        let recorded = _units::source_digest("the prose as its owner published it");
        let declaring = |digest: &str| Published {
            unit: Some(format!(
                "---\nsource_sha256: {digest}\nmodel: m\n---\n- take care\n"
            )),
            recorded: Some(recorded.clone()),
            ..with_unit("")
        };
        let (by_record, _) = computed(&declaring(&recorded));
        let prose_digest = _units::source_digest(docs::split_frontmatter(CARE).1);
        let (by_prose, _) = computed(&declaring(&prose_digest));
        let (none, _) = computed(&Published {
            recorded: Some(recorded.clone()),
            care: Some(CARE.to_owned()),
            ..Published::default()
        });
        assert_ne!(by_record.units, none.units);
        assert_eq!(by_prose.units, none.units);
    }

    #[test]
    fn a_unit_behind_a_link_counts_as_none_and_is_named() {
        let (linked, degraded) = computed(&Published {
            unit_linked: true,
            ..with_unit("- take care\n")
        });
        let (none, _) = computed(&Published {
            care: Some(CARE.to_owned()),
            ..Published::default()
        });
        assert_eq!(linked.units, none.units);
        assert!(matches!(
            degraded.as_slice(),
            [UnitFinding {
                kind: _units::UnitFindingKind::Link,
                ..
            }]
        ));
    }
}
