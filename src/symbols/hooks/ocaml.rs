use super::{find_ancestor, find_child_by_kind, LanguageHooks, Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// For methods, resolve the parent class name.
/// For definitions inside modules, resolve the parent module name.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    let kind = node.kind();

    // Methods belong to a class
    if kind == "method_definition" || kind == "method_specification" {
        let class_binding = find_ancestor(node, "class_binding")?;
        return class_binding
            .child_by_field_name("class_name")
            .or_else(|| find_child_by_kind(&class_binding, "class_name"))
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .map(|s| s.to_string());
    }

    // Definitions inside a module struct/sig body
    let module_binding = find_ancestor(node, "module_binding")?;
    find_child_by_kind(&module_binding, "module_name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

/// Build a signature string for an OCaml symbol.
///
/// For let bindings: source span from the `let` keyword to the `=` sign.
/// For value specifications: full source span (val name : type).
/// For types: "type name".
/// For modules: "module Name".
/// For module types: "module type Name".
/// For classes: "class name".
/// For exceptions: source span of the exception_definition.
/// For externals: source span up to the `=` for the C binding string.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match node.kind() {
        "let_binding" => {
            let start = node
                .parent()
                .map(|p| p.start_byte())
                .unwrap_or(node.start_byte());
            let end = find_child_by_kind(node, "=")
                .map(|n| n.start_byte())
                .unwrap_or(node.end_byte());
            source[start..end.min(source.len())].trim().to_string()
        }
        "value_specification" => {
            let start = node.start_byte();
            let end = node.end_byte();
            source[start..end.min(source.len())].trim().to_string()
        }
        "method_definition" => {
            let start = node.start_byte();
            let end = find_child_by_kind(node, "=")
                .map(|n| n.start_byte())
                .unwrap_or(node.end_byte());
            source[start..end.min(source.len())].trim().to_string()
        }
        "method_specification" => {
            let start = node.start_byte();
            let end = node.end_byte();
            source[start..end.min(source.len())].trim().to_string()
        }
        "external" => {
            let start = node.start_byte();
            // Stop before the C binding string literal
            let end = node
                .child_by_field_name("body")
                .or_else(|| {
                    // Find the last `=` which precedes the C string
                    let mut last_eq = None;
                    for i in 0..node.child_count() {
                        let child = node.child(i).unwrap();
                        if child.kind() == "=" {
                            last_eq = Some(child);
                        }
                    }
                    last_eq
                })
                .map(|n| n.start_byte())
                .unwrap_or(node.end_byte());
            source[start..end.min(source.len())].trim().to_string()
        }
        "module_type_definition" => format!("module type {}", name),
        _ => {
            let keyword = match kind {
                SymbolKind::Type => "type",
                SymbolKind::Class => match node.kind() {
                    "module_binding" => "module",
                    _ => "class",
                },
                SymbolKind::Interface => "module type",
                _ => "let",
            };
            format!("{} {}", keyword, name)
        }
    }
}

/// Extract parameters from a let_binding node.
/// OCaml parameters are direct children of the let_binding node.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    match node.kind() {
        "let_binding" => extract_let_binding_params(node, source),
        "value_specification" => extract_val_spec_params(node, source),
        "external" => extract_external_params(node, source),
        _ => Vec::new(),
    }
}

