use super::hooks::LanguageHooks;
use super::{query_extract, SymbolInfo};
use tree_sitter::Language;

struct LanguageEntry {
    extensions: &'static [&'static str],
    ts_language: fn() -> Language,
    query: &'static str,
    hooks: fn() -> LanguageHooks,
}

/// All tree-sitter-based language entries.
fn registry() -> Vec<LanguageEntry> {
    vec![
        LanguageEntry {
            extensions: &["py"],
            ts_language: || tree_sitter_python::LANGUAGE.into(),
            query: include_str!("queries/python.scm"),
            hooks: super::hooks::python::hooks,
        },
        LanguageEntry {
            extensions: &["go"],
            ts_language: || tree_sitter_go::LANGUAGE.into(),
            query: include_str!("queries/go.scm"),
            hooks: super::hooks::go::hooks,
        },
        LanguageEntry {
            extensions: &["rs"],
            ts_language: || tree_sitter_rust::LANGUAGE.into(),
            query: include_str!("queries/rust.scm"),
            hooks: super::hooks::rust_lang::hooks,
        },
        LanguageEntry {
            extensions: &["ts"],
            ts_language: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            query: include_str!("queries/typescript.scm"),
            hooks: super::hooks::typescript::hooks,
        },
        LanguageEntry {
            extensions: &["tsx"],
            ts_language: || tree_sitter_typescript::LANGUAGE_TSX.into(),
            query: include_str!("queries/typescript.scm"),
            hooks: super::hooks::typescript::hooks,
        },
        LanguageEntry {
            extensions: &["js", "jsx"],
            ts_language: || tree_sitter_javascript::LANGUAGE.into(),
            query: include_str!("queries/javascript.scm"),
            hooks: super::hooks::typescript::hooks,
        },
        LanguageEntry {
            extensions: &["java"],
            ts_language: || tree_sitter_java::LANGUAGE.into(),
            query: include_str!("queries/java.scm"),
            hooks: super::hooks::java::hooks,
        },
        LanguageEntry {
            extensions: &["kt"],
            ts_language: || tree_sitter_kotlin_ng::LANGUAGE.into(),
            query: include_str!("queries/kotlin.scm"),
            hooks: super::hooks::kotlin::hooks,
        },
        LanguageEntry {
            extensions: &["proto"],
            ts_language: || tree_sitter_proto::LANGUAGE.into(),
            query: include_str!("queries/proto.scm"),
            hooks: super::hooks::proto::hooks,
        },
    ]
}

/// Extract symbols from a single file by extension.
pub fn extract_file(ext: &str, source: &str, file_path: &str) -> Vec<SymbolInfo> {
    // Regex-based extractors (no tree-sitter)
    match ext {
        "pm" | "pl" => return super::perl::extract(source, file_path),
        "rb" => return super::ruby::extract(source, file_path),
        _ => {}
    }

    // Tree-sitter query-based extractors
    for entry in registry() {
        if entry.extensions.contains(&ext) {
            let language = (entry.ts_language)();
            let hooks = (entry.hooks)();
            return query_extract::extract(&language, entry.query, source, file_path, &hooks);
        }
    }

    Vec::new()
}
