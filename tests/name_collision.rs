use std::path::Path;

use gitsense::index::{SymbolIndex, SymbolResolution};

/// #8 Part B gating test: `SymbolIndex::resolve_symbol` must not silently
/// pick an arbitrary definition when multiple defs share a name — it must
/// report `Ambiguous` with all candidates, and resolve to exactly one
/// `SymbolDef` once a `file`/`line` disambiguator narrows it down.
///
/// Fixture: tests/fixtures/collision/code.rs
///   impl Alpha { fn new() -> Alpha { .. } }
///   impl Beta  { fn new() -> Beta  { .. } }
#[test]
fn resolve_symbol_disambiguates_name_collision() -> anyhow::Result<()> {
    let fixture = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/collision"
    ));
    let idx = SymbolIndex::build(fixture)?;

    // Sanity: both `new` defs must be present in the raw index.
    let news = idx.definitions("new");
    assert_eq!(news.len(), 2, "expected 2 defs named 'new'; got {:?}", news);

    // No disambiguator -> Ambiguous, listing both candidates. Must NOT
    // silently guess one.
    match idx.resolve_symbol("new", None, None) {
        SymbolResolution::Ambiguous(candidates) => {
            assert_eq!(
                candidates.len(),
                2,
                "expected 2 candidates; got {:?}",
                candidates
            );
        }
        other => panic!("expected Ambiguous, got {:?}", other),
    }

    // Disambiguate with the exact file + line of one candidate -> Resolved
    // to precisely that definition.
    let target = &news[0];
    let file_str = target.location.file.to_string_lossy().into_owned();
    let line = target.location.line;

    match idx.resolve_symbol("new", Some(&file_str), Some(line)) {
        SymbolResolution::Resolved(def) => {
            assert_eq!(def.location.file, target.location.file);
            assert_eq!(def.location.line, target.location.line);
        }
        other => panic!("expected Resolved, got {:?}", other),
    }

    // Both `new` defs live in the same fixture file, so `file` alone (no
    // `line`) does not narrow anything -> still Ambiguous, with the
    // (file-matching) candidate set.
    match idx.resolve_symbol("new", Some(&file_str), None) {
        SymbolResolution::Ambiguous(candidates) => {
            assert_eq!(
                candidates.len(),
                2,
                "file alone matches both same-file defs; expected 2 candidates, got {:?}",
                candidates
            );
        }
        other => panic!(
            "expected Ambiguous when file alone doesn't narrow to one def, got {:?}",
            other
        ),
    }

    // Unknown name -> NotFound.
    match idx.resolve_symbol("no_such_symbol_xyz", None, None) {
        SymbolResolution::NotFound => {}
        other => panic!("expected NotFound, got {:?}", other),
    }

    // A unique (non-colliding) name resolves regardless of file/line.
    match idx.resolve_symbol("Alpha", None, None) {
        SymbolResolution::Resolved(def) => assert_eq!(def.name, "Alpha"),
        other => panic!("expected Resolved for unique name 'Alpha', got {:?}", other),
    }

    Ok(())
}
