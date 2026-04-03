use super::{find_ancestor, node_text, LanguageHooks, SymbolKind};
use crate::symbols::SymbolInfo;
use tree_sitter::Node;

/// Build a readable signature for TOML symbols.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Class => {
            // Class comes from definition.module — could be table or table_array_element
            if node.kind() == "table_array_element" {
                format!("[[{}]]", name)
            } else {
                format!("[{}]", name)
            }
        }
        SymbolKind::Constant => {
            // Top-level key-value pair — show key = <value_type>
            if let Some(pair_node) = if node.kind() == "pair" {
                Some(*node)
            } else {
                find_ancestor(node, "pair")
            } {
                let value_type = pair_value_type(&pair_node, source);
                format!("{} = {}", name, value_type)
            } else {
                format!("{} = ...", name)
            }
        }
        _ => node_text(node, source)
            .unwrap_or(name)
            .trim()
            .to_string(),
    }
}

/// Determine the type label for a TOML pair's value.
fn pair_value_type(pair_node: &Node, _source: &str) -> &'static str {
    // The value is the last named child of the pair (after `=`)
    for i in (0..pair_node.child_count()).rev() {
        if let Some(child) = pair_node.child(i) {
            if child.is_named() {
                return match child.kind() {
                    "string" | "literal_string" => "<string>",
                    "integer" => "<integer>",
                    "float" => "<float>",
                    "boolean" => "<boolean>",
                    "local_date" | "local_time" | "local_date_time" | "offset_date_time" => {
                        "<datetime>"
                    }
                    "array" => "<array>",
                    "inline_table" => "<table>",
                    _ => "<value>",
                };
            }
        }
    }
    "<value>"
}

