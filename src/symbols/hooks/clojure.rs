use super::{LanguageHooks, Parameter, SymbolInfo, SymbolKind, Visibility, node_text};
use tree_sitter::Node;

/// Def keywords whose form defines a symbol. `defmethod` is deliberately
/// absent: it adds an implementation to an existing `defmulti` rather than
/// defining a new name.
const DEF_KEYWORDS: &[&str] = &[
    "defn",
    "defn-",
    "def",
    "defmacro",
    "defprotocol",
    "defrecord",
    "deftype",
    "defmulti",
    "ns",
];

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

/// The query matches every list form whose first two elements are symbols —
/// `(println x)` as much as `(defn f ...)`. Only def forms are definitions.
fn is_definition(node: &Node, source: &str) -> bool {
    node.kind() == "list_lit"
        && def_keyword_text(node, source).is_some_and(|k| DEF_KEYWORDS.contains(&k))
}

/// Clojure visibility: `defn-` and `^:private` (or `^{:private true}`)
/// metadata on the name make a var private to its namespace.
fn visibility(node: &Node, source: &str) -> Visibility {
    if def_keyword_text(node, source) == Some("defn-") {
        return Visibility::Private;
    }
    let private_meta = second_sym_lit(node).is_some_and(|name| {
        (0..name.named_child_count())
            .filter_map(|i| name.named_child(i))
            .filter(|c| c.kind().ends_with("meta_lit"))
            .any(|meta| {
                node_text(&meta, source).is_some_and(|t| {
                    let t: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
                    t == "^:private" || t.contains(":private true")
                })
            })
    });
    if private_meta {
        Visibility::Private
    } else {
        Visibility::Public
    }
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
        "defn" | "defn-" | "defmacro" | "defmulti" => {
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
        _ => format!("({keyword} {name})"),
    }
}

/// Extract parameters from defn/defmacro parameter vector.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let keyword = match def_keyword_text(node, source) {
        Some(k) => k,
        None => return Vec::new(),
    };

    if !matches!(keyword, "defn" | "defn-" | "defmacro") {
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
            && let Some(text) = node_text(&child, source)
        {
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
        "defn" | "defn-" | "defmacro" | "defmulti" => sym.kind = SymbolKind::Function,
        _ => {}
    }

    // For ns, use the full namespace name including dots.
    // The @name capture gets sym_name (just the last segment after any dot).
    // We need the full sym_lit text for dotted namespaces like "my.namespace".
    if keyword == "ns"
        && let Some(ns_sym) = second_sym_lit(node)
        && let Some(full_name) = node_text(&ns_sym, source)
    {
        sym.name = full_name.to_string();
        sym.signature = Some(format!("(ns {full_name})"));
    }

    // Metadata on the name (`(def ^:private x ...)`) lives inside the name's
    // `sym_lit`, so the captured text is `^:private x`. Strip it: the symbol
    // is `x`, and the metadata has already been read by `visibility`.
    if let Some(name_sym) = second_sym_lit(node)
        && let Some(meta_end) = (0..name_sym.named_child_count())
            .filter_map(|i| name_sym.named_child(i))
            .filter(|c| c.kind().ends_with("meta_lit"))
            .map(|c| c.end_byte())
            .max()
        && let Some(bare) = source.get(meta_end..name_sym.end_byte())
    {
        let bare = bare.trim();
        sym.name = bare.to_string();
        sym.signature = Some(build_signature(node, source, bare, sym.kind));
    }

    Some(sym)
}

pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_definition: Some(is_definition),
        visibility: Some(visibility),
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
        query_extract::extract(
            &mut parser,
            &query,
            source,
            Arc::from("test.clj"),
            &hooks,
            true,
            0,
        )
        .0
    }

    #[test]
    fn test_defn_function() {
        let syms = extract(r#"(defn greet [name] (str "Hello, " name))"#);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "greet");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].signature.as_deref(), Some("(defn greet [name])"));
        let params = syms[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "name");
    }

    #[test]
    fn test_private_defn_is_private() {
        let syms = extract("(defn- private-fn [x] x)");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "private-fn");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].visibility, Visibility::Private);
        assert_eq!(syms[0].signature.as_deref(), Some("(defn- private-fn [x])"));
        let params = syms[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_private_metadata_is_private() {
        let syms = extract(
            "(def ^:private secret 42)\n(defn ^{:private true} helper [] 1)\n(defn open [] 1)",
        );
        let vis_of = |name: &str| {
            syms.iter()
                .find(|s| s.name == name)
                .unwrap_or_else(|| panic!("no {name} in {syms:?}"))
                .visibility
        };
        assert_eq!(vis_of("secret"), Visibility::Private);
        assert_eq!(vis_of("helper"), Visibility::Private);
        assert_eq!(vis_of("open"), Visibility::Public);
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
        assert_eq!(syms[0].signature.as_deref(), Some("(defrecord Person)"));
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
        let syms =
            extract("(defmethod area :circle [shape] (* Math/PI (:radius shape) (:radius shape)))");
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
        // Should include: my.namespace, greet, private-fn, pi, Greetable,
        // Person, unless, area. Should NOT include the defmethod impl.
        assert_eq!(syms.len(), 8, "got symbols: {:?}", names);
        assert!(names.contains(&"my.namespace"));
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"private-fn"));
        assert!(names.contains(&"pi"));
        assert!(names.contains(&"Greetable"));
        assert!(names.contains(&"Person"));
        assert!(names.contains(&"unless"));
        assert!(names.contains(&"area"));
    }
}
