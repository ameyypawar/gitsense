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

use std::collections::HashSet;

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
/// For each Fn/Method definition named `name`, collects every `SymbolRef` in
/// the same file whose source line falls within the definition's `line_range`,
/// keeping only names that resolve to at least one known definition.
fn callees_of(index: &SymbolIndex, name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    for def in index.search_symbols(None, None) {
        if def.name != name || !is_fn_like(&def.kind) {
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

// ── DFS expander ─────────────────────────────────────────────────────────────

/// Recursive DFS with explicit path tracking for cycle detection.
///
/// `path` always contains the names from the root down to (and including)
/// `current`.  When a neighbour already appears in `path`, the slice from its
/// first occurrence to `current` plus the repeated name is recorded as a cycle.
#[allow(clippy::too_many_arguments)]
fn expand(
    index: &SymbolIndex,
    current: &str,
    current_depth: usize,
    max_hops: usize,
    path: &mut Vec<String>,
    visited: &mut HashSet<String>,
    edges: &mut Vec<CallEdge>,
    cycles: &mut Vec<Vec<String>>,
    truncated: &mut bool,
    neighbor_fn: fn(&SymbolIndex, &str) -> Vec<String>,
) {
    let neighbors = neighbor_fn(index, current);
    let child_depth = current_depth + 1;

    for neighbor in neighbors {
        // Always record the edge, regardless of cycle / visited status.
        edges.push(CallEdge {
            from: current.to_string(),
            to: neighbor.clone(),
            depth: child_depth,
        });

        if let Some(pos) = path.iter().position(|n| n == &neighbor) {
            // `neighbor` is already on the current DFS path → cycle.
            // Encode: path[pos..] + [neighbor] (the repeated node closes it).
            let mut cycle = path[pos..].to_vec();
            cycle.push(neighbor.clone());
            cycles.push(cycle);
        } else if !visited.contains(&neighbor) {
            if child_depth >= max_hops {
                // Depth limit reached; flag truncation if the node has further neighbors.
                if !neighbor_fn(index, &neighbor).is_empty() {
                    *truncated = true;
                }
            } else {
                visited.insert(neighbor.clone());
                path.push(neighbor.clone());
                expand(
                    index,
                    &neighbor,
                    child_depth,
                    max_hops,
                    path,
                    visited,
                    edges,
                    cycles,
                    truncated,
                    neighbor_fn,
                );
                path.pop();
            }
        }
        // If neighbor is already in `visited` but NOT in `path`, it was fully
        // expanded via another branch — skip to avoid duplicate subgraph traversal.
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Build a call-graph rooted at `symbol`.
///
/// - `max_hops == 0` returns only the root with no edges.
/// - Cyclic graphs are safe: the DFS path guard guarantees termination.
/// - `cycles_detected` entries have the form `[first_node, …, first_node]`
///   (the repeated name closes the cycle path).
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
        let mut path = vec![symbol.to_string()];
        let mut visited: HashSet<String> = HashSet::from([symbol.to_string()]);
        expand(
            index,
            symbol,
            0,
            max_hops,
            &mut path,
            &mut visited,
            &mut result.callees,
            &mut result.cycles_detected,
            &mut result.truncated,
            callees_of,
        );
    }

    if matches!(direction, Direction::Callers | Direction::Both) {
        let mut path = vec![symbol.to_string()];
        let mut visited: HashSet<String> = HashSet::from([symbol.to_string()]);
        expand(
            index,
            symbol,
            0,
            max_hops,
            &mut path,
            &mut visited,
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
