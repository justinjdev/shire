use super::{find_child_by_kind, node_text, LanguageHooks, Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Nim visibility: only exported symbols (those with `*` suffix, represented as
/// `exported_symbol` nodes in the AST) are considered visible.
fn is_visible(node: &Node, _source: &str) -> bool {
    match node.kind() {
        // Procedure-like declarations: name field is exported_symbol or identifier
        "proc_declaration" | "func_declaration" | "method_declaration"
        | "iterator_declaration" | "template_declaration" | "macro_declaration"
        | "converter_declaration" => {
            node.child_by_field_name("name")
                .is_some_and(|n| n.kind() == "exported_symbol")
        }
        // type_symbol_declaration: name field is exported_symbol or identifier
        "type_symbol_declaration" => {
            node.child_by_field_name("name")
                .is_some_and(|n| n.kind() == "exported_symbol")
        }
        // variable_declaration inside const_section: check symbol_declaration's name
        "variable_declaration" => has_exported_symbol(node),
        _ => false,
    }
}

/// Check if a variable_declaration contains an exported_symbol in its symbol_declaration_list.
fn has_exported_symbol(node: &Node) -> bool {
    let sdl = match find_child_by_kind(node, "symbol_declaration_list") {
        Some(n) => n,
        None => return false,
    };
    let sd = match find_child_by_kind(&sdl, "symbol_declaration") {
        Some(n) => n,
        None => return false,
    };
    sd.child_by_field_name("name")
        .is_some_and(|n| n.kind() == "exported_symbol")
}

/// Build signature string for Nim symbols.
fn build_signature(node: &Node, source: &str, name: &str, _kind: SymbolKind) -> String {
    match node.kind() {
        "proc_declaration" | "func_declaration" | "method_declaration"
        | "iterator_declaration" | "template_declaration" | "macro_declaration"
        | "converter_declaration" => {
            let keyword = declaration_keyword(node.kind());
            let params = node
                .child_by_field_name("parameters")
                .and_then(|n| node_text(&n, source))
                .unwrap_or("()");
            let ret = node
                .child_by_field_name("return_type")
                .and_then(|n| node_text(&n, source));
            match ret {
                Some(rt) => format!("{} {}{}: {}", keyword, name, params, rt),
                None => format!("{} {}{}", keyword, name, params),
            }
        }
        "type_symbol_declaration" => {
            // Look at sibling to determine what kind of type it is
            let parent = node.parent();
            if let Some(parent) = parent {
                if parent.kind() == "type_declaration" {
                    if find_child_by_kind(&parent, "enum_declaration").is_some() {
                        return format!("type {} = enum", name);
                    }
                    if find_child_by_kind(&parent, "object_declaration").is_some() {
                        return format!("type {} = object", name);
                    }
                }
            }
            format!("type {}", name)
        }
        "variable_declaration" => {
            let value = node
                .child_by_field_name("value")
                .and_then(|n| node_text(&n, source));
            match value {
                Some(v) => {
                    let mut end = 40.min(v.len());
                    while end > 0 && !v.is_char_boundary(end) {
                        end -= 1;
                    }
                    let v_short = &v[..end];
                    format!("const {} = {}", name, v_short)
                }
                None => format!("const {}", name),
            }
        }
        _ => name.to_string(),
    }
}

/// Map declaration node kind to Nim keyword.
fn declaration_keyword(kind: &str) -> &'static str {
    match kind {
        "proc_declaration" => "proc",
        "func_declaration" => "func",
        "method_declaration" => "method",
        "iterator_declaration" => "iterator",
        "template_declaration" => "template",
        "macro_declaration" => "macro",
        "converter_declaration" => "converter",
        _ => "proc",
    }
}

/// Extract parameters from proc/func/method/iterator/template/macro/converter declarations.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let params_node = match node.child_by_field_name("parameters") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();
    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        if child.kind() == "parameter_declaration" {
            let type_ann = child
                .child_by_field_name("type")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string());

            // parameter_declaration > symbol_declaration_list > symbol_declaration(s)
            if let Some(sdl) = find_child_by_kind(&child, "symbol_declaration_list") {
                for j in 0..sdl.child_count() {
                    let sd = sdl.child(j).unwrap();
                    if sd.kind() == "symbol_declaration" {
                        if let Some(name_node) = sd.child_by_field_name("name") {
                            let pname = match name_node.kind() {
                                "exported_symbol" => {
                                    find_child_by_kind(&name_node, "identifier")
                                        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                                        .unwrap_or("")
                                }
                                _ => name_node
                                    .utf8_text(source.as_bytes())
                                    .unwrap_or(""),
                            };
                            if !pname.is_empty() {
                                params.push(Parameter {
                                    name: pname.to_string(),
                                    type_annotation: type_ann.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    params
}

/// Extract return type from proc/func/method/iterator declarations.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    node.child_by_field_name("return_type")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

/// Post-process: reclassify symbol kinds based on the declaration node type.
fn post_process(mut sym: SymbolInfo, node: &Node, _source: &str) -> Option<SymbolInfo> {
    match node.kind() {
        "method_declaration" => {
            sym.kind = SymbolKind::Method;
        }
        "type_symbol_declaration" => {
            // Check sibling nodes in the parent type_declaration to determine the actual kind
            if let Some(parent) = node.parent() {
                if parent.kind() == "type_declaration" {
                    if find_child_by_kind(&parent, "enum_declaration").is_some() {
                        sym.kind = SymbolKind::Enum;
                        return Some(sym);
                    }
                    if find_child_by_kind(&parent, "object_declaration").is_some() {
                        sym.kind = SymbolKind::Class;
                        return Some(sym);
                    }
                }
            }
            sym.kind = SymbolKind::Type;
        }
        "variable_declaration" => {
            sym.kind = SymbolKind::Constant;
        }
        _ => {}
    }
    Some(sym)
}

/// Return Nim language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: None,
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::query_extract;
    use std::sync::Arc;
    use tree_sitter::{Parser, Query};

    fn extract(source: &str) -> Vec<SymbolInfo> {
        let language = tree_sitter_nim::language();
        let query_source = include_str!("../queries/nim.scm");
        let query = Query::new(&language, query_source).expect("failed to compile nim query");
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let hooks = hooks();
        query_extract::extract(&mut parser, &query, source, Arc::from("test.nim"), &hooks)
    }

    #[test]
    fn test_exported_proc() {
        let syms = extract("proc greet*(name: string): string =\n  \"Hello\"");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "greet");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].return_type.as_deref(), Some("string"));
        let sig = syms[0].signature.as_ref().unwrap();
        assert!(sig.starts_with("proc greet"), "sig: {}", sig);
    }

    #[test]
    fn test_private_proc_filtered() {
        let syms = extract("proc privateHelper(x: int): int = x * 2");
        assert!(syms.is_empty(), "private proc should be filtered out");
    }

    #[test]
    fn test_exported_func() {
        let syms = extract("func add*(a, b: int): int = a + b");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "add");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].return_type.as_deref(), Some("int"));
    }

    #[test]
    fn test_exported_method() {
        let syms = extract("method draw*(self: Shape) =\n  discard");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "draw");
        assert_eq!(syms[0].kind, SymbolKind::Method);
    }

    #[test]
    fn test_exported_iterator() {
        let syms = extract("iterator items*[T](a: seq[T]): T =\n  for item in a:\n    yield item");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "items");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].return_type.as_deref(), Some("T"));
    }

    #[test]
    fn test_exported_template() {
        let syms = extract("template log*(msg: string) =\n  echo msg");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "log");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        let sig = syms[0].signature.as_ref().unwrap();
        assert!(sig.starts_with("template log"), "sig: {}", sig);
    }

    #[test]
    fn test_exported_macro() {
        let syms = extract("macro genCode*(body: untyped): untyped =\n  body");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "genCode");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        let sig = syms[0].signature.as_ref().unwrap();
        assert!(sig.starts_with("macro genCode"), "sig: {}", sig);
    }

    #[test]
    fn test_exported_converter() {
        let syms = extract("converter toFloat*(x: int): float =\n  float(x)");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "toFloat");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        let sig = syms[0].signature.as_ref().unwrap();
        assert!(sig.starts_with("converter toFloat"), "sig: {}", sig);
    }

    #[test]
    fn test_enum_type() {
        let source = "type\n  Color* = enum\n    Red, Green, Blue";
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Color");
        assert_eq!(syms[0].kind, SymbolKind::Enum);
        let sig = syms[0].signature.as_ref().unwrap();
        assert!(sig.contains("enum"), "sig: {}", sig);
    }

    #[test]
    fn test_object_type() {
        let source = "type\n  Person* = object\n    name*: string\n    age*: int";
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Person");
        assert_eq!(syms[0].kind, SymbolKind::Class);
        let sig = syms[0].signature.as_ref().unwrap();
        assert!(sig.contains("object"), "sig: {}", sig);
    }

    #[test]
    fn test_private_type_filtered() {
        let source = "type\n  Internal = object\n    x: int";
        let syms = extract(source);
        assert!(syms.is_empty(), "private type should be filtered out");
    }

    #[test]
    fn test_const() {
        let source = "const MaxSize* = 100";
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MaxSize");
        assert_eq!(syms[0].kind, SymbolKind::Constant);
        let sig = syms[0].signature.as_ref().unwrap();
        assert!(sig.contains("const"), "sig: {}", sig);
    }

    #[test]
    fn test_private_const_filtered() {
        let source = "const internalSize = 50";
        let syms = extract(source);
        assert!(syms.is_empty(), "private const should be filtered out");
    }

    #[test]
    fn test_parameters() {
        let syms = extract("proc greet*(name: string): string =\n  \"Hello\"");
        assert_eq!(syms.len(), 1);
        let params = syms[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "name");
        assert_eq!(params[0].type_annotation.as_deref(), Some("string"));
    }

    #[test]
    fn test_multi_params_same_type() {
        let syms = extract("func add*(a, b: int): int = a + b");
        assert_eq!(syms.len(), 1);
        let params = syms[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "a");
        assert_eq!(params[0].type_annotation.as_deref(), Some("int"));
        assert_eq!(params[1].name, "b");
        assert_eq!(params[1].type_annotation.as_deref(), Some("int"));
    }

    #[test]
    fn test_mixed_symbols() {
        let source = r#"type
  Color* = enum
    Red, Green, Blue

  Person* = object
    name*: string
    age*: int

proc greet*(name: string): string =
  "Hello, " & name

proc privateHelper(x: int): int = x * 2

func add*(a, b: int): int = a + b

method draw*(self: Shape) =
  discard

iterator items*[T](a: seq[T]): T =
  for item in a:
    yield item

template log*(msg: string) =
  echo msg

macro genCode*(body: untyped): untyped =
  body

const MaxSize* = 100
"#;
        let syms = extract(source);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Color"), "missing Color, got: {:?}", names);
        assert!(names.contains(&"Person"), "missing Person, got: {:?}", names);
        assert!(names.contains(&"greet"), "missing greet, got: {:?}", names);
        assert!(!names.contains(&"privateHelper"), "privateHelper should be filtered");
        assert!(names.contains(&"add"), "missing add, got: {:?}", names);
        assert!(names.contains(&"draw"), "missing draw, got: {:?}", names);
        assert!(names.contains(&"items"), "missing items, got: {:?}", names);
        assert!(names.contains(&"log"), "missing log, got: {:?}", names);
        assert!(names.contains(&"genCode"), "missing genCode, got: {:?}", names);
        assert!(names.contains(&"MaxSize"), "missing MaxSize, got: {:?}", names);
        assert_eq!(syms.len(), 9, "expected 9 symbols, got {}: {:?}", syms.len(), names);

        // Verify kinds
        let color = syms.iter().find(|s| s.name == "Color").unwrap();
        assert_eq!(color.kind, SymbolKind::Enum);
        let person = syms.iter().find(|s| s.name == "Person").unwrap();
        assert_eq!(person.kind, SymbolKind::Class);
        let draw = syms.iter().find(|s| s.name == "draw").unwrap();
        assert_eq!(draw.kind, SymbolKind::Method);
        let max_size = syms.iter().find(|s| s.name == "MaxSize").unwrap();
        assert_eq!(max_size.kind, SymbolKind::Constant);
    }
}
