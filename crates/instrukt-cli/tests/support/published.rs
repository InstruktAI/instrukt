//! A temporary home seeded as publication leaves it, for retrieval tests
//! that run while publication is built separately.
//!
//! Published files are copied raw into `~/.instrukt/docs/`, local files are
//! written into their project, and the index is built from the production
//! snippet model and saved with the production [`GlobalIndex`] type, so the
//! tests read the shared persisted contract, not hand-written YAML.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code,
    reason = "test setup: a failed step fails the test; each test crate uses what it needs"
)]

use std::path::{Path, PathBuf};

use instrukt_core::docs::{Corpus, Scope, SourceFile};
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use instrukt_core::index::{GlobalIndex, IndexEntry, IndexScope, ThirdPartyEntry};
use instrukt_core::publication::PublishedRecord;

use crate::common::{Repo, TestEnv};

/// A snippet text: `description`, `extra` frontmatter lines, the H1 and
/// `body`.
pub fn doc(title: &str, description: &str, extra: &str, body: &str) -> String {
    format!("---\ndescription: {description}\n{extra}---\n\n# {title}\n{body}")
}

/// Publishes the documentation files of `project` (repository paths with
/// their text) into the test's home: global and third-party files are
/// copied into the home, local files written into `repo`, and the project's
/// index entries replace the ones it owned. With `repo`, the project's
/// published commit is its current `HEAD`.
pub fn publish(env: &TestEnv, project: &str, repo: Option<&Repo>, files: &[(&str, &str)]) {
    let home = Home::at_user_home(env.home().to_path_buf());
    let slug = ProjectSlug::parse(project).unwrap();
    let docs = home.docs_dir();
    for (path, text) in files {
        let target = if let Some(rest) = path.strip_prefix("docs/global/") {
            docs.join(rest)
        } else if let Some(rest) = path.strip_prefix("docs/") {
            if rest.starts_with("local/") {
                repo.expect("local files belong to a repository")
                    .path()
                    .join(path)
            } else {
                docs.join(rest)
            }
        } else {
            panic!("not a documentation path: {path}");
        };
        write(&target, text);
    }
    let corpus = Corpus::from_sources(
        Some(slug.clone()),
        &home,
        files.iter().map(|(path, text)| SourceFile {
            path: (*path).to_owned(),
            text: (*text).to_owned(),
        }),
    );
    let mut index = GlobalIndex::load(&home).unwrap().unwrap_or_default();
    index.snippets.retain(|e| e.source_project != slug);
    index.third_party.retain(|e| e.source_project != slug);
    for snippet in corpus.snippets() {
        let rel = snippet.path().to_str().unwrap();
        let (scope, path) = match snippet.scope() {
            Scope::Global => (
                IndexScope::Global,
                PathBuf::from(rel.strip_prefix("docs/global/").unwrap()),
            ),
            Scope::Project => (
                IndexScope::Local,
                repo.expect("local files belong to a repository")
                    .path()
                    .join(rel),
            ),
        };
        index.snippets.push(IndexEntry {
            id: snippet.id().clone(),
            description: snippet
                .current()
                .frontmatter()
                .description
                .clone()
                .unwrap_or_default(),
            taxonomy: snippet.taxonomy().dir().to_owned(),
            scope,
            domain: snippet.domain().to_string(),
            path,
            source_project: slug.clone(),
            visibility: None,
            delivered_by: snippet.delivered_by().to_vec(),
            superseded_by: snippet.superseded_by().into_iter().collect(),
            activation_trigger: None,
            role: None,
            subscriptions: Vec::new(),
            parameters: Vec::new(),
            dispatch: None,
            event_delivery: None,
            interruptible: false,
        });
    }
    for doc in corpus.third_party() {
        index.third_party.push(ThirdPartyEntry {
            id: doc.id().clone(),
            description: doc.description().unwrap_or_default().to_owned(),
            path: doc.path().strip_prefix("docs").unwrap().to_path_buf(),
            source_project: slug.clone(),
        });
    }
    index.save(&home).unwrap();
    if let Some(repo) = repo {
        PublishedRecord {
            project: slug,
            commit: CommitSha::parse(&repo.head()).unwrap(),
            checkout: repo.path().to_path_buf(),
        }
        .save(&home)
        .unwrap();
    }
}

/// Writes a file publication generates (a bundle) at `rel` below
/// `~/.instrukt/docs/`.
pub fn generated(env: &TestEnv, rel: &str, text: &str) {
    write(&env.instrukt_home().join("docs").join(rel), text);
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}
