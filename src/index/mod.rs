pub mod model;
pub mod parse;
pub mod walk;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use model::{SymbolDef, SymbolKind, SymbolRef};
use parse::RustTagger;
use walk::collect_rust_files;

/// All definitions and references extracted from a single file.
pub struct FileTags {
    pub defs: Vec<SymbolDef>,
    pub refs: Vec<SymbolRef>,
}

/// Aggregate statistics returned by [`SymbolIndex::stats`].
#[derive(Debug, Serialize)]
pub struct RepoStats {
    pub file_count: usize,
    pub def_count: usize,
    pub ref_count: usize,
    /// Per-kind counts as `(kind_name, count)` sorted by count descending.
    pub by_kind: Vec<(String, usize)>,
    /// Module names (`mod` defs), or file stems if no `Mod`-kind defs exist.
    pub modules: Vec<String>,
}

/// In-memory cross-file symbol index built from a Rust workspace.
pub struct SymbolIndex {
    pub repo_root: PathBuf,
    defs_by_name: HashMap<String, Vec<SymbolDef>>,
    refs_by_name: HashMap<String, Vec<SymbolRef>>,
    tags_by_file: HashMap<PathBuf, FileTags>,
    all_defs: Vec<SymbolDef>,
}

impl SymbolIndex {
    /// Walk `repo_root`, parse every `.rs` file with tree-sitter, and build
    /// the index.  Per-file errors are logged and skipped; they do not abort
    /// the build.
    pub fn build(repo_root: &Path) -> anyhow::Result<SymbolIndex> {
        let files = collect_rust_files(repo_root)?;
        let mut tagger = RustTagger::new()?;

        let mut defs_by_name: HashMap<String, Vec<SymbolDef>> = HashMap::new();
        let mut refs_by_name: HashMap<String, Vec<SymbolRef>> = HashMap::new();
        let mut tags_by_file: HashMap<PathBuf, FileTags> = HashMap::new();
        let mut all_defs: Vec<SymbolDef> = Vec::new();

        let mut parsed_files = 0usize;
        let mut total_refs = 0usize;

        for path in &files {
            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!("skip {}: read error: {e}", path.display());
                    continue;
                }
            };

            let (defs, refs) = match tagger.extract(&bytes, path) {
                Ok(result) => result,
                Err(e) => {
                    tracing::warn!("skip {}: extract error: {e}", path.display());
                    continue;
                }
            };

            parsed_files += 1;
            total_refs += refs.len();

            for def in &defs {
                defs_by_name
                    .entry(def.name.clone())
                    .or_default()
                    .push(def.clone());
                all_defs.push(def.clone());
            }

            for r in &refs {
                refs_by_name
                    .entry(r.name.clone())
                    .or_default()
                    .push(r.clone());
            }

            tags_by_file.insert(path.clone(), FileTags { defs, refs });
        }

        tracing::info!(
            "index built: {} files parsed, {} defs, {} refs",
            parsed_files,
            all_defs.len(),
            total_refs,
        );

        Ok(SymbolIndex {
            repo_root: repo_root.to_path_buf(),
            defs_by_name,
            refs_by_name,
            tags_by_file,
            all_defs,
        })
    }

    /// Filter all defs by optional case-insensitive name substring and/or
    /// exact kind match.  Both filters are ANDed.
    pub fn search_symbols(
        &self,
        name_substr: Option<&str>,
        kind: Option<SymbolKind>,
    ) -> Vec<&SymbolDef> {
        self.all_defs
            .iter()
            .filter(|def| {
                let name_ok = name_substr.map_or(true, |substr| {
                    def.name.to_lowercase().contains(&substr.to_lowercase())
                });
                let kind_ok = kind.as_ref().map_or(true, |k| &def.kind == k);
                name_ok && kind_ok
            })
            .collect()
    }

    /// All call-site references recorded for `name` (exact match).
    pub fn find_references(&self, name: &str) -> Vec<&SymbolRef> {
        self.refs_by_name
            .get(name)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// All definitions recorded for `name` (exact match).  Returns `&[]` when
    /// the name is absent.
    pub fn definitions(&self, name: &str) -> &[SymbolDef] {
        self.defs_by_name
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Definitions whose name has no entry in the references map — candidate
    /// dead code.  Used by the `find_dead_code` MCP tool (Phase 5).
    pub fn unreferenced_defs(&self) -> Vec<&SymbolDef> {
        self.all_defs
            .iter()
            .filter(|def| !self.refs_by_name.contains_key(&def.name))
            .collect()
    }

    /// Per-file tags accessor used by the Phase 5 call-graph builder.
    pub fn file_tags(&self, file: &Path) -> Option<&FileTags> {
        self.tags_by_file.get(file)
    }

    /// Aggregate statistics over the full index.
    pub fn stats(&self) -> RepoStats {
        let file_count = self.tags_by_file.len();
        let def_count = self.all_defs.len();
        let ref_count: usize = self.refs_by_name.values().map(Vec::len).sum();

        // Count defs per kind.
        let mut kind_counts: HashMap<String, usize> = HashMap::new();
        for def in &self.all_defs {
            *kind_counts.entry(format!("{:?}", def.kind)).or_insert(0) += 1;
        }
        let mut by_kind: Vec<(String, usize)> = kind_counts.into_iter().collect();
        by_kind.sort_by(|a, b| b.1.cmp(&a.1));

        // Prefer Mod-kind def names; fall back to file stems.
        let modules: Vec<String> = {
            let mod_names: Vec<String> = self
                .all_defs
                .iter()
                .filter(|d| d.kind == SymbolKind::Mod)
                .map(|d| d.name.clone())
                .collect();
            if mod_names.is_empty() {
                let mut stems: Vec<String> = self
                    .tags_by_file
                    .keys()
                    .filter_map(|p| p.file_stem()?.to_str().map(str::to_owned))
                    .collect();
                stems.sort();
                stems.dedup();
                stems
            } else {
                mod_names
            }
        };

        RepoStats {
            file_count,
            def_count,
            ref_count,
            by_kind,
            modules,
        }
    }
}
