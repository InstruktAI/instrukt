//! Use-case trace (`docs/project/spec/feature/blueprint/use-case-trace.md`):
//! the specs context's bindings joined with reach over a graph, in both
//! directions.
//!
//! A test is located by its own node — a node in the test's file named
//! like its function (the node's label, else its id, without a leading `.`
//! or trailing `()`) — when the graph has one, otherwise by every node of
//! its file. Test files are the files that hold a binding.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use instrukt_core::ids::UseCaseRef;
use instrukt_core::specs::{Binding, Inventory, TestLocation};

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

/// Files traced back to the use cases at stake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTrace {
    /// One group per spec folder, ordered by folder.
    pub groups: Vec<SpecFolder>,
    /// The given files no use-case-binding test reaches, in the order
    /// given; files the graph does not know are among them.
    pub untraced: Vec<UntracedFile>,
}

/// The use cases at stake whose spec files share a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecFolder {
    /// The repository-relative folder holding the spec files, with a
    /// trailing `/`.
    pub folder: String,
    /// Each use case at stake once, ordered by spec id, then use-case id.
    pub use_cases: Vec<AtStake>,
}

/// A use case at stake, with the tests binding it that reach any of the
/// traced files, in test order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtStake {
    pub use_case: UseCaseRef,
    pub tests: Vec<TestLocation>,
}

/// A traced file that no use-case-binding test reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UntracedFile {
    pub file: String,
    /// Whether the graph defines any node in it.
    pub known: bool,
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

/// The use cases at stake when `files` change: every use case bound by a
/// test from whose nodes a node of any of `files` is reachable over
/// dependency links, grouped by the folder of its spec file in
/// `inventory`, and the files no such test reaches. `files` are
/// repository-relative; a file given twice counts once. A binding naming a
/// use case the inventory does not hold has no spec folder and is left out.
pub fn trace_files<S: AsRef<str>>(
    graph: &Graph,
    bindings: &[Binding],
    inventory: &Inventory,
    files: &[S],
) -> FileTrace {
    let mut given: Vec<String> = Vec::new();
    for file in files {
        let file = normalize_path(file.as_ref());
        if !given.contains(&file) {
            given.push(file);
        }
    }
    let tests: Vec<(TestLocation, BTreeSet<UseCaseRef>, Vec<&str>)> = tests(bindings)
        .into_iter()
        .map(|(test, use_cases)| {
            let nodes = test_nodes(graph, &test);
            (test, use_cases, nodes)
        })
        .collect();
    let mut reached_files: BTreeSet<&str> = BTreeSet::new();
    let mut reaching_tests: BTreeSet<usize> = BTreeSet::new();
    for file in &given {
        let targets: BTreeSet<&str> = graph.nodes_in_file(file).map(|n| n.id.as_str()).collect();
        if targets.is_empty() {
            continue;
        }
        let mut reaching = graph.reach(targets.iter().copied(), Direction::Backward);
        reaching.extend(targets.iter().copied());
        for (index, (_, _, nodes)) in tests.iter().enumerate() {
            if nodes.iter().any(|id| reaching.contains(id)) {
                reaching_tests.insert(index);
                reached_files.insert(file.as_str());
            }
        }
    }
    let mut folders: BTreeMap<String, BTreeMap<UseCaseRef, Vec<TestLocation>>> = BTreeMap::new();
    for index in &reaching_tests {
        let (test, use_cases, _) = &tests[*index];
        for use_case in use_cases {
            let Some(known) = inventory.use_case(use_case) else {
                continue;
            };
            folders
                .entry(spec_folder(known.file()))
                .or_default()
                .entry(use_case.clone())
                .or_default()
                .push(test.clone());
        }
    }
    let groups = folders
        .into_iter()
        .map(|(folder, use_cases)| SpecFolder {
            folder,
            use_cases: use_cases
                .into_iter()
                .map(|(use_case, tests)| AtStake { use_case, tests })
                .collect(),
        })
        .collect();
    let untraced = given
        .iter()
        .filter(|file| !reached_files.contains(file.as_str()))
        .map(|file| UntracedFile {
            file: file.clone(),
            known: graph.nodes_in_file(file).next().is_some(),
        })
        .collect();
    FileTrace { groups, untraced }
}

