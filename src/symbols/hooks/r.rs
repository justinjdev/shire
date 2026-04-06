use super::{field_text, node_text, LanguageHooks, Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// R class-definition function names that we recognize.
const S4_CLASS_FNS: &[&str] = &["setClass", "setRefClass"];
const R6_CLASS_FNS: &[&str] = &["R6Class"];

/// Build signature string for R symbols.
///
/// Functions: `name <- function(params)`
/// S4 classes: `setClass("Name")`
/// R6 classes: `Name <- R6Class(...)`
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function => {
            let operator = node
                .child_by_field_name("operator")
                .and_then(|n| node_text(&n, source))
                .unwrap_or("<-");
            let params_text = node
                .child_by_field_name("rhs")
                .and_then(|rhs| rhs.child_by_field_name("parameters"))
                .and_then(|n| node_text(&n, source))
                .unwrap_or("()");
            format!("{} {} function{}", name, operator, params_text)
        }
        SymbolKind::Class => {
            if node.kind() == "call" {
                // S4: setClass("Name", ...)
                let fn_name = node
                    .child_by_field_name("function")
                    .and_then(|n| node_text(&n, source))
                    .unwrap_or("setClass");
                format!("{}(\"{}\")", fn_name, name)
            } else {
                // R6: Name <- R6Class(...)
                let fn_name = node
                    .child_by_field_name("rhs")
                    .and_then(|rhs| rhs.child_by_field_name("function"))
                    .and_then(|n| node_text(&n, source))
                    .unwrap_or("R6Class");
                format!("{} <- {}(...)", name, fn_name)
            }
        }
        _ => name.to_string(),
    }
}

/// Extract parameters from an R function definition.
///
/// The definition node is the `binary_operator`; the `rhs` field is the
/// `function_definition` containing the `parameters` node.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let func_def = match node.child_by_field_name("rhs") {
        Some(n) if n.kind() == "function_definition" => n,
        _ => return Vec::new(),
    };

    let params_node = match func_def.child_by_field_name("parameters") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        if child.kind() == "parameter" {
            let name = match field_text(&child, "name", source) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if !name.is_empty() {
                params.push(Parameter {
                    name,
                    type_annotation: None,
                });
            }
        }
    }

    params
}

/// Post-process: filter class definitions to only recognized patterns.
///
/// - For `call` nodes (S4): keep only if function is setClass/setRefClass
/// - For `binary_operator` nodes (R6): keep only if rhs call function is R6Class
fn post_process(sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    if sym.kind == SymbolKind::Class {
        if node.kind() == "call" {
            // S4 pattern: setClass("Name", ...)
            let fn_name = node
                .child_by_field_name("function")
                .and_then(|n| node_text(&n, source))?;
            if !S4_CLASS_FNS.contains(&fn_name) {
                return None;
            }
        } else if node.kind() == "binary_operator" {
            // R6 pattern: Name <- R6Class(...)
            let rhs = node.child_by_field_name("rhs")?;
            if rhs.kind() != "call" {
                return None;
            }
            let fn_name = rhs
                .child_by_field_name("function")
                .and_then(|n| node_text(&n, source))?;
            if !R6_CLASS_FNS.contains(&fn_name) {
                return None;
            }
        }
    }
    Some(sym)
}