/// Extract parameters from a let_binding.
/// Parameters are direct `parameter` children containing `value_pattern`.
fn extract_let_binding_params(node: &Node, source: &str) -> Vec<Parameter> {
    let mut params = Vec::new();
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "parameter" {
            let name = find_child_by_kind(&child, "value_pattern")
                .or_else(|| find_child_by_kind(&child, "parenthesized_pattern"))
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or("")
                .to_string();

            // Check for type annotation: (param : type)
            let type_ann = if child.kind() == "parameter" {
                extract_typed_param(&child, source)
            } else {
                None
            };

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

/// Extract type annotation from a typed parameter like `(x : int)`.
/// Searches for typed_pattern directly or inside a parenthesized_pattern child.
fn extract_typed_param(param_node: &Node, source: &str) -> Option<String> {
    fn try_extract_type(node: &Node, source: &str) -> Option<String> {
        for i in 0..node.child_count() {
            let child = node.child(i).unwrap();
            if child.kind() == "typed_pattern" {
                return child
                    .child_by_field_name("type")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .map(|s| s.to_string());
            }
            if child.kind() == "parenthesized_pattern" {
                if let Some(result) = try_extract_type(&child, source) {
                    return Some(result);
                }
            }
        }
        None
    }
    try_extract_type(param_node, source)
}

/// Extract parameter types from a value_specification's function type.
/// `val add : int -> int -> int` → params from domain types.
fn extract_val_spec_params(node: &Node, source: &str) -> Vec<Parameter> {
    let type_node = match node.child_by_field_name("type") {
        Some(n) => n,
        None => {
            // Find the type after the ":"
            let mut found_colon = false;
            let mut type_node = None;
            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                if child.kind() == ":" {
                    found_colon = true;
                    continue;
                }
                if found_colon {
                    type_node = Some(child);
                    break;
                }
            }
            match type_node {
                Some(n) => n,
                None => return Vec::new(),
            }
        }
    };

    extract_function_type_params(&type_node, source)
}

/// Extract parameter types from an external declaration's function type.
fn extract_external_params(node: &Node, source: &str) -> Vec<Parameter> {
    // Find the type after the ":"
    let mut found_colon = false;
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == ":" {
            found_colon = true;
            continue;
        }
        if found_colon && child.kind() != "=" {
            return extract_function_type_params(&child, source);
        }
    }
    Vec::new()
}

/// Recursively extract domain types from a function_type chain.
/// `int -> string -> bool` yields params for int, string (last type is return).
fn extract_function_type_params(node: &Node, source: &str) -> Vec<Parameter> {
    if node.kind() != "function_type" {
        return Vec::new();
    }

    let mut params = Vec::new();
    let domain = node.child_by_field_name("domain");
    let codomain = node.child_by_field_name("codomain");

    if let Some(d) = domain {
        let type_text = d
            .utf8_text(source.as_bytes())
            .ok()
            .unwrap_or("")
            .to_string();
        if !type_text.is_empty() {
            params.push(Parameter {
                name: format!("_{}", params.len()),
                type_annotation: Some(type_text),
            });
        }
    }

    // If codomain is also a function_type, recurse; otherwise it's the return type
    if let Some(c) = codomain {
        if c.kind() == "function_type" {
            let mut sub_params = extract_function_type_params(&c, source);
            // Renumber parameters
            for p in &mut sub_params {
                let idx = params.len();
                p.name = format!("_{}", idx);
                params.push(Parameter {
                    name: p.name.clone(),
                    type_annotation: p.type_annotation.clone(),
                });
            }
        }
        // else: it's the return type, not a parameter
    }

    params
}

/// Extract return type for a let_binding or value_specification.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "let_binding" => {
            // Check for explicit type annotation: `let f x : int = ...`
            node.child_by_field_name("type")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())
        }
        "value_specification" | "external" => {
            // Extract the final return type from the function type chain
            let mut found_colon = false;
            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                if child.kind() == ":" {
                    found_colon = true;
                    continue;
                }
                if found_colon && child.kind() != "=" {
                    return extract_final_return_type(&child, source);
                }
            }
            None
        }
        "method_specification" => {
            // method name : type
            let mut found_colon = false;
            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                if child.kind() == ":" {
                    found_colon = true;
                    continue;
                }
                if found_colon {
                    return extract_final_return_type(&child, source);
                }
            }
            None
        }
        _ => None,
    }
}

