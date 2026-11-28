//! Review coverage of an integration candidate
//! (`project/instrukt/spec/feature/workflow/integration-gate`, Review
//! coverage of a candidate): derived on request and stored nowhere. Each
//! commit the candidate adds to the canonical branch, whichever parent
//! brought it in, is covered by one row, decided by comparing identifiers
//! and trees; no content is judged.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use instrukt_core::git;
use instrukt_core::ids::{CommitSha, TodoSlug};

use super::_observe::{MergeReproduction, reproduce_merge};

/// The kind of evidence a covering item rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoveringKind {
    /// A head approved in a todo's record or by work's head verdict.
    ApprovedHead,
    /// A merge git reproduces from its parents.
    ReproducedMerge,
    /// One delivery's documentation fold, reproduced from its parent.
    DocumentationFold,
    /// A rendering of a company repository's agent material, reproduced
    /// from its parent.
    AgentMaterialRendering,
    /// A revert of the failing delivery, reproduced.
    Revert,
    /// A commit its author exempted.
    Exempted,
}

impl CoveringKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CoveringKind::ApprovedHead => "approved_head",
            CoveringKind::ReproducedMerge => "reproduced_merge",
            CoveringKind::DocumentationFold => "documentation_fold",
            CoveringKind::AgentMaterialRendering => "agent_material_rendering",
            CoveringKind::Revert => "revert",
            CoveringKind::Exempted => "exempted",
        }
    }
}

/// One covering item: the commit, its kind and the reference to the
/// evidence it rests on, as that evidence's producer names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveringItem {
    pub commit: CommitSha,
    pub kind: CoveringKind,
    pub reference: String,
}

/// A commit no row covers, with the authoring sessions its `Session:`
/// trailers name, as work reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UncoveredCommit {
    pub commit: CommitSha,
    pub sessions: Vec<String>,
}

/// What a derivation found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coverage {
    /// Every commit is covered, by these items.
    Covered(Vec<CoveringItem>),
    /// These commits are covered by no row, in the order the walk met them.
    Uncovered(Vec<UncoveredCommit>),
    /// The repository cannot answer.
    CannotAnswer(String),
}

/// An approved head as the evidence that approves it names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approved {
    pub reference: String,
    /// The todo whose record approves it, for a todo's head.
    pub todo: Option<TodoSlug>,
}

/// The evidence a derivation reads besides the repository.
pub trait CoverageEvidence {
    /// The approval of exactly `commit`: in a todo's record, or work's
    /// current approving head verdict that covers the canonical branch.
    fn approved(&self, commit: &CommitSha) -> Result<Option<Approved>, String>;
    /// Work's exemption of `commit`, as its reference.
    fn exempted(&self, commit: &CommitSha) -> Result<Option<String>, String>;
    /// The approved head of the delivery `todo`, when its record holds one.
    fn delivery_head(&self, todo: &TodoSlug) -> Result<Option<CommitSha>, String>;
    /// The canonical head the red canonical head `head` was promoted from,
    /// where the runner observed `head` red and work holds evidence of it as
    /// a candidate.
    fn red_promotion(&self, head: &CommitSha) -> Result<Option<CommitSha>, String>;
    /// The authoring sessions `commit`'s `Session:` trailers name.
    fn sessions(&self, commit: &CommitSha) -> Result<Vec<String>, String>;
}

fn unanswered(error: impl std::fmt::Display) -> Coverage {
    Coverage::CannotAnswer(error.to_string())
}

/// Whether every commit `candidate` adds to `canonical` (a revision of
/// the repository at `root`) is covered.
pub fn coverage(
    root: &Path,
    candidate: &CommitSha,
    canonical: &str,
    evidence: &dyn CoverageEvidence,
) -> Coverage {
    match derive(root, candidate, canonical, evidence) {
        Ok(coverage) => coverage,
        Err(error) => unanswered(error),
    }
}

