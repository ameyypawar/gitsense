use std::path::Path;

use gitsense::index::model::SymbolKind;
use gitsense::index::SymbolIndex;

/// #8 Part A gating test: `enum` must be distinguishable from `struct` via
/// the enclosing item-node kind, and the `search_symbols` kind filter must
/// actually select it (not return empty, and not conflate it with struct).
///
/// Fixture: tests/fixtures/kinds/code.rs
///   struct Widget { id: u32 }
///   enum Shape { Circle, Square }
///   trait Drawable { fn draw(&self); }
#[test]
fn enum_struct_trait_kinds_are_distinct_and_filterable() -> anyhow::Result<()> {
    let fixture = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/kinds"));
    let idx = SymbolIndex::build(fixture)?;

    let widget = idx
        .search_symbols(Some("Widget"), None)
        .into_iter()
        .find(|d| d.name == "Widget")
        .expect("def 'Widget' missing");
    assert_eq!(
        widget.kind,
        SymbolKind::Struct,
        "Widget should be Struct, got {:?}",
        widget.kind
    );

    let shape = idx
        .search_symbols(Some("Shape"), None)
        .into_iter()
        .find(|d| d.name == "Shape")
        .expect("def 'Shape' missing");
    assert_eq!(
        shape.kind,
        SymbolKind::Enum,
        "Shape should be Enum, got {:?}",
        shape.kind
    );

    let drawable = idx
        .search_symbols(Some("Drawable"), None)
        .into_iter()
        .find(|d| d.name == "Drawable")
        .expect("def 'Drawable' missing");
    assert_eq!(
        drawable.kind,
        SymbolKind::Trait,
        "Drawable should be Trait, got {:?}",
        drawable.kind
    );

    // The kind filter must select the enum, not empty and not the struct.
    let enum_only = idx.search_symbols(None, Some(SymbolKind::Enum));
    assert!(
        enum_only.iter().any(|d| d.name == "Shape"),
        "search_symbols(kind=Enum) must return 'Shape'; got {:?}",
        enum_only.iter().map(|d| &d.name).collect::<Vec<_>>()
    );
    assert!(
        !enum_only.iter().any(|d| d.name == "Widget"),
        "search_symbols(kind=Enum) must NOT return the struct 'Widget'; got {:?}",
        enum_only.iter().map(|d| &d.name).collect::<Vec<_>>()
    );

    // And the struct filter must not pick up the enum either.
    let struct_only = idx.search_symbols(None, Some(SymbolKind::Struct));
    assert!(struct_only.iter().any(|d| d.name == "Widget"));
    assert!(!struct_only.iter().any(|d| d.name == "Shape"));

    Ok(())
}
