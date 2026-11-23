//! The waiting graph of a project's work (`dependencies-and-holds`): what
//! each todo waits on, and the cycles its edges and memberships make. Pure
//! rules over records, independent of where the records are kept: a
//! command's check of the graph it read and its transaction's check of the
//! graph it commits to apply the same ones.

use std::collections::BTreeSet;

use instrukt_core::ids::TodoSlug;

use super::_model::{TodoId, TodoRecord};
use crate::evidence::TodoStatus;

/// A todo as the waiting graph sees it (`dependencies-and-holds`): its
/// prerequisites, its group, and whether it is a structural holder, which
/// waits on its children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WaitNode {
    pub id: TodoId,
    pub slug: TodoSlug,
    pub after: Vec<TodoId>,
    pub group: Option<TodoId>,
    pub holder: bool,
    pub live: bool,
    /// A local item, rather than a listing copy.
    pub local: bool,
}

impl WaitNode {
    /// `record` as the waiting graph sees it; `local` for a local item's
    /// record, rather than a listing copy's.
    pub(crate) fn of(record: &TodoRecord, local: bool) -> Self {
        Self {
            id: record.id.clone(),
            slug: record.slug.clone(),
            after: record.after.iter().map(|r| r.id.clone()).collect(),
            group: record.group.as_ref().map(|g| g.id.clone()),
            holder: record.status == TodoStatus::StructuralHolder,
            live: record.is_live(),
            local,
        }
    }
}

/// What `of` waits on in `graph` (`dependencies-and-holds`): its
/// prerequisites, and for a structural holder its children, a listing
/// copy's only while it is live. A prerequisite the graph does not hold is
/// named all the same; what it waits on is unknown.
pub(crate) fn waits_on<'g>(graph: &'g [WaitNode], of: &'g WaitNode) -> Vec<&'g TodoId> {
    let mut next: Vec<&TodoId> = of.after.iter().collect();
    if of.holder {
        next.extend(
            graph
                .iter()
                .filter(|c| c.group.as_ref() == Some(&of.id) && (c.local || c.live))
                .map(|c| &c.id),
        );
    }
    next
}

/// Every todo waiting from `start` reaches in `graph` ([`waits_on`]),
/// `start` included.
pub(crate) fn reached(graph: &[WaitNode], start: &TodoId) -> BTreeSet<TodoId> {
    let mut found = BTreeSet::new();
    let mut frontier = vec![start.clone()];
    while let Some(id) = frontier.pop() {
        if !found.insert(id.clone()) {
            continue;
        }
        if let Some(node) = graph.iter().find(|n| n.id == id) {
            frontier.extend(waits_on(graph, node).into_iter().cloned());
        }
    }
    found
}

/// The first cycle of waiting ([`waits_on`]) through one of `through` in
/// `graph`: the one rule both a command's check of the graph it read and
/// its transaction's check of the graph it commits to apply.
pub(crate) fn first_cycle(graph: &[WaitNode], through: &[TodoId]) -> Option<Vec<TodoSlug>> {
    let node = |id: &TodoId| graph.iter().find(|n| &n.id == id);
    let waits = |of: &WaitNode| -> Vec<&WaitNode> {
        waits_on(graph, of).into_iter().filter_map(node).collect()
    };
    for start in through.iter().filter_map(node) {
        let mut stack: Vec<Vec<&WaitNode>> = waits(start)
            .into_iter()
            .map(|next| vec![start, next])
            .collect();
        let mut seen: BTreeSet<&TodoId> = BTreeSet::new();
        while let Some(path) = stack.pop() {
            let Some(last) = path.last().copied() else {
                continue;
            };
            if last.id == start.id {
                return Some(path.iter().map(|n| n.slug.clone()).collect());
            }
            if !seen.insert(&last.id) {
                continue;
            }
            for next in waits(last) {
                let mut longer = path.clone();
                longer.push(next);
                stack.push(longer);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node of the waiting graph named `slug`, its identity derived from
    /// the slug's first letter.
    fn node(slug: &str, after: &[&str], group: Option<&str>) -> WaitNode {
        let id = |slug: &str| TodoId::parse(&format!("{:02x}", slug.as_bytes()[0]).repeat(16));
        WaitNode {
            id: id(slug).unwrap(),
            slug: TodoSlug::parse(slug).unwrap(),
            after: after.iter().map(|a| id(a).unwrap()).collect(),
            group: group.map(|g| id(g).unwrap()),
            holder: false,
            live: true,
            local: true,
        }
    }

    fn holder(mut node: WaitNode) -> WaitNode {
        node.holder = true;
        node
    }

    #[test]
    fn a_prerequisite_on_a_holder_whose_member_waits_on_the_dependent_is_a_cycle() {
        // H holds X and, through a membership committed meanwhile, B; B
        // waits on A; A now waits on H.
        let graph = [
            node("alpha", &["hold"], None),
            node("beta", &["alpha"], Some("hold")),
            holder(node("hold", &[], None)),
            node("xray", &[], Some("hold")),
        ];
        let cycle = first_cycle(&graph, &[graph[0].id.clone()]).unwrap();
        let names: Vec<&str> = cycle.iter().map(TodoSlug::as_str).collect();
        assert_eq!(names, ["alpha", "hold", "beta", "alpha"]);
    }

    #[test]
    fn waiting_reaches_a_holder_s_children_and_a_prerequisite_the_graph_does_not_hold() {
        let graph = [
            node("alpha", &["hold"], None),
            holder(node("hold", &[], None)),
            node("beta", &["zulu"], Some("hold")),
            node("xray", &[], None),
        ];
        // alpha, beta, hold and the unknown zulu; never the unrelated xray.
        let zulu = node("zulu", &[], None).id;
        let expected: BTreeSet<TodoId> = [&graph[0].id, &graph[1].id, &graph[2].id, &zulu]
            .into_iter()
            .cloned()
            .collect();
        assert_eq!(reached(&graph, &graph[0].id), expected);
    }

    #[test]
    fn edges_committed_from_both_ends_of_a_pair_are_a_cycle() {
        let graph = [
            node("alpha", &["beta"], None),
            node("beta", &["alpha"], None),
        ];
        assert!(first_cycle(&graph, &[graph[0].id.clone()]).is_some());
    }

    #[test]
    fn a_withdrawn_listing_copy_is_no_member_its_holder_waits_on() {
        let mut gone = node("beta", &["alpha"], Some("hold"));
        gone.local = false;
        gone.live = false;
        let graph = [
            node("alpha", &["hold"], None),
            gone,
            holder(node("hold", &[], None)),
        ];
        assert_eq!(first_cycle(&graph, &[graph[0].id.clone()]), None);
    }
}
