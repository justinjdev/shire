use super::{node_text, LanguageHooks, Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Public def keywords that should be extracted as symbols.
const PUBLIC_DEF_KEYWORDS: &[&str] = &[
    "defn",
    "def",
    "defmacro",
    "defprotocol",
    "defrecord",
    "deftype",
    "defmulti",
    "ns",
];

/// Keywords that should be skipped (private or implementation details).
const SKIP_KEYWORDS: &[&str] = &["defn-", "defmethod"];

/// Get the first sym_lit named child of a list_lit, returning its text.
/// This is the def keyword (defn, def, defprotocol, etc.).
fn def_keyword_text<'a>(node: &Node, source: &'a str) -> Option<&'a str> {
    for i in 0..node.named_child_count() {
        let child = node.named_child(i).unwrap();
        if child.kind() == "sym_lit" {
            return node_text(&child, source);
        }
    }
    None
}

/// Filter: only include list_lit nodes where the first sym is a public def keyword.
fn is_visible(node: &Node, source: &str) -> bool {
    if node.kind() != "list_lit" {
        return false;
    }
    let keyword = match def_keyword_text(node, source) {
        Some(k) => k,
        None => return false,
    };
    if SKIP_KEYWORDS.contains(&keyword) {
        return false;
    }
    PUBLIC_DEF_KEYWORDS.contains(&keyword)
}

/// Find the parameter vector for a defn/defmacro form.
/// Single-arity: `(defn f [x y] ...)` — vec_lit is a direct child.
/// Multi-arity: `(defn f ([x] ...) ([x y] ...))` — vec_lit is inside nested list_lit children.
/// For multi-arity, returns the first arity's parameter vector.
fn find_param_vector<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    // Try single-arity first: direct vec_lit child
    for i in 0..node.named_child_count() {
        let child = node.named_child(i).unwrap();
        if child.kind() == "vec_lit" {
            return Some(child);
        }
    }
    // Multi-arity fallback: find first nested list_lit containing a vec_lit
    for i in 0..node.named_child_count() {
        let child = node.named_child(i).unwrap();
        if child.kind() == "list_lit" {
            for j in 0..child.named_child_count() {
                let gc = child.named_child(j).unwrap();
                if gc.kind() == "vec_lit" {
                    return Some(gc);
                }
            }
        }
    }
    None
}

/// Get the second sym_lit named child of a list_lit (the name symbol).
fn second_sym_lit<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    let mut count = 0;
    for i in 0..node.named_child_count() {
        let child = node.named_child(i).unwrap();
        if child.kind() == "sym_lit" {
            count += 1;
            if count == 2 {
                return Some(child);
            }
        }
    }
    None
}

/// Build signature string for Clojure symbols.
fn build_signature(node: &Node, source: &str, name: &str, _kind: SymbolKind) -> String {
    let keyword = def_keyword_text(node, source).unwrap_or("def");

    match keyword {
        "defn" | "defmacro" | "defmulti" => {
            if let Some(vec_node) = find_param_vector(node) {
                let params_text = node_text(&vec_node, source).unwrap_or("[]");
                format!("({keyword} {name} {params_text})")
            } else {
                format!("({keyword} {name})")
            }
        }
        "defprotocol" | "defrecord" | "deftype" | "ns" => {
            format!("({keyword} {name})")
        }
        "def" => {
            format!("(def {name})")
        }
        _ => format!("({keyword} {name})")
    }
}

/// Extract parameters from defn/defmacro parameter vector.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let keyword = match def_keyword_text(node, source) {
        Some(k) => k,
        None => return Vec::new(),
    };

    if keyword != "defn" && keyword != "defmacro" {
        return Vec::new();
    }

    let vec_node = match find_param_vector(node) {
        Some(v) => v,
        None => return Vec::new(),
    };

    let mut params = Vec::new();
    for i in 0..vec_node.named_child_count() {
        let child = vec_node.named_child(i).unwrap();
        if child.kind() == "sym_lit"
            && let Some(text) = node_text(&child, source) {
                // Skip the & rest parameter marker
                if text == "&" {
                    continue;
                }
                params.push(Parameter {
                    name: text.to_string(),
                    type_annotation: None,
                });
            }
    }
    params
}

