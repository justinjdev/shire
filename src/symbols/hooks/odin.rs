use super::{node_text, LanguageHooks, Parameter, SymbolKind};
use tree_sitter::Node;

/// Odin visibility: all top-level declarations are visible.
/// Odin uses a package-level export annotation (`@(export)`) but it's not a
/// standard syntax-level visibility modifier, so we include everything.
fn is_visible(_node: &Node, _source: &str) -> bool {
    true
}

/// Helper: find first child node with the given kind (local version to avoid lifetime issues).
fn child_by_kind<'a>(node: &Node<'a>, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

/// Find the `parameters` node inside a procedure_declaration.
/// Navigates: procedure_declaration > procedure > parameters
fn find_parameters_node<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let proc_node = child_by_kind(node, "procedure")?;
    child_by_kind(&proc_node, "parameters")
}

/// Find the return type text from a procedure_declaration.
/// The return type is a `type` child of the `procedure` node, appearing after `->`.
fn get_return_type_text<'a>(node: &Node<'a>, source: &'a str) -> Option<&'a str> {
    let proc_node = child_by_kind(node, "procedure")?;
    let mut found_arrow = false;
    for i in 0..proc_node.child_count() {
        let child = proc_node.child(i).unwrap();
        if child.kind() == "->" {
            found_arrow = true;
            continue;
        }
        if found_arrow && child.kind() == "type" {
            return node_text(&child, source);
        }
    }
    None
}

/// Build signature for Odin symbols.
///
/// - Procedures: `name :: proc(params) -> ReturnType`
/// - Structs: `Name :: struct`
/// - Enums: `Name :: enum`
/// - Unions: `Name :: union`
/// - Constants: `NAME :: <value_kind>`
fn build_signature(node: &Node, source: &str, name: &str, _kind: SymbolKind) -> String {
    match node.kind() {
        "procedure_declaration" => {
            let params = find_parameters_node(node)
                .and_then(|n| node_text(&n, source))
                .unwrap_or("()");
            match get_return_type_text(node, source) {
                Some(r) => format!("{} :: proc{} -> {}", name, params, r),
                None => format!("{} :: proc{}", name, params),
            }
        }
        "struct_declaration" => format!("{} :: struct", name),
        "enum_declaration" => format!("{} :: enum", name),
        "union_declaration" => format!("{} :: union", name),
        "const_declaration" => {
            // Try to show the value (number, string, identifier, etc.)
            // Skip the identifier (name) and `::` to find the value node.
            let mut value_text = None;
            let mut past_colons = false;
            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                if child.kind() == "::" {
                    past_colons = true;
                    continue;
                }
                if past_colons && child.is_named() {
                    value_text = node_text(&child, source);
                    break;
                }
            }
            match value_text {
                Some(v) if v.len() <= 40 => format!("{} :: {}", name, v),
                _ => format!("{} :: ...", name),
            }
        }
        _ => name.to_string(),
    }
}

/// Extract parameters from an Odin procedure declaration.
///
/// Odin parameter syntax: `a, b: int` means both `a` and `b` have type `int`.
/// A `parameter` node contains identifier children and one `type` child.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    if node.kind() != "procedure_declaration" {
        return Vec::new();
    }

    let params_node = match find_parameters_node(node) {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        if child.kind() == "parameter" {
            // Collect all identifiers (parameter names) and the type node.
            let mut names = Vec::new();
            let mut type_ann = None;

            for j in 0..child.child_count() {
                let c = child.child(j).unwrap();
                match c.kind() {
                    "identifier" => {
                        if let Some(text) = node_text(&c, source) {
                            names.push(text.to_string());
                        }
                    }
                    "type" => {
                        type_ann = node_text(&c, source).map(|s| s.to_string());
                    }
                    _ => {}
                }
            }

            // Each identifier shares the same type annotation.
            for name in names {
                params.push(Parameter {
                    name,
                    type_annotation: type_ann.clone(),
                });
            }
        }
    }

    params
}

/// Extract return type from an Odin procedure declaration.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    if node.kind() != "procedure_declaration" {
        return None;
    }
    get_return_type_text(node, source).map(|s| s.to_string())
}

