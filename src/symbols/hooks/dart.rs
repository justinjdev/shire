use super::{find_ancestor, find_child_by_kind, LanguageHooks, Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Dart visibility: names starting with `_` are private.
/// Also checks ancestor class/mixin/extension visibility.
fn is_visible(node: &Node, source: &str) -> bool {
    let name = extract_name(node, source).unwrap_or_default();
    if name.starts_with('_') {
        return false;
    }

    // Check all ancestor type declarations for private names
    let mut current = node.parent();
    while let Some(n) = current {
        match n.kind() {
            "class_definition" | "mixin_declaration" | "enum_declaration"
            | "extension_declaration" => {
                if let Some(ancestor_name) = extract_name(&n, source) {
                    if ancestor_name.starts_with('_') {
                        return false;
                    }
                }
            }
            _ => {}
        }
        current = n.parent();
    }

    true
}

/// Extract the name from various Dart node types.
fn extract_name<'a>(node: &Node, source: &'a str) -> Option<&'a str> {
    // Try the `name` field first (works for class_definition, function_signature, etc.)
    if let Some(name) = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
    {
        return Some(name);
    }

    // For nodes without a name field (mixin_declaration, factory constructors),
    // find the first identifier child
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "identifier" || child.kind() == "type_identifier" {
            return child.utf8_text(source.as_bytes()).ok();
        }
    }

    None
}

/// For methods, getters, setters, and constructors inside class/mixin/extension bodies,
/// resolve the parent type name.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "method_signature" | "constructor_signature" | "constant_constructor_signature"
        | "factory_constructor_signature" | "redirecting_factory_constructor_signature" => {}
        _ => return None,
    }

    let parent = find_ancestor(node, "class_definition")
        .or_else(|| find_ancestor(node, "mixin_declaration"))
        .or_else(|| find_ancestor(node, "enum_declaration"))
        .or_else(|| find_ancestor(node, "extension_declaration"))?;

    extract_name(&parent, source).map(|s| s.to_string())
}

/// Build a signature string for a Dart symbol.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function | SymbolKind::Method => build_callable_signature(node, source),
        SymbolKind::Enum => format!("enum {}", name),
        SymbolKind::Type => format!("typedef {}", name),
        _ => build_type_signature(node, source, name),
    }
}

/// Build signature for functions, methods, getters, setters, constructors.
/// Extracts source text from node start to function_body start.
fn build_callable_signature(node: &Node, source: &str) -> String {
    let start = node.start_byte();

    // For method_signature / function_signature, the body is a sibling (in class_member_definition
    // or program), not a child. Look at the next sibling for function_body.
    let end = next_sibling_body_start(node)
        .or_else(|| find_child_by_kind(node, "function_body").map(|n| n.start_byte()))
        .unwrap_or(node.end_byte());

    source[start..end.min(source.len())].trim().to_string()
}

/// Find the start byte of a function_body sibling that follows this node.
fn next_sibling_body_start(node: &Node) -> Option<usize> {
    let mut sibling = node.next_sibling();
    while let Some(s) = sibling {
        if s.kind() == "function_body" {
            return Some(s.start_byte());
        }
        if s.is_named() {
            break;
        }
        sibling = s.next_sibling();
    }
    None
}

/// Build signature for class-like types (class, mixin, extension).
fn build_type_signature(node: &Node, source: &str, name: &str) -> String {
    match node.kind() {
        "class_definition" | "mixin_declaration" | "extension_declaration" => {
            let start = node.start_byte();
            let end = node
                .child_by_field_name("body")
                .map(|n| n.start_byte())
                .unwrap_or_else(|| {
                    // mixin_declaration uses class_body child, extension uses extension_body
                    find_child_by_kind(node, "class_body")
                        .or_else(|| find_child_by_kind(node, "extension_body"))
                        .map(|n| n.start_byte())
                        .unwrap_or(node.end_byte())
                });
            let sig = source[start..end.min(source.len())].trim();
            if sig.is_empty() {
                format!("{} {}", node.kind().split('_').next().unwrap_or("class"), name)
            } else {
                sig.to_string()
            }
        }
        _ => format!("class {}", name),
    }
}

/// Extract parameters from a Dart function/method/constructor.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    // For method_signature, dig into the inner signature node
    let sig_node = find_inner_signature(node).unwrap_or(*node);

    let params_node = match sig_node
        .child_by_field_name("parameters")
        .or_else(|| find_child_by_kind(&sig_node, "formal_parameter_list"))
    {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();
    collect_parameters(&params_node, source, &mut params);
    params
}

