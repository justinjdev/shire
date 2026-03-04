use crate::db::queries;
use rmcp::model::{
    GetPromptResult, Prompt, PromptArgument, PromptMessage, PromptMessageContent,
    PromptMessageRole,
};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

pub enum PromptError {
    InvalidParams(String),
    NotFound(String),
    Internal(String),
}

pub fn list() -> Vec<Prompt> {
    vec![
        Prompt::new(
            "explore",
            Some("Semantic codebase exploration — search packages, symbols, and files for a concept and return a structured context map"),
            Some(vec![PromptArgument {
                name: "query".into(),
                description: Some("Concept to explore (e.g. \"authentication\", \"error handling\", \"messaging interfaces\")".into()),
                required: Some(true),
            }]),
        ),
        Prompt::new(
            "explore-package",
            Some("Deep dive into a specific package — metadata, internal dependencies, dependents, public API surface, and file tree"),
            Some(vec![PromptArgument {
                name: "name".into(),
                description: Some("Exact package name".into()),
                required: Some(true),
            }]),
        ),
        Prompt::new(
            "impact-analysis",
            Some("Analyze blast radius — what breaks if this package changes? Shows direct and transitive dependents"),
            Some(vec![PromptArgument {
                name: "name".into(),
                description: Some("Package name to analyze impact for".into()),
                required: Some(true),
            }]),
        ),
    ]
}

pub fn handle(
    conn: &Connection,
    name: &str,
    args: &HashMap<String, String>,
) -> Result<GetPromptResult, PromptError> {
    match name {
        "explore" => handle_explore(conn, args),
        "explore-package" => handle_explore_package(conn, args),
        "impact-analysis" => handle_impact_analysis(conn, args),
        _ => Err(PromptError::InvalidParams(format!("Unknown prompt: {name}"))),
    }
}

/// Call a prompt handler and extract the markdown text result.
/// Used by MCP tools that expose prompts as callable tools.
pub fn call_prompt(
    conn: &Connection,
    name: &str,
    args: &HashMap<String, String>,
) -> Result<String, String> {
    let result = handle(conn, name, args).map_err(|e| match e {
        PromptError::InvalidParams(msg) => msg,
        PromptError::NotFound(msg) => msg,
        PromptError::Internal(msg) => msg,
    })?;
    Ok(result
        .messages
        .into_iter()
        .next()
        .map(|m| match m.content {
            PromptMessageContent::Text { text, .. } => text,
            _ => String::new(),
        })
        .unwrap_or_default())
}

fn require_arg<'a>(args: &'a HashMap<String, String>, key: &str) -> Result<&'a str, PromptError> {
    args.get(key)
        .map(|s| s.as_str())
        .ok_or_else(|| PromptError::InvalidParams(format!("Missing required argument: {key}")))
}