/// Return the language hooks for Odin.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: None,
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: None,
        reference_hooks: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::query_extract;
    use super::super::super::SymbolKind;
    use super::hooks;
    use std::sync::Arc;
    use tree_sitter::{Parser, Query};

    /// Helper: parse Odin source and extract symbols using our query + hooks.
    fn extract_odin(source: &str, file_path: &str) -> Vec<super::super::super::SymbolInfo> {
        let lang: tree_sitter::Language = tree_sitter_odin::LANGUAGE.into();
        let query =
            Query::new(&lang, include_str!("../queries/odin.scm")).expect("query should compile");
        let hooks = hooks();
        let mut parser = Parser::new();
        parser.set_language(&lang).unwrap();
        query_extract::extract(&mut parser, &query, source, Arc::from(file_path), &hooks, true, 0).0
    }

    #[test]
    fn test_odin_basic_extraction() {
        let source = r#"package main

import "core:fmt"

Vector2 :: struct {
    x, y: f32,
}

Direction :: enum {
    North,
    South,
    East,
    West,
}

add :: proc(a, b: int) -> int {
    return a + b
}

greet :: proc(name: string) {
    fmt.printf("Hello, %s\n", name)
}

MAX_SIZE :: 1024
"#;

        let symbols = extract_odin(source, "test.odin");

        // Expect: Vector2, Direction, add, greet, MAX_SIZE
        assert_eq!(
            symbols.len(),
            5,
            "expected 5 symbols, got {}: {:?}",
            symbols.len(),
            symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
        );

        let vector2 = symbols.iter().find(|s| s.name == "Vector2").unwrap();
        assert_eq!(vector2.kind, SymbolKind::Class);
        assert_eq!(vector2.signature.as_deref(), Some("Vector2 :: struct"));

        let direction = symbols.iter().find(|s| s.name == "Direction").unwrap();
        assert_eq!(direction.kind, SymbolKind::Enum);
        assert_eq!(
            direction.signature.as_deref(),
            Some("Direction :: enum")
        );

        let add = symbols.iter().find(|s| s.name == "add").unwrap();
        assert_eq!(add.kind, SymbolKind::Function);
        assert_eq!(
            add.signature.as_deref(),
            Some("add :: proc(a, b: int) -> int")
        );
        assert_eq!(add.return_type.as_deref(), Some("int"));
        let params = add.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "a");
        assert_eq!(params[0].type_annotation.as_deref(), Some("int"));
        assert_eq!(params[1].name, "b");
        assert_eq!(params[1].type_annotation.as_deref(), Some("int"));

        let greet = symbols.iter().find(|s| s.name == "greet").unwrap();
        assert_eq!(greet.kind, SymbolKind::Function);
        assert_eq!(
            greet.signature.as_deref(),
            Some("greet :: proc(name: string)")
        );
        assert_eq!(greet.return_type, None);
        let params = greet.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "name");
        assert_eq!(params[0].type_annotation.as_deref(), Some("string"));

        let max_size = symbols.iter().find(|s| s.name == "MAX_SIZE").unwrap();
        assert_eq!(max_size.kind, SymbolKind::Constant);
        assert_eq!(max_size.signature.as_deref(), Some("MAX_SIZE :: 1024"));
    }

    #[test]
    fn test_odin_union() {
        let source = r#"package main

Result :: union {
    int,
    string,
}
"#;
        let symbols = extract_odin(source, "union.odin");
        assert_eq!(symbols.len(), 1);
        let result = &symbols[0];
        assert_eq!(result.name, "Result");
        assert_eq!(result.kind, SymbolKind::Class);
        assert_eq!(result.signature.as_deref(), Some("Result :: union"));
    }

    #[test]
    fn test_odin_multi_return() {
        let source = r#"package main

swap :: proc(a, b: int) -> (int, int) {
    return b, a
}
"#;
        let symbols = extract_odin(source, "swap.odin");
        assert_eq!(symbols.len(), 1);
        let swap = &symbols[0];
        assert_eq!(swap.kind, SymbolKind::Function);
        assert_eq!(swap.return_type.as_deref(), Some("(int, int)"));
        assert_eq!(
            swap.signature.as_deref(),
            Some("swap :: proc(a, b: int) -> (int, int)")
        );
    }
}
