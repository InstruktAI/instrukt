//! The answers the blueprint computes itself from a graph: blast radius
//! (`docs/local/spec/feature/blueprint/blast-radius.md`) and path
//! (`docs/local/spec/feature/blueprint/graph-navigation.md`).

use std::collections::BTreeSet;
use std::path::Path;

use crate::_contract::{Graph, Link, Node, normalize_path};
use crate::_traversal::Direction;

/// The dependents of a set of changed files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlastRadius {
    /// How many distinct changed files were asked about.
    pub changed_files: usize,
    /// Ordered by file, line, then name.
    pub dependents: Vec<Node>,
    /// Changed files that define no node, in the order given.
    pub unknown_files: Vec<String>,
}

/// Every node reachable backwards over dependency links from the nodes the
/// `changed` files define, outside those files. `changed` are
/// repository-relative paths ([`repo_relative`]).
pub fn blast_radius<S: AsRef<str>>(graph: &Graph, changed: &[S]) -> BlastRadius {
    let mut files: Vec<String> = Vec::new();
    for file in changed {
        let file = normalize_path(file.as_ref());
        if !files.contains(&file) {
            files.push(file);
        }
    }
    let changed_set: BTreeSet<&str> = files.iter().map(String::as_str).collect();
    let in_changed = |node: &Node| changed_set.contains(normalize_path(&node.source_file).as_str());
    let seeds: BTreeSet<&str> = graph
        .nodes()
        .iter()
        .filter(|node| in_changed(node))
        .map(|node| node.id.as_str())
        .collect();
    let reached = graph.reach(seeds.iter().copied(), Direction::Backward);
    let mut dependents: Vec<Node> = graph
        .nodes()
        .iter()
        .filter(|node| reached.contains(node.id.as_str()))
        .filter(|node| !in_changed(node))
        .cloned()
        .collect();
    dependents.sort_by(|a, b| {
        (
            a.source_file.as_str(),
            a.location.map(|l| l.start),
            a.name(),
        )
            .cmp(&(
                b.source_file.as_str(),
                b.location.map(|l| l.start),
                b.name(),
            ))
    });
    dependents.dedup();
    let unknown_files = files
        .iter()
        .filter(|file| graph.nodes_in_file(file).next().is_none())
        .cloned()
        .collect();
    BlastRadius {
        changed_files: files.len(),
        dependents,
        unknown_files,
    }
}

/// A file argument as a repository-relative path: an absolute path inside
/// `root` is made relative to it and a relative one is normalized
/// ([`normalize_path`]); an absolute path outside `root` is taken as given
/// (and so matches no node).
pub fn repo_relative(root: &Path, arg: &str) -> String {
    let path = Path::new(arg);
    if path.is_absolute() {
        let inside = path
            .strip_prefix(root)
            .ok()
            .map(Path::to_path_buf)
            .or_else(|| {
                let canonical_root = root.canonicalize().ok()?;
                let canonical = path.canonicalize().ok()?;
                canonical
                    .strip_prefix(canonical_root)
                    .ok()
                    .map(Path::to_path_buf)
            });
        if let Some(relative) = inside {
            return normalize_path(&relative.to_string_lossy());
        }
        return arg.to_owned();
    }
    normalize_path(arg)
}

/// A `path` endpoint that names no node.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown endpoint `{0}`: it names no node id or source file in the graph")]
pub struct UnknownEndpoint(pub String);

/// The answer to `path <A> <B>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathAnswer {
    /// The shortest chain of links from A to B; empty when A and B name a
    /// common node.
    Found(Vec<Link>),
    /// B is not reachable from A.
    Unreachable,
}

/// The shortest chain of links from `a` to `b`, each a node id or a source
/// file (a node id wins when both match). An endpoint naming neither is
/// refused.
pub fn path(graph: &Graph, a: &str, b: &str) -> Result<PathAnswer, UnknownEndpoint> {
    let from = endpoint(graph, a)?;
    let to = endpoint(graph, b)?;
    Ok(match graph.shortest_path(&from, &to) {
        Some(chain) => PathAnswer::Found(chain.into_iter().cloned().collect()),
        None => PathAnswer::Unreachable,
    })
}