fn handle_explore(conn: &Connection, args: &HashMap<String, String>) -> Result<GetPromptResult, PromptError> {
    let query = require_arg(args, "query")?;

    let packages = queries::search_packages(conn, query, 20).map_err(|e| PromptError::Internal(e.to_string()))?;
    let symbols = queries::search_symbols(conn, query, None, None, 20).map_err(|e| PromptError::Internal(e.to_string()))?;
    let files = queries::search_files(conn, query, None, None).map_err(|e| PromptError::Internal(e.to_string()))?;

    let mut text = format!("# Codebase exploration: \"{query}\"\n\n");

    // Organize symbols by package
    let mut symbols_by_pkg: HashMap<&str, Vec<&queries::SymbolRow>> = HashMap::new();
    for sym in &symbols {
        symbols_by_pkg.entry(&sym.package).or_default().push(sym);
    }

    // Organize files by package
    let mut files_by_pkg: HashMap<Option<&str>, Vec<&queries::FileRow>> = HashMap::new();
    for file in &files {
        files_by_pkg.entry(file.package.as_deref()).or_default().push(file);
    }

    if packages.is_empty() && symbols.is_empty() && files.is_empty() {
        text.push_str("No results found.\n");
    } else {
        // Package matches
        if !packages.is_empty() {
            text.push_str(&format!("## Matching packages ({})\n\n", packages.len()));
            for pkg in &packages {
                text.push_str(&format!("### {} ({})\n", pkg.name, pkg.kind));
                text.push_str(&format!("- **Path:** `{}`\n", pkg.path));
                if let Some(v) = &pkg.version {
                    text.push_str(&format!("- **Version:** {v}\n"));
                }
                if let Some(d) = &pkg.description {
                    text.push_str(&format!("- **Description:** {d}\n"));
                }

                // Symbols in this package
                if let Some(syms) = symbols_by_pkg.get(pkg.name.as_str()) {
                    text.push_str(&format!("\n**Matching symbols ({}):**\n", syms.len()));
                    for sym in syms {
                        let sig = sym.signature.as_deref().unwrap_or(&sym.name);
                        text.push_str(&format!("- `{}` ({}) — `{}:{}`\n", sig, sym.kind, sym.file_path, sym.line));
                    }
                }

                // Files in this package
                if let Some(fls) = files_by_pkg.get(&Some(pkg.name.as_str())) {
                    text.push_str(&format!("\n**Matching files ({}):**\n", fls.len()));
                    for f in fls {
                        text.push_str(&format!("- `{}`\n", f.path));
                    }
                }
                text.push('\n');
            }
        }

        // Symbols not in matched packages
        let matched_pkg_names: HashSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();
        let orphan_symbols: Vec<_> = symbols.iter().filter(|s| !matched_pkg_names.contains(s.package.as_str())).collect();
        if !orphan_symbols.is_empty() {
            text.push_str(&format!("## Additional symbol matches ({})\n\n", orphan_symbols.len()));
            for sym in &orphan_symbols {
                let sig = sym.signature.as_deref().unwrap_or(&sym.name);
                text.push_str(&format!("- `{}` ({}) in **{}** — `{}:{}`\n", sig, sym.kind, sym.package, sym.file_path, sym.line));
            }
            text.push('\n');
        }

        // Files not in matched packages
        let orphan_files: Vec<_> = files.iter().filter(|f| {
            match &f.package {
                Some(pkg) => !matched_pkg_names.contains(pkg.as_str()),
                None => true,
            }
        }).collect();
        if !orphan_files.is_empty() {
            text.push_str(&format!("## Additional file matches ({})\n\n", orphan_files.len()));
            for f in &orphan_files {
                let pkg_label = f.package.as_deref().unwrap_or("(unowned)");
                text.push_str(&format!("- `{}` [{}]\n", f.path, pkg_label));
            }
            text.push('\n');
        }
    }

    Ok(GetPromptResult {
        description: Some(format!("Codebase exploration for \"{query}\"")),
        messages: vec![PromptMessage {
            role: PromptMessageRole::User,
            content: PromptMessageContent::text(text),
        }],
    })
}

fn handle_explore_package(conn: &Connection, args: &HashMap<String, String>) -> Result<GetPromptResult, PromptError> {
    let name = require_arg(args, "name")?;

    let pkg = queries::get_package(conn, name)
        .map_err(|e| PromptError::Internal(e.to_string()))?
        .ok_or_else(|| PromptError::NotFound(format!("Package '{name}' not found")))?;

    let internal_deps = queries::package_dependencies(conn, name, true).map_err(|e| PromptError::Internal(e.to_string()))?;
    let dependents = queries::package_dependents(conn, name).map_err(|e| PromptError::Internal(e.to_string()))?;
    let symbols = queries::get_package_symbols(conn, name, None).map_err(|e| PromptError::Internal(e.to_string()))?;
    let files = queries::list_package_files(conn, name, None).map_err(|e| PromptError::Internal(e.to_string()))?;

    let mut text = format!("# Package: {}\n\n", pkg.name);

    // Metadata
    text.push_str("## Metadata\n\n");
    text.push_str(&format!("- **Kind:** {}\n", pkg.kind));
    text.push_str(&format!("- **Path:** `{}`\n", pkg.path));
    if let Some(v) = &pkg.version {
        text.push_str(&format!("- **Version:** {v}\n"));
    }
    if let Some(d) = &pkg.description {
        text.push_str(&format!("- **Description:** {d}\n"));
    }
    text.push('\n');

    // Internal dependencies
    text.push_str(&format!("## Internal dependencies ({})\n\n", internal_deps.len()));
    if internal_deps.is_empty() {
        text.push_str("None.\n\n");
    } else {
        for dep in &internal_deps {
            let ver = dep.version_req.as_deref().unwrap_or("");
            text.push_str(&format!("- **{}** ({}) {}\n", dep.dependency, dep.dep_kind, ver));
        }
        text.push('\n');
    }

    // Dependents
    let internal_dependents: Vec<_> = dependents.iter().filter(|d| d.is_internal).collect();
    text.push_str(&format!("## Depended on by ({})\n\n", internal_dependents.len()));
    if internal_dependents.is_empty() {
        text.push_str("No internal packages depend on this.\n\n");
    } else {
        for dep in &internal_dependents {
            text.push_str(&format!("- **{}** ({})\n", dep.package, dep.dep_kind));
        }
        text.push('\n');
    }

    // Symbols (public API surface)
    text.push_str(&format!("## Symbols ({})\n\n", symbols.len()));
    if symbols.is_empty() {
        text.push_str("No symbols extracted.\n\n");
    } else {
        // Group by kind
        let mut by_kind: HashMap<&str, Vec<&queries::SymbolRow>> = HashMap::new();
        for sym in &symbols {
            by_kind.entry(&sym.kind).or_default().push(sym);
        }
        let mut kinds: Vec<_> = by_kind.keys().copied().collect();
        kinds.sort();
        for kind in kinds {
            let syms = &by_kind[kind];
            text.push_str(&format!("### {} ({})\n\n", kind, syms.len()));
            for sym in syms {
                let sig = sym.signature.as_deref().unwrap_or(&sym.name);
                text.push_str(&format!("- `{}` — `{}:{}`\n", sig, sym.file_path, sym.line));
            }
            text.push('\n');
        }
    }

    // Files
    text.push_str(&format!("## Files ({})\n\n", files.len()));
    if files.is_empty() {
        text.push_str("No files indexed.\n\n");
    } else {
        for f in &files {
            text.push_str(&format!("- `{}`\n", f.path));
        }
        text.push('\n');
    }

    Ok(GetPromptResult {
        description: Some(format!("Deep dive into package \"{name}\"")),
        messages: vec![PromptMessage {
            role: PromptMessageRole::User,
            content: PromptMessageContent::text(text),
        }],
    })
}

