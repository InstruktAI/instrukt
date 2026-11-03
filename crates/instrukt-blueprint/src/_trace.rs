//! Use-case trace (`docs/local/spec/feature/blueprint/use-case-trace.md`):
//! the specs context's bindings joined with reach over a graph, in both
//! directions.
//!
//! A test is located by its own node — a node in the test's file named
//! like its function (the node's label, else its id, without a leading `.`
//! or trailing `()`) — when the graph has one, otherwise by every node of
//! its file. Test files are the files that hold a binding.

use std::collections::BTreeSet;

use instrukt_core::ids::UseCaseRef;
use instrukt_core::specs::{Binding, TestLocation};

use crate::_contract::{Graph, Node, normalize_path};
use crate::_traversal::Direction;

/// A use case traced forward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseCaseTrace {
    /// The tests binding the use case, in order.
    pub tests: Vec<TestLocation>,
    /// The code those tests reach outside test files, ordered by file,
    /// line, then name; `None` without a graph to reach over.
    pub reached: Option<Vec<Node>>,
}

/// A test reaching a traced file, with the use cases it binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachingTest {
    pub test: TestLocation,
    pub use_cases: Vec<UseCaseRef>,
}

/// The tests binding `use_case` and, over `graph` when there is one, the
/// code they reach by following dependency links forward from their nodes.
pub fn trace_use_case(
    graph: Option<&Graph>,
    bindings: &[Binding],
    use_case: &UseCaseRef,
) -> UseCaseTrace {
    let tests: Vec<TestLocation> = tests(bindings)
        .into_iter()
        .filter(|(_, bound)| bound.contains(use_case))
        .map(|(test, _)| test)
        .collect();
    let reached = graph.map(|graph| {
        let test_files = test_files(bindings);
        let seeds: BTreeSet<&str> = tests
            .iter()
            .flat_map(|test| test_nodes(graph, test))
            .collect();
        let reach = graph.reach(seeds.iter().copied(), Direction::Forward);
        let mut nodes: Vec<Node> = graph
            .nodes()
            .iter()
            .filter(|node| reach.contains(node.id.as_str()))
            .filter(|node| !test_files.contains(&normalize_path(&node.source_file)))
            .cloned()
            .collect();
        nodes.sort_by(|a, b| {
            (&a.source_file, a.location.map(|l| l.start), a.name()).cmp(&(
                &b.source_file,
                b.location.map(|l| l.start),
                b.name(),
            ))
        });
        nodes.dedup();
        nodes
    });
    UseCaseTrace { tests, reached }
}

/// The tests from whose nodes a node of `file` is reachable over
/// dependency links, each with the use cases it binds, in test order.
/// `file` is repository-relative.
pub fn trace_file(graph: &Graph, bindings: &[Binding], file: &str) -> Vec<ReachingTest> {
    let targets: BTreeSet<&str> = graph.nodes_in_file(file).map(|n| n.id.as_str()).collect();
    if targets.is_empty() {
        return Vec::new();
    }
    let mut reaching = graph.reach(targets.iter().copied(), Direction::Backward);
    reaching.extend(targets.iter().copied());
    tests(bindings)
        .into_iter()
        .filter(|(test, _)| {
            test_nodes(graph, test)
                .iter()
                .any(|id| reaching.contains(id))
        })
        .map(|(test, use_cases)| ReachingTest {
            test,
            use_cases: use_cases.into_iter().collect(),
        })
        .collect()
}

/// Every bound test with the use cases it binds, in binding order. A test
/// with several bindings is one test: its file and function, located at
/// its first binding.
fn tests(bindings: &[Binding]) -> Vec<(TestLocation, BTreeSet<UseCaseRef>)> {
    let mut tests: Vec<(TestLocation, BTreeSet<UseCaseRef>)> = Vec::new();
    for binding in bindings {
        let Some(use_case) = binding.use_case() else {
            continue;
        };
        let same = |test: &TestLocation| {
            test.file == binding.test.file && test.function == binding.test.function
        };
        match tests.iter_mut().find(|(test, _)| same(test)) {
            Some((_, bound)) => {
                bound.insert(use_case.clone());
            }
            None => tests.push((binding.test.clone(), BTreeSet::from([use_case.clone()]))),
        }
    }
    tests
}

/// The files that hold a binding.
fn test_files(bindings: &[Binding]) -> BTreeSet<String> {
    bindings
        .iter()
        .map(|binding| normalize_path(&binding.test.file.to_string_lossy()))
        .collect()
}

