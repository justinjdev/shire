pub mod cobol;
mod hooks;
mod query_extract;
mod registry;
pub mod walker;

use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    pub signature: Option<String>,
    pub file_path: Arc<str>,
    pub line: usize,
    pub visibility: Visibility,
    pub parent_symbol: Option<String>,
    pub return_type: Option<String>,
    pub parameters: Option<Vec<Parameter>>,
}

/// Symbol visibility. Most symbols are `Public`; other values come from
/// language-specific post_process hooks (e.g., PHP, C#, Java).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Protected,
    Private,
    Internal,
}

impl Visibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Protected => "protected",
            Visibility::Private => "private",
            Visibility::Internal => "internal",
        }
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Class,
    Struct,
    Interface,
    Type,
    Enum,
    Trait,
    Method,
    Constant,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Class => "class",
            SymbolKind::Struct => "struct",
            SymbolKind::Interface => "interface",
            SymbolKind::Type => "type",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Method => "method",
            SymbolKind::Constant => "constant",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Parameter {
    pub name: String,
    #[serde(rename = "type")]
    pub type_annotation: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReferenceInfo {
    pub name: String,
    pub kind: ReferenceKind,
    pub file_path: Arc<str>,
    pub line: usize,
    pub enclosing_symbol: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    Call,
    Type,
    Import,
    Impl,
}

impl ReferenceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReferenceKind::Call => "call",
            ReferenceKind::Type => "type",
            ReferenceKind::Import => "import",
            ReferenceKind::Impl => "impl",
        }
    }
}

/// Extract both symbols and references from a single file by extension.
pub fn extract_file_full(
    ext: &str,
    source: &str,
    file_path: Arc<str>,
) -> (Vec<SymbolInfo>, Vec<ReferenceInfo>) {
    registry::extract_file(ext, source, file_path, false)
}

/// Extract only symbols (backward-compatible convenience wrapper). Skips
/// reference-capture processing entirely so callers that don't need refs
/// don't pay the per-match `resolve_enclosing_symbol` + allocation cost.
pub fn extract_file(ext: &str, source: &str, file_path: Arc<str>) -> Vec<SymbolInfo> {
    registry::extract_file(ext, source, file_path, true).0
}

#[cfg(test)]
mod tests;