fn derive(
    root: &Path,
    candidate: &CommitSha,
    canonical: &str,
    evidence: &dyn CoverageEvidence,
) -> Result<Coverage, String> {
    let based = git::run_output(root, ["merge-base", candidate.as_str(), canonical])
        .map_err(|e| e.to_string())?;
    if !based.status.success() {
        return Ok(Coverage::Uncovered(vec![uncovered(candidate, evidence)?]));
    }
    let based = sha(String::from_utf8_lossy(&based.stdout).trim())?;
    // Exactly the commits the candidate contains and the canonical branch
    // does not, also under criss-cross history; descendants come before
    // their ancestors, so an approved head is met before what it contains.
    let listing = added(
        root,
        candidate.as_str(),
        canonical,
        &["--parents", "--topo-order"],
    )?;
    let mut approved_part: HashSet<String> = HashSet::new();
    let mut items = Vec::new();
    let mut found = Vec::new();
    for line in listing.lines() {
        let ids: Vec<&str> = line.split_whitespace().collect();
        let Some(first_id) = ids.first() else {
            continue;
        };
        if approved_part.contains(*first_id) {
            continue;
        }
        let commit = sha(first_id)?;
        if let Some(approved) = evidence.approved(&commit)? {
            items.push(item(
                &commit,
                CoveringKind::ApprovedHead,
                approved.reference,
            ));
            approved_part.extend(
                added(root, commit.as_str(), canonical, &[])?
                    .lines()
                    .map(|id| id.trim().to_owned()),
            );
            continue;
        }
        if let [_, first, second] = ids.as_slice() {
            match reproduce_merge(root, commit.as_str(), first, second)
                .map_err(|e| e.to_string())?
            {
                MergeReproduction::Reproduced => {
                    items.push(item(
                        &commit,
                        CoveringKind::ReproducedMerge,
                        format!("reproduced from {first} and {second}"),
                    ));
                    continue;
                }
                MergeReproduction::Undetermined => {
                    return Ok(unanswered(format!(
                        "git could not reproduce merge {commit}"
                    )));
                }
                // A resolution is its maker's to decide.
                MergeReproduction::Differs | MergeReproduction::Conflicted => {}
            }
        }
        if let Some(reference) = evidence.exempted(&commit)? {
            items.push(item(&commit, CoveringKind::Exempted, reference));
            continue;
        }
        if let [_, parent] = ids.as_slice() {
            let parent = sha(parent)?;
            if let Some(reference) = fold(root, &commit, &parent, evidence)? {
                items.push(item(&commit, CoveringKind::DocumentationFold, reference));
                continue;
            }
            if let Some(reference) = rendering(root, &commit, &parent)? {
                items.push(item(
                    &commit,
                    CoveringKind::AgentMaterialRendering,
                    reference,
                ));
                continue;
            }
            if let Some(reference) = revert(root, &commit, &parent, &based, evidence)? {
                items.push(item(&commit, CoveringKind::Revert, reference));
                continue;
            }
        }
        found.push(uncovered(&commit, evidence)?);
    }
    Ok(if found.is_empty() {
        Coverage::Covered(items)
    } else {
        Coverage::Uncovered(found)
    })
}

/// The commits `head` contains and `canonical` does not, one line each,
/// with `options` given to `git rev-list`.
fn added(root: &Path, head: &str, canonical: &str, options: &[&str]) -> Result<String, String> {
    let mut args = vec!["rev-list"];
    args.extend(options);
    args.extend([head, "--not", canonical]);
    git::run(root, args).map_err(|e| e.to_string())
}

fn uncovered(
    commit: &CommitSha,
    evidence: &dyn CoverageEvidence,
) -> Result<UncoveredCommit, String> {
    Ok(UncoveredCommit {
        commit: commit.clone(),
        sessions: evidence.sessions(commit)?,
    })
}

fn item(commit: &CommitSha, kind: CoveringKind, reference: String) -> CoveringItem {
    CoveringItem {
        commit: commit.clone(),
        kind,
        reference,
    }
}

fn sha(text: &str) -> Result<CommitSha, String> {
    CommitSha::parse(text.trim()).map_err(|_| format!("git named `{text}` as a commit"))
}

fn contains(root: &Path, ancestor: &str, descendant: &str) -> Result<bool, String> {
    git::succeeds(root, ["merge-base", "--is-ancestor", ancestor, descendant])
        .map_err(|e| e.to_string())
}

fn tree(root: &Path, revision: &str) -> Result<String, String> {
    git::run(root, ["rev-parse", &format!("{revision}^{{tree}}")])
        .map(|tree| tree.trim().to_owned())
        .map_err(|e| e.to_string())
}

