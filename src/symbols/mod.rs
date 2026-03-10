pub mod generic;
pub mod go;
pub mod java;
pub mod kotlin;
pub mod lang_spec;
pub mod languages;
pub mod perl;
pub mod proto;
pub mod python;
pub mod ruby;
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

        let mut file_symbols = extract_file(ext, &source, &relative_path);
        symbols.append(&mut file_symbols);
    }

    Ok(symbols)
}

/// Extract symbols from a single file by extension.
/// Uses the generic table-driven extractor for languages with specs,
/// and falls back to custom extractors for languages with unique AST patterns.
pub fn extract_file(ext: &str, source: &str, file_path: &str) -> Vec<SymbolInfo> {
    match ext {
        // Languages using the generic table-driven extractor
        "py" => generic::extract(&languages::python(), source, file_path),

        // Languages with custom extractors (unique AST patterns)
        "ts" | "tsx" => typescript::extract(source, file_path, ext == "tsx"),
        "js" | "jsx" => typescript::extract_js(source, file_path),
        "go" => go::extract(source, file_path),
        "rs" => rust_lang::extract(source, file_path),
        "proto" => proto::extract(source, file_path),
        "java" => java::extract(source, file_path),
        "kt" => kotlin::extract(source, file_path),
        "pm" | "pl" => perl::extract(source, file_path),
        "rb" => ruby::extract(source, file_path),
        _ => Vec::new(),
    }
}
