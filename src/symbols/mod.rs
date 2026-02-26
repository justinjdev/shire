pub mod go;
pub mod python;
pub mod rust_lang;
pub mod typescript;
pub mod walker;

use anyhow::Result;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    pub signature: Option<String>,
    pub file_path: String,
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

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "function" => Some(SymbolKind::Function),
            "class" => Some(SymbolKind::Class),
            "struct" => Some(SymbolKind::Struct),
            "interface" => Some(SymbolKind::Interface),
            "type" => Some(SymbolKind::Type),
            "enum" => Some(SymbolKind::Enum),
            "trait" => Some(SymbolKind::Trait),
            "method" => Some(SymbolKind::Method),
            "constant" => Some(SymbolKind::Constant),
            _ => None,
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
    package_kind: &str,
) -> Result<Vec<SymbolInfo>> {
    let package_dir = repo_root.join(package_path);
    if !package_dir.is_dir() {
        return Ok(Vec::new());
    }

    let extensions = walker::extensions_for_kind(package_kind);
    let source_files = walker::walk_source_files(&package_dir, &extensions)?;

    let mut symbols = Vec::new();

    for file_path in source_files {
        let source = match std::fs::read_to_string(&file_path) {
            Ok(s) => s,
            Err(_) => continue, // skip binary/unreadable files
        };

        let relative_path = file_path
            .strip_prefix(repo_root)
            .unwrap_or(&file_path)
            .to_string_lossy()
            .to_string();

        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let mut file_symbols = match ext {
            "ts" | "tsx" => typescript::extract(&source, &relative_path, ext == "tsx"),
            "js" | "jsx" => typescript::extract_js(&source, &relative_path),
            "go" => go::extract(&source, &relative_path),
            "rs" => rust_lang::extract(&source, &relative_path),
            "py" => python::extract(&source, &relative_path),
            _ => Vec::new(),
        };

        symbols.append(&mut file_symbols);
    }

    Ok(symbols)
}
