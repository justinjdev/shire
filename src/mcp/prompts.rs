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
    ]
}

pub fn handle(
    conn: &Connection,
    name: &str,
    args: &HashMap<String, String>,
) -> Result<GetPromptResult, PromptError> {
    match name {
        "explore" => handle_explore(conn, args),
        _ => Err(PromptError::InvalidParams(format!("Unknown prompt: {name}"))),
    }
}

/// Call a prompt handler and extract the markdown text result.
/// Used by MCP tools that expose prompts as callable tools.
pub fn call_prompt(
    conn: &Connection,
    name: &str,
    args: &HashMap<String, String>,
) -> Result<String, PromptError> {
    let result = handle(conn, name, args)?;
    let msg = result
        .messages
        .into_iter()
        .next()
        .ok_or_else(|| PromptError::Internal("Prompt returned no messages".into()))?;
    match msg.content {
        PromptMessageContent::Text { text, .. } => Ok(text),
        _ => Err(PromptError::Internal("Prompt returned non-text content".into())),
    }
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