/// Post-process: reclassify symbol kinds based on the def keyword.
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    let keyword = def_keyword_text(node, source)?;

    match keyword {
        "defprotocol" => sym.kind = SymbolKind::Interface,
        "defrecord" | "deftype" => sym.kind = SymbolKind::Class,
        "ns" => sym.kind = SymbolKind::Class, // No Module variant; Class is the convention
        "def" => sym.kind = SymbolKind::Constant,
        "defn" | "defmacro" | "defmulti" => sym.kind = SymbolKind::Function,
        _ => {}
    }

    // For ns, use the full namespace name including dots.
    // The @name capture gets sym_name (just the last segment after any dot).
    // We need the full sym_lit text for dotted namespaces like "my.namespace".
    if keyword == "ns"
        && let Some(ns_sym) = second_sym_lit(node)
            && let Some(full_name) = node_text(&ns_sym, source) {
                sym.name = full_name.to_string();
                sym.signature = Some(format!("(ns {full_name})"));
            }

    Some(sym)
}

pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: None,
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: None,
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
    use tree_sitter::{Parser, Query};

    fn extract(source: &str) -> Vec<SymbolInfo> {
        let language: tree_sitter::Language = tree_sitter_clojure_orchard::LANGUAGE.into();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let query_source = include_str!("../queries/clojure.scm");
        let query = Query::new(&language, query_source).unwrap();
        let hooks = hooks();
        query_extract::extract(&mut parser, &query, source, Arc::from("test.clj"), &hooks, true).0
    }

    #[test]
    fn test_defn_function() {
        let syms = extract(r#"(defn greet [name] (str "Hello, " name))"#);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "greet");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(
            syms[0].signature.as_deref(),
            Some("(defn greet [name])")
        );
        let params = syms[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "name");
    }

    #[test]
    fn test_private_defn_skipped() {
        let syms = extract("(defn- private-fn [x] x)");
        assert!(syms.is_empty(), "defn- should be filtered out");
    }

    #[test]
    fn test_def_variable() {
        let syms = extract("(def pi 3.14)");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "pi");
        assert_eq!(syms[0].kind, SymbolKind::Constant);
        assert_eq!(syms[0].signature.as_deref(), Some("(def pi)"));
    }

    #[test]
    fn test_ns_module() {
        let syms = extract("(ns my.namespace)");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "my.namespace");
        assert_eq!(syms[0].kind, SymbolKind::Class);
        assert_eq!(syms[0].signature.as_deref(), Some("(ns my.namespace)"));
    }

    #[test]
    fn test_defprotocol_interface() {
        let syms = extract("(defprotocol Greetable (greet-me [this]))");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Greetable");
        assert_eq!(syms[0].kind, SymbolKind::Interface);
    }

    #[test]
    fn test_defrecord_class() {
        let syms = extract("(defrecord Person [name age])");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Person");
        assert_eq!(syms[0].kind, SymbolKind::Class);
        assert_eq!(
            syms[0].signature.as_deref(),
            Some("(defrecord Person)")
        );
    }

    #[test]
    fn test_defmacro_function() {
        let syms = extract("(defmacro unless [pred & body] `(if (not ~pred) ~@body))");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "unless");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        let params = syms[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "pred");
        assert_eq!(params[1].name, "body");
    }

    #[test]
    fn test_defmulti_function() {
        let syms = extract("(defmulti area :shape)");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "area");
        assert_eq!(syms[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_defmethod_skipped() {
        let syms = extract("(defmethod area :circle [shape] (* Math/PI (:radius shape) (:radius shape)))");
        assert!(syms.is_empty(), "defmethod should be filtered out");
    }

    #[test]
    fn test_comprehensive() {
        let source = r#"
(ns my.namespace)
(defn greet [name] (str "Hello, " name))
(defn- private-fn [x] x)
(def pi 3.14)
(defprotocol Greetable (greet-me [this]))
(defrecord Person [name age])
(defmacro unless [pred & body] `(if (not ~pred) ~@body))
(defmulti area :shape)
(defmethod area :circle [shape] (* Math/PI (:radius shape) (:radius shape)))
"#;
        let syms = extract(source);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        // Should include: my.namespace, greet, pi, Greetable, Person, unless, area
        // Should NOT include: private-fn, defmethod impl
        assert_eq!(syms.len(), 7, "got symbols: {:?}", names);
        assert!(names.contains(&"my.namespace"));
        assert!(names.contains(&"greet"));
        assert!(!names.contains(&"private-fn"));
        assert!(names.contains(&"pi"));
        assert!(names.contains(&"Greetable"));
        assert!(names.contains(&"Person"));
        assert!(names.contains(&"unless"));
        assert!(names.contains(&"area"));
    }
}