fn endpoint<'g>(graph: &'g Graph, name: &str) -> Result<Vec<&'g str>, UnknownEndpoint> {
    let by_id: Vec<&str> = graph
        .nodes()
        .iter()
        .filter(|node| node.id == name)
        .map(|node| node.id.as_str())
        .take(1)
        .collect();
    if !by_id.is_empty() {
        return Ok(by_id);
    }
    let by_file: Vec<&str> = graph
        .nodes()
        .iter()
        .filter(|node| node.is_in_file(name))
        .map(|node| node.id.as_str())
        .collect();
    if by_file.is_empty() {
        Err(UnknownEndpoint(name.to_owned()))
    } else {
        Ok(by_file)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::{Value, json};

    use super::*;

    fn node(id: &str, file: &str) -> Value {
        json!({"id": id, "source_file": file})
    }

    fn link(s: &str, t: &str, r: &str) -> Value {
        json!({"source": s, "target": t, "relation": r})
    }

    #[test]
    fn dependents_are_ordered_and_unknown_files_listed() {
        let g = Graph::from_value(json!({
            "nodes": [
                node("a", "src/a.rs"),
                {"id": "b2", "source_file": "src/b.rs", "label": "late", "source_location": "L20"},
                {"id": "b1", "source_file": "src/b.rs", "label": "zeta", "source_location": "L3"},
                node("c", "src/c.rs"),
                node("a_test", "src/a.rs"),
            ],
            "links": [
                link("b2", "a", "calls"),
                link("b1", "a", "references"),
                link("c", "b1", "imports"),
                link("a_test", "a", "calls"),
                link("src/a.rs", "a", "contains"),
            ]
        }))
        .unwrap();
        let answer = blast_radius(&g, &["src/a.rs", "src/new.rs", "src/a.rs"]);
        let ids: Vec<_> = answer.dependents.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["b1", "b2", "c"]);
        assert_eq!(answer.unknown_files, ["src/new.rs"]);
        assert_eq!(answer.changed_files, 2);
    }

    #[test]
    fn equivalent_spellings_of_a_file_match_on_both_sides() {
        let g = Graph::from_value(json!({
            "nodes": [node("a", "./src/a.rs"), node("b", "src//b.rs"), node("c", "src/c.rs")],
            "links": [link("b", "a", "calls"), link("c", "b", "calls")]
        }))
        .unwrap();
        let answer = blast_radius(&g, &["src/a.rs"]);
        let ids: Vec<_> = answer.dependents.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["b", "c"]);
        assert!(answer.unknown_files.is_empty());
        let answer = blast_radius(&g, &["./src//b.rs"]);
        let ids: Vec<_> = answer.dependents.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["c"]);
        assert!(
            matches!(path(&g, "src/c.rs", "src/a.rs"), Ok(PathAnswer::Found(chain)) if chain.len() == 2)
        );
    }

    #[test]
    fn file_arguments_become_repository_relative() {
        let root = Path::new("/repo");
        assert_eq!(repo_relative(root, "/repo/src/a.rs"), "src/a.rs");
        assert_eq!(repo_relative(root, "./src/a.rs"), "src/a.rs");
        assert_eq!(repo_relative(root, "src/a.rs"), "src/a.rs");
        assert_eq!(repo_relative(root, "/elsewhere/a.rs"), "/elsewhere/a.rs");
    }

    #[test]
    fn path_endpoints_are_ids_or_files_and_unknown_ones_are_refused() {
        let g = Graph::from_value(json!({
            "nodes": [node("a", "a.rs"), node("b", "b.rs"), node("b_helper", "b.rs")],
            "links": [link("a", "b_helper", "calls")]
        }))
        .unwrap();
        match path(&g, "a.rs", "b.rs").unwrap() {
            PathAnswer::Found(chain) => assert_eq!(chain.len(), 1),
            other => panic!("{other:?}"),
        }
        assert_eq!(path(&g, "b", "a").unwrap(), PathAnswer::Unreachable);
        assert_eq!(path(&g, "a", "nope"), Err(UnknownEndpoint("nope".into())));
    }

    fn arb_graph() -> impl Strategy<Value = (Graph, Vec<String>)> {
        let relations =
            prop::sample::select(vec!["calls", "imports", "contains", "method", "uses"]);
        (
            prop::collection::vec(0usize..4, 1..12),
            prop::collection::vec((0usize..12, 0usize..12, relations), 0..30),
            prop::collection::vec(0usize..5, 1..3),
        )
            .prop_map(|(files, links, changed)| {
                let n = files.len();
                let nodes: Vec<_> = files
                    .iter()
                    .enumerate()
                    .map(|(i, f)| node(&format!("n{i}"), &format!("f{f}.rs")))
                    .collect();
                let links: Vec<_> = links
                    .into_iter()
                    .map(|(s, t, r)| link(&format!("n{}", s % n), &format!("n{}", t % n), r))
                    .collect();
                let graph = Graph::from_value(json!({"nodes": nodes, "links": links})).unwrap();
                (graph, changed.iter().map(|f| format!("f{f}.rs")).collect())
            })
    }

    proptest! {
        #[test]
        fn blast_radius_never_includes_the_changed_files_nodes((graph, changed) in arb_graph()) {
            let answer = blast_radius(&graph, &changed);
            for dependent in &answer.dependents {
                prop_assert!(!changed.contains(&dependent.source_file));
            }
        }

        #[test]
        fn containment_never_counts((graph, changed) in arb_graph()) {
            let without: Vec<Value> = graph
                .links()
                .iter()
                .filter(|l| l.relation.is_dependency())
                .map(|l| link(&l.source, &l.target, l.relation.as_str()))
                .collect();
            let nodes: Vec<Value> = graph.nodes().iter().map(|n| node(&n.id, &n.source_file)).collect();
            let stripped = Graph::from_value(json!({"nodes": nodes, "links": without})).unwrap();
            prop_assert_eq!(blast_radius(&graph, &changed), blast_radius(&stripped, &changed));
        }
    }
}