/// Whether `commit` is exactly the documentation fold of the delivery its
/// `Todo:` trailer names, reproduced from `parent`'s tree, with that
/// delivery's approved head incorporated below it.
fn fold(
    root: &Path,
    commit: &CommitSha,
    parent: &CommitSha,
    evidence: &dyn CoverageEvidence,
) -> Result<Option<String>, String> {
    let named = git::run(
        root,
        [
            "log",
            "-1",
            "--format=%(trailers:key=Todo,valueonly)",
            commit.as_str(),
        ],
    )
    .map_err(|e| e.to_string())?;
    let Some(todo) = named.lines().map(str::trim).find(|l| !l.is_empty()) else {
        return Ok(None);
    };
    let Ok(todo) = TodoSlug::parse(todo) else {
        return Ok(None);
    };
    let Some(head) = evidence.delivery_head(&todo)? else {
        return Ok(None);
    };
    if !contains(root, head.as_str(), parent.as_str())? {
        return Ok(None);
    }
    let reproduced = reproduced_from_parent(root, commit, parent, |tree_dir| {
        let Ok(outcome) = instrukt_core::docs::fold(tree_dir, &todo) else {
            return Ok(None);
        };
        Ok(Some(
            outcome.written.into_iter().chain(outcome.removed).collect(),
        ))
    })?;
    Ok(reproduced.then(|| format!("fold of {todo} reproduced from {parent}")))
}

/// Whether `commit` is exactly the rendering of the agent material of a
/// company repository, reproduced from `parent`'s tree through
/// publication's one operation; a parent that is no company repository by
/// the rule rendering decides by, or whose rendering finds anything, covers
/// nothing.
fn rendering(
    root: &Path,
    commit: &CommitSha,
    parent: &CommitSha,
) -> Result<Option<String>, String> {
    let reproduced = reproduced_from_parent(root, commit, parent, |tree_dir| {
        use instrukt_core::publication;
        if !publication::is_company_repository(tree_dir).is_ok_and(|company| company) {
            return Ok(None);
        }
        let Ok(material) = publication::plan_agent_material(tree_dir) else {
            return Ok(None);
        };
        if material.has_findings() {
            return Ok(None);
        }
        Ok(publication::write_agent_material(tree_dir, &material).ok())
    })?;
    Ok(reproduced.then(|| format!("rendering of the agent material reproduced from {parent}")))
}

