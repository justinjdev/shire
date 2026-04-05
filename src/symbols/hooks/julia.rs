use super::{find_child_by_kind, node_text, LanguageHooks, Parameter, SymbolKind};
use tree_sitter::Node;

/// Julia visibility: all symbols are visible (Julia uses `export` at module level,
/// not syntax-level access modifiers).
fn is_visible(_node: &Node, _source: &str) -> bool {
    true
}

/// Build signature string for Julia symbols.
fn build_signature(node: &Node, source: &str, name: &str, _kind: SymbolKind) -> String {
    match node.kind() {
        "function_definition" => {
            // Extract the full signature from the signature child
            if let Some(sig) = find_child_by_kind(node, "signature") {
                let sig_text = node_text(&sig, source).unwrap_or(name);
                return format!("function {}", sig_text);
            }
            format!("function {}", name)
        }
        "assignment" => {
            // Short-form: f(x) = expr → show "name(params) = ..."
            if let Some(call) = find_child_by_kind(node, "call_expression") {
                let call_text = node_text(&call, source).unwrap_or(name);
                return format!("{} = ...", call_text);
            }
            format!("{} = ...", name)
        }
        "struct_definition" => {
            let is_mutable = find_child_by_kind(node, "mutable").is_some();
            if is_mutable {
                format!("mutable struct {}", name)
            } else {
                format!("struct {}", name)
            }
        }
        "abstract_definition" => format!("abstract type {}", name),
        "module_definition" => format!("module {}", name),
        "macro_definition" => {
            if let Some(sig) = find_child_by_kind(node, "signature") {
                let sig_text = node_text(&sig, source).unwrap_or(name);
                return format!("macro {}", sig_text);
            }
            format!("macro {}", name)
        }
        "const_statement" => format!("const {}", name),
        _ => name.to_string(),
    }
}

