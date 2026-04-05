use super::{find_child_by_kind, node_text, LanguageHooks, Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Gleam visibility: only `pub` symbols are visible.
/// Checks for a `visibility_modifier` child node.
fn is_visible(node: &Node, _source: &str) -> bool {
    find_child_by_kind(node, "visibility_modifier").is_some()
}

/// Build signature string for Gleam symbols.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match node.kind() {
        "function" => {
            let start = node.start_byte();
            // End at return_type if present, otherwise parameters, excluding the body
            let end = node
                .child_by_field_name("return_type")
                .map(|n| n.end_byte())
                .or_else(|| node.child_by_field_name("parameters").map(|n| n.end_byte()))
                .unwrap_or(node.end_byte());

            // Don't include body — stop before '{'
            let actual_end = find_child_by_kind(node, "{")
                .map(|n| n.start_byte())
                .unwrap_or(end);

            let sig = source[start..actual_end.min(source.len())].trim();
            if sig.is_empty() {
                format!("pub fn {}", name)
            } else {
                sig.to_string()
            }
        }
        "type_definition" => {
            let has_opaque = find_child_by_kind(node, "opacity_modifier").is_some();
            if has_opaque {
                format!("pub opaque type {}", name)
            } else {
                format!("pub type {}", name)
            }
        }
        "type_alias" => format!("pub type {}", name),
        "constant" => {
            let type_ann = node
                .child_by_field_name("type")
                .and_then(|t| {
                    t.child_by_field_name("name")
                        .and_then(|n| node_text(&n, source))
                })
                .or_else(|| {
                    node.child_by_field_name("type")
                        .and_then(|n| node_text(&n, source))
                });
            match type_ann {
                Some(t) => format!("pub const {}: {}", name, t),
                None => format!("pub const {}", name),
            }
        }
        _ => match kind {
            SymbolKind::Function => format!("pub fn {}", name),
            SymbolKind::Class => format!("pub type {}", name),
            SymbolKind::Type => format!("pub type {}", name),
            SymbolKind::Constant => format!("pub const {}", name),
            _ => name.to_string(),
        },
    }
}

/// Extract parameters from a Gleam function.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let params_node = match node.child_by_field_name("parameters") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();
    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        if child.kind() == "function_parameter" {
            let name = child
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or("")
                .to_string();

            let type_ann = child
                .child_by_field_name("type")
                .and_then(|t| node_text(&t, source))
                .map(|s| s.to_string());

            if !name.is_empty() {
                params.push(Parameter {
                    name,
                    type_annotation: type_ann,
                });
            }
        }
    }
    params
}

/// Extract return type from a Gleam function.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    node.child_by_field_name("return_type")
        .and_then(|n| node_text(&n, source))
        .map(|s| s.to_string())
}

/// Post-process: no reclassification needed — query captures map correctly.
fn post_process(sym: SymbolInfo, _node: &Node, _source: &str) -> Option<SymbolInfo> {
    Some(sym)
}