/// Get the final return type from a (possibly nested) function_type.
/// For `int -> string -> bool`, returns "bool".
/// For non-function types, returns the type itself.
fn extract_final_return_type(node: &Node, source: &str) -> Option<String> {
    if node.kind() == "function_type" {
        let codomain = node.child_by_field_name("codomain")?;
        extract_final_return_type(&codomain, source)
    } else {
        node.utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.to_string())
    }
}

/// Determine if a let_binding defines a function (has parameters or fun/function body).
fn is_function_binding(node: &Node) -> bool {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "parameter" {
            return true;
        }
    }

    // Check for fun/function expression body
    if let Some(body) = node.child_by_field_name("body") {
        let bk = body.kind();
        if bk == "fun_expression" || bk == "function_expression" {
            return true;
        }
    }

    false
}

/// Post-process OCaml symbols.
/// - Reclassify parameterless let_bindings (non-fun) from Function to Constant.
/// - Map exception_definition constructor to Type kind (already correct from query).
fn post_process(mut sym: SymbolInfo, node: &Node, _source: &str) -> Option<SymbolInfo> {
    match node.kind() {
        "let_binding" => {
            if !is_function_binding(node) {
                sym.kind = SymbolKind::Constant;
                sym.parameters = None;
                sym.return_type = None;
            }
        }
        "module_binding" => {
            // Keep as Class (definition.module maps to Class)
            sym.signature = Some(format!("module {}", sym.name));
        }
        "constructor_declaration" => {
            // Exception definitions — already Type from query
        }
        _ => {}
    }
    Some(sym)
}