/// Whether `commit` is exactly what `reproduce` makes of `parent`'s tree.
///
/// The tree is checked out into a scratch repository of its own, whose
/// index holds exactly the parent's tree, so the project's index and working
/// trees are never entered; the directory is removed on every way out.
/// `reproduce` changes that tree and returns the paths it wrote or removed
/// (`None`: it reproduces nothing). The commit is reproduced when the set
/// is not empty and equals the files the commit changes against `parent`,
/// each written file's blob equals the commit's, and each removed file is
/// absent from the commit. Trees are compared; no content is judged.
fn reproduced_from_parent(
    root: &Path,
    commit: &CommitSha,
    parent: &CommitSha,
    reproduce: impl FnOnce(&Path) -> Result<Option<Vec<PathBuf>>, String>,
) -> Result<bool, String> {
    let scratch =
        instrukt_core::fsx::temp_dir(tempfile::Builder::new().prefix("instrukt-reproduce-"))
            .map_err(|e| e.to_string())?;
    let tree_dir = scratch.path().join("tree");
    instrukt_core::fsx::create_dir_all(&tree_dir).map_err(|e| e.to_string())?;
    git::run(&tree_dir, ["init", "-q"]).map_err(|e| e.to_string())?;
    git::check_out_tree(
        root,
        parent.as_str(),
        &tree_dir.join(".git/index"),
        &tree_dir,
    )
    .map_err(|e| e.to_string())?;
    let Some(touched) = reproduce(&tree_dir)? else {
        return Ok(false);
    };
    let changed = git::run(
        root,
        [
            "diff",
            "--name-only",
            "--no-renames",
            parent.as_str(),
            commit.as_str(),
        ],
    )
    .map_err(|e| e.to_string())?;
    let changed: Vec<String> = changed.lines().map(str::to_owned).collect();
    let mut paths: Vec<String> = touched
        .iter()
        .map(|path| {
            path.strip_prefix(&tree_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    paths.sort();
    paths.dedup();
    let mut reproduced = Vec::new();
    let mut committed = BTreeMap::new();
    for path in paths {
        // The reproduction's blob of a file it wrote; none for one it
        // removed. The commit's blob at that path; none where it holds none.
        let blob = if tree_dir.join(&path).exists() {
            Some(
                git::run(&tree_dir, ["hash-object", path.as_str()])
                    .map_err(|e| e.to_string())?
                    .trim()
                    .to_owned(),
            )
        } else {
            None
        };
        let held = git::run_output(root, ["rev-parse", &format!("{commit}:{path}")])
            .map_err(|e| e.to_string())?;
        committed.insert(
            path.clone(),
            held.status
                .success()
                .then(|| String::from_utf8_lossy(&held.stdout).trim().to_owned()),
        );
        reproduced.push((path, blob));
    }
    Ok(same_change(&reproduced, &changed, &committed))
}

/// Whether a reproduction that wrote or removed `reproduced` (each path with
/// the blob it wrote, or none where it removed the file) is exactly a commit
/// that changed `changed` and holds `committed` (each reproduced path with
/// the commit's blob, or none where the commit holds no file there): a
/// non-empty set of the same paths, each with the same blob or absent alike.
/// `git diff --name-only` lists a deleted path and a modified one alike, so
/// a removal is matched by the commit holding no file there.
fn same_change(
    reproduced: &[(String, Option<String>)],
    changed: &[String],
    committed: &BTreeMap<String, Option<String>>,
) -> bool {
    let mut paths: Vec<&str> = reproduced.iter().map(|(path, _)| path.as_str()).collect();
    paths.sort_unstable();
    paths.dedup();
    let mut changed: Vec<&str> = changed.iter().map(String::as_str).collect();
    changed.sort_unstable();
    !paths.is_empty()
        && paths == changed
        && reproduced
            .iter()
            .all(|(path, blob)| committed.get(path) == Some(blob))
}

/// Whether `commit` reverts the failing delivery: the canonical head `based`
/// was observed red, and `commit` undoes only what that head's promotion
/// added, reproduced from `parent`'s tree.
fn revert(
    root: &Path,
    commit: &CommitSha,
    parent: &CommitSha,
    based: &CommitSha,
    evidence: &dyn CoverageEvidence,
) -> Result<Option<String>, String> {
    let Some(promoted_from) = evidence.red_promotion(based)? else {
        return Ok(None);
    };
    let target = tree(root, commit.as_str())?;
    if target == tree(root, promoted_from.as_str())? {
        return Ok(Some(format!(
            "restores the tree of {promoted_from}, before the red head {based}'s promotion"
        )));
    }
    let range = format!("{promoted_from}..{based}");
    let added = git::run(root, ["rev-list", "--first-parent", range.as_str()])
        .map_err(|e| e.to_string())?;
    for undone in added.lines() {
        let before = format!("{undone}^");
        let reverted = git::run_output(
            root,
            [
                "merge-tree",
                "--write-tree",
                &format!("--merge-base={undone}"),
                parent.as_str(),
                before.as_str(),
            ],
        )
        .map_err(|e| e.to_string())?;
        if reverted.status.success()
            && String::from_utf8_lossy(&reverted.stdout)
                .lines()
                .next()
                .map(str::trim)
                == Some(target.as_str())
        {
            return Ok(Some(format!(
                "reverts {undone} of the red head {based}'s promotion"
            )));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Repo(tempfile::TempDir);

    impl Repo {
        fn new() -> Self {
            let repo = Self(tempfile::tempdir().unwrap());
            for args in [
                vec!["init", "-q", "-b", "main"],
                vec!["config", "user.name", "Test"],
                vec!["config", "user.email", "test@example.com"],
                vec!["config", "commit.gpgsign", "false"],
            ] {
                repo.git(&args);
            }
            repo.commit("base.txt", "base", "base");
            repo
        }
        fn path(&self) -> &Path {
            self.0.path()
        }
        fn git(&self, args: &[&str]) -> String {
            git::run(self.path(), args).unwrap()
        }
        fn commit(&self, file: &str, text: &str, message: &str) -> CommitSha {
            let path = self.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
            self.git(&["add", "-A"]);
            self.git(&["commit", "-qm", message]);
            self.head()
        }
        fn head(&self) -> CommitSha {
            CommitSha::parse(&self.git(&["rev-parse", "HEAD"])).unwrap()
        }
    }

    #[derive(Default)]
    struct Evidence {
        approved: BTreeMap<String, String>,
        exempted: BTreeMap<String, String>,
        deliveries: BTreeMap<String, CommitSha>,
        red: BTreeMap<String, CommitSha>,
        sessions: BTreeMap<String, Vec<String>>,
    }

    impl CoverageEvidence for Evidence {
        fn approved(&self, commit: &CommitSha) -> Result<Option<Approved>, String> {
            Ok(self
                .approved
                .get(commit.as_str())
                .map(|reference| Approved {
                    reference: reference.clone(),
                    todo: None,
                }))
        }
        fn exempted(&self, commit: &CommitSha) -> Result<Option<String>, String> {
            Ok(self.exempted.get(commit.as_str()).cloned())
        }
        fn delivery_head(&self, todo: &TodoSlug) -> Result<Option<CommitSha>, String> {
            Ok(self.deliveries.get(todo.as_str()).cloned())
        }
        fn red_promotion(&self, head: &CommitSha) -> Result<Option<CommitSha>, String> {
            Ok(self.red.get(head.as_str()).cloned())
        }
        fn sessions(&self, commit: &CommitSha) -> Result<Vec<String>, String> {
            Ok(self
                .sessions
                .get(commit.as_str())
                .cloned()
                .unwrap_or_default())
        }
    }

    fn uncovered(coverage: &Coverage) -> Vec<(CommitSha, Vec<String>)> {
        match coverage {
            Coverage::Uncovered(found) => found
                .iter()
                .map(|u| (u.commit.clone(), u.sessions.clone()))
                .collect(),
            other => panic!("not uncovered: {other:?}"),
        }
    }

    fn kinds(coverage: &Coverage) -> Vec<CoveringKind> {
        match coverage {
            Coverage::Covered(items) => items.iter().map(|item| item.kind).collect(),
            other => panic!("not covered: {other:?}"),
        }
    }

    #[test]
    fn an_approved_head_covers_itself_and_what_it_contains() {
        let repo = Repo::new();
        repo.git(&["checkout", "-qb", "integrate"]);
        repo.commit("a.txt", "a", "change a");
        let head = repo.commit("b.txt", "b", "change b");
        let evidence = Evidence {
            approved: BTreeMap::from([(head.to_string(), "verdict".into())]),
            ..Evidence::default()
        };
        assert_eq!(
            kinds(&coverage(repo.path(), &head, "main", &evidence)),
            [CoveringKind::ApprovedHead]
        );
    }

    #[test]
    fn a_reproduced_merge_of_an_approved_head_is_covered() {
        let repo = Repo::new();
        repo.git(&["checkout", "-qb", "work"]);
        let approved = repo.commit("a.txt", "a", "change a");
        repo.git(&["checkout", "-q", "main"]);
        repo.git(&["checkout", "-qb", "integrate"]);
        repo.git(&["merge", "-q", "--no-ff", "-m", "merge work", "work"]);
        let candidate = repo.head();
        let evidence = Evidence {
            approved: BTreeMap::from([(approved.to_string(), "build review".into())]),
            ..Evidence::default()
        };
        assert_eq!(
            kinds(&coverage(repo.path(), &candidate, "main", &evidence)),
            [CoveringKind::ReproducedMerge, CoveringKind::ApprovedHead]
        );
    }

    #[test]
    fn a_reproduced_merge_of_the_canonical_branch_is_covered() {
        let repo = Repo::new();
        repo.git(&["checkout", "-qb", "work"]);
        let approved = repo.commit("a.txt", "a", "change a");
        repo.git(&["checkout", "-q", "main"]);
        repo.commit("c.txt", "c", "canonical moves");
        repo.git(&["checkout", "-q", "work"]);
        repo.git(&["merge", "-q", "--no-ff", "-m", "merge main", "main"]);
        let candidate = repo.head();
        let evidence = Evidence {
            approved: BTreeMap::from([(approved.to_string(), "verdict".into())]),
            ..Evidence::default()
        };
        assert_eq!(
            kinds(&coverage(repo.path(), &candidate, "main", &evidence)),
            [CoveringKind::ReproducedMerge, CoveringKind::ApprovedHead]
        );
    }

    #[test]
    fn an_exempted_commit_is_covered() {
        let repo = Repo::new();
        repo.git(&["checkout", "-qb", "integrate"]);
        let exempted = repo.commit("a.txt", "a", "chore: bump");
        let evidence = Evidence {
            exempted: BTreeMap::from([(exempted.to_string(), "trivial".into())]),
            ..Evidence::default()
        };
        assert_eq!(
            kinds(&coverage(repo.path(), &exempted, "main", &evidence)),
            [CoveringKind::Exempted]
        );
    }

    #[test]
    fn a_reproduced_documentation_fold_of_an_incorporated_delivery_is_covered() {
        let repo = Repo::new();
        repo.commit(
            "instrukt.yaml",
            "schema_version: 4\nproject: demo\n",
            "adopt",
        );
        repo.commit(
            "docs/project/spec/thing.md",
            "---\ndescription: A thing.\ndelivered_by:\n  - shipped\n---\n# Thing — Spec\n\n## What it is\n\nA thing.\n",
            "docs: pending thing",
        );
        repo.git(&["checkout", "-qb", "integrate"]);
        let delivery = repo.commit("src.txt", "code", "feat: the thing");
        let fold_root = tempfile::tempdir().unwrap();
        let tree_dir = fold_root.path().join("tree");
        git::check_out_tree(
            repo.path(),
            delivery.as_str(),
            &fold_root.path().join("index"),
            &tree_dir,
        )
        .unwrap();
        let slug = TodoSlug::parse("shipped").unwrap();
        git::run(&tree_dir, ["init", "-q"]).unwrap();
        std::fs::copy(fold_root.path().join("index"), tree_dir.join(".git/index")).unwrap();
        instrukt_core::docs::fold(&tree_dir, &slug).unwrap();
        let folded = std::fs::read_to_string(tree_dir.join("docs/project/spec/thing.md")).unwrap();
        repo.commit(
            "docs/project/spec/thing.md",
            &folded,
            "docs: fold shipped\n\nTodo: shipped",
        );
        let candidate = repo.head();
        let evidence = Evidence {
            approved: BTreeMap::from([(delivery.to_string(), "build review".into())]),
            deliveries: BTreeMap::from([("shipped".to_owned(), delivery.clone())]),
            ..Evidence::default()
        };
        assert_eq!(
            kinds(&coverage(repo.path(), &candidate, "main", &evidence)),
            [CoveringKind::DocumentationFold, CoveringKind::ApprovedHead]
        );
    }

    /// A delivery whose pending documentation is a companion, on an
    /// integration branch: the repository, the delivery head, the commit the
    /// fold applies to and the fold's own tree written into a scratch copy.
    fn companion_delivery() -> (Repo, CommitSha, tempfile::TempDir) {
        let repo = Repo::new();
        repo.commit(
            "instrukt.yaml",
            "schema_version: 4\nproject: demo\n",
            "adopt",
        );
        repo.commit(
            "docs/project/spec/thing.md",
            "---\ndescription: A thing.\n---\n# Thing — Spec\n\n## What it is\n\nThe old thing.\n",
            "docs: thing",
        );
        repo.commit(
            "docs/project/spec/thing.shipped.md",
            "---\ndescription: A thing.\n---\n# Thing — Spec\n\n## What it is\n\nThe new thing.\n",
            "docs: pending thing",
        );
        repo.git(&["checkout", "-qb", "integrate"]);
        let delivery = repo.commit("src.txt", "code", "feat: the thing");
        let fold_root = tempfile::tempdir().unwrap();
        let tree_dir = fold_root.path().join("tree");
        git::check_out_tree(
            repo.path(),
            delivery.as_str(),
            &fold_root.path().join("index"),
            &tree_dir,
        )
        .unwrap();
        git::run(&tree_dir, ["init", "-q"]).unwrap();
        std::fs::copy(fold_root.path().join("index"), tree_dir.join(".git/index")).unwrap();
        let outcome =
            instrukt_core::docs::fold(&tree_dir, &TodoSlug::parse("shipped").unwrap()).unwrap();
        assert_eq!(
            outcome.removed,
            [std::path::PathBuf::from(
                "docs/project/spec/thing.shipped.md"
            )]
        );
        (repo, delivery, fold_root)
    }

    fn shipped(delivery: &CommitSha) -> Evidence {
        Evidence {
            approved: BTreeMap::from([(delivery.to_string(), "build review".into())]),
            deliveries: BTreeMap::from([("shipped".to_owned(), delivery.clone())]),
            ..Evidence::default()
        }
    }

    #[test]
    fn a_fold_that_removes_its_companion_is_reproduced() {
        let (repo, delivery, fold_root) = companion_delivery();
        let folded =
            std::fs::read_to_string(fold_root.path().join("tree/docs/project/spec/thing.md"))
                .unwrap();
        std::fs::write(repo.path().join("docs/project/spec/thing.md"), folded).unwrap();
        repo.git(&["rm", "-q", "docs/project/spec/thing.shipped.md"]);
        repo.git(&["commit", "-qam", "docs: fold shipped\n\nTodo: shipped"]);
        assert_eq!(
            kinds(&coverage(
                repo.path(),
                &repo.head(),
                "main",
                &shipped(&delivery)
            )),
            [CoveringKind::DocumentationFold, CoveringKind::ApprovedHead]
        );
    }

    #[test]
    fn a_fold_that_keeps_a_changed_companion_instead_of_removing_it_is_not_reproduced() {
        let (repo, delivery, fold_root) = companion_delivery();
        let folded =
            std::fs::read_to_string(fold_root.path().join("tree/docs/project/spec/thing.md"))
                .unwrap();
        std::fs::write(repo.path().join("docs/project/spec/thing.md"), folded).unwrap();
        std::fs::write(
            repo.path().join("docs/project/spec/thing.shipped.md"),
            "kept, not removed\n",
        )
        .unwrap();
        repo.git(&["commit", "-qam", "docs: fold shipped\n\nTodo: shipped"]);
        let fold = repo.head();
        assert_eq!(
            uncovered(&coverage(repo.path(), &fold, "main", &shipped(&delivery))),
            [(fold, Vec::new())]
        );
    }

    #[test]
    fn a_reproduced_revert_of_the_red_heads_promotion_is_covered() {
        let repo = Repo::new();
        let before = repo.head();
        let broken = repo.commit("bad.txt", "bad", "feat: breaks main");
        repo.git(&["checkout", "-qb", "integrate"]);
        repo.git(&["revert", "--no-edit", broken.as_str()]);
        let candidate = repo.head();
        let evidence = Evidence {
            red: BTreeMap::from([(broken.to_string(), before)]),
            ..Evidence::default()
        };
        assert_eq!(
            kinds(&coverage(repo.path(), &candidate, "main", &evidence)),
            [CoveringKind::Revert]
        );
    }

    #[test]
    fn a_commit_no_row_covers_is_found_uncovered() {
        let repo = Repo::new();
        repo.git(&["checkout", "-qb", "integrate"]);
        let stray = repo.commit("a.txt", "a", "change a");
        assert_eq!(
            coverage(repo.path(), &stray, "main", &Evidence::default()),
            Coverage::Uncovered(vec![UncoveredCommit {
                commit: stray,
                sessions: Vec::new(),
            }])
        );
    }

    #[test]
    fn an_exempted_one_off_commit_behind_the_integrators_merge_is_covered() {
        let repo = Repo::new();
        repo.git(&["checkout", "-qb", "ad-hoc"]);
        let exempted = repo.commit("a.txt", "a", "chore: bump");
        repo.git(&["checkout", "-q", "main"]);
        repo.git(&["checkout", "-qb", "integrate"]);
        let approved = repo.commit("b.txt", "b", "feat: reviewed work");
        repo.git(&["merge", "-q", "--no-ff", "-m", "merge ad-hoc", "ad-hoc"]);
        let candidate = repo.head();
        let evidence = Evidence {
            approved: BTreeMap::from([(approved.to_string(), "verdict".into())]),
            exempted: BTreeMap::from([(exempted.to_string(), "trivial".into())]),
            ..Evidence::default()
        };
        let mut found = kinds(&coverage(repo.path(), &candidate, "main", &evidence));
        found.sort_by_key(|kind| kind.as_str());
        assert_eq!(
            found,
            [
                CoveringKind::ApprovedHead,
                CoveringKind::Exempted,
                CoveringKind::ReproducedMerge
            ]
        );
    }

    #[test]
    fn a_conflict_resolution_merge_is_uncovered_with_its_sessions() {
        let repo = Repo::new();
        repo.git(&["checkout", "-qb", "work"]);
        let ours = repo.commit("a.txt", "work", "change a on work");
        repo.git(&["checkout", "-q", "main"]);
        repo.git(&["checkout", "-qb", "integrate"]);
        let theirs = repo.commit("a.txt", "integrate", "change a on integrate");
        assert!(
            git::run(
                repo.path(),
                ["merge", "-q", "--no-ff", "-m", "merge work", "work"]
            )
            .is_err()
        );
        std::fs::write(repo.path().join("a.txt"), "resolved").unwrap();
        repo.git(&["add", "a.txt"]);
        repo.git(&["commit", "-qm", "merge work, resolved"]);
        let resolution = repo.head();
        let evidence = Evidence {
            exempted: BTreeMap::from([
                (ours.to_string(), "trivial".into()),
                (theirs.to_string(), "trivial".into()),
            ]),
            sessions: BTreeMap::from([(resolution.to_string(), vec!["t3:integrator".into()])]),
            ..Evidence::default()
        };
        assert_eq!(
            uncovered(&coverage(repo.path(), &resolution, "main", &evidence)),
            [(resolution, vec!["t3:integrator".to_owned()])]
        );
    }

    #[test]
    fn an_approved_head_covers_what_it_contains_on_every_parent() {
        let repo = Repo::new();
        repo.git(&["checkout", "-qb", "side"]);
        repo.commit("s.txt", "s", "side work");
        repo.git(&["checkout", "-q", "main"]);
        repo.git(&["checkout", "-qb", "work"]);
        repo.commit("a.txt", "a", "change a");
        repo.git(&["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
        let head = repo.commit("b.txt", "b", "change b");
        let evidence = Evidence {
            approved: BTreeMap::from([(head.to_string(), "verdict".into())]),
            ..Evidence::default()
        };
        assert_eq!(
            kinds(&coverage(repo.path(), &head, "main", &evidence)),
            [CoveringKind::ApprovedHead]
        );
    }

    #[test]
    fn an_octopus_merge_is_not_reproduced() {
        let repo = Repo::new();
        let mut exempted = BTreeMap::new();
        for branch in ["one", "two"] {
            repo.git(&["checkout", "-q", "main"]);
            repo.git(&["checkout", "-qb", branch]);
            let commit = repo.commit(&format!("{branch}.txt"), branch, branch);
            exempted.insert(commit.to_string(), "trivial".to_owned());
        }
        repo.git(&["checkout", "-q", "main"]);
        repo.git(&["checkout", "-qb", "integrate"]);
        repo.git(&["merge", "-q", "--no-ff", "-m", "octopus", "one", "two"]);
        let octopus = repo.head();
        let evidence = Evidence {
            exempted,
            ..Evidence::default()
        };
        assert_eq!(
            uncovered(&coverage(repo.path(), &octopus, "main", &evidence)),
            [(octopus, Vec::new())]
        );
    }

    #[test]
    fn every_uncovered_commit_is_returned() {
        let repo = Repo::new();
        repo.git(&["checkout", "-qb", "integrate"]);
        let first = repo.commit("a.txt", "a", "change a");
        let exempted = repo.commit("b.txt", "b", "chore: bump");
        let last = repo.commit("c.txt", "c", "change c");
        let evidence = Evidence {
            exempted: BTreeMap::from([(exempted.to_string(), "trivial".into())]),
            sessions: BTreeMap::from([
                (first.to_string(), vec!["t3:a".into()]),
                (last.to_string(), vec!["t3:b".into(), "t3:c".into()]),
            ]),
            ..Evidence::default()
        };
        assert_eq!(
            uncovered(&coverage(repo.path(), &last, "main", &evidence)),
            [
                (last, vec!["t3:b".to_owned(), "t3:c".to_owned()]),
                (first, vec!["t3:a".to_owned()])
            ]
        );
    }

    fn written(path: &str, blob: &str) -> (String, Option<String>) {
        (path.to_owned(), Some(blob.to_owned()))
    }

    fn holds(entries: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
        entries
            .iter()
            .map(|(path, blob)| ((*path).to_owned(), blob.map(str::to_owned)))
            .collect()
    }

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn the_same_paths_with_the_same_blobs_or_absent_alike_are_the_same_change() {
        let reproduced = [
            written("AGENTS.md", "a1"),
            written("skills/acme/SKILL.md", "s1"),
            ("skills/acme/references/old.md".to_owned(), None),
        ];
        let committed = holds(&[
            ("AGENTS.md", Some("a1")),
            ("skills/acme/SKILL.md", Some("s1")),
            ("skills/acme/references/old.md", None),
        ]);
        let changed = paths(&[
            "skills/acme/references/old.md",
            "AGENTS.md",
            "skills/acme/SKILL.md",
        ]);
        assert!(same_change(&reproduced, &changed, &committed));
    }

    #[test]
    fn the_same_paths_with_other_content_are_not_the_same_change() {
        let reproduced = [
            written("AGENTS.md", "a1"),
            written("skills/acme/SKILL.md", "s1"),
        ];
        let committed = holds(&[
            ("AGENTS.md", Some("a2")),
            ("skills/acme/SKILL.md", Some("s1")),
        ]);
        let changed = paths(&["AGENTS.md", "skills/acme/SKILL.md"]);
        assert!(!same_change(&reproduced, &changed, &committed));
    }

    #[test]
    fn a_commit_changing_a_path_the_reproduction_did_not_is_not_the_same_change() {
        let reproduced = [written("AGENTS.md", "a1")];
        let committed = holds(&[("AGENTS.md", Some("a1"))]);
        let changed = paths(&["AGENTS.md", "docs/company.md"]);
        assert!(!same_change(&reproduced, &changed, &committed));
    }

    #[test]
    fn a_removal_the_commit_still_holds_is_not_the_same_change() {
        let reproduced = [("skills/acme/references/old.md".to_owned(), None)];
        let committed = holds(&[("skills/acme/references/old.md", Some("o1"))]);
        let changed = paths(&["skills/acme/references/old.md"]);
        assert!(!same_change(&reproduced, &changed, &committed));
    }

    #[test]
    fn a_reproduction_that_changes_nothing_reproduces_no_commit() {
        assert!(!same_change(&[], &[], &BTreeMap::new()));
    }
}
