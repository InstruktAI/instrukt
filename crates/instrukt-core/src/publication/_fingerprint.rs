//! The eval fingerprint: a digest of every published snippet that declares
//! `eval: true`, recorded per published commit
//! (`project/instrukt/spec/feature/publication/eval-fingerprint`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::PublicationError;
use crate::docs::{Corpus, SourceFile, published_origin};
use crate::home::Home;
use crate::ids::{CommitSha, ProjectSlug, SnippetId};
use crate::index::{GlobalIndex, IndexScope};

/// The name of the record of the current fingerprint.
const CURRENT: &str = "current.yaml";

/// What the digest starts from, so an empty corpus has a defined digest of
/// its own and a later scheme cannot collide with this one.
const SCHEME: &[u8] = b"instrukt-eval-fingerprint-v1\n";

/// The eval fingerprint one publishing sync computed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalFingerprint {
    /// `sha256:<hex>`.
    pub fingerprint: String,
    /// How many `eval: true` snippets it covers.
    pub snippets: usize,
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
/// project and commit it names. A record written takes the sync's time.
fn decide(
    record: Record,
    existing: Option<&EvalFingerprint>,
    computed: &EvalFingerprint,
    now: jiff::Timestamp,
) -> Option<EvalFingerprint> {
    let current = existing.is_some_and(|existing| match record {
        Record::PerCommit => {
            existing.fingerprint == computed.fingerprint && existing.snippets == computed.snippets
        }
        Record::Current => existing.fingerprint == computed.fingerprint,
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
/// `index` describes it: every global entry's file read from the home,
/// kept when its frontmatter declares `eval: true`. It carries no time:
/// [`records`] decides whether a record is written, and when.
pub(super) fn compute(
    home: &Home,
    index: &GlobalIndex,
    project: &ProjectSlug,
    commit: &CommitSha,
) -> Result<EvalFingerprint, PublicationError> {
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
    let evaluated: Vec<(&SnippetId, &[u8])> = corpus
        .snippets()
        .iter()
        .filter(|s| s.current().frontmatter().eval)
        .filter_map(|s| {
            let text = texts.get(s.path().to_str()?)?;
            Some((s.id(), text.as_bytes()))
        })
        .collect();
    Ok(EvalFingerprint {
        fingerprint: digest(&evaluated),
        snippets: evaluated.len(),
        project: project.clone(),
        commit: commit.clone(),
        recorded_at: None,
    })
}

/// The digest of `(id, raw content)` pairs, independent of their order.
fn digest(snippets: &[(&SnippetId, &[u8])]) -> String {
    let mut sorted: Vec<&(&SnippetId, &[u8])> = snippets.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let mut hasher = Sha256::new();
    hasher.update(SCHEME);
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
        let one = digest(&[(&a, b"x"), (&b, b"y")]);
        assert_eq!(one, digest(&[(&b, b"y"), (&a, b"x")]));
        assert_ne!(one, digest(&[(&a, b"x"), (&b, b"z")]));
        assert_ne!(one, digest(&[(&a, b"x")]));
    }

    #[test]
    fn an_empty_corpus_has_its_own_digest() {
        let a = id("sdlc/policy/a");
        assert_ne!(digest(&[]), digest(&[(&a, b"")]));
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
    }
}
