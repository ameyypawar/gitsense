use std::path::Path;

use gitsense::index::SymbolIndex;

#[test]
fn build_indexes_own_repo() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let idx = SymbolIndex::build(repo).expect("SymbolIndex::build failed");

    let stats = idx.stats();

    assert!(stats.def_count > 0, "expected at least one def, got 0");
    assert!(
        stats.file_count >= 3,
        "expected >= 3 files parsed, got {}",
        stats.file_count
    );

    // The `build` function itself must appear as a definition.
    let build_hits = idx.search_symbols(Some("build"), None);
    assert!(
        build_hits.iter().any(|d| d.name == "build"),
        "expected a def named exactly 'build'; got: {:?}",
        build_hits.iter().map(|d| &d.name).collect::<Vec<_>>()
    );
}