/// Return the language hooks for Gleam.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: None,
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
        enclosing_ancestors: &[],
        reference_stoplist: &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::query_extract;
    use std::sync::Arc;
    use tree_sitter::{Language, Parser, Query};

    fn extract(source: &str) -> Vec<SymbolInfo> {
        let language: Language = tree_sitter_gleam::LANGUAGE.into();
        let query_source = include_str!("../queries/gleam.scm");
        let query = Query::new(&language, query_source).expect("failed to compile gleam query");
        let hooks = hooks();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        query_extract::extract(&mut parser, &query, source, Arc::from("test.gleam"), &hooks).0
    }

    #[test]
    fn test_pub_function() {
        let syms = extract(
            r#"pub fn greet(name: String) -> String {
  "Hello, " <> name
}"#,
        );
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "greet");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].visibility, crate::symbols::Visibility::Public);
        let sig = syms[0].signature.as_ref().unwrap();
        assert!(sig.contains("pub fn greet"), "signature: {}", sig);
        assert!(
            sig.contains("-> String"),
            "signature should contain return type: {}",
            sig
        );
    }

    #[test]
    fn test_private_function_filtered() {
        let syms = extract(
            r#"fn private_helper() -> Int {
  42
}"#,
        );
        assert!(syms.is_empty(), "private function should be filtered out");
    }

    #[test]
    fn test_type_definition() {
        let syms = extract(
            r#"pub type Color {
  Red
  Green
  Blue
  Custom(r: Int, g: Int, b: Int)
}"#,
        );
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Color");
        assert_eq!(syms[0].kind, SymbolKind::Class);
        let sig = syms[0].signature.as_ref().unwrap();
        assert_eq!(sig, "pub type Color");
    }

    #[test]
    fn test_constant() {
        let syms = extract("pub const max_size: Int = 100");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "max_size");
        assert_eq!(syms[0].kind, SymbolKind::Constant);
        let sig = syms[0].signature.as_ref().unwrap();
        assert!(sig.contains("pub const max_size"), "signature: {}", sig);
    }

    #[test]
    fn test_type_alias() {
        let syms = extract("pub type UserId = Int");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "UserId");
        assert_eq!(syms[0].kind, SymbolKind::Type);
        let sig = syms[0].signature.as_ref().unwrap();
        assert_eq!(sig, "pub type UserId");
    }

    #[test]
    fn test_private_type_filtered() {
        let syms = extract(
            r#"type InternalState {
  Loading
  Ready
}"#,
        );
        assert!(syms.is_empty(), "private type should be filtered out");
    }

    #[test]
    fn test_private_constant_filtered() {
        let syms = extract("const internal_limit = 50");
        assert!(syms.is_empty(), "private constant should be filtered out");
    }

    #[test]
    fn test_function_parameters() {
        let syms = extract("pub fn add(a: Int, b: Int) -> Int { a + b }");
        assert_eq!(syms.len(), 1);
        let params = syms[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "a");
        assert_eq!(params[0].type_annotation.as_deref(), Some("Int"));
        assert_eq!(params[1].name, "b");
        assert_eq!(params[1].type_annotation.as_deref(), Some("Int"));
    }

    #[test]
    fn test_function_return_type() {
        let syms = extract("pub fn greet(name: String) -> String { name }");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].return_type.as_deref(), Some("String"));
    }

    #[test]
    fn test_external_function() {
        let source = r#"@external(erlang, "io", "format")
pub fn print(text: String) -> Nil"#;
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "print");
        assert_eq!(syms[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_opaque_type() {
        let source = r#"pub opaque type Counter {
  Counter(count: Int)
}"#;
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Counter");
        assert_eq!(syms[0].kind, SymbolKind::Class);
        let sig = syms[0].signature.as_ref().unwrap();
        assert!(
            sig.contains("opaque"),
            "signature should contain 'opaque': {}",
            sig
        );
    }

    #[test]
    fn test_mixed_symbols() {
        let source = r#"pub fn greet(name: String) -> String {
  "Hello, " <> name
}

fn private_helper() -> Int {
  42
}

pub type Color {
  Red
  Green
  Blue
  Custom(r: Int, g: Int, b: Int)
}

pub const max_size: Int = 100

pub type UserId = Int

@external(erlang, "io", "format")
pub fn print(text: String) -> Nil

pub opaque type Counter {
  Counter(count: Int)
}
"#;
        let syms = extract(source);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"greet"), "missing greet, got: {:?}", names);
        assert!(
            !names.contains(&"private_helper"),
            "should not contain private_helper"
        );
        assert!(names.contains(&"Color"), "missing Color, got: {:?}", names);
        assert!(
            names.contains(&"max_size"),
            "missing max_size, got: {:?}",
            names
        );
        assert!(
            names.contains(&"UserId"),
            "missing UserId, got: {:?}",
            names
        );
        assert!(names.contains(&"print"), "missing print, got: {:?}", names);
        assert!(
            names.contains(&"Counter"),
            "missing Counter, got: {:?}",
            names
        );
        assert_eq!(syms.len(), 6, "expected 6 public symbols, got: {:?}", names);
    }

    #[test]
    fn test_function_no_params() {
        let syms = extract("pub fn main() { todo }");
        assert_eq!(syms.len(), 1);
        let params = syms[0].parameters.as_ref().unwrap();
        assert!(params.is_empty());
    }

    #[test]
    fn test_constant_type_annotation() {
        let syms = extract("pub const max_size: Int = 100");
        assert_eq!(syms.len(), 1);
        let sig = syms[0].signature.as_ref().unwrap();
        assert!(
            sig.contains("Int"),
            "signature should contain type annotation: {}",
            sig
        );
    }
}
