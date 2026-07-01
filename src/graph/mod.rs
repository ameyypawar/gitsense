//! Phase 5: cycle-safe call-graph builder.
//!
//! Operates purely over the in-memory [`SymbolIndex`] — no gix, no I/O.
//! The model is *name-based* and *approximate* (documented v0):
//!
//! - **callees_of**: refs within the definition's `line_range` that resolve to
//!   a known definition.  For multi-line functions this covers only the name
//!   line (the v0 `line_range` is the name-node span); single-line functions
//!   work correctly.
//! - **callers_of**: O(all_fn_defs × refs_per_file) scan — fine for demo repos;
//!   for production, build an inverted callers index at `SymbolIndex::build` time.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::index::model::SymbolKind;
use crate::index::SymbolIndex;

// ── Public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Direction {
    Callees,
    Callers,
    Both,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallEdge {
    pub from: String,
    pub to: String,
    /// 1-based hop level of the `to` node (direct neighbour of root = 1).
    pub depth: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallGraphResult {
    pub root: String,
    pub callees: Vec<CallEdge>,
    pub callers: Vec<CallEdge>,
    /// Each entry is a cycle path: names from the first repeated node back to
    /// itself, e.g. `["a", "b", "a"]` for the mutual-recursion `a ↔ b`.
    pub cycles_detected: Vec<Vec<String>>,
    /// `true` if `max_hops` cut off traversal before the graph was fully expanded.
    pub truncated: bool,
}

// ── Core helpers ─────────────────────────────────────────────────────────────

fn is_fn_like(kind: &SymbolKind) -> bool {
    matches!(kind, SymbolKind::Fn | SymbolKind::Method)
}

/// Names that `name` calls.
///
/// Looks up the Fn/Method definition(s) named `name` via the O(1)
/// `SymbolIndex::definitions` lookup, then collects every `SymbolRef` in the
/// same file whose source line falls within each definition's `line_range`,
/// keeping only names that resolve to at least one known definition.
fn callees_of(index: &SymbolIndex, name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    for def in index.definitions(name) {
        if !is_fn_like(&def.kind) {
            continue;
        }
        let Some(tags) = index.file_tags(&def.location.file) else {
            continue;
        };
        let (lo, hi) = def.line_range;
        for r in &tags.refs {
            let ln = r.location.line;
            if ln >= lo && ln <= hi && !index.definitions(&r.name).is_empty() {
                out.push(r.name.clone());
            }
        }
    }

    out.sort();
    out.dedup();
    out
}

/// Names that call `name`.
///
/// Scans every Fn/Method definition across the repo; if any ref within its
/// `line_range` matches `name`, that function is a caller.
///
/// # Performance note
/// O(all_fn_defs × refs_per_file) — acceptable for v0 / demo-sized repos.
/// TODO: inverted index — build a name → callers map once at
/// `SymbolIndex::build` time instead of re-scanning every definition on each
/// `call_graph` invocation.
fn callers_of(index: &SymbolIndex, name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    for g in index.search_symbols(None, None) {
        if !is_fn_like(&g.kind) {
            continue;
        }
        let Some(tags) = index.file_tags(&g.location.file) else {
            continue;
        };
        let (lo, hi) = g.line_range;
        let calls_name = tags
            .refs
            .iter()
            .any(|r| r.name == name && r.location.line >= lo && r.location.line <= hi);
        if calls_name {
            out.push(g.name.clone());
        }
    }

    out.sort();
    out.dedup();
    out
}

// ── BFS expander ─────────────────────────────────────────────────────────────

/// If `ancestor` appears in `child`'s parent chain on the BFS traversal tree
/// (or `child == ancestor`, a direct self-call), returns the closed cycle path
/// `[ancestor, …, child, ancestor]`.  Returns `None` for a "cross" edge into a
/// node visited via an unrelated branch — that is not a cycle, just a
/// re-discovery, and is correctly ignored (same spirit as the old DFS's
/// path-vs-visited distinction).
///
/// v0 approximation: this reports edges that close a loop back to an ancestor
/// on the traversal tree actually built, not every cycle latent in the full
/// call graph.
fn find_cycle(
    parent: &HashMap<String, String>,
    child: &str,
    ancestor: &str,
) -> Option<Vec<String>> {
    let mut chain = vec![child.to_string()];
    let mut node = child.to_string();
    while let Some(next) = parent.get(&node) {
        chain.push(next.clone());
        node = next.clone();
    }
    // `chain` is [child, parent(child), parent(parent(child)), …, root].
    let pos = chain.iter().position(|n| n == ancestor)?;
    let mut cycle = chain[..=pos].to_vec(); // [child, …, ancestor]
    cycle.reverse(); // [ancestor, …, child]
    cycle.push(ancestor.to_string()); // close it: [ancestor, …, child, ancestor]
    Some(cycle)
}

/// BFS traversal keyed by node, so the first (and only) visit to any node is
/// its MINIMAL hop count — no in-budget edge is dropped because a node was
/// first reached via a longer path.  Also removes the stack-overflow risk of
/// deep recursion.
///
/// Each dequeued node's neighbour set is computed exactly once: if the node's
/// depth is still under `max_hops` it is expanded (edges recorded, unvisited
/// neighbours enqueued at `depth + 1`); if the node sits exactly at
/// `max_hops`, its neighbours are only checked for emptiness, to set
/// `truncated` without expanding further.
fn bfs_expand(
    index: &SymbolIndex,
    root: &str,
    max_hops: usize,
    edges: &mut Vec<CallEdge>,
    cycles: &mut Vec<Vec<String>>,
    truncated: &mut bool,
    neighbor_fn: fn(&SymbolIndex, &str) -> Vec<String>,
) {
    let mut visited: HashMap<String, usize> = HashMap::from([(root.to_string(), 0)]);
    let mut parent: HashMap<String, String> = HashMap::new();
    let mut queue: VecDeque<String> = VecDeque::from([root.to_string()]);

    while let Some(current) = queue.pop_front() {
        let depth = visited[&current];
        let neighbors = neighbor_fn(index, &current);

        if depth >= max_hops {
            // Boundary node: only check whether it hides further, unexpanded edges.
            if !neighbors.is_empty() {
                *truncated = true;
            }
            continue;
        }

        let child_depth = depth + 1;
        for neighbor in neighbors {
            if visited.contains_key(&neighbor) {
                if let Some(cycle) = find_cycle(&parent, &current, &neighbor) {
                    cycles.push(cycle);
                }
            } else {
                edges.push(CallEdge {
                    from: current.clone(),
                    to: neighbor.clone(),
                    depth: child_depth,
                });
                visited.insert(neighbor.clone(), child_depth);
                parent.insert(neighbor.clone(), current.clone());
                queue.push_back(neighbor);
            }
        }
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Build a call-graph rooted at `symbol`.
///
/// - `max_hops == 0` returns only the root with no edges.
/// - Cyclic graphs are safe: the BFS visited-map guard guarantees termination.
/// - `cycles_detected` entries have the form `[first_node, …, first_node]`
///   (the repeated name closes the cycle path).
/// - Every reported `CallEdge.depth` is the MINIMAL hop count to that node
///   (BFS first-visit order), not merely the hop count of whichever path the
///   traversal happened to try first.
pub fn build(
    index: &SymbolIndex,
    symbol: &str,
    max_hops: usize,
    direction: Direction,
) -> CallGraphResult {
    let mut result = CallGraphResult {
        root: symbol.to_string(),
        callees: Vec::new(),
        callers: Vec::new(),
        cycles_detected: Vec::new(),
        truncated: false,
    };

    if max_hops == 0 {
        return result;
    }

    if matches!(direction, Direction::Callees | Direction::Both) {
        bfs_expand(
            index,
            symbol,
            max_hops,
            &mut result.callees,
            &mut result.cycles_detected,
            &mut result.truncated,
            callees_of,
        );
    }

    if matches!(direction, Direction::Callers | Direction::Both) {
        bfs_expand(
            index,
            symbol,
            max_hops,
            &mut result.callers,
            &mut result.cycles_detected,
            &mut result.truncated,
            callers_of,
        );
    }

    // Both directions may discover the same cycle; deduplicate.
    result.cycles_detected.sort();
    result.cycles_detected.dedup();

    result
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::index::SymbolIndex;

    /// Gate test: the a↔b mutual recursion in tests/fixtures/recursive/recurse.rs
    /// must terminate and report the cycle.
    #[test]
    fn cycle_safe_and_detected() -> anyhow::Result<()> {
        let fixture = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/recursive"
        ));
        let idx = SymbolIndex::build(fixture)?;

        // Sanity: the fixture defs must be present.
        assert!(
            !idx.definitions("a").is_empty(),
            "fixture def 'a' missing — check tests/fixtures/recursive/recurse.rs"
        );
        assert!(!idx.definitions("b").is_empty(), "fixture def 'b' missing");

        // This call MUST RETURN (not hang). The test finishing proves it.
        let g = build(&idx, "a", 10, Direction::Both);

        assert_eq!(g.root, "a");

        // The a↔b cycle must be detected in at least one direction.
        assert!(
            !g.cycles_detected.is_empty(),
            "expected at least one cycle; cycles_detected = {:?}",
            g.cycles_detected
        );

        // Every cycle entry must be a closed path (first == last).
        for cycle in &g.cycles_detected {
            assert!(
                cycle.len() >= 2 && cycle.first() == cycle.last(),
                "cycle path should start and end with the same name: {:?}",
                cycle
            );
        }

        // Callees must contain a→b edge.
        let has_a_to_b = g.callees.iter().any(|e| e.from == "a" && e.to == "b");
        assert!(
            has_a_to_b,
            "expected callee edge a→b; callees = {:?}",
            g.callees
        );

        Ok(())
    }
}