/// The test's own node when the graph has one, otherwise its file's nodes.
fn test_nodes<'g>(graph: &'g Graph, test: &TestLocation) -> Vec<&'g str> {
    let file = normalize_path(&test.file.to_string_lossy());
    let in_file: Vec<&Node> = graph
        .nodes()
        .iter()
        .filter(|node| normalize_path(&node.source_file) == file)
        .collect();
    let own: Vec<&str> = match &test.function {
        Some(function) => in_file
            .iter()
            .filter(|node| node.id == *function || bare_name(node.name()) == function)
            .map(|node| node.id.as_str())
            .collect(),
        None => Vec::new(),
    };
    if own.is_empty() {
        in_file.iter().map(|node| node.id.as_str()).collect()
    } else {
        own
    }
}

/// A node name without a method's leading `.` or a call's trailing `()`.
fn bare_name(name: &str) -> &str {
    let name = name.strip_prefix('.').unwrap_or(name);
    name.strip_suffix("()").unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use instrukt_core::ids::{SnippetId, UseCaseId};
    use instrukt_core::specs::Citation;
    use serde_json::json;

    use super::*;

    fn reference(uc: &str) -> UseCaseRef {
        UseCaseRef {
            spec: SnippetId::parse("local/projects/demo/spec/feature/a/b").unwrap(),
            use_case: UseCaseId::parse(uc).unwrap(),
        }
    }

    fn binding(file: &str, function: &str, uc: &str) -> Binding {
        Binding {
            test: TestLocation {
                file: PathBuf::from(file),
                line: 1,
                function: Some(function.to_owned()),
            },
            cites: Citation::UseCase(reference(uc)),
        }
    }

    fn graph() -> Graph {
        Graph::from_value(json!({
            "nodes": [
                {"id": "t_one", "label": "one()", "source_file": "tests/t.rs"},
                {"id": "t_two", "label": "two()", "source_file": "tests/t.rs"},
                {"id": "t_helper", "label": "helper()", "source_file": "tests/t.rs"},
                {"id": "lib_a", "source_file": "src/a.rs"},
                {"id": "lib_b", "source_file": "src/b.rs"},
                {"id": "lib_c", "source_file": "src/c.rs"}
            ],
            "links": [
                {"source": "t_one", "target": "t_helper", "relation": "calls"},
                {"source": "t_helper", "target": "lib_a", "relation": "calls"},
                {"source": "lib_a", "target": "lib_b", "relation": "imports"},
                {"source": "t_two", "target": "lib_c", "relation": "calls"},
                {"source": "lib_b", "target": "lib_c", "relation": "contains"}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn a_use_case_reaches_code_outside_test_files_from_its_tests_own_nodes() {
        let bindings = [
            binding("tests/t.rs", "one", "UC-A1"),
            binding("tests/t.rs", "two", "UC-A2"),
        ];
        let trace = trace_use_case(Some(&graph()), &bindings, &reference("UC-A1"));
        assert_eq!(trace.tests, [bindings[0].test.clone()]);
        let reached: Vec<String> = trace.reached.unwrap().into_iter().map(|n| n.id).collect();
        assert_eq!(reached, ["lib_a", "lib_b"]);
    }

    #[test]
    fn a_test_without_its_own_node_is_located_by_its_file() {
        let bindings = [binding("tests/t.rs", "unmapped", "UC-A1")];
        let trace = trace_use_case(Some(&graph()), &bindings, &reference("UC-A1"));
        let reached: Vec<String> = trace.reached.unwrap().into_iter().map(|n| n.id).collect();
        assert_eq!(reached, ["lib_a", "lib_b", "lib_c"]);
    }

    #[test]
    fn without_a_graph_only_the_binding_tests_are_known() {
        let bindings = [binding("tests/t.rs", "one", "UC-A1")];
        let trace = trace_use_case(None, &bindings, &reference("UC-A1"));
        assert_eq!(trace.tests.len(), 1);
        assert_eq!(trace.reached, None);
    }

    #[test]
    fn a_file_is_reached_by_the_tests_whose_nodes_lead_to_it() {
        let bindings = [
            binding("tests/t.rs", "one", "UC-A1"),
            binding("tests/t.rs", "two", "UC-A2"),
        ];
        let reaching = trace_file(&graph(), &bindings, "src/b.rs");
        assert_eq!(
            reaching,
            [ReachingTest {
                test: bindings[0].test.clone(),
                use_cases: vec![reference("UC-A1")],
            }]
        );
        assert!(trace_file(&graph(), &bindings, "src/unknown.rs").is_empty());
    }
}