/// Collect parameters from a formal_parameter_list, including optional parameter sections.
fn collect_parameters(params_node: &Node, source: &str, params: &mut Vec<Parameter>) {
    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        match child.kind() {
            "formal_parameter" => {
                if let Some(param) = extract_single_param(&child, source) {
                    params.push(param);
                }
            }
            "optional_formal_parameters" => {
                for j in 0..child.child_count() {
                    let opt_child = child.child(j).unwrap();
                    if opt_child.kind() == "formal_parameter" {
                        if let Some(param) = extract_single_param(&opt_child, source) {
                            params.push(param);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Extract a single parameter's name and type.
fn extract_single_param(param_node: &Node, source: &str) -> Option<Parameter> {
    let name = param_node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string());

    // If no name field, find the last identifier (in `Type name`, name is last)
    let name = name.or_else(|| {
        let mut last_ident = None;
        for i in 0..param_node.child_count() {
            let child = param_node.child(i).unwrap();
            if child.kind() == "identifier" {
                last_ident = Some(child);
            }
        }
        last_ident.and_then(|n| n.utf8_text(source.as_bytes()).ok().map(|s| s.to_string()))
    });

    let name = name?;
    if name.is_empty() {
        return None;
    }

    let type_ann = find_param_type(param_node, source);

    Some(Parameter {
        name,
        type_annotation: type_ann,
    })
}

/// Find the type annotation for a parameter.
fn find_param_type(param_node: &Node, source: &str) -> Option<String> {
    let type_kinds = ["type_identifier", "void_type", "function_type", "inferred_type"];

    for i in 0..param_node.child_count() {
        let child = param_node.child(i).unwrap();
        if type_kinds.contains(&child.kind()) {
            let mut type_text = child.utf8_text(source.as_bytes()).ok()?.to_string();
            // Check for nullable `?` following the type
            if let Some(next) = param_node.child(i + 1) {
                if next.kind() == "?" {
                    type_text.push('?');
                }
            }
            return Some(type_text);
        }
        // Handle constructor_param (this.name)
        if child.kind() == "constructor_param" {
            return find_param_type(&child, source);
        }
    }
    None
}

/// Extract return type from a Dart function/getter signature.
/// In the 0.0.4 grammar, function_signature has no `return_type` field;
/// the return type is a type_identifier or void_type child.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    let sig_node = find_inner_signature(node).unwrap_or(*node);

    let type_kinds = ["type_identifier", "void_type", "function_type"];

    for i in 0..sig_node.child_count() {
        let child = sig_node.child(i).unwrap();
        if type_kinds.contains(&child.kind()) {
            let mut type_text = child.utf8_text(source.as_bytes()).ok()?.to_string();
            // Check for nullable `?`
            if let Some(next) = sig_node.child(i + 1) {
                if next.kind() == "?" {
                    type_text.push('?');
                }
            }
            return Some(type_text);
        }
    }
    None
}

/// For method_signature nodes, find the inner signature (function_signature,
/// getter_signature, setter_signature, or constructor_signature).
fn find_inner_signature<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    if node.kind() != "method_signature" {
        return None;
    }

    let inner_kinds = [
        "function_signature",
        "getter_signature",
        "setter_signature",
        "constructor_signature",
        "operator_signature",
    ];

    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if inner_kinds.contains(&child.kind()) {
            return Some(child);
        }
    }
    None
}

/// Post-process Dart symbols.
fn post_process(mut sym: SymbolInfo, node: &Node, _source: &str) -> Option<SymbolInfo> {
    // Set visibility based on name
    if sym.name.starts_with('_') {
        sym.visibility = "private".to_string();
    }

    // Skip operator methods (they have no useful name capture)
    if node.kind() == "method_signature" {
        if let Some(inner) = find_inner_signature(node) {
            if inner.kind() == "operator_signature" {
                return None;
            }
        }
    }

    Some(sym)
}

/// Return Dart language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::registry::extract_file;
    use std::sync::Arc;

    fn extract(source: &str) -> Vec<SymbolInfo> {
        extract_file("dart", source, Arc::from("test.dart"))
    }

    #[test]
    fn test_public_class() {
        let syms = extract("class MyApp {}");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MyApp");
        assert_eq!(syms[0].kind, SymbolKind::Class);
        assert_eq!(syms[0].visibility, "public");
    }

    #[test]
    fn test_private_class_filtered() {
        let syms = extract("class _Internal {}");
        assert!(syms.is_empty(), "private class should be filtered out");
    }

    #[test]
    fn test_abstract_class() {
        let syms = extract("abstract class Animal {}");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Animal");
        assert_eq!(syms[0].kind, SymbolKind::Class);
        assert!(syms[0].signature.as_ref().unwrap().contains("abstract"));
    }

    #[test]
    fn test_class_with_extends() {
        let syms = extract("class Dog extends Animal {}");
        assert_eq!(syms.len(), 1);
        let sig = syms[0].signature.as_ref().unwrap();
        assert!(sig.contains("extends Animal"), "signature: {}", sig);
    }

    #[test]
    fn test_enum() {
        let syms = extract("enum Color { red, green, blue }");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Color");
        assert_eq!(syms[0].kind, SymbolKind::Enum);
    }

    #[test]
    fn test_mixin() {
        let syms = extract("mixin Swimming {}");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Swimming");
        assert_eq!(syms[0].kind, SymbolKind::Class);
        assert!(syms[0].signature.as_ref().unwrap().contains("mixin"));
    }

    #[test]
    fn test_extension() {
        let syms = extract("extension StringExt on String {}");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "StringExt");
        assert_eq!(syms[0].kind, SymbolKind::Class);
        let sig = syms[0].signature.as_ref().unwrap();
        assert!(sig.contains("extension"), "signature: {}", sig);
    }

    #[test]
    fn test_top_level_function() {
        let syms = extract("void greet(String name) {}");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "greet");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].return_type.as_deref(), Some("void"));
    }

    #[test]
    fn test_top_level_function_params() {
        let syms = extract("int add(int a, int b) => a + b;");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "add");
        let params = syms[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "a");
        assert_eq!(params[0].type_annotation.as_deref(), Some("int"));
        assert_eq!(params[1].name, "b");
    }

    #[test]
    fn test_private_function_filtered() {
        let syms = extract("void _helper() {}");
        assert!(syms.is_empty(), "private function should be filtered out");
    }

    #[test]
    fn test_method_with_parent() {
        let source = r#"
class Dog {
  void bark() {}
}
"#;
        let syms = extract(source);
        let methods: Vec<_> = syms.iter().filter(|s| s.kind == SymbolKind::Method).collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "bark");
        assert_eq!(methods[0].parent_symbol.as_deref(), Some("Dog"));
    }

    #[test]
    fn test_getter() {
        let source = r#"
class Foo {
  String get name => 'foo';
}
"#;
        let syms = extract(source);
        let methods: Vec<_> = syms.iter().filter(|s| s.kind == SymbolKind::Method).collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "name");
    }

    #[test]
    fn test_setter() {
        let source = r#"
class Foo {
  set value(int v) {}
}
"#;
        let syms = extract(source);
        let methods: Vec<_> = syms.iter().filter(|s| s.kind == SymbolKind::Method).collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "value");
    }

    #[test]
    fn test_top_level_getter() {
        let source = "int get count => 42;";
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "count");
        assert_eq!(syms[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_constructor() {
        let source = r#"
class Dog {
  Dog(String name) {}
}
"#;
        let syms = extract(source);
        let ctors: Vec<_> = syms
            .iter()
            .filter(|s| s.kind == SymbolKind::Method && s.name == "Dog")
            .collect();
        assert_eq!(ctors.len(), 1);
        assert_eq!(ctors[0].parent_symbol.as_deref(), Some("Dog"));
    }

    #[test]
    fn test_typedef() {
        let source = "typedef StringCallback = void Function(String);";
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "StringCallback");
        assert_eq!(syms[0].kind, SymbolKind::Type);
    }

    #[test]
    fn test_method_in_private_class_filtered() {
        let source = r#"
class _Internal {
  void doStuff() {}
}
"#;
        let syms = extract(source);
        assert!(syms.is_empty(), "methods in private class should be filtered");
    }

    #[test]
    fn test_optional_params() {
        let source = "void greet(String name, {int? age}) {}";
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        let params = syms[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "name");
        assert_eq!(params[1].name, "age");
    }

    #[test]
    fn test_return_type() {
        let source = "String getName() => 'test';";
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].return_type.as_deref(), Some("String"));
    }

    #[test]
    fn test_mixed_symbols() {
        let source = r#"
class Animal {
  void makeSound() {}
}

mixin Swimming {}

enum Color { red, green, blue }

void greet() {}

typedef Callback = void Function();
"#;
        let syms = extract(source);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Animal"));
        assert!(names.contains(&"Swimming"));
        assert!(names.contains(&"Color"));
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"Callback"));
        assert!(names.contains(&"makeSound"));
    }
}
