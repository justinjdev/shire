use super::{LanguageHooks, Parameter, SymbolInfo, SymbolKind, node_text};
use tree_sitter::Node;

/// Nix has no visibility modifiers — all bindings are visible.
fn is_visible(_node: &Node, _source: &str) -> bool {
    true
}

/// Get the value expression node from a binding (the `expression` field).
fn binding_value<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    node.child_by_field_name("expression")
}

/// Collect the names of formal parameters from a `formals` node.
fn collect_formal_names(formals: &Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let count = formals.child_count();
    for i in 0..count {
        let child = formals.child(i).unwrap();
        if child.kind() == "formal"
            && let Some(name_node) = child.child_by_field_name("name")
            && let Some(text) = node_text(&name_node, source)
        {
            names.push(text.to_string());
        }
    }
    names
}

/// Check if a formals node has an ellipsis (`...`).
fn has_ellipsis(formals: &Node) -> bool {
    formals.child_by_field_name("ellipses").is_some()
}

/// Collect all parameter info from a function expression, walking through
/// curried function chains (universal params: `a: b: ...`) and pattern
/// params (`{ pname, version, ... }:`).
fn collect_all_params(func_node: &Node, source: &str) -> Vec<Parameter> {
    let mut params = Vec::new();
    let mut current = *func_node;

    loop {
        // Check for universal parameter (simple `arg:` binding)
        if let Some(universal) = current.child_by_field_name("universal")
            && let Some(text) = node_text(&universal, source)
        {
            params.push(Parameter {
                name: text.to_string(),
                type_annotation: None,
            });
        }

        // Check for formals ({ arg1, arg2, ... }: pattern)
        if let Some(formals) = current.child_by_field_name("formals") {
            for name in collect_formal_names(&formals, source) {
                params.push(Parameter {
                    name,
                    type_annotation: None,
                });
            }
        }

        // Walk into the body if it's another function_expression (curried fn)
        match current.child_by_field_name("body") {
            Some(body) if body.kind() == "function_expression" => {
                current = body;
            }
            _ => break,
        }
    }

    params
}

/// Build a signature for a Nix binding.
fn build_signature(node: &Node, source: &str, name: &str, _kind: SymbolKind) -> String {
    let value = match binding_value(node) {
        Some(v) => v,
        None => return format!("{} = ...", name),
    };

    match value.kind() {
        "function_expression" => {
            let param_str = build_param_string(&value, source);
            format!("{} = {}: ...", name, param_str)
        }
        "attrset_expression" | "rec_attrset_expression" => {
            let prefix = if value.kind() == "rec_attrset_expression" {
                "rec "
            } else {
                ""
            };
            format!("{} = {}{{ ... }}", name, prefix)
        }
        _ => {
            // For simple values, show a short preview
            let text = node_text(&value, source).unwrap_or("...");
            let mut end = 40.min(text.len());
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            let preview = &text[..end];
            let preview = preview.replace('\n', " ");
            format!("{} = {}", name, preview)
        }
    }
}

/// Build the parameter portion of a function signature.
/// For pattern args: `{ pname, version, ... }`
/// For universal args: `arg`
/// For curried: `a: b`
fn build_param_string(func_node: &Node, source: &str) -> String {
    let mut parts = Vec::new();
    let mut current = *func_node;

    loop {
        if let Some(universal) = current.child_by_field_name("universal")
            && let Some(text) = node_text(&universal, source)
        {
            parts.push(text.to_string());
        }

        if let Some(formals) = current.child_by_field_name("formals") {
            let names = collect_formal_names(&formals, source);
            let ellipsis = if has_ellipsis(&formals) { ", ..." } else { "" };
            parts.push(format!("{{ {}{} }}", names.join(", "), ellipsis));
        }

        match current.child_by_field_name("body") {
            Some(body) if body.kind() == "function_expression" => {
                current = body;
            }
            _ => break,
        }
    }

    parts.join(": ")
}

/// Extract parameters from a binding whose value is a function expression.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let value = match binding_value(node) {
        Some(v) => v,
        None => return Vec::new(),
    };

    if value.kind() != "function_expression" {
        return Vec::new();
    }

    collect_all_params(&value, source)
}

/// Nix is dynamically typed — no return type information.
fn extract_return_type(_node: &Node, _source: &str) -> Option<String> {
    None
}