/// Return OCaml language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: None, // OCaml has no visibility modifiers
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::registry::extract_file;
    use super::super::super::{SymbolInfo, SymbolKind};
    use std::sync::Arc;

    fn extract_ml(source: &str) -> Vec<SymbolInfo> {
        extract_file("ml", source, Arc::from("test.ml"))
    }

    fn extract_mli(source: &str) -> Vec<SymbolInfo> {
        extract_file("mli", source, Arc::from("test.mli"))
    }

    #[test]
    fn test_ocaml_function_with_params() {
        let source = "let add x y = x + y\n";
        let symbols = extract_ml(source);
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "add");
        assert_eq!(sym.kind, SymbolKind::Function);
        let params = sym.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "x");
        assert_eq!(params[1].name, "y");
        assert!(sym.signature.as_ref().unwrap().contains("let add x y"));
    }

    #[test]
    fn test_ocaml_value_binding() {
        let source = "let value = 42\n";
        let symbols = extract_ml(source);
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "value");
        assert_eq!(sym.kind, SymbolKind::Constant);
        assert!(sym.parameters.is_none());
    }

    #[test]
    fn test_ocaml_fun_expression() {
        let source = "let square : int -> int = fun x -> x * x\n";
        let symbols = extract_ml(source);
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "square");
        assert_eq!(sym.kind, SymbolKind::Function);
    }

    #[test]
    fn test_ocaml_type_definitions() {
        let source = r#"type color = Red | Green | Blue
type point = { x: float; y: float }
"#;
        let symbols = extract_ml(source);
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "color");
        assert_eq!(symbols[0].kind, SymbolKind::Type);
        assert_eq!(symbols[1].name, "point");
        assert_eq!(symbols[1].kind, SymbolKind::Type);
    }

    #[test]
    fn test_ocaml_module_with_nested() {
        let source = r#"module MyModule = struct
  let helper x = x * 2
  type config = { debug: bool }
end
"#;
        let symbols = extract_ml(source);
        assert!(symbols.len() >= 3);

        let module_sym = symbols.iter().find(|s| s.name == "MyModule").unwrap();
        assert_eq!(module_sym.kind, SymbolKind::Class);

        let helper_sym = symbols.iter().find(|s| s.name == "helper").unwrap();
        assert_eq!(helper_sym.kind, SymbolKind::Function);
        assert_eq!(helper_sym.parent_symbol.as_deref(), Some("MyModule"));

        let config_sym = symbols.iter().find(|s| s.name == "config").unwrap();
        assert_eq!(config_sym.kind, SymbolKind::Type);
        assert_eq!(config_sym.parent_symbol.as_deref(), Some("MyModule"));
    }

    #[test]
    fn test_ocaml_module_type() {
        let source = r#"module type Printable = sig
  type t
  val to_string : t -> string
end
"#;
        let symbols = extract_ml(source);
        let mt = symbols.iter().find(|s| s.name == "Printable").unwrap();
        assert_eq!(mt.kind, SymbolKind::Interface);
    }

    #[test]
    fn test_ocaml_class_with_methods() {
        let source = r#"class counter = object
  method increment = count <- count + 1
  method get_count = count
end
"#;
        let symbols = extract_ml(source);
        let class_sym = symbols.iter().find(|s| s.name == "counter").unwrap();
        assert_eq!(class_sym.kind, SymbolKind::Class);

        let inc = symbols.iter().find(|s| s.name == "increment").unwrap();
        assert_eq!(inc.kind, SymbolKind::Method);
        assert_eq!(inc.parent_symbol.as_deref(), Some("counter"));

        let get = symbols.iter().find(|s| s.name == "get_count").unwrap();
        assert_eq!(get.kind, SymbolKind::Method);
        assert_eq!(get.parent_symbol.as_deref(), Some("counter"));
    }

    #[test]
    fn test_ocaml_exception() {
        let source = "exception Not_found_custom of string\n";
        let symbols = extract_ml(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Not_found_custom");
        assert_eq!(symbols[0].kind, SymbolKind::Type);
    }

    #[test]
    fn test_ocaml_external() {
        let source = "external c_function : int -> int = \"c_function_impl\"\n";
        let symbols = extract_ml(source);
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "c_function");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.return_type.as_deref(), Some("int"));
    }

    #[test]
    fn test_ocaml_mli_val_specifications() {
        let source = r#"val add : int -> int -> int
val greet : string -> unit
"#;
        let symbols = extract_mli(source);
        assert_eq!(symbols.len(), 2);

        let add = &symbols[0];
        assert_eq!(add.name, "add");
        assert_eq!(add.kind, SymbolKind::Function);
        assert_eq!(add.return_type.as_deref(), Some("int"));
        let params = add.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].type_annotation.as_deref(), Some("int"));
        assert_eq!(params[1].type_annotation.as_deref(), Some("int"));

        let greet = &symbols[1];
        assert_eq!(greet.name, "greet");
        assert_eq!(greet.kind, SymbolKind::Function);
        assert_eq!(greet.return_type.as_deref(), Some("unit"));
    }

    #[test]
    fn test_ocaml_mli_types_and_modules() {
        let source = r#"type color = Red | Green | Blue

module MyModule : sig
  val helper : int -> int
end

module type Printable = sig
  type t
  val to_string : t -> string
end
"#;
        let symbols = extract_mli(source);

        let color = symbols.iter().find(|s| s.name == "color").unwrap();
        assert_eq!(color.kind, SymbolKind::Type);

        let module_sym = symbols.iter().find(|s| s.name == "MyModule").unwrap();
        assert_eq!(module_sym.kind, SymbolKind::Class);

        let mt = symbols.iter().find(|s| s.name == "Printable").unwrap();
        assert_eq!(mt.kind, SymbolKind::Interface);
    }

    #[test]
    fn test_ocaml_mli_class_with_methods() {
        let source = r#"class counter : object
  method increment : unit
  method get_count : int
end
"#;
        let symbols = extract_mli(source);
        let class_sym = symbols.iter().find(|s| s.name == "counter").unwrap();
        assert_eq!(class_sym.kind, SymbolKind::Class);

        let inc = symbols.iter().find(|s| s.name == "increment").unwrap();
        assert_eq!(inc.kind, SymbolKind::Method);
        assert_eq!(inc.parent_symbol.as_deref(), Some("counter"));
        assert_eq!(inc.return_type.as_deref(), Some("unit"));
    }
}
