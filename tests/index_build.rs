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

/// #7: `search_symbols` now iterates a `HashMap` (`defs_by_name`) instead of
/// a dedicated insertion-order `Vec`, so its output must be explicitly
/// sorted by `(file, line)` to stay deterministic.
#[test]
fn search_symbols_output_sorted_by_file_then_line() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let idx = SymbolIndex::build(repo).expect("SymbolIndex::build failed");

    let all = idx.search_symbols(None, None);
    assert!(
        all.len() > 1,
        "expected multiple defs in gitsense's own repo to meaningfully check ordering"
    );

    for pair in all.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let key_a = (&a.location.file, a.location.line);
        let key_b = (&b.location.file, b.location.line);
        assert!(
            key_a <= key_b,
            "search_symbols output not sorted: {}:{} came before {}:{}",
            a.location.file.display(),
            a.location.line,
            b.location.file.display(),
            b.location.line
        );
    }
}
