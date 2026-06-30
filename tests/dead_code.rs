use std::path::Path;

use gitsense::index::SymbolIndex;

/// Verifies that `unreferenced_defs()` — the underlying path of the
/// `find_dead_code` MCP tool — correctly identifies an unused function
/// while excluding one that is explicitly called within the same file.
///
/// The fixture at `tests/fixtures/dead_code/ghost.rs` contains:
///   - `truly_dead` — never called anywhere in the fixture → must appear
///   - `referenced` — called by `caller` → must NOT appear
///
/// No `.git` directory is required; the test is deterministic.
#[test]
fn dead_code_finds_unreferenced_fn() -> anyhow::Result<()> {
    let fixture = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/dead_code"
    ));

    let idx = SymbolIndex::build(fixture)?;
    let dead = idx.unreferenced_defs();
    let dead_names: Vec<&str> = dead.iter().map(|d| d.name.as_str()).collect();

    assert!(
        dead_names.contains(&"truly_dead"),
        "expected 'truly_dead' in unreferenced_defs; got: {:?}",
        dead_names
    );

    assert!(
        !dead_names.contains(&"referenced"),
        "'referenced' must not appear in unreferenced_defs (it is called by 'caller'); got: {:?}",
        dead_names
    );

    Ok(())
}
