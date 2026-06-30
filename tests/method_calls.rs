use std::path::Path;

use gitsense::index::SymbolIndex;

/// Fix #2 gating test: method calls (`s.m()`) and path/associated calls
/// (`S::make()`) must produce `SymbolRef` entries.
///
/// Fixture: tests/fixtures/method_calls/code.rs
///   struct S;
///   impl S {
///       fn m(&self) {}
///       fn make() -> S { S }
///   }
///   fn driver() {
///       let s = S::make();   // scoped_identifier → captures "make"
///       s.m();               // field_expression  → captures "m"
///   }
///
/// Before the fix: `CUSTOM_REFS_QUERY` only captures bare `ident()` calls,
/// so `.m()` and `S::make()` produce no SymbolRef, and both appear in
/// `find_dead_code`.  After the fix: both are captured.  Refs #9.
#[test]
fn method_and_path_call_refs_captured() -> anyhow::Result<()> {
    let fixture = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/method_calls"
    ));
    let idx = SymbolIndex::build(fixture)?;

    // s.m() must produce a SymbolRef for "m".
    let m_refs = idx.find_references("m");
    assert!(
        !m_refs.is_empty(),
        "find_references('m') must be non-empty (the s.m() call is not captured); \
         refs: {:?}",
        idx.find_references("m")
    );

    // S::make() must produce a SymbolRef for "make".
    let make_refs = idx.find_references("make");
    assert!(
        !make_refs.is_empty(),
        "find_references('make') must be non-empty (the S::make() call is not captured); \
         refs: {:?}",
        idx.find_references("make")
    );

    // Sanity: with refs captured, neither m nor make should be in unreferenced_defs.
    let dead_names: Vec<&str> = idx
        .unreferenced_defs()
        .iter()
        .map(|d| d.name.as_str())
        .collect();

    assert!(
        !dead_names.contains(&"m"),
        "'m' must not appear in unreferenced_defs after Fix #2; dead = {:?}",
        dead_names
    );
    assert!(
        !dead_names.contains(&"make"),
        "'make' must not appear in unreferenced_defs after Fix #2; dead = {:?}",
        dead_names
    );

    Ok(())
}
