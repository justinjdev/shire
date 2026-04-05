use super::{node_text, LanguageHooks};
use crate::symbols::SymbolInfo;
use tree_sitter::Node;

/// Determine the type label for a TOML pair's value.
fn pair_value_type(pair_node: &Node) -> &'static str {
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

/// Build the signature for a TOML symbol given its cleaned name and definition node.
fn format_signature(name: &str, node: &Node) -> String {
    match node.kind() {
        "table_array_element" => format!("[[{}]]", name),
        "table" => format!("[{}]", name),
        "pair" => {
            let value_type = pair_value_type(node);
            format!("{} = {}", name, value_type)
        }
        _ => name.to_string(),
    }
}

/// Normalize the symbol name (handle dotted/quoted keys) and build the signature.
/// All name cleaning and signature formatting happens here in a single pass.
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    let name_node = find_name_child(node);
    let clean_name = match name_node.as_ref().map(|n| n.kind()) {
        Some("dotted_key") => {
            let mut parts = Vec::new();
            collect_dotted_key_parts(name_node.as_ref().unwrap(), source, &mut parts);
            if parts.is_empty() { None } else { Some(parts.join(".")) }
        }
        Some("quoted_key") => {
            node_text(name_node.as_ref().unwrap(), source)
                .map(|t| strip_quotes(t).to_string())
        }
        _ => None,
    };

    if let Some(name) = clean_name {
        sym.name = name;
    }

    sym.signature = Some(format_signature(&sym.name, node));
    Some(sym)
}

/// Recursively collect leaf key parts from a dotted_key node.
/// dotted_key nodes nest recursively: `a.b.c` → dotted_key(dotted_key(bare_key("a"), bare_key("b")), bare_key("c"))
fn collect_dotted_key_parts(node: &Node, source: &str, parts: &mut Vec<String>) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "dotted_key" => collect_dotted_key_parts(&child, source, parts),
                "bare_key" => {
                    if let Some(text) = node_text(&child, source) {
                        parts.push(text.to_string());
                    }
                }
                "quoted_key" => {
                    if let Some(text) = node_text(&child, source) {
                        parts.push(strip_quotes(text).to_string());
                    }
                }
                _ => {}
            }
        }
    }
}

/// Strip surrounding quotes (double or single) from a TOML key.
fn strip_quotes(s: &str) -> &str {
    if let Some(stripped) = s.strip_prefix('"').and_then(|inner| inner.strip_suffix('"')) {
        return stripped;
    }
    if let Some(stripped) = s.strip_prefix('\'').and_then(|inner| inner.strip_suffix('\'')) {
        return stripped;
    }
    s
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
        extract_file("toml", source, Arc::from("test.toml"), true).0
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

    #[test]
    fn test_deep_dotted_table() {
        let source = r#"[tool.poetry.dependencies]
python = "^3.8"
"#;
        let symbols = extract(source);
        let table = symbols.iter().find(|s| s.kind == SymbolKind::Class).unwrap();
        assert_eq!(table.name, "tool.poetry.dependencies");
        assert_eq!(table.signature.as_deref(), Some("[tool.poetry.dependencies]"));
    }

    #[test]
    fn test_quoted_key_table() {
        let source = r#"["special-table"]
key = "value"
"#;
        let symbols = extract(source);
        let tables: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Class).collect();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "special-table");
        assert_eq!(tables[0].signature.as_deref(), Some("[special-table]"));
    }

    #[test]
    fn test_mixed_dotted_quoted_key() {
        let source = r#"[[servers."beta".config]]
host = "10.0.0.1"
"#;
        let symbols = extract(source);
        let arrays: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Class).collect();
        assert_eq!(arrays.len(), 1);
        assert_eq!(arrays[0].name, "servers.beta.config");
        assert_eq!(arrays[0].signature.as_deref(), Some("[[servers.beta.config]]"));
    }
}
