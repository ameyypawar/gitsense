use std::path::Path;

use gitsense::graph::{self, Direction};
use gitsense::index::SymbolIndex;

/// Verifies that the cycle-safe call-graph builder terminates on a mutually
/// recursive fixture, reports the a↔b cycle, and emits the a→b callee edge.
#[test]
fn call_graph_cycle_safe() -> anyhow::Result<()> {
    let fixture =
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/recursive"));
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