/// Collect parameters from an argument_list node, handling typed and untyped params.
fn collect_params(args_node: &Node, source: &str) -> Vec<Parameter> {
    let mut params = Vec::new();
    for i in 0..args_node.child_count() {
        let child = args_node.child(i).unwrap();
        match child.kind() {
            "identifier" => {
                let pname = child
                    .utf8_text(source.as_bytes())
                    .unwrap_or("")
                    .to_string();
                if !pname.is_empty() {
                    params.push(Parameter {
                        name: pname,
                        type_annotation: None,
                    });
                }
            }
            "typed_expression" => {
                // name::Type — first named child is name, last is type
                // Type can be identifier, parametrized_type_expression, etc.
                let first = child.named_child(0);
                let last_idx = child.named_child_count().saturating_sub(1);
                let last = if last_idx > 0 { child.named_child(last_idx) } else { None };
                if let Some(name_node) = first {
                    let pname = name_node.utf8_text(source.as_bytes()).unwrap_or("").to_string();
                    if !pname.is_empty() {
                        let ptype = last.and_then(|t| {
                            t.utf8_text(source.as_bytes()).ok().map(|s| s.to_string())
                        });
                        params.push(Parameter {
                            name: pname,
                            type_annotation: ptype,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    params
}

/// Find a child by kind without lifetime constraints (uses tree-sitter cursor).
fn child_by_kind<'a>(node: &Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

/// Find the argument_list node for a function-like definition.
fn find_arg_list<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    match node.kind() {
        "function_definition" | "macro_definition" => {
            let sig = child_by_kind(node, "signature")?;
            if let Some(call) = child_by_kind(&sig, "call_expression") {
                child_by_kind(&call, "argument_list")
            } else {
                let typed = child_by_kind(&sig, "typed_expression")?;
                let call = child_by_kind(&typed, "call_expression")?;
                child_by_kind(&call, "argument_list")
            }
        }
        "assignment" => {
            // Direct: f(x) = expr
            if let Some(call) = child_by_kind(node, "call_expression") {
                return child_by_kind(&call, "argument_list");
            }
            // Typed: f(x)::T = expr
            let typed = child_by_kind(node, "typed_expression")?;
            let call = child_by_kind(&typed, "call_expression")?;
            child_by_kind(&call, "argument_list")
        }
        _ => None,
    }
}

/// Extract parameters from Julia function definitions.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    match find_arg_list(node) {
        Some(a) => collect_params(&a, source),
        None => Vec::new(),
    }
}

/// Extract return type from Julia function definitions and typed assignments.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    let typed = match node.kind() {
        "function_definition" => {
            // signature > typed_expression
            let sig = child_by_kind(node, "signature")?;
            child_by_kind(&sig, "typed_expression")?
        }
        "assignment" => {
            // Direct typed_expression child: f(x)::T = expr
            child_by_kind(node, "typed_expression")?
        }
        _ => return None,
    };
    // The last named child of typed_expression is the type
    let last_idx = typed.named_child_count().checked_sub(1)?;
    let type_node = typed.named_child(last_idx)?;
    node_text(&type_node, source).map(|s| s.to_string())
}

/// Post-process: reclassify macro_definition symbols.
fn post_process(mut sym: super::SymbolInfo, node: &Node, _source: &str) -> Option<super::SymbolInfo> {
    // Tag macros by prefixing name with @
    if node.kind() == "macro_definition" {
        sym.name = format!("@{}", sym.name);
    }
    Some(sym)
}

/// Return the language hooks for Julia.
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
    use crate::symbols::SymbolInfo;
    use std::sync::Arc;
    use tree_sitter::{Parser, Query};

    fn extract(source: &str) -> Vec<SymbolInfo> {
        let language: tree_sitter::Language = tree_sitter_julia::LANGUAGE.into();
        let query_source = include_str!("../queries/julia.scm");
        let query = Query::new(&language, query_source).expect("failed to compile julia query");
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let hooks = super::hooks();
        query_extract::extract(&mut parser, &query, source, Arc::from("test.jl"), &hooks)
    }

    #[test]
    fn test_full_module() {
        let source = r#"module MyModule

abstract type Shape end

struct Point
    x::Float64
    y::Float64
end

mutable struct Counter
    count::Int
end

function greet(name::String)::String
    println("Hello, $name")
end

area(r::Float64) = π * r^2

macro assert_positive(x)
    quote abs($(esc(x))) end
end

const MAX_SIZE = 100

end"#;
        let syms = extract(source);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();

        // Module
        assert!(names.contains(&"MyModule"), "missing MyModule, got: {:?}", names);
        let module = syms.iter().find(|s| s.name == "MyModule").unwrap();
        assert_eq!(module.kind, SymbolKind::Class); // @definition.module → Class
        assert_eq!(
            module.signature.as_deref(),
            Some("module MyModule")
        );

        // Abstract type
        assert!(names.contains(&"Shape"), "missing Shape, got: {:?}", names);
        let shape = syms.iter().find(|s| s.name == "Shape").unwrap();
        assert_eq!(shape.kind, SymbolKind::Interface);
        assert_eq!(
            shape.signature.as_deref(),
            Some("abstract type Shape")
        );

        // Struct
        assert!(names.contains(&"Point"), "missing Point, got: {:?}", names);
        let point = syms.iter().find(|s| s.name == "Point").unwrap();
        assert_eq!(point.kind, SymbolKind::Class);
        assert_eq!(point.signature.as_deref(), Some("struct Point"));

        // Mutable struct
        assert!(names.contains(&"Counter"), "missing Counter, got: {:?}", names);
        let counter = syms.iter().find(|s| s.name == "Counter").unwrap();
        assert_eq!(counter.kind, SymbolKind::Class);
        assert_eq!(
            counter.signature.as_deref(),
            Some("mutable struct Counter")
        );

        // Function with return type
        assert!(names.contains(&"greet"), "missing greet, got: {:?}", names);
        let greet = syms.iter().find(|s| s.name == "greet").unwrap();
        assert_eq!(greet.kind, SymbolKind::Function);
        assert_eq!(
            greet.signature.as_deref(),
            Some("function greet(name::String)::String")
        );
        assert_eq!(greet.return_type.as_deref(), Some("String"));
        let params = greet.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "name");
        assert_eq!(params[0].type_annotation.as_deref(), Some("String"));

        // Short-form function
        assert!(names.contains(&"area"), "missing area, got: {:?}", names);
        let area = syms.iter().find(|s| s.name == "area").unwrap();
        assert_eq!(area.kind, SymbolKind::Function);
        assert_eq!(
            area.signature.as_deref(),
            Some("area(r::Float64) = ...")
        );
        let area_params = area.parameters.as_ref().unwrap();
        assert_eq!(area_params.len(), 1);
        assert_eq!(area_params[0].name, "r");
        assert_eq!(area_params[0].type_annotation.as_deref(), Some("Float64"));

        // Macro (post-processed with @ prefix)
        assert!(
            names.contains(&"@assert_positive"),
            "missing @assert_positive, got: {:?}",
            names
        );
        let mac = syms.iter().find(|s| s.name == "@assert_positive").unwrap();
        assert_eq!(mac.kind, SymbolKind::Function);
        assert_eq!(
            mac.signature.as_deref(),
            Some("macro assert_positive(x)")
        );

        // Constant
        assert!(names.contains(&"MAX_SIZE"), "missing MAX_SIZE, got: {:?}", names);
        let constant = syms.iter().find(|s| s.name == "MAX_SIZE").unwrap();
        assert_eq!(constant.kind, SymbolKind::Constant);
        assert_eq!(constant.signature.as_deref(), Some("const MAX_SIZE"));
    }

    #[test]
    fn test_simple_function() {
        let syms = extract("function hello()\n    println(\"hi\")\nend");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "hello");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(
            syms[0].signature.as_deref(),
            Some("function hello()")
        );
    }

    #[test]
    fn test_function_no_return_type() {
        let syms = extract("function add(a, b)\n    a + b\nend");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "add");
        assert_eq!(syms[0].return_type, None);
        let params = syms[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "a");
        assert_eq!(params[0].type_annotation, None);
        assert_eq!(params[1].name, "b");
    }

    #[test]
    fn test_short_form_function() {
        let syms = extract("square(x) = x * x");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "square");
        assert_eq!(syms[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_struct() {
        let syms = extract("struct Foo\n    x::Int\nend");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Foo");
        assert_eq!(syms[0].kind, SymbolKind::Class);
        assert_eq!(syms[0].signature.as_deref(), Some("struct Foo"));
    }

    #[test]
    fn test_mutable_struct() {
        let syms = extract("mutable struct Bar\n    y::Float64\nend");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Bar");
        assert_eq!(syms[0].kind, SymbolKind::Class);
        assert_eq!(
            syms[0].signature.as_deref(),
            Some("mutable struct Bar")
        );
    }

    #[test]
    fn test_abstract_type() {
        let syms = extract("abstract type Animal end");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Animal");
        assert_eq!(syms[0].kind, SymbolKind::Interface);
    }

    #[test]
    fn test_module() {
        let syms = extract("module Foo\nend");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Foo");
        assert_eq!(syms[0].kind, SymbolKind::Class); // @definition.module → Class
    }

    #[test]
    fn test_macro() {
        let syms = extract("macro my_macro(expr)\n    expr\nend");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "@my_macro");
        assert_eq!(syms[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_const() {
        let syms = extract("const PI = 3.14");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "PI");
        assert_eq!(syms[0].kind, SymbolKind::Constant);
        assert_eq!(syms[0].signature.as_deref(), Some("const PI"));
    }

    #[test]
    fn test_function_with_typed_params() {
        let syms = extract("function compute(x::Int, y::Float64)::Bool\n    x > y\nend");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "compute");
        assert_eq!(syms[0].return_type.as_deref(), Some("Bool"));
        let params = syms[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "x");
        assert_eq!(params[0].type_annotation.as_deref(), Some("Int"));
        assert_eq!(params[1].name, "y");
        assert_eq!(params[1].type_annotation.as_deref(), Some("Float64"));
    }

    #[test]
    fn test_all_visibility_public() {
        let source = r#"
function foo() end
struct Bar end
const X = 1
"#;
        let syms = extract(source);
        for sym in &syms {
            assert_eq!(sym.visibility, crate::symbols::Visibility::Public, "all Julia symbols should be public");
        }
    }
}
