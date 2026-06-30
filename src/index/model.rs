use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum SymbolKind {
    Fn,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Mod,
    Const,
    Macro,
    Other,
}

/// Source location (1-based line and column).
#[derive(Debug, Clone, Serialize)]
pub struct Location {
    pub file: PathBuf,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number.
    pub col: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolDef {
    pub name: String,
    pub kind: SymbolKind,
    pub location: Location,
    /// Inclusive (start, end) line range of the symbol body (1-based).
    pub line_range: (usize, usize),
    pub is_pub: bool,
    pub docs: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolRef {
    pub name: String,
    pub location: Location,
}
