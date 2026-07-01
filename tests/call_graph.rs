use std::path::Path;

use gitsense::graph::{self, Direction};
use gitsense::index::SymbolIndex;

/// Verifies that the cycle-safe call-graph builder terminates on a mutually
/// recursive fixture, reports the a↔b cycle, and emits the a→b callee edge.
#[test]
fn call_graph_cycle_safe() -> anyhow::Result<()> {
    let fixture = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/recursive"
    ));
    let idx = SymbolIndex::build(fixture)?;

    // MUST RETURN — finishing the test proves no infinite loop.
    let g = graph::build(&idx, "a", 10, Direction::Both);

    assert_eq!(g.root, "a");

    // Cycle must be detected.
    assert!(
        !g.cycles_detected.is_empty(),
        "expected a↔b cycle; cycles_detected = {:?}",
        g.cycles_detected
    );

    // Every recorded cycle must be a closed path (first name == last name).
    for cycle in &g.cycles_detected {
        assert!(
            cycle.len() >= 2 && cycle.first() == cycle.last(),
            "cycle not closed: {:?}",
            cycle
        );
    }

    // The direct callee edge a→b must be present at depth 1.
    let edge = g.callees.iter().find(|e| e.from == "a" && e.to == "b");
    assert!(
        edge.is_some(),
        "callee edge a→b missing; callees = {:?}",
        g.callees
    );
    assert_eq!(edge.unwrap().depth, 1, "a→b should be at depth 1");

    Ok(())
}

/// Fix #4 regression: BFS must report the MINIMAL hop count to a node
/// reachable both directly and via a longer path.
///
/// Fixture: tests/fixtures/diamond/code.rs
///   fn a() { b(); c(); }   // a→b and a→c, both depth 1
///   fn b() { c(); }        // a→b→c would be depth 2
///   fn c() {}
///
/// The old DFS traversed `b` before `c` (sorted neighbor order) and recursed
/// into `b` first, reaching `c` at depth 2 via `b` and recording that edge
/// before the direct depth-1 a→c edge — so a first-match lookup returned the
/// wrong (longer) depth. BFS visits `c` for the first time via the direct
/// edge, so exactly one edge targets `c`, at depth 1.
#[test]
fn call_graph_reports_minimal_depth() -> anyhow::Result<()> {
    let fixture = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/diamond"
    ));
    let idx = SymbolIndex::build(fixture)?;

    let g = graph::build(&idx, "a", 3, Direction::Callees);

    let edges_to_c: Vec<_> = g.callees.iter().filter(|e| e.to == "c").collect();
    assert_eq!(
        edges_to_c.len(),
        1,
        "expected exactly one edge reaching 'c'; callees = {:?}",
        g.callees
    );
    assert_eq!(
        edges_to_c[0].depth, 1,
        "edge reaching 'c' should be the direct depth-1 edge, not the depth-2 path via 'b'; callees = {:?}",
        g.callees
    );

    Ok(())
}

/// Fix #8 Part B: `graph::build_rooted_at` must scope the ROOT's callees to
/// one exact definition, not union across every def sharing its name.
///
/// Fixture: tests/fixtures/collision_calls/{a,b}.rs — each defines its own
/// `fn helper()` that calls a different target (`target_a` / `target_b`).
/// The old name-based `graph::build` unions both; `build_rooted_at`, given
/// one specific `helper` definition, must see only that definition's own
/// callee.
#[test]
fn build_rooted_at_scopes_callees_to_exact_def() -> anyhow::Result<()> {
    let fixture = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/collision_calls"
    ));
    let idx = SymbolIndex::build(fixture)?;

    let helpers = idx.definitions("helper");
    assert_eq!(
        helpers.len(),
        2,
        "expected 2 'helper' defs; got {:?}",
        helpers
    );

    // Baseline: name-based `build` unions callees across BOTH same-named defs.
    let unioned = graph::build(&idx, "helper", 2, Direction::Callees);
    let unioned_names: Vec<&str> = unioned.callees.iter().map(|e| e.to.as_str()).collect();
    assert!(
        unioned_names.contains(&"target_a") && unioned_names.contains(&"target_b"),
        "expected build() to union both siblings' callees; got {:?}",
        unioned_names
    );

    // build_rooted_at, scoped to one specific def, must see ONLY that def's
    // own callee — never the sibling's.
    for def in helpers {
        let g = graph::build_rooted_at(&idx, def, 2, Direction::Callees);
        let names: Vec<&str> = g.callees.iter().map(|e| e.to.as_str()).collect();
        assert_eq!(
            names.len(),
            1,
            "rooted-at graph should see exactly 1 direct callee; got {:?}",
            names
        );
        assert!(
            names[0] == "target_a" || names[0] == "target_b",
            "unexpected callee {:?}",
            names
        );
    }

    Ok(())
}