/// Post-process to fix names for dotted_key and quoted_key nodes.
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    // The @name capture on a dotted_key gives the full node text (e.g., "database.auth")
    // but we want to ensure it's clean. For quoted_key, strip the quotes.
    let name_node = find_name_child(node);
    if let Some(name_node) = name_node {
        match name_node.kind() {
            "dotted_key" => {
                // Extract all bare_key children and join with "."
                let mut parts = Vec::new();
                for i in 0..name_node.child_count() {
                    if let Some(child) = name_node.child(i) {
                        if child.kind() == "bare_key" || child.kind() == "quoted_key" {
                            if let Some(text) = node_text(&child, source) {
                                let text = text.trim_matches('"').trim_matches('\'');
                                parts.push(text.to_string());
                            }
                        }
                    }
                }
                if !parts.is_empty() {
                    let dotted_name = parts.join(".");
                    // Update signature with the clean dotted name
                    sym.signature = Some(if node.kind() == "table_array_element" {
                        format!("[[{}]]", dotted_name)
                    } else if node.kind() == "table" {
                        format!("[{}]", dotted_name)
                    } else {
                        // constant (pair) with dotted key
                        let pair_node = find_ancestor(&name_node, "pair");
                        if let Some(pair_node) = pair_node {
                            let value_type = pair_value_type(&pair_node, source);
                            format!("{} = {}", dotted_name, value_type)
                        } else {
                            format!("{} = ...", dotted_name)
                        }
                    });
                    sym.name = dotted_name;
                }
            }
            "quoted_key" => {
                if let Some(text) = node_text(&name_node, source) {
                    let clean = text.trim_matches('"').trim_matches('\'');
                    sym.name = clean.to_string();
                    // Rebuild signature with clean name
                    if sym.kind == SymbolKind::Constant {
                        let pair_node = find_ancestor(&name_node, "pair");
                        if let Some(pair_node) = pair_node {
                            let value_type = pair_value_type(&pair_node, source);
                            sym.signature = Some(format!("{} = {}", clean, value_type));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(sym)
}

/// Find the @name child node within a definition node.
fn find_name_child<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "bare_key" | "dotted_key" | "quoted_key" => return Some(child),
                _ => {}
            }
        }
    }
    None
}

/// Return TOML language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        build_signature: Some(build_signature),
        post_process: Some(post_process),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::symbols::registry::extract_file;
    use crate::symbols::SymbolKind;

    fn extract(source: &str) -> Vec<crate::symbols::SymbolInfo> {
        extract_file("toml", source, Arc::from("test.toml"))
    }

    #[test]
    fn test_simple_table() {
        let source = r#"[database]
server = "192.168.1.1"
port = 5432
"#;
        let symbols = extract(source);
        let tables: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Class).collect();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "database");
        assert_eq!(
            tables[0].signature.as_deref(),
            Some("[database]")
        );
    }

    #[test]
    fn test_dotted_table() {
        let source = r#"[database.auth]
user = "admin"
"#;
        let symbols = extract(source);
        let tables: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Class).collect();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "database.auth");
        assert_eq!(
            tables[0].signature.as_deref(),
            Some("[database.auth]")
        );
    }

    #[test]
    fn test_array_of_tables() {
        let source = r#"[[products]]
name = "Hammer"

[[products]]
name = "Nail"
"#;
        let symbols = extract(source);
        let arrays: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Class).collect();
        // Both [[products]] entries should produce symbols
        assert_eq!(arrays.len(), 2);
        assert_eq!(arrays[0].name, "products");
        assert_eq!(
            arrays[0].signature.as_deref(),
            Some("[[products]]")
        );
    }

    #[test]
    fn test_top_level_keys() {
        let source = r#"title = "TOML Example"
debug = true
port = 8080
"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 3);
        assert!(symbols.iter().all(|s| s.kind == SymbolKind::Constant));
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"title"));
        assert!(names.contains(&"debug"));
        assert!(names.contains(&"port"));
    }

    #[test]
    fn test_quoted_key() {
        let source = r#""quoted-key" = "value"
"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "quoted-key");
        assert_eq!(symbols[0].kind, SymbolKind::Constant);
    }

    #[test]
    fn test_signature_value_types() {
        let source = r#"name = "text"
count = 42
ratio = 3.14
enabled = true
"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 4);

        let find = |name: &str| symbols.iter().find(|s| s.name == name).unwrap();
        assert_eq!(find("name").signature.as_deref(), Some("name = <string>"));
        assert_eq!(find("count").signature.as_deref(), Some("count = <integer>"));
        assert_eq!(find("ratio").signature.as_deref(), Some("ratio = <float>"));
        assert_eq!(
            find("enabled").signature.as_deref(),
            Some("enabled = <boolean>")
        );
    }

    #[test]
    fn test_mixed_document() {
        let source = r#"title = "My Project"

[database]
server = "localhost"

[[plugins]]
name = "auth"

[[plugins]]
name = "logging"

[server.http]
port = 8080
"#;
        let symbols = extract(source);

        // Top-level key: title
        let top_level: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Constant)
            .collect();
        assert_eq!(top_level.len(), 1);
        assert_eq!(top_level[0].name, "title");

        // Tables: database, server.http
        let tables: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class && s.signature.as_deref().map_or(false, |s| s.starts_with('[')))
            .filter(|s| !s.signature.as_deref().unwrap_or("").starts_with("[["))
            .collect();
        assert_eq!(tables.len(), 2);
        let table_names: Vec<&str> = tables.iter().map(|s| s.name.as_str()).collect();
        assert!(table_names.contains(&"database"));
        assert!(table_names.contains(&"server.http"));

        // Array of tables: plugins (2 entries)
        let arrays: Vec<_> = symbols
            .iter()
            .filter(|s| {
                s.kind == SymbolKind::Class
                    && s.signature
                        .as_deref()
                        .map_or(false, |sig| sig.starts_with("[["))
            })
            .collect();
        assert_eq!(arrays.len(), 2);
        assert!(arrays.iter().all(|s| s.name == "plugins"));
    }

    #[test]
    fn test_no_nested_pairs_extracted() {
        // Only top-level pairs should be extracted, not pairs inside tables
        let source = r#"title = "Top Level"

[database]
server = "localhost"
port = 5432
"#;
        let symbols = extract(source);
        let constants: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Constant)
            .collect();
        // Only "title" should be a constant, not "server" or "port"
        assert_eq!(constants.len(), 1);
        assert_eq!(constants[0].name, "title");
    }

    #[test]
    fn test_line_numbers() {
        let source = r#"title = "TOML"

[database]
server = "localhost"
"#;
        let symbols = extract(source);
        let title = symbols.iter().find(|s| s.name == "title").unwrap();
        assert_eq!(title.line, 1);
        let db = symbols.iter().find(|s| s.name == "database").unwrap();
        assert_eq!(db.line, 3);
    }
}
