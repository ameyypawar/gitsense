use std::path::Path;

use gitsense::index::SymbolIndex;

/// #9 regression test: `is_pub` detection in `index/parse.rs` relies on
/// `tree-sitter-tags` handing back a whitespace-TRIMMED `tag.line_range`, so
/// a plain `line.starts_with("pub ")` check works even for indented items.
/// This was verified true for tree-sitter-tags 0.25.9/0.25.10 during the #6
/// conflict reconciliation, but was never locked down by a test — a future
/// dep bump could silently break `is_pub` for indented items (methods inside
/// `impl`, nested items) with nothing to catch it.
///
/// Fixture: tests/fixtures/indented_pub/code.rs
///   struct S;
///   impl S {
///       pub fn visible(&self) {}
///       fn hidden(&self) {}
///   }
#[test]
fn indented_pub_detected_despite_leading_whitespace() -> anyhow::Result<()> {
    let fixture = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/indented_pub"
    ));
    let idx = SymbolIndex::build(fixture)?;

    let visible_defs = idx.definitions("visible");
    assert!(
        !visible_defs.is_empty(),
        "def 'visible' missing — check tests/fixtures/indented_pub/code.rs"
    );
    assert!(
        visible_defs[0].is_pub,
        "indented 'pub fn visible' must be detected as pub; got is_pub = false"
    );

    let hidden_defs = idx.definitions("hidden");
    assert!(
        !hidden_defs.is_empty(),
        "def 'hidden' missing — check tests/fixtures/indented_pub/code.rs"
    );
    assert!(
        !hidden_defs[0].is_pub,
        "indented non-pub 'fn hidden' must not be flagged pub; got is_pub = true"
    );

    Ok(())
}
