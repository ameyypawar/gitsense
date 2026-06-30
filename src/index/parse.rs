//! Phase 2: tree-sitter tag extraction for Rust source files.
//!
//! Uses `tree_sitter_rust::TAGS_QUERY` (definitions only) augmented with a
//! custom `@reference.*` query (call_expression / macro_invocation) so that a
//! single `generate_tags` pass yields both `SymbolDef` and `SymbolRef`.

use std::path::Path;
use std::sync::atomic::AtomicUsize;

use anyhow::Context;
use tree_sitter_tags::{TagsConfiguration, TagsContext};

use super::model::{Location, SymbolDef, SymbolKind, SymbolRef};

/// Additional `@reference.*` patterns appended to the crate's TAGS_QUERY.
///
/// Covers three call forms:
///   - direct identifier calls: `foo()`
///   - method calls: `obj.method()` (field_expression → field_identifier)
///   - path/associated calls: `Type::assoc()` (scoped_identifier → identifier)
///   - macro invocations: `mac!()`
///
/// Each pattern has an `@name` sub-capture (the rightmost identifier that names
/// the callee) and an `@reference.call` outer capture on the call expression.
/// tree-sitter-tags uses the `@name` range to extract the callee text.
const CUSTOM_REFS_QUERY: &str = r#"
; direct function-call references: foo()
(call_expression
  function: (identifier) @name) @reference.call

; method-call references: obj.method()
(call_expression
  function: (field_expression
    field: (field_identifier) @name)) @reference.call

; path/associated-call references: Type::assoc() or mod::func()
(call_expression
  function: (scoped_identifier
    name: (identifier) @name)) @reference.call

; macro-call references: mac!()
(macro_invocation
  macro: (identifier) @name) @reference.call
"#;

/// tree-sitter query that captures the full enclosing item node for every
/// definition kind emitted by tree-sitter-rust's tags.scm.
///
/// Used in a second parse pass inside `extract()` to widen `line_range` from
/// the name-node span to the full item body.
const BODY_RANGE_QUERY: &str = r"
(function_item) @def
(impl_item) @def
(struct_item) @def
(enum_item) @def
(trait_item) @def
(mod_item) @def
(const_item) @def
(macro_definition) @def
";

/// Single-language tagger — holds the parsed `TagsConfiguration` and a
/// reusable `TagsContext` (owns the tree-sitter `Parser` + `QueryCursor`).
/// Build once, call `extract` many times.
pub struct RustTagger {
    config: TagsConfiguration,
    ctx: TagsContext,
    /// Pre-compiled query for the body-range second pass.
    body_query: tree_sitter::Query,
}

impl RustTagger {
    /// Build a `RustTagger` from the tree-sitter-rust 0.24.2 grammar.
    ///
    /// Combines the crate's `TAGS_QUERY` (definitions) with `CUSTOM_REFS_QUERY`
    /// (references) into a single `TagsConfiguration`.  Also pre-compiles
    /// `BODY_RANGE_QUERY` for the second-pass body-range extraction in `extract`.
    pub fn new() -> anyhow::Result<RustTagger> {
        // LanguageFn → Language via Into<Language> (tree-sitter-language 0.1).
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();

        // Body-range query: pre-compiled here so `extract` doesn't rebuild it
        // on every call.  Uses the language value before it moves into config.
        let body_query = tree_sitter::Query::new(&language, BODY_RANGE_QUERY)
            .context("failed to build body range query")?;

        // Append our reference patterns so generate_tags yields SymbolRef too.
        let combined_query = format!("{}\n{}", tree_sitter_rust::TAGS_QUERY, CUSTOM_REFS_QUERY);

        let config = TagsConfiguration::new(language, &combined_query, "")
            .context("failed to build TagsConfiguration for Rust")?;

        Ok(RustTagger {
            config,
            ctx: TagsContext::new(),
            body_query,
        })
    }