/// The repository-relative folder holding `spec_file`, with a trailing
/// `/`; empty for a spec at the repository root.
fn spec_folder(spec_file: &Path) -> String {
    let folder = normalize_path(
        &spec_file
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    if folder.is_empty() {
        folder
    } else {
        format!("{folder}/")
    }
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

    use instrukt_core::docs::{Corpus, SourceFile};
    use instrukt_core::home::Home;
    use instrukt_core::ids::{ProjectSlug, SnippetId, UseCaseId};
    use instrukt_core::specs::Citation;
    use serde_json::json;

    use super::*;

    fn reference(uc: &str) -> UseCaseRef {
        UseCaseRef {
            spec: SnippetId::parse("project/demo/spec/feature/a/b").unwrap(),
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

    fn spec(title: &str, use_cases: &[&str]) -> String {
        let cases: String = use_cases
            .iter()
            .map(|uc| format!("#### {uc}: case\n\n```gherkin\nGiven a\nWhen b\nThen c\n```\n\n"))
            .collect();
        format!(
            "---\ndescription: {title}.\n---\n\n# {title} — Spec\n\n## What it is\n\nx\n\n\
             ## Canonical fields\n\n### Use cases\n\n{cases}"
        )
    }

    /// `a/b` holds UC-A1 and UC-A2, `c/d` holds UC-C1.
    fn inventory() -> Inventory {
        let sources = [
            (
                "docs/project/spec/feature/a/b.md",
                spec("B", &["UC-A1", "UC-A2"]),
            ),
            ("docs/project/spec/feature/c/d.md", spec("D", &["UC-C1"])),
        ];
        Inventory::read(&Corpus::from_sources(
            ProjectSlug::parse("demo").ok(),
            &Home::at_user_home(PathBuf::from("/nonexistent-home")),
            sources.into_iter().map(|(path, text)| SourceFile {
                path: path.to_owned(),
                text,
            }),
        ))
    }

    fn cites(spec: &str, uc: &str) -> UseCaseRef {
        UseCaseRef {
            spec: SnippetId::parse(spec).unwrap(),
            use_case: UseCaseId::parse(uc).unwrap(),
        }
    }

    fn bound(file: &str, function: &str, spec: &str, uc: &str) -> Binding {
        Binding {
            test: TestLocation {
                file: PathBuf::from(file),
                line: 1,
                function: Some(function.to_owned()),
            },
            cites: Citation::UseCase(cites(spec, uc)),
        }
    }

    const B: &str = "project/demo/spec/feature/a/b";
    const D: &str = "project/demo/spec/feature/c/d";

    /// `one` binds UC-C1 and UC-A2 and reaches a.rs, b.rs and c.rs; `two`
    /// binds UC-A1 and reaches c.rs only.
    fn file_bindings() -> [Binding; 3] {
        [
            bound("tests/t.rs", "one", D, "UC-C1"),
            bound("tests/t.rs", "two", B, "UC-A1"),
            bound("tests/t.rs", "one", B, "UC-A2"),
        ]
    }

    #[test]
    fn files_put_each_use_case_at_stake_once_grouped_by_spec_folder() {
        let bindings = file_bindings();
        let trace = trace_files(&graph(), &bindings, &inventory(), &["src/b.rs", "src/c.rs"]);
        let one = bindings[0].test.clone();
        let two = bindings[1].test.clone();
        assert_eq!(
            trace.groups,
            [
                SpecFolder {
                    folder: "docs/project/spec/feature/a/".to_owned(),
                    use_cases: vec![
                        AtStake {
                            use_case: cites(B, "UC-A1"),
                            tests: vec![two],
                        },
                        AtStake {
                            use_case: cites(B, "UC-A2"),
                            tests: vec![one.clone()],
                        },
                    ],
                },
                SpecFolder {
                    folder: "docs/project/spec/feature/c/".to_owned(),
                    use_cases: vec![AtStake {
                        use_case: cites(D, "UC-C1"),
                        tests: vec![one],
                    }],
                },
            ]
        );
    }

    #[test]
    fn files_no_binding_test_reaches_follow_once_each_with_unknown_files_among_them() {
        let trace = trace_files(
            &graph(),
            &file_bindings(),
            &inventory(),
            &["src/unknown.rs", "src/b.rs", "./src/unknown.rs"],
        );
        let lone = Graph::from_value(json!({
            "nodes": [{"id": "lone", "source_file": "src/lone.rs"}],
            "links": []
        }))
        .unwrap();
        let unreached = trace_files(&lone, &file_bindings(), &inventory(), &["src/lone.rs"]);
        assert_eq!(
            (trace.untraced, unreached.untraced),
            (
                vec![UntracedFile {
                    file: "src/unknown.rs".to_owned(),
                    known: false,
                }],
                vec![UntracedFile {
                    file: "src/lone.rs".to_owned(),
                    known: true,
                }],
            )
        );
    }
}
