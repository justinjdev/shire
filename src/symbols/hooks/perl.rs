use super::{find_ancestor, node_text, LanguageHooks, ReferenceHooks, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Skip private subs (starting with _).
fn is_visible(node: &Node, source: &str) -> bool {
    if node.kind() == "subroutine_declaration_statement" {
        if let Some(name_node) = node.child_by_field_name("name")
            && let Some(name) = node_text(&name_node, source) {
                return !name.starts_with('_');
            }
        // No field name — find first identifier child
        for i in 0..node.child_count() {
            let child = node.child(i).unwrap();
            if child.kind() == "bareword"
                && let Some(name) = node_text(&child, source) {
                    return !name.starts_with('_');
                }
        }
    }
    true
}

/// Resolve package name as parent for subs defined inside a package.
/// Walk backwards through siblings from the subroutine_declaration_statement to find the
/// nearest preceding package_statement.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    if node.kind() != "subroutine_declaration_statement" {
        return None;
    }

    // First check if we're inside a package_statement ancestor
    if let Some(pkg) = find_ancestor(node, "package_statement")
        && let Some(name_node) = find_package_name(&pkg) {
            return node_text(&name_node, source).map(|s| s.to_string());
        }

    // Otherwise scan preceding siblings for a package_statement
    let mut sibling = node.prev_named_sibling();
    while let Some(sib) = sibling {
        if sib.kind() == "package_statement"
            && let Some(name_node) = find_package_name(&sib) {
                return node_text(&name_node, source).map(|s| s.to_string());
            }
        sibling = sib.prev_named_sibling();
    }
    None
}

/// Find the package name node from a package_statement.
fn find_package_name<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    node.child_by_field_name("name")
}

/// Build signature for Perl symbols.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Class => format!("package {name}"),
        _ => {
            // Check for parent package — qualifies the sub name
            if let Some(parent) = resolve_parent(node, source) {
                format!("sub {parent}::{name}")
            } else {
                format!("sub {name}")
            }
        }
    }
}

/// Post-process: reclassify subs inside a package as methods.
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    if sym.kind == SymbolKind::Function
        && resolve_parent(node, source).is_some() {
            sym.kind = SymbolKind::Method;
        }
    Some(sym)
}

/// Return Perl language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: None,
        extract_return_type: None,
        post_process: Some(post_process),
        reference_hooks: Some(ReferenceHooks {
            enclosing_ancestors: &[
                "subroutine_declaration_statement",
                "package_statement",
            ],
            reference_stoplist: &[
                "strict", "warnings", "utf8", "feature", "parent", "base",
                "print", "say", "die", "warn", "use", "require",
                "my", "our", "local", "sub", "return", "if", "unless",
                "undef", "defined",
            ],
        }),
    }
}
