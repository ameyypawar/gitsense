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
/// The crate's `queries/tags.scm` only has `@definition.*` captures; these
/// patterns add call-site and macro references that produce `SymbolRef`
/// entries.  We only capture the common `(identifier)` function node — enough
/// for the gate test and basic call-graph use in Phase 3.
const CUSTOM_REFS_QUERY: &str = r#"
; function-call references (direct identifier only — covers the common case)
(call_expression
  function: (identifier) @name) @reference.call

; macro-call references
(macro_invocation
  macro: (identifier) @name) @reference.call
"#;

/// Single-language tagger — holds the parsed `TagsConfiguration` and a
/// reusable `TagsContext` (owns the tree-sitter `Parser` + `QueryCursor`).
/// Build once, call `extract` many times.
pub struct RustTagger {
    config: TagsConfiguration,
    ctx: TagsContext,
}

impl RustTagger {
    /// Build a `RustTagger` from the tree-sitter-rust 0.24.2 grammar.
    ///
    /// Combines the crate's `TAGS_QUERY` (definitions) with `CUSTOM_REFS_QUERY`
    /// (references) into a single `TagsConfiguration`.
    pub fn new() -> anyhow::Result<RustTagger> {
        // LanguageFn → Language via Into<Language> (tree-sitter-language 0.1).
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();

        // Append our reference patterns so generate_tags yields SymbolRef too.
        let combined_query = format!("{}\n{}", tree_sitter_rust::TAGS_QUERY, CUSTOM_REFS_QUERY);

        let config = TagsConfiguration::new(language, &combined_query, "")
            .context("failed to build TagsConfiguration for Rust")?;

        Ok(RustTagger {
            config,
            ctx: TagsContext::new(),
        })
    }

    /// Extract definitions and references from `source` bytes.
    ///
    /// `path` is used only to populate `Location::file`; it is not read from disk.
    pub fn extract(
        &mut self,
        source: &[u8],
        path: &Path,
    ) -> anyhow::Result<(Vec<SymbolDef>, Vec<SymbolRef>)> {
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
            let end_row = tag.span.end.row; // 0-based

            let location = Location {
                file: path.to_path_buf(),
                line: start_row + 1, // 1-based
                col: start_col + 1,  // 1-based
            };

            if tag.is_definition {
                // v0 is_pub heuristic: `tag.line_range` is already stripped of
                // leading whitespace by the tags iterator; check if the line
                // begins with "pub " or "pub(" (visibility qualifiers).
                let line_bytes = &source[tag.line_range.clone()];
                let is_pub = line_bytes.starts_with(b"pub ") || line_bytes.starts_with(b"pub(");

                let kind = kind_from_syntax_name(self.config.syntax_type_name(tag.syntax_type_id));

                defs.push(SymbolDef {
                    name,
                    kind,
                    location,
                    // line_range is (start, end) of the name span, 1-based.
                    // Phase 3 can widen this to the full body via node ranges.
                    line_range: (start_row + 1, end_row + 1),
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
}