/// Return the language hooks for R.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: None,
        resolve_parent: None,
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: None,
        post_process: Some(post_process),
        reference_hooks: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{extract_file, SymbolKind};
    use std::sync::Arc;

    #[test]
    fn test_r_function_arrow_assignment() {
        let source = r#"my_func <- function(x, y) {
    x + y
}
"#;
        let (symbols, _) = extract_file("r", source, Arc::from("math.r"), true);
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "my_func");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(
            sym.signature.as_deref(),
            Some("my_func <- function(x, y)")
        );
        let params = sym.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "x");
        assert_eq!(params[1].name, "y");
    }

    #[test]
    fn test_r_function_equals_assignment() {
        let source = r#"process = function(data) {
    data
}
"#;
        let (symbols, _) = extract_file("r", source, Arc::from("proc.r"), true);
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "process");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.signature.as_deref(), Some("process = function(data)"));
    }

    #[test]
    fn test_r_function_with_defaults() {
        let source = r#"greet <- function(name, greeting = "hello") {
    paste(greeting, name)
}
"#;
        let (symbols, _) = extract_file("r", source, Arc::from("greet.r"), true);
        assert_eq!(symbols.len(), 1);
        let params = symbols[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "name");
        assert_eq!(params[1].name, "greeting");
    }

    #[test]
    fn test_r_s4_class() {
        let source = r#"setClass("Person",
  representation(
    name = "character",
    age = "numeric"
  )
)
"#;
        let (symbols, _) = extract_file("r", source, Arc::from("person.r"), true);
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "Person");
        assert_eq!(sym.kind, SymbolKind::Class);
        assert_eq!(sym.signature.as_deref(), Some("setClass(\"Person\")"));
    }

    #[test]
    fn test_r_r6_class() {
        let source = r#"Animal <- R6Class("Animal",
  public = list(
    name = NULL,
    initialize = function(name) {
      self$name <- name
    }
  )
)
"#;
        let (symbols, _) = extract_file("r", source, Arc::from("animal.r"), true);
        // Should capture the R6 class definition (Animal <- R6Class)
        // The inner function assignment (self$name <- name) should not match
        // since self$name is an extract_operator, not an identifier
        let class_symbols: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Class).collect();
        assert_eq!(class_symbols.len(), 1);
        assert_eq!(class_symbols[0].name, "Animal");
        assert_eq!(
            class_symbols[0].signature.as_deref(),
            Some("Animal <- R6Class(...)")
        );
    }

    #[test]
    fn test_r_non_class_call_filtered() {
        let source = r#"library("dplyr")
x <- some_function()
"#;
        let (symbols, _) = extract_file("r", source, Arc::from("load.r"), true);
        // library("dplyr") should be filtered out (not setClass/setRefClass)
        // x <- some_function() should be filtered out (not R6Class)
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_r_multiple_functions() {
        let source = r#"add <- function(a, b) a + b

subtract <- function(a, b) a - b

multiply <- function(a, b) a * b
"#;
        let (symbols, _) = extract_file("r", source, Arc::from("ops.r"), true);
        assert_eq!(symbols.len(), 3);
        assert_eq!(symbols[0].name, "add");
        assert_eq!(symbols[1].name, "subtract");
        assert_eq!(symbols[2].name, "multiply");
    }

    #[test]
    fn test_r_no_params_function() {
        let source = r#"get_pi <- function() 3.14159
"#;
        let (symbols, _) = extract_file("r", source, Arc::from("const.r"), true);
        assert_eq!(symbols.len(), 1);
        let params = symbols[0].parameters.as_ref().unwrap();
        assert!(params.is_empty());
        assert_eq!(
            symbols[0].signature.as_deref(),
            Some("get_pi <- function()")
        );
    }

    #[test]
    fn test_r_uppercase_extension() {
        let source = r#"analyze <- function(data) data
"#;
        let (symbols, _) = extract_file("R", source, Arc::from("script.R"), true);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "analyze");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_r_dots_parameter() {
        let source = r#"wrapper <- function(x, ...) {
    inner(x, ...)
}
"#;
        let (symbols, _) = extract_file("r", source, Arc::from("wrap.r"), true);
        assert_eq!(symbols.len(), 1);
        let params = symbols[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "x");
        assert_eq!(params[1].name, "...");
    }

    #[test]
    fn test_r_setrefclass() {
        let source = r#"setRefClass("Counter",
  fields = list(
    count = "numeric"
  ),
  methods = list(
    increment = function() {
      count <<- count + 1
    }
  )
)
"#;
        let (symbols, _) = extract_file("r", source, Arc::from("counter.r"), true);
        let class_symbols: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Class).collect();
        assert_eq!(class_symbols.len(), 1);
        assert_eq!(class_symbols[0].name, "Counter");
        assert_eq!(
            class_symbols[0].signature.as_deref(),
            Some("setRefClass(\"Counter\")")
        );
    }
}
