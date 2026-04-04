use std::sync::{Arc, OnceLock};

use super::hooks::LanguageHooks;
use super::{query_extract, SymbolInfo};
use tree_sitter::{Language, Parser, Query};

struct LanguageEntry {
    extensions: &'static [&'static str],
    ts_language: fn() -> Language,
    query_source: &'static str,
    hooks: fn() -> LanguageHooks,
    compiled_query: OnceLock<Query>,
}

impl LanguageEntry {
    /// Get-or-compile the tree-sitter query for this language.
    /// The query is compiled once and cached for the lifetime of the process.
    fn query(&self) -> &Query {
        self.compiled_query.get_or_init(|| {
            Query::new(&(self.ts_language)(), self.query_source)
                .expect("failed to compile tree-sitter query")
        })
    }
}

/// All tree-sitter-based language entries.
fn registry() -> &'static [LanguageEntry] {
    static REGISTRY: OnceLock<Vec<LanguageEntry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        vec![
            LanguageEntry {
                extensions: &["py"],
                ts_language: || tree_sitter_python::LANGUAGE.into(),
                query_source: include_str!("queries/python.scm"),
                hooks: super::hooks::python::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["go"],
                ts_language: || tree_sitter_go::LANGUAGE.into(),
                query_source: include_str!("queries/go.scm"),
                hooks: super::hooks::go::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["rs"],
                ts_language: || tree_sitter_rust::LANGUAGE.into(),
                query_source: include_str!("queries/rust.scm"),
                hooks: super::hooks::rust_lang::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["ts"],
                ts_language: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                query_source: include_str!("queries/typescript.scm"),
                hooks: super::hooks::typescript::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["tsx"],
                ts_language: || tree_sitter_typescript::LANGUAGE_TSX.into(),
                query_source: include_str!("queries/typescript.scm"),
                hooks: super::hooks::typescript::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["js", "jsx"],
                ts_language: || tree_sitter_javascript::LANGUAGE.into(),
                query_source: include_str!("queries/javascript.scm"),
                hooks: super::hooks::typescript::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["java"],
                ts_language: || tree_sitter_java::LANGUAGE.into(),
                query_source: include_str!("queries/java.scm"),
                hooks: super::hooks::java::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["kt"],
                ts_language: || tree_sitter_kotlin_ng::LANGUAGE.into(),
                query_source: include_str!("queries/kotlin.scm"),
                hooks: super::hooks::kotlin::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["dart"],
                ts_language: || tree_sitter_dart::LANGUAGE.into(),
                query_source: include_str!("queries/dart.scm"),
                hooks: super::hooks::dart::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["proto"],
                ts_language: || tree_sitter_proto::LANGUAGE.into(),
                query_source: include_str!("queries/proto.scm"),
                hooks: super::hooks::proto::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["cs"],
                ts_language: || tree_sitter_c_sharp::LANGUAGE.into(),
                query_source: include_str!("queries/csharp.scm"),
                hooks: super::hooks::csharp::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["swift"],
                ts_language: || tree_sitter_swift::LANGUAGE.into(),
                query_source: include_str!("queries/swift.scm"),
                hooks: super::hooks::swift::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["c", "h"],
                ts_language: || tree_sitter_c::LANGUAGE.into(),
                query_source: include_str!("queries/c.scm"),
                hooks: super::hooks::c::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["cpp", "cc", "cxx", "hpp", "hxx"],
                ts_language: || tree_sitter_cpp::LANGUAGE.into(),
                query_source: include_str!("queries/cpp.scm"),
                hooks: super::hooks::cpp::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["php"],
                ts_language: || tree_sitter_php::LANGUAGE_PHP.into(),
                query_source: include_str!("queries/php.scm"),
                hooks: super::hooks::php::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["scala", "sc"],
                ts_language: || tree_sitter_scala::LANGUAGE.into(),
                query_source: include_str!("queries/scala.scm"),
                hooks: super::hooks::scala::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["zig"],
                ts_language: || tree_sitter_zig::LANGUAGE.into(),
                query_source: include_str!("queries/zig.scm"),
                hooks: super::hooks::zig::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["sh", "bash"],
                ts_language: || tree_sitter_bash::LANGUAGE.into(),
                query_source: include_str!("queries/bash.scm"),
                hooks: super::hooks::bash::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["r", "R"],
                ts_language: || tree_sitter_r::LANGUAGE.into(),
                query_source: include_str!("queries/r.scm"),
                hooks: super::hooks::r::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["hs"],
                ts_language: || tree_sitter_haskell::LANGUAGE.into(),
                query_source: include_str!("queries/haskell.scm"),
                hooks: super::hooks::haskell::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["yaml", "yml"],
                ts_language: || tree_sitter_yaml::LANGUAGE.into(),
                query_source: include_str!("queries/yaml.scm"),
                hooks: super::hooks::yaml::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["sql"],
                ts_language: || tree_sitter_sequel::LANGUAGE.into(),
                query_source: include_str!("queries/sql.scm"),
                hooks: super::hooks::sql::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["hcl", "tf"],
                ts_language: || tree_sitter_hcl::LANGUAGE.into(),
                query_source: include_str!("queries/hcl.scm"),
                hooks: super::hooks::hcl::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["toml"],
                ts_language: || tree_sitter_toml_ng::LANGUAGE.into(),
                query_source: include_str!("queries/toml.scm"),
                hooks: super::hooks::toml_lang::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["pm", "pl"],
                ts_language: || ts_parser_perl::LANGUAGE.into(),
                query_source: include_str!("queries/perl.scm"),
                hooks: super::hooks::perl::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["rb"],
                ts_language: || tree_sitter_ruby::LANGUAGE.into(),
                query_source: include_str!("queries/ruby.scm"),
                hooks: super::hooks::ruby::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["ml"],
                ts_language: || tree_sitter_ocaml::LANGUAGE_OCAML.into(),
                query_source: include_str!("queries/ocaml.scm"),
                hooks: super::hooks::ocaml::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["mli"],
                ts_language: || tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into(),
                query_source: include_str!("queries/ocaml.scm"),
                hooks: super::hooks::ocaml::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["lua"],
                ts_language: || tree_sitter_lua::LANGUAGE.into(),
                query_source: include_str!("queries/lua.scm"),
                hooks: super::hooks::lua::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["ex", "exs"],
                ts_language: || tree_sitter_elixir::LANGUAGE.into(),
                query_source: include_str!("queries/elixir.scm"),
                hooks: super::hooks::elixir::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["clj", "cljs", "cljc", "edn"],
                ts_language: || tree_sitter_clojure_orchard::LANGUAGE.into(),
                query_source: include_str!("queries/clojure.scm"),
                hooks: super::hooks::clojure::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["erl", "hrl"],
                ts_language: || tree_sitter_erlang::LANGUAGE.into(),
                query_source: include_str!("queries/erlang.scm"),
                hooks: super::hooks::erlang::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["jl"],
                ts_language: || tree_sitter_julia::LANGUAGE.into(),
                query_source: include_str!("queries/julia.scm"),
                hooks: super::hooks::julia::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["gleam"],
                ts_language: || tree_sitter_gleam::LANGUAGE.into(),
                query_source: include_str!("queries/gleam.scm"),
                hooks: super::hooks::gleam::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["odin"],
                ts_language: || tree_sitter_odin::LANGUAGE.into(),
                query_source: include_str!("queries/odin.scm"),
                hooks: super::hooks::odin::hooks,
                compiled_query: OnceLock::new(),
            },
            LanguageEntry {
                extensions: &["nix"],
                ts_language: || tree_sitter_nix::LANGUAGE.into(),
                query_source: include_str!("queries/nix.scm"),
                hooks: super::hooks::nix::hooks,
                compiled_query: OnceLock::new(),
            },
            // tree-sitter-nim is a git dep that exports language() fn, not a LANGUAGE const
            LanguageEntry {
                extensions: &["nim", "nims"],
                ts_language: tree_sitter_nim::language,
                query_source: include_str!("queries/nim.scm"),
                hooks: super::hooks::nim::hooks,
                compiled_query: OnceLock::new(),
            },
        ]
    })
}

/// Extract symbols from a single file by extension.
///
/// For tree-sitter languages, the `Query` is compiled once per language (cached via `OnceLock`)
/// and a `Parser` is created per call. The parser creation is cheap; query compilation is the
/// expensive operation that we avoid repeating.
pub fn extract_file(ext: &str, source: &str, file_path: Arc<str>) -> Vec<SymbolInfo> {
    // Regex-based extractors (no tree-sitter)
    match ext {
        "cob" | "cbl" | "cpy" => return super::cobol::extract(source, file_path),
        _ => {}
    }

    // Tree-sitter query-based extractors
    for entry in registry() {
        if entry.extensions.contains(&ext) {
            let query = entry.query();
            let hooks = (entry.hooks)();
            let language = (entry.ts_language)();
            let mut parser = Parser::new();
            if parser.set_language(&language).is_err() {
                return Vec::new();
            }
            return query_extract::extract(&mut parser, query, source, file_path, &hooks);
        }
    }

    Vec::new()
}
