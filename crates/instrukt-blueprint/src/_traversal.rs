//! Traversal over a graph's links: reach over dependency links in either
//! direction (blast radius, use-case trace) and the shortest directed chain
//! between two sets of nodes over any link (path).
//!
//! Traversal works on node ids. Links naming an id no node carries are
//! still followed; such ids simply match no node when answers are built.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::_contract::{Graph, Link};

/// Which way a dependency link is followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// From a link's `source` to its `target`: what the seeds depend on.
    Forward,
    /// From a link's `target` to its `source`: what depends on the seeds.
    Backward,
}

impl Graph {
    /// Every node id reachable from `seeds` over dependency links in
    /// `direction`, without a depth limit, each visited once. The seeds
    /// themselves are not part of the answer.
    pub fn reach<'g, I>(&'g self, seeds: I, direction: Direction) -> BTreeSet<&'g str>
    where
        I: IntoIterator<Item = &'g str>,
    {
        let mut next: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for link in self.links().iter().filter(|l| l.relation.is_dependency()) {
            let (from, to) = match direction {
                Direction::Forward => (link.source.as_str(), link.target.as_str()),
                Direction::Backward => (link.target.as_str(), link.source.as_str()),
            };
            next.entry(from).or_default().push(to);
        }
        let seeds: BTreeSet<&str> = seeds.into_iter().collect();
        let mut seen = seeds.clone();
        let mut queue: VecDeque<&str> = seeds.iter().copied().collect();
        while let Some(id) = queue.pop_front() {
            for &to in next.get(id).map(Vec::as_slice).unwrap_or_default() {
                if seen.insert(to) {
                    queue.push_back(to);
                }
            }
        }
        seen.retain(|id| !seeds.contains(id));
        seen
    }

    /// The shortest chain of links, followed in their direction, leading
    /// from any id in `from` to any id in `to`; any relation counts. Empty
    /// when the two sets share an id; `None` when no chain exists. Among
    /// chains of equal length the answer is stable: the chain from the
    /// starting id first in byte order wins, and at each step a node's
    /// outgoing links are tried in document order.
    pub fn shortest_path<'g>(&'g self, from: &[&'g str], to: &[&'g str]) -> Option<Vec<&'g Link>> {
        let targets: BTreeSet<&str> = to.iter().copied().collect();
        let mut out: BTreeMap<&str, Vec<&Link>> = BTreeMap::new();
        for link in self.links() {
            out.entry(link.source.as_str()).or_default().push(link);
        }
        let mut via: BTreeMap<&str, Option<&Link>> = BTreeMap::new();
        let mut queue = VecDeque::new();
        let mut starts: Vec<&str> = from.to_vec();
        starts.sort_unstable();
        starts.dedup();
        for start in starts {
            via.insert(start, None);
            queue.push_back(start);
        }
        while let Some(id) = queue.pop_front() {
            if targets.contains(id) {
                let mut chain = Vec::new();
                let mut at = id;
                while let Some(Some(link)) = via.get(at) {
                    chain.push(*link);
                    at = link.source.as_str();
                }
                chain.reverse();
                return Some(chain);
            }
            for &link in out.get(id).map(Vec::as_slice).unwrap_or_default() {
                let to = link.target.as_str();
                if !via.contains_key(to) {
                    via.insert(to, Some(link));
                    queue.push_back(to);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn graph(links: &[(&str, &str, &str)]) -> Graph {
        let mut ids: BTreeSet<&str> = BTreeSet::new();
        for (s, t, _) in links {
            ids.insert(s);
            ids.insert(t);
        }
        let nodes: Vec<_> = ids
            .iter()
            .map(|id| json!({"id": id, "source_file": format!("{id}.rs")}))
            .collect();
        let links: Vec<_> = links
            .iter()
            .map(|(s, t, r)| json!({"source": s, "target": t, "relation": r}))
            .collect();
        Graph::from_value(json!({"nodes": nodes, "links": links})).unwrap()
    }

    #[test]
    fn reach_follows_dependencies_both_ways_through_cycles_and_skips_containment() {
        let g = graph(&[
            ("b", "a", "calls"),
            ("c", "b", "imports"),
            ("a", "c", "uses"),
            ("d", "a", "contains"),
            ("a", "e", "calls"),
        ]);
        let back = g.reach(["a"], Direction::Backward);
        assert_eq!(back, BTreeSet::from(["b", "c"]));
        let fwd = g.reach(["a"], Direction::Forward);
        assert_eq!(fwd, BTreeSet::from(["b", "c", "e"]));
    }

    #[test]
    fn shortest_path_follows_direction_over_any_relation() {
        let g = graph(&[
            ("a", "x", "contains"),
            ("x", "c", "calls"),
            ("a", "b", "calls"),
            ("b", "y", "calls"),
            ("y", "c", "calls"),
        ]);
        let path = g.shortest_path(&["a"], &["c"]).unwrap();
        let steps: Vec<_> = path
            .iter()
            .map(|l| (l.source.as_str(), l.target.as_str()))
            .collect();
        assert_eq!(steps, [("a", "x"), ("x", "c")]);
        assert!(g.shortest_path(&["c"], &["a"]).is_none());
        assert_eq!(g.shortest_path(&["a"], &["a"]).unwrap().len(), 0);
    }
}