    /// Extract definitions and references from `source` bytes.
    ///
    /// `path` is used only to populate `Location::file`; it is not read from disk.
    ///
    /// Two passes are performed:
    /// 1. `BODY_RANGE_QUERY` parse — collects (start_byte, end_byte, start_row,
    ///    end_row) for every definition-node type.  For each definition tag the
    ///    *smallest* enclosing body node determines `line_range`; the name span
    ///    is kept for `location`.
    /// 2. `generate_tags` — yields `SymbolDef` and `SymbolRef` entries.
    pub fn extract(
        &mut self,
        source: &[u8],
        path: &Path,
    ) -> anyhow::Result<(Vec<SymbolDef>, Vec<SymbolRef>)> {
        // ── Pass 1: body-range query ──────────────────────────────────────────
        // Build a second parser for the full-body node ranges.  We use a fresh
        // Language conversion here (a static fn-pointer, negligible cost) so
        // that `self.body_query` can be borrowed immutably at the same time as
        // `self.ctx` is borrowed mutably in the tags pass below.
        let body_ranges: Vec<(usize, usize, usize, usize)> = {
            let body_lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
            let mut body_parser = tree_sitter::Parser::new();
            body_parser
                .set_language(&body_lang)
                .context("body-range parser: set_language failed")?;
            let tree = body_parser
                .parse(source, None)
                .context("body-range parse returned None")?;

            // tree-sitter 0.25: QueryMatches implements StreamingIterator, not
            // the standard Iterator.  Use the re-exported StreamingIterator
            // trait (tree_sitter::StreamingIterator) for the advance/get loop.
            use tree_sitter::StreamingIterator as _;
            let mut cursor = tree_sitter::QueryCursor::new();
            let mut ranges: Vec<(usize, usize, usize, usize)> = Vec::new();
            let mut qm = cursor.matches(
                &self.body_query,
                tree.root_node(),
                |n: tree_sitter::Node| std::iter::once(&source[n.start_byte()..n.end_byte()]),
            );
            while let Some(m) = qm.next() {
                for cap in m.captures {
                    let n = cap.node;
                    ranges.push((
                        n.start_byte(),
                        n.end_byte(),
                        n.start_position().row,
                        n.end_position().row,
                    ));
                }
            }
            ranges
            // body_parser, tree, cursor all dropped here
        };

        // ── Pass 2: generate_tags ─────────────────────────────────────────────
        let cancel: Option<&AtomicUsize> = None;
        let (iter, _has_parse_error) = self
            .ctx
            .generate_tags(&self.config, source, cancel)
            .context("generate_tags failed")?;

        let mut defs = Vec::new();
        let mut refs = Vec::new();

        for tag_result in iter {
            let tag = tag_result.context("tag iterator error")?;

            // Name text from source bytes (must be valid UTF-8 for Rust source).
            let name = std::str::from_utf8(&source[tag.name_range.clone()])
                .context("tag name was not valid UTF-8")?
                .to_owned();

            // `tag.span` covers the name identifier node; rows/cols are 0-based.
            let start_row = tag.span.start.row; // 0-based
            let start_col = tag.span.start.column; // 0-based

            let location = Location {
                file: path.to_path_buf(),
                line: start_row + 1, // 1-based — name position, not body start
                col: start_col + 1,  // 1-based
            };

            if tag.is_definition {
                // v0 is_pub heuristic: `tag.line_range` is already stripped of
                // leading whitespace by the tags iterator; check if the line
                // begins with "pub " or "pub(" (visibility qualifiers).
                let line_bytes = &source[tag.line_range.clone()];
                let is_pub = line_bytes.starts_with(b"pub ") || line_bytes.starts_with(b"pub(");

                let kind = kind_from_syntax_name(self.config.syntax_type_name(tag.syntax_type_id));

                // Find the smallest body-range node that contains the name span.
                // The name_range bytes must fall entirely within the node.
                // Using min by node size (end_byte - start_byte) ensures we pick
                // the innermost match (e.g. a method function_item rather than
                // the enclosing impl_item).
                let name_start = tag.name_range.start;
                let name_end = tag.name_range.end;
                let (body_start_row, body_end_row) = body_ranges
                    .iter()
                    .filter(|(bs, be, _, _)| *bs <= name_start && name_end <= *be)
                    .min_by_key(|(bs, be, _, _)| be - bs)
                    .map(|&(_, _, sr, er)| (sr, er))
                    .unwrap_or((start_row, start_row)); // fallback: single-line

                defs.push(SymbolDef {
                    name,
                    kind,
                    location,
                    // line_range now spans the full item body (1-based, inclusive).
                    line_range: (body_start_row + 1, body_end_row + 1),
                    is_pub,
                    docs: tag.docs,
                });
            } else {
                refs.push(SymbolRef { name, location });
            }
        }

        Ok((defs, refs))
    }
}

