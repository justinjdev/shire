use crate::db::queries;
use rmcp::model::{
    GetPromptResult, Prompt, PromptArgument, PromptMessage, PromptMessageContent,
    PromptMessageRole,
};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

pub enum PromptError {
    InvalidParams(String),
    #[allow(dead_code)]
    NotFound(String),
    Internal(String),
}

pub fn list() -> Vec<Prompt> {
    vec![
        Prompt::new(
            "explore",
            Some("Semantic codebase exploration — search packages, symbols, files, and documentation for a concept and return a structured context map"),
            Some(vec![PromptArgument {
                name: "query".into(),
                description: Some("Concept to explore (e.g. \"authentication\", \"error handling\", \"messaging interfaces\")".into()),
                required: Some(true),
            }]),
        ),
        Prompt::new(
            "reference_audit",
            Some("Refactor safety audit — traces all references to a symbol, classifies them by kind, follows call chains upward, and summarizes rename/change risk"),
            Some(vec![PromptArgument {
                name: "name".into(),
                description: Some("Symbol name to audit (e.g. \"parse_manifest\", \"UserService\", \"MAX_RETRIES\")".into()),
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
        "reference_audit" => handle_reference_audit(args),
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

fn handle_reference_audit(args: &HashMap<String, String>) -> Result<GetPromptResult, PromptError> {
    let name = require_arg(args, "name")?;

    let text = format!(
r#"# Reference audit: `{name}`

Perform a refactor safety analysis for the symbol `{name}` by following these steps.

## Prerequisite

This prompt relies on the cross-reference index, which is **experimental and
opt-in**. If `symbol_references` returns an empty list on a symbol you know
is used, the index is disabled — add this to `shire.toml` and run
`shire build --force`:

```toml
[symbols]
references_enabled = true
```

Then retry the audit.

## Step 1 — Gather all references

Call `symbol_references` with `name={name}` to retrieve every location where this symbol
is referenced across the codebase.

## Step 2 — Classify by kind

Group the results by the `kind` field and note what each signals:

| Kind     | What it means for refactor safety |
|----------|------------------------------------|
| `call`   | Active invocation — callers will break if the signature or name changes |
| `type`   | Used as a type annotation — renaming requires updating all type sites |
| `import` | Imported by other modules — public API surface, possibly cross-package |
| `impl`   | Implements or extends this symbol — structural coupling, subtypes affected |

Record the counts per kind and the packages that contain them.

## Step 3 — Trace call chains upward

For each unique `enclosing_symbol` returned in the `call` results, call
`symbol_callers` on that enclosing symbol to walk the call chain one level higher.
Repeat for any new enclosing symbols if the chain is shallow (≤ 3 hops).

This reveals whether the blast radius is contained within one package or fans out
across many.

## Step 4 — Identify cross-package references

Cross-package references are rows where the `package` field differs from the package
that defines `{name}`. These are the highest-risk references because they cross module
boundaries and may not be visible to a local search.

List each cross-package reference with: caller package, file, line, kind.

## Step 5 — Summarize

Produce a concise safety summary covering:

- **Direct call sites:** count and packages
- **External callers:** packages outside the defining package that call this symbol
- **Implementers / subtypes:** count of `impl`-kind references
- **Type sites:** count of `type`-kind references
- **Overall risk:** Low / Medium / High with a one-sentence rationale
  - Low — all refs are internal to one package, no cross-package callers
  - Medium — cross-package refs exist but are limited in number/depth
  - High — widely imported public API with deep or broad call chains

> **Known limitation:** Matching is by name only. If two different symbols share the
> same name across packages, refs to both will appear. Use the `package` field to
> distinguish them and discard false positives before drawing conclusions.
"#
    );

    Ok(GetPromptResult {
        description: Some(format!("Reference audit for symbol \"{name}\"")),
        messages: vec![PromptMessage {
            role: PromptMessageRole::User,
            content: PromptMessageContent::text(text),
        }],
    })
}

fn handle_explore(conn: &Connection, args: &HashMap<String, String>) -> Result<GetPromptResult, PromptError> {
    let query = require_arg(args, "query")?;

    let packages = queries::search_packages(conn, query, 20).map_err(|e| PromptError::Internal(e.to_string()))?;
    let symbols = queries::search_symbols(conn, query, None, None, 20).map_err(|e| PromptError::Internal(e.to_string()))?;
    let files = queries::search_files(conn, query, None, None).map_err(|e| PromptError::Internal(e.to_string()))?;
    let docs = queries::search_docs(conn, query, None, 10).map_err(|e| PromptError::Internal(e.to_string()))?;

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

    // Organize docs by package
    let mut docs_by_pkg: HashMap<Option<&str>, Vec<&queries::DocRow>> = HashMap::new();
    for doc in &docs {
        docs_by_pkg.entry(doc.package.as_deref()).or_default().push(doc);
    }

    if packages.is_empty() && symbols.is_empty() && files.is_empty() && docs.is_empty() {
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

                // Docs in this package
                if let Some(ds) = docs_by_pkg.get(&Some(pkg.name.as_str())) {
                    text.push_str(&format!("\n**Matching docs ({}):**\n", ds.len()));
                    for d in ds {
                        let title = d.title.as_deref().unwrap_or("(untitled)");
                        text.push_str(&format!("- **{}** — `{}`\n  {}\n", title, d.path, d.snippet));
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

        // Docs not in matched packages
        let orphan_docs: Vec<_> = docs.iter().filter(|d| {
            match &d.package {
                Some(pkg) => !matched_pkg_names.contains(pkg.as_str()),
                None => true,
            }
        }).collect();
        if !orphan_docs.is_empty() {
            text.push_str(&format!("## Documentation matches ({})\n\n", orphan_docs.len()));
            for d in &orphan_docs {
                let title = d.title.as_deref().unwrap_or("(untitled)");
                let pkg_label = d.package.as_deref().unwrap_or("(unowned)");
                text.push_str(&format!("- **{}** — `{}` [{}]\n  {}\n", title, d.path, pkg_label, d.snippet));
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
