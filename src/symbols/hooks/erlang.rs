use super::{LanguageHooks, Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Erlang visibility: all symbols are visible.
/// Erlang uses -export([...]) to control visibility, but we index everything.
fn is_visible(_node: &Node, _source: &str) -> bool {
    true
}

/// Build signature string for Erlang symbols.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function => {
            // fun_decl node: count args in the first function_clause
            let arity = fun_decl_arity(node, source);
            format!("{}/{}", name, arity)
        }
        SymbolKind::Type => format!("-type {}()", name),
        SymbolKind::Class => format!("-record({})", name),
        SymbolKind::Method => {
            // callback: count args from type_sig > expr_args
            let arity = callback_arity(node);
            format!("-callback {}/{}", name, arity)
        }
        SymbolKind::Constant => format!("-define({})", name),
        _ => name.to_string(),
    }
}

/// Count parameters in a fun_decl node by finding the first function_clause's expr_args.
fn fun_decl_arity(node: &Node, _source: &str) -> usize {
    // node is a fun_decl; find clause -> args (expr_args)
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "function_clause" {
            return count_named_children_in_args(&child);
        }
    }
    0
}

/// Count named children in the `args` field (expr_args) of a node.
fn count_named_children_in_args(clause: &Node) -> usize {
    if let Some(args) = clause.child_by_field_name("args") {
        // Count only the `args:` field children (the actual parameters)
        let mut count = 0;
        for j in 0..args.child_count() {
            if args.field_name_for_child(j as u32) == Some("args") {
                count += 1;
            }
        }
        if count > 0 {
            return count;
        }
        // Fallback: count all named children that aren't parentheses
        let mut count = 0;
        for j in 0..args.child_count() {
            let gc = args.child(j).unwrap();
            if gc.is_named() {
                count += 1;
            }
        }
        count
    } else {
        0
    }
}

/// Count parameters in a callback node.
fn callback_arity(node: &Node) -> usize {
    // callback > sigs: type_sig > args: expr_args
    if let Some(sigs) = node.child_by_field_name("sigs") {
        if let Some(args) = sigs.child_by_field_name("args") {
            let mut count = 0;
            for j in 0..args.child_count() {
                let gc = args.child(j).unwrap();
                if gc.is_named() {
                    count += 1;
                }
            }
            return count;
        }
    }
    0
}

/// Extract parameters from an Erlang fun_decl node.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    // node is a fun_decl; find the first function_clause's args
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "function_clause" {
            if let Some(args) = child.child_by_field_name("args") {
                return extract_params_from_args(&args, source);
            }
        }
    }
    Vec::new()
}

/// Extract parameter names from an expr_args node.
fn extract_params_from_args(args: &Node, source: &str) -> Vec<Parameter> {
    let mut params = Vec::new();
    for j in 0..args.child_count() {
        let gc = args.child(j).unwrap();
        if gc.is_named() {
            let text = gc
                .utf8_text(source.as_bytes())
                .unwrap_or("")
                .to_string();
            if !text.is_empty() {
                params.push(Parameter {
                    name: text,
                    type_annotation: None,
                });
            }
        }
    }
    params
}

/// Erlang is dynamically typed — no return type extraction.
fn extract_return_type(_node: &Node, _source: &str) -> Option<String> {
    None
}

/// Post-process: no transformations needed.
/// Multiple function clauses are kept since the framework deduplicates by position.
/// Module attributes are mapped to Class by the query, which is acceptable for display.
fn post_process(sym: SymbolInfo, _node: &Node, _source: &str) -> Option<SymbolInfo> {
    Some(sym)
}

