use std::path::Path;

use gitsense::graph::{self, Direction};
use gitsense::index::SymbolIndex;

/// Fix #1 gating test: `line_range` must span the full function body for a
/// multi-line function, and `call_graph` must return non-empty callees.
///
/// Fixture: tests/fixtures/multiline/code.rs
///   pub fn outer() {   // line 1
///       helper();      // line 2
///       helper();      // line 3
///   }                  // line 4
///   fn helper() {}     // line 5
///
/// Before the fix: `outer.line_range == (1, 1)` (name span only), so
/// `call_graph` finds no callees because lines 2-3 are outside the range.
/// After the fix:  `outer.line_range == (1, 4)` (full body), so
/// `call_graph` finds the `helper` callee on lines 2 and 3.  Refs #9.
#[test]
fn multiline_line_range_and_call_graph() -> anyhow::Result<()> {
    let fixture = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/multiline"
    ));
    let idx = SymbolIndex::build(fixture)?;

    // ── line_range check ─────────────────────────────────────────────────────

    let outer_defs = idx.definitions("outer");
    assert!(
        !outer_defs.is_empty(),
        "def 'outer' missing — check tests/fixtures/multiline/code.rs"
    );
    let outer = &outer_defs[0];

    assert!(
        outer.line_range.1 > outer.line_range.0,
        "outer's line_range must span multiple lines (start < end); got {:?}",
        outer.line_range
    );
    assert!(
        outer.line_range.1.saturating_sub(outer.line_range.0) >= 2,
        "outer spans at least 3 lines (end - start ≥ 2); got {:?}",
        outer.line_range
    );

    // ── call_graph callees check ─────────────────────────────────────────────

    let g = graph::build(&idx, "outer", 3, Direction::Callees);

    assert!(
        !g.callees.is_empty(),
        "call_graph(outer, Callees) must be non-empty after line_range fix; callees = {:?}",
        g.callees
    );

    let has_outer_to_helper = g
        .callees
        .iter()
        .any(|e| e.from == "outer" && e.to == "helper");
    assert!(
        has_outer_to_helper,
        "expected callee edge outer→helper; callees = {:?}",
        g.callees
    );

    Ok(())
}