/// Reclassify bindings based on their value expression type and extract parameters
/// for functions (parameters aren't extracted by the normal hook because the kind
/// is initially Constant from the query capture).
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    let value = match binding_value(node) {
        Some(v) => v,
        None => return Some(sym),
    };

    match value.kind() {
        "function_expression" => {
            sym.kind = SymbolKind::Function;
            sym.parameters = Some(collect_all_params(&value, source));
        }
        "attrset_expression" | "rec_attrset_expression" => {
            sym.kind = SymbolKind::Class;
        }
        _ => {
            sym.kind = SymbolKind::Constant;
        }
    }

    Some(sym)
}

/// Return the language hooks for Nix.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: None,
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
        reference_hooks: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::query_extract;
    use std::sync::Arc;
    use tree_sitter::{Parser, Query};

    fn extract(source: &str) -> Vec<SymbolInfo> {
        let language: tree_sitter::Language = tree_sitter_nix::LANGUAGE.into();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let query_source = include_str!("../queries/nix.scm");
        let query = Query::new(&language, query_source).unwrap();
        let hooks = hooks();
        query_extract::extract(
            &mut parser,
            &query,
            source,
            Arc::from("test.nix"),
            &hooks,
            true,
        )
        .0
    }

    #[test]
    fn test_basic_attrset() {
        let source = r#"{
  buildInputs = [ pkgs.gcc ];

  hello = name: "Hello, ${name}";

  add = a: b: a + b;

  mkDerivation = { pname, version, src, ... }: derivation {
    inherit pname version src;
  };

  config = {
    enable = true;
    port = 8080;
  };

  greeting = "hello world";
}"#;
        let symbols = extract(source);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        // All top-level bindings should be extracted
        assert!(
            names.contains(&"buildInputs"),
            "missing buildInputs, got: {:?}",
            names
        );
        assert!(names.contains(&"hello"), "missing hello, got: {:?}", names);
        assert!(names.contains(&"add"), "missing add, got: {:?}", names);
        assert!(
            names.contains(&"mkDerivation"),
            "missing mkDerivation, got: {:?}",
            names
        );
        assert!(
            names.contains(&"config"),
            "missing config, got: {:?}",
            names
        );
        assert!(
            names.contains(&"greeting"),
            "missing greeting, got: {:?}",
            names
        );
    }

    #[test]
    fn test_kind_classification() {
        let source = r#"{
  buildInputs = [ pkgs.gcc ];
  hello = name: "Hello, ${name}";
  add = a: b: a + b;
  mkDerivation = { pname, version, src, ... }: derivation {
    inherit pname version src;
  };
  config = {
    enable = true;
    port = 8080;
  };
  greeting = "hello world";
}"#;
        let symbols = extract(source);
        let find = |name: &str| symbols.iter().find(|s| s.name == name).unwrap();

        // Functions
        assert_eq!(
            find("hello").kind,
            SymbolKind::Function,
            "hello should be Function"
        );
        assert_eq!(
            find("add").kind,
            SymbolKind::Function,
            "add should be Function"
        );
        assert_eq!(
            find("mkDerivation").kind,
            SymbolKind::Function,
            "mkDerivation should be Function"
        );

        // Attrset → Class
        assert_eq!(
            find("config").kind,
            SymbolKind::Class,
            "config should be Class"
        );

        // Simple values → Constant
        assert_eq!(
            find("buildInputs").kind,
            SymbolKind::Constant,
            "buildInputs should be Constant"
        );
        assert_eq!(
            find("greeting").kind,
            SymbolKind::Constant,
            "greeting should be Constant"
        );
    }

    #[test]
    fn test_function_parameters() {
        let source = r#"{
  hello = name: "Hello, ${name}";
  add = a: b: a + b;
  mkDerivation = { pname, version, src, ... }: derivation { };
}"#;
        let symbols = extract(source);
        let find = |name: &str| symbols.iter().find(|s| s.name == name).unwrap();

        let hello_params = find("hello").parameters.as_ref().unwrap();
        assert_eq!(hello_params.len(), 1);
        assert_eq!(hello_params[0].name, "name");

        let add_params = find("add").parameters.as_ref().unwrap();
        assert_eq!(add_params.len(), 2);
        assert_eq!(add_params[0].name, "a");
        assert_eq!(add_params[1].name, "b");

        let mk_params = find("mkDerivation").parameters.as_ref().unwrap();
        let mk_names: Vec<&str> = mk_params.iter().map(|p| p.name.as_str()).collect();
        assert!(
            mk_names.contains(&"pname"),
            "missing pname, got: {:?}",
            mk_names
        );
        assert!(
            mk_names.contains(&"version"),
            "missing version, got: {:?}",
            mk_names
        );
        assert!(
            mk_names.contains(&"src"),
            "missing src, got: {:?}",
            mk_names
        );
    }

    #[test]
    fn test_signatures() {
        let source = r#"{
  hello = name: "Hello";
  config = { enable = true; };
  greeting = "hello world";
  mkDrv = { pname, version, ... }: derivation { };
}"#;
        let symbols = extract(source);
        let find = |name: &str| symbols.iter().find(|s| s.name == name).unwrap();

        let hello_sig = find("hello").signature.as_deref().unwrap();
        assert!(
            hello_sig.contains("hello"),
            "sig should contain name: {}",
            hello_sig
        );
        assert!(
            hello_sig.contains("name"),
            "sig should contain param: {}",
            hello_sig
        );

        let config_sig = find("config").signature.as_deref().unwrap();
        assert!(
            config_sig.contains("config"),
            "sig should contain name: {}",
            config_sig
        );
        assert!(
            config_sig.contains("{ ... }"),
            "sig should contain attrset marker: {}",
            config_sig
        );

        let greeting_sig = find("greeting").signature.as_deref().unwrap();
        assert!(
            greeting_sig.contains("greeting"),
            "sig should contain name: {}",
            greeting_sig
        );
        assert!(
            greeting_sig.contains("hello world"),
            "sig should contain value: {}",
            greeting_sig
        );

        let mk_sig = find("mkDrv").signature.as_deref().unwrap();
        assert!(
            mk_sig.contains("mkDrv"),
            "sig should contain name: {}",
            mk_sig
        );
        assert!(
            mk_sig.contains("pname"),
            "sig should contain param: {}",
            mk_sig
        );
    }

    #[test]
    fn test_let_expression_bindings() {
        let source = r#"let
  name = "world";
  greet = x: "Hello, ${x}";
in
  greet name"#;
        let symbols = extract(source);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

        assert!(names.contains(&"name"), "missing name, got: {:?}", names);
        assert!(names.contains(&"greet"), "missing greet, got: {:?}", names);

        let find = |name: &str| symbols.iter().find(|s| s.name == name).unwrap();
        assert_eq!(find("name").kind, SymbolKind::Constant);
        assert_eq!(find("greet").kind, SymbolKind::Function);
    }

    #[test]
    fn test_rec_attrset() {
        let source = r#"rec {
  x = 1;
  y = x + 1;
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 2);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"x"));
        assert!(names.contains(&"y"));
    }

    #[test]
    fn test_line_numbers() {
        let source = r#"{
  first = 1;
  second = 2;
  third = 3;
}"#;
        let symbols = extract(source);
        let find = |name: &str| symbols.iter().find(|s| s.name == name).unwrap();

        assert_eq!(find("first").line, 2);
        assert_eq!(find("second").line, 3);
        assert_eq!(find("third").line, 4);
    }

    #[test]
    fn test_nested_attrset_bindings_extracted() {
        // Bindings inside nested attrsets should also be extracted
        // (the query captures all `binding` nodes regardless of depth)
        let source = r#"{
  outer = {
    inner = 42;
  };
}"#;
        let symbols = extract(source);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"outer"), "missing outer, got: {:?}", names);
        assert!(names.contains(&"inner"), "missing inner, got: {:?}", names);
    }

    #[test]
    fn test_nixpkgs_style_function() {
        // Common nixpkgs pattern: file is a function taking dependencies
        let source = r#"{ lib, stdenv, fetchurl }:

stdenv.mkDerivation {
  pname = "hello";
  version = "2.10";
}"#;
        let symbols = extract(source);
        // The top-level is a function_expression, not a binding.
        // Bindings inside the mkDerivation attrset should still be found.
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"pname"), "missing pname, got: {:?}", names);
        assert!(
            names.contains(&"version"),
            "missing version, got: {:?}",
            names
        );
    }
}