/// Return the language hooks for Erlang.
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
    use crate::symbols::registry::extract_file;
    use std::sync::Arc;

    fn extract(source: &str) -> Vec<SymbolInfo> {
        extract_file("erl", source, Arc::from("test.erl")).0
    }

    #[test]
    fn test_module_declaration() {
        let syms = extract("-module(my_module).");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "my_module");
        assert_eq!(syms[0].kind, SymbolKind::Class); // definition.module maps to Class
    }

    #[test]
    fn test_simple_function() {
        let source = "greet(Name) -> io:format(\"Hello ~s~n\", [Name]).";
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "greet");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].signature.as_deref(), Some("greet/1"));
    }

    #[test]
    fn test_function_two_params() {
        let source = "add(A, B) -> A + B.";
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "add");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].signature.as_deref(), Some("add/2"));
        let params = syms[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "A");
        assert_eq!(params[1].name, "B");
    }

    #[test]
    fn test_type_definition() {
        let source = "-type name() :: binary().";
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "name");
        assert_eq!(syms[0].kind, SymbolKind::Type);
        assert_eq!(syms[0].signature.as_deref(), Some("-type name()"));
    }

    #[test]
    fn test_record_declaration() {
        let source = "-record(person, {name :: binary(), age :: integer()}).";
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "person");
        assert_eq!(syms[0].kind, SymbolKind::Class);
        assert_eq!(syms[0].signature.as_deref(), Some("-record(person)"));
    }

    #[test]
    fn test_callback_declaration() {
        let source = "-callback init(Args :: term()) -> {ok, State :: term()}.";
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "init");
        assert_eq!(syms[0].kind, SymbolKind::Method);
        assert_eq!(syms[0].signature.as_deref(), Some("-callback init/1"));
    }

    #[test]
    fn test_macro_definition() {
        let source = "-define(MAX_SIZE, 100).";
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "MAX_SIZE");
        assert_eq!(syms[0].kind, SymbolKind::Constant);
        assert_eq!(syms[0].signature.as_deref(), Some("-define(MAX_SIZE)"));
    }

    #[test]
    fn test_multiple_clauses() {
        let source = r#"fib(0) -> 0;
fib(1) -> 1;
fib(N) -> fib(N-1) + fib(N-2).
"#;
        let syms = extract(source);
        // Each clause is a separate fun_decl, so we get 3 entries
        let fib_syms: Vec<_> = syms.iter().filter(|s| s.name == "fib").collect();
        assert!(
            fib_syms.len() >= 1,
            "at least one fib clause should be captured"
        );
        assert!(fib_syms.iter().all(|s| s.kind == SymbolKind::Function));
    }

    #[test]
    fn test_mixed_symbols() {
        let source = r#"-module(my_module).
-export([greet/1, add/2]).

-type name() :: binary().
-record(person, {name :: binary(), age :: integer()}).
-callback init(Args :: term()) -> {ok, State :: term()}.

greet(Name) -> io:format("Hello ~s~n", [Name]).
add(A, B) -> A + B.
"#;
        let syms = extract(source);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        let kinds: Vec<_> = syms.iter().map(|s| (s.name.as_str(), s.kind)).collect();

        assert!(names.contains(&"my_module"), "module missing, got: {:?}", names);
        assert!(names.contains(&"name"), "type missing, got: {:?}", names);
        assert!(names.contains(&"person"), "record missing, got: {:?}", names);
        assert!(names.contains(&"init"), "callback missing, got: {:?}", names);
        assert!(names.contains(&"greet"), "function greet missing, got: {:?}", names);
        assert!(names.contains(&"add"), "function add missing, got: {:?}", names);

        // Verify kinds
        assert!(
            kinds.iter().any(|(n, k)| *n == "my_module" && *k == SymbolKind::Class),
            "my_module should be Class (module)"
        );
        assert!(
            kinds.iter().any(|(n, k)| *n == "name" && *k == SymbolKind::Type),
            "name should be Type"
        );
        assert!(
            kinds.iter().any(|(n, k)| *n == "person" && *k == SymbolKind::Class),
            "person should be Class (record)"
        );
        assert!(
            kinds.iter().any(|(n, k)| *n == "init" && *k == SymbolKind::Method),
            "init should be Method (callback)"
        );
        assert!(
            kinds.iter().any(|(n, k)| *n == "greet" && *k == SymbolKind::Function),
            "greet should be Function"
        );
        assert!(
            kinds.iter().any(|(n, k)| *n == "add" && *k == SymbolKind::Function),
            "add should be Function"
        );
    }

    #[test]
    fn test_zero_arity_function() {
        let source = "start() -> ok.";
        let syms = extract(source);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "start");
        assert_eq!(syms[0].signature.as_deref(), Some("start/0"));
        let params = syms[0].parameters.as_ref().unwrap();
        assert!(params.is_empty());
    }

    #[test]
    fn test_header_file_record() {
        // .hrl files (headers) commonly define records and macros
        let source = r#"-record(state, {pid :: pid(), name :: atom()}).
-define(TIMEOUT, 5000).
"#;
        let syms = extract_file("hrl", source, Arc::from("test.hrl")).0;
        assert_eq!(syms.len(), 2);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"state"));
        assert!(names.contains(&"TIMEOUT"));
    }
}