/// Map `syntax_type_name` string → `SymbolKind`.
///
/// The Rust `tags.scm` uses these kind strings (derived from capture names):
/// | kind string  | source construct                              |
/// |--------------|-----------------------------------------------|
/// | "function"   | `fn` at module/file scope                     |
/// | "method"     | `fn` inside a `declaration_list` (impl block) |
/// | "class"      | `struct`, `enum`, `union`, type alias (v0: all → Struct) |
/// | "interface"  | `trait`                                        |
/// | "module"     | `mod`                                          |
/// | "macro"      | `macro_definition`                             |
///
/// Reference captures use kind "call"; those are already routed as `SymbolRef`
/// (non-definition tags) and should never reach this function.
fn kind_from_syntax_name(name: &str) -> SymbolKind {
    match name {
        "function" => SymbolKind::Fn,
        "method" => SymbolKind::Method,
        // v0: struct/enum/union/type alias all map to @definition.class in
        // tree-sitter-rust's tags.scm, so we can't distinguish them here.
        "class" => SymbolKind::Struct,
        "interface" => SymbolKind::Trait,
        "module" => SymbolKind::Mod,
        "macro" => SymbolKind::Macro,
        _ => SymbolKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Three-line snippet exercising all branches of the gate test.
    const SNIPPET: &[u8] = b"pub fn alpha() { beta(); }\nfn beta() {}\nstruct Thing;\n";

    #[test]
    fn gate_test() {
        let mut tagger = RustTagger::new().expect("RustTagger::new failed");
        let (defs, refs) = tagger
            .extract(SNIPPET, Path::new("test.rs"))
            .expect("extract failed");

        // ── definitions ──────────────────────────────────────────────────────

        let alpha = defs
            .iter()
            .find(|d| d.name == "alpha")
            .expect("def 'alpha' missing");
        assert!(
            matches!(alpha.kind, SymbolKind::Fn),
            "alpha: expected Fn, got {:?}",
            alpha.kind
        );
        assert!(alpha.is_pub, "alpha should be pub");
        assert_eq!(alpha.location.line, 1, "alpha on line 1");

        let beta_def = defs
            .iter()
            .find(|d| d.name == "beta")
            .expect("def 'beta' missing");
        assert!(
            matches!(beta_def.kind, SymbolKind::Fn),
            "beta: expected Fn, got {:?}",
            beta_def.kind
        );
        assert!(!beta_def.is_pub, "beta should not be pub");
        assert_eq!(beta_def.location.line, 2, "beta on line 2");

        let thing = defs
            .iter()
            .find(|d| d.name == "Thing")
            .expect("def 'Thing' missing");
        assert!(
            matches!(thing.kind, SymbolKind::Struct),
            "Thing: expected Struct, got {:?}",
            thing.kind
        );
        assert_eq!(thing.location.line, 3, "Thing on line 3");

        // ── references ───────────────────────────────────────────────────────

        let beta_ref = refs.iter().find(|r| r.name == "beta");
        assert!(
            beta_ref.is_some(),
            "expected ≥1 ref named 'beta', got refs: {:?}",
            refs.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
    }

    /// Single-line functions must not regress: alpha.line_range == (1, 1) still.
    #[test]
    fn single_line_fn_range_unchanged() {
        let mut tagger = RustTagger::new().expect("RustTagger::new failed");
        let (defs, _) = tagger
            .extract(SNIPPET, Path::new("test.rs"))
            .expect("extract failed");

        let alpha = defs.iter().find(|d| d.name == "alpha").unwrap();
        // alpha is single-line: body spans exactly 1 line
        assert_eq!(
            alpha.line_range,
            (1, 1),
            "single-line fn should have line_range (1,1); got {:?}",
            alpha.line_range
        );
    }

    /// Multi-line function: line_range must span the full body.
    #[test]
    fn multiline_fn_body_range() {
        let src = b"pub fn outer() {\n    helper();\n    helper();\n}\nfn helper() {}\n";
        let mut tagger = RustTagger::new().expect("RustTagger::new failed");
        let (defs, _) = tagger
            .extract(src, Path::new("multi.rs"))
            .expect("extract failed");

        let outer = defs
            .iter()
            .find(|d| d.name == "outer")
            .expect("def 'outer' missing");
        assert!(
            outer.line_range.1 > outer.line_range.0,
            "outer line_range should span multiple lines; got {:?}",
            outer.line_range
        );
        assert!(
            outer.line_range.1 - outer.line_range.0 >= 2,
            "outer spans at least 3 lines (diff ≥ 2); got {:?}",
            outer.line_range
        );
    }

    /// Method and path-call references must be captured.
    #[test]
    fn method_and_path_call_refs() {
        let src = b"struct S;\nimpl S { fn m(&self) {} fn make() -> S { S } }\nfn driver() { let s = S::make(); s.m(); }\n";
        let mut tagger = RustTagger::new().expect("RustTagger::new failed");
        let (_, refs) = tagger
            .extract(src, Path::new("calls.rs"))
            .expect("extract failed");

        assert!(
            refs.iter().any(|r| r.name == "m"),
            "method call s.m() not captured; refs = {:?}",
            refs.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
        assert!(
            refs.iter().any(|r| r.name == "make"),
            "path call S::make() not captured; refs = {:?}",
            refs.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
    }
}