fn handle_impact_analysis(conn: &Connection, args: &HashMap<String, String>) -> Result<GetPromptResult, PromptError> {
    let name = require_arg(args, "name")?;

    // Verify the package exists
    let pkg = queries::get_package(conn, name)
        .map_err(|e| PromptError::Internal(e.to_string()))?
        .ok_or_else(|| PromptError::NotFound(format!("Package '{name}' not found")))?;

    let direct_dependents = queries::package_dependents(conn, name).map_err(|e| PromptError::Internal(e.to_string()))?;
    let reverse_edges = queries::reverse_dependency_graph(conn, name, 10).map_err(|e| PromptError::Internal(e.to_string()))?;

    // Collect all unique transitively affected packages
    let mut all_affected: HashSet<&str> = HashSet::new();
    for edge in &reverse_edges {
        all_affected.insert(&edge.from);
    }

    let direct_names: HashSet<&str> = direct_dependents.iter().map(|d| d.package.as_str()).collect();
    let transitive_only: Vec<&str> = all_affected.iter().filter(|n| !direct_names.contains(**n)).copied().collect();

    let mut text = format!("# Impact analysis: {}\n\n", pkg.name);

    text.push_str(&format!("- **Path:** `{}`\n", pkg.path));
    text.push_str(&format!("- **Kind:** {}\n", pkg.kind));
    if let Some(d) = &pkg.description {
        text.push_str(&format!("- **Description:** {d}\n"));
    }
    text.push('\n');

    // Direct dependents
    text.push_str(&format!("## Direct dependents ({})\n\n", direct_dependents.len()));
    if direct_dependents.is_empty() {
        text.push_str("No packages directly depend on this.\n\n");
    } else {
        for dep in &direct_dependents {
            let internal = if dep.is_internal { "" } else { " (external)" };
            text.push_str(&format!("- **{}** ({}){}\n", dep.package, dep.dep_kind, internal));
        }
        text.push('\n');
    }

    // Transitive-only dependents
    if !transitive_only.is_empty() {
        text.push_str(&format!("## Transitive dependents ({})\n\n", transitive_only.len()));
        text.push_str("These packages don't depend directly but are affected through the dependency chain:\n\n");
        for name in &transitive_only {
            text.push_str(&format!("- **{name}**\n"));
        }
        text.push('\n');
    }

    // Full blast radius
    text.push_str("## Blast radius\n\n");
    text.push_str(&format!("- **Direct:** {}\n", direct_dependents.len()));
    text.push_str(&format!("- **Transitive:** {}\n", transitive_only.len()));
    text.push_str(&format!("- **Total affected:** {}\n", all_affected.len()));

    if !reverse_edges.is_empty() {
        text.push_str("\n## Dependency chain\n\n");
        for edge in &reverse_edges {
            text.push_str(&format!("- {} → {} ({})\n", edge.from, edge.to, edge.dep_kind));
        }
        text.push('\n');
    }

    Ok(GetPromptResult {
        description: Some(format!("Impact analysis for \"{name}\"")),
        messages: vec![PromptMessage {
            role: PromptMessageRole::User,
            content: PromptMessageContent::text(text),
        }],
    })
}

