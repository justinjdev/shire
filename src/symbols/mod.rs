pub mod elixir;
mod hooks;
pub mod perl;
mod query_extract;
mod registry;
pub mod ruby;
pub mod walker;

use anyhow::Result;
use rayon::prelude::*;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    pub signature: Option<String>,
    pub file_path: Arc<str>,
    pub line: usize,
    pub visibility: String,
    pub parent_symbol: Option<String>,
    pub return_type: Option<String>,
    pub parameters: Option<Vec<Parameter>>,
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

/// Extract symbols from all source files in a package directory.
pub fn extract_symbols_for_package(
    repo_root: &Path,
    package_path: &str,
    _package_kind: &str,
    exclude_extensions: &[String],
) -> Result<Vec<SymbolInfo>> {
    let package_dir = repo_root.join(package_path);
    if !package_dir.is_dir() {
        return Ok(Vec::new());
    }

    let all_exts = walker::all_extensions();
    let extensions: Vec<&str> = all_exts
        .into_iter()
        .filter(|ext| {
            let with_dot = format!(".{}", ext);
            !exclude_extensions.contains(&with_dot)
        })
        .collect();
    let source_files = walker::walk_source_files(&package_dir, &extensions)?;

    let symbols: Vec<SymbolInfo> = source_files
        .par_iter()
        .flat_map(|file_path| {
            let source = match std::fs::read_to_string(file_path) {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };
            let relative_path = file_path
                .strip_prefix(repo_root)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();
            let file_path_arc: Arc<str> = Arc::from(relative_path.as_str());
            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            extract_file(ext, &source, file_path_arc)
        })
        .collect();

    Ok(symbols)
}

/// Extract symbols from a single file by extension.
pub fn extract_file(ext: &str, source: &str, file_path: Arc<str>) -> Vec<SymbolInfo> {
    registry::extract_file(ext, source, file_path)
}

#[cfg(test)]
mod tests;
