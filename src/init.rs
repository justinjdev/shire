use anyhow::{Context, Result};
use console::style;
use dialoguer::{Confirm, Input, Select};
use serde_json::{Map, Value, json};
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::config::default_exclude;

fn print_created(msg: &str) {
    eprintln!("  {} {msg}", style("✓").green().bold());
}

fn print_skipped(msg: &str) {
    eprintln!("  {} {msg}", style("–").dim());
}

fn print_header(msg: &str) {
    eprintln!("\n{}", style(msg).cyan().bold());
}

const CLAUDE_MD_LINE: &str = "When searching code, use Shire MCP tools (search_symbols, search_files, explore) instead of Grep/Glob.";

const RULES_CONTENT: &str = r#"# Shire — codebase search index

When you need to find code, use Shire MCP tools first:

- **Find functions/classes/types:** `search_symbols` (not Grep)
- **Find files:** `search_files` (not Glob)
- **Explore a concept:** `explore` (not Grep)
- **Check dependencies:** `package_dependents` / `package_dependencies`

Fall back to Grep only for literal strings or content inside function bodies.
"#;

/// Additional rule-file section surfaced only when the user opts into the
/// experimental cross-reference index, so the generated guidance actually
/// mentions the MCP tools it enables.
const RULES_CONTENT_REFS: &str = r#"
## Cross-reference index (experimental)

This repo has `symbols.references_enabled = true`, which populates the
`symbol_refs` table. Use these tools before Grep-ing for usage sites:

- **Find where a symbol is used:** `symbol_references` (not Grep)
- **Find who calls a function/method:** `symbol_callers`
- **Find what a function calls:** `symbol_callees`
- **Audit a rename/refactor:** the `reference_audit` prompt

Ref tools match by exact name — same-name symbols across packages are
merged. Pass the optional `package` filter when you know the owning package.
"#;

/// Extract the top-level directory component from a relative db_path.
/// Returns `None` for absolute paths or paths starting with `~`.
fn gitignore_dir_from_db_path(db_path: &str) -> Option<String> {
    let path = Path::new(db_path);
    if path.is_absolute() || db_path.starts_with('~') {
        return None;
    }
    let parent = path.parent()?;
    let mut parts: Vec<String> = Vec::new();
    for component in parent.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(s) => parts.push(s.to_str()?.to_string()),
            _ => return None,
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// Escape special characters for TOML string values.
fn escape_toml_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

pub struct InitOptions {
    pub use_hook: bool,
    pub db_path: String,
    pub extra_excludes: Vec<String>,
    /// When true, populate the symbol_refs table for `symbol_references`,
    /// `symbol_callers`, and `symbol_callees` MCP tools. EXPERIMENTAL.
    pub refs_enabled: bool,
    pub generate_rules: bool,
    /// When true, append Shire guidance to ~/.claude/CLAUDE.md.
    pub patch_claude_md: bool,
    /// When true, add the db directory to .gitignore.
    pub gitignore_db_dir: bool,
    /// When true, skip interactive prompts for existing files.
    pub non_interactive: bool,
}

impl InitOptions {
    pub fn default_local() -> Self {
        Self {
            use_hook: true,
            db_path: ".shire/index.db".into(),
            extra_excludes: Vec::new(),
            refs_enabled: false,
            generate_rules: true,
            patch_claude_md: false,
            gitignore_db_dir: true,
            non_interactive: true,
        }
    }

    pub fn default_global() -> Self {
        Self {
            use_hook: true,
            db_path: "~/.claude/shire/{repo}/{worktree}/index.db".into(),
            extra_excludes: Vec::new(),
            refs_enabled: false,
            generate_rules: true,
            patch_claude_md: false,
            gitignore_db_dir: false,
            non_interactive: true,
        }
    }
}

fn prompt_options(global: bool, no_hook_flag: bool) -> Result<InitOptions> {
    let defaults = if global {
        InitOptions::default_global()
    } else {
        InitOptions::default_local()
    };

    // 1. Rebuild strategy
    let use_hook = if no_hook_flag {
        false
    } else {
        let items = &["PostToolUse hook (recommended)", "On-demand (serve --root)"];
        let selection = Select::new()
            .with_prompt("Rebuild strategy")
            .items(items)
            .default(0)
            .interact()?;
        selection == 0
    };

    // 2. Database path
    let db_path: String = Input::new()
        .with_prompt("Database path")
        .default(defaults.db_path.clone())
        .interact_text()?;

    // 3. Gitignore the db directory? (local only — global init has no project .gitignore)
    let gitignore_db_dir = if !global {
        if let Some(dir) = gitignore_dir_from_db_path(&db_path) {
            Confirm::new()
                .with_prompt(format!("Add `{dir}` to .gitignore?"))
                .default(true)
                .interact()?
        } else {
            false
        }
    } else {
        false
    };

    // 4. Additional exclude directories
    let extra_input: String = Input::new()
        .with_prompt("Additional exclude directories (comma-separated, or empty)")
        .default(String::new())
        .allow_empty(true)
        .interact_text()?;
    let extra_excludes: Vec<String> = extra_input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // 5. Enable cross-reference index (experimental)
    let refs_enabled = Confirm::new()
        .with_prompt(
            "Enable cross-reference index (experimental)? Adds symbol_references/callers/callees MCP tools. DB grows substantially (roughly 30%-150% depending on language mix)",
        )
        .default(false)
        .interact()?;

    // 6. Generate .claude/rules/shire.md
    let generate_rules = Confirm::new()
        .with_prompt("Generate .claude/rules/shire.md with tool usage guidance?")
        .default(true)
        .interact()?;

    // 7. Add Shire guidance to ~/.claude/CLAUDE.md
    let patch_claude_md = Confirm::new()
        .with_prompt("Add Shire search guidance to ~/.claude/CLAUDE.md?")
        .default(true)
        .interact()?;

    Ok(InitOptions {
        use_hook,
        db_path,
        extra_excludes,
        refs_enabled,
        generate_rules,
        patch_claude_md,
        gitignore_db_dir,
        non_interactive: false,
    })
}

pub fn generate_config_toml(opts: &InitOptions, global: bool) -> String {
    let mut lines = Vec::new();

    if global {
        lines.push("# Shire global configuration — shared across all repositories".into());
        lines.push(
            "# {repo} = repository name, {worktree} = worktree name (\"_primary\" for primary)"
                .into(),
        );
    } else {
        lines.push("# Shire configuration".into());
    }
    lines.push(String::new());

    lines.push(format!(
        "db_path = \"{}\"",
        escape_toml_string(&opts.db_path)
    ));
    lines.push(String::new());

    if !opts.extra_excludes.is_empty() {
        let mut all_excludes = default_exclude();
        for ex in &opts.extra_excludes {
            if !all_excludes.contains(ex) {
                all_excludes.push(ex.clone());
            }
        }
        lines.push("[discovery]".into());
        let quoted: Vec<String> = all_excludes
            .iter()
            .map(|e| format!("\"{}\"", escape_toml_string(e)))
            .collect();
        lines.push(format!("exclude = [{}]", quoted.join(", ")));
        lines.push(String::new());
    }

    if opts.refs_enabled {
        lines.push("[symbols]".into());
        lines.push("# Cross-reference index (experimental) — powers the symbol_references,".into());
        lines.push("# symbol_callers, and symbol_callees MCP tools. DB grows substantially".into());
        lines.push("# (roughly 30% on TS/JS repos to 150% on Go-heavy repos); set to".into());
        lines.push("# false to opt out.".into());
        lines.push("references_enabled = true".into());
        lines.push(String::new());
    }

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub fn run_init(root: &Path, no_hook: bool, yes: bool) -> Result<()> {
    print_header("Shire — codebase search index");

    // In interactive mode, ask local vs global first
    if !yes && std::io::stdin().is_terminal() {
        let items = &["Local (this project only)", "Global (all projects)"];
        let selection = Select::new()
            .with_prompt("Install scope")
            .items(items)
            .default(0)
            .interact()?;
        if selection == 1 {
            return run_init_global(no_hook, false);
        }
    }

    let opts = if yes || !std::io::stdin().is_terminal() {
        let mut defaults = InitOptions::default_local();
        if no_hook {
            defaults.use_hook = false;
        }
        defaults
    } else {
        prompt_options(false, no_hook)?
    };

    // 1. Create or update shire.toml
    let config_path = root.join("shire.toml");
    let config_exists = config_path.exists();
    let should_write = if config_exists {
        if opts.non_interactive {
            print_skipped("shire.toml already exists, skipping");
            false
        } else {
            Confirm::new()
                .with_prompt("shire.toml already exists. Overwrite with new settings?")
                .default(true)
                .interact()?
        }
    } else {
        true
    };
    if should_write {
        let content = generate_config_toml(&opts, false);
        fs::write(&config_path, content)
            .with_context(|| format!("Failed to write {}", config_path.display()))?;
        print_created(&format!(
            "{} {}",
            if config_exists { "Updated" } else { "Created" },
            config_path.display()
        ));
    }

    // 2. Write .mcp.json for MCP server config
    let serve_args = if opts.use_hook {
        json!(["serve"])
    } else {
        json!(["serve", "--root", "."])
    };
    write_mcp_json(root, serve_args, "shire init")?;

    // 3. Write hooks to .claude/settings.json (only in hook mode)
    if opts.use_hook {
        let claude_dir = root.join(".claude");
        fs::create_dir_all(&claude_dir)
            .with_context(|| format!("Failed to create directory {}", claude_dir.display()))?;
        let settings_path = claude_dir.join("settings.json");
        patch_claude_hooks(&settings_path, ".claude/settings.json", "shire init")?;
    }

    // 4. Write .claude/rules/shire.md
    if opts.generate_rules {
        let rules_dir = root.join(".claude/rules");
        write_rules_file(&rules_dir, ".claude/rules/shire.md", opts.refs_enabled)?;
    }

    // 5. Append Shire guidance to ~/.claude/CLAUDE.md
    if opts.patch_claude_md {
        ensure_claude_md_line()?;
    }

    // 6. Ensure the db directory is in .gitignore (only when config was actually written)
    if should_write
        && opts.gitignore_db_dir
        && let Some(dir) = gitignore_dir_from_db_path(&opts.db_path)
    {
        ensure_gitignore(root, &dir)?;
    }

    if opts.use_hook {
        eprintln!(
            "\n  Next: run {} in this repo to create the index.",
            style("shire build").green().bold()
        );
    } else {
        eprintln!(
            "\n  On-demand reindexing enabled. The MCP server will rebuild the index automatically when needed."
        );
        print_skipped("No PostToolUse hook installed.");
    }
    Ok(())
}

fn home_dir() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .context("HOME environment variable not set")
}

pub fn run_init_global(no_hook: bool, yes: bool) -> Result<()> {
    let claude_dir = home_dir()?.join(".claude");
    let opts = if yes || !std::io::stdin().is_terminal() {
        let mut defaults = InitOptions::default_global();
        if no_hook {
            defaults.use_hook = false;
        }
        defaults
    } else {
        prompt_options(true, no_hook)?
    };
    run_init_global_in(&claude_dir, &opts)
}

fn run_init_global_in(claude_dir: &Path, opts: &InitOptions) -> Result<()> {
    fs::create_dir_all(claude_dir)
        .with_context(|| format!("Failed to create directory {}", claude_dir.display()))?;

    // 1. Create or update shire.toml
    let config_path = claude_dir.join("shire.toml");
    let config_exists = config_path.exists();
    let should_write = if config_exists {
        if opts.non_interactive {
            print_skipped("~/.claude/shire.toml already exists, skipping");
            false
        } else {
            Confirm::new()
                .with_prompt("~/.claude/shire.toml already exists. Overwrite with new settings?")
                .default(true)
                .interact()?
        }
    } else {
        true
    };
    if should_write {
        let content = generate_config_toml(opts, true);
        fs::write(&config_path, content)
            .with_context(|| format!("Failed to write {}", config_path.display()))?;
        print_created(&format!(
            "{} ~/.claude/shire.toml",
            if config_exists { "Updated" } else { "Created" }
        ));
    }

    // 2. Write MCP server to ~/.claude.json (user-scoped MCP config)
    let home = claude_dir
        .parent()
        .context("Cannot determine parent directory of ~/.claude")?;
    let claude_json_path = home.join(".claude.json");
    let serve_args = if opts.use_hook {
        json!(["serve"])
    } else {
        json!(["serve", "--root", "."])
    };
    patch_claude_json(&claude_json_path, serve_args, "shire init --global")
        .context("Failed to configure ~/.claude.json. ~/.claude/shire.toml was created successfully. Fix the issue above and re-run `shire init --global`.")?;

    // 3. Write hooks to ~/.claude/settings.json (only in hook mode)
    if opts.use_hook {
        let settings_path = claude_dir.join("settings.json");
        patch_claude_hooks(
            &settings_path,
            "~/.claude/settings.json",
            "shire init --global",
        )?;
    }

    // 4. Write ~/.claude/rules/shire.md
    if opts.generate_rules {
        let rules_dir = claude_dir.join("rules");
        write_rules_file(&rules_dir, "~/.claude/rules/shire.md", opts.refs_enabled)?;
    }

    // 5. Append Shire guidance to ~/.claude/CLAUDE.md
    if opts.patch_claude_md {
        ensure_claude_md_line()?;
    }

    if opts.use_hook {
        eprintln!(
            "\n  Next: run {} in each repo you want to index.",
            style("shire build").green().bold()
        );
    } else {
        eprintln!(
            "\n  On-demand reindexing enabled globally. The MCP server will rebuild the index automatically when needed."
        );
        print_skipped("No PostToolUse hook installed.");
    }
    Ok(())
}

/// Write MCP server config to .mcp.json at the project root.
fn write_mcp_json(root: &Path, serve_args: Value, reinit_cmd: &str) -> Result<()> {
    let mcp_path = root.join(".mcp.json");
    let mut mcp: Map<String, Value> = if mcp_path.exists() {
        let content = fs::read_to_string(&mcp_path)
            .with_context(|| format!("Failed to read {}", mcp_path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", mcp_path.display()))?
    } else {
        Map::new()
    };

    let servers = mcp.entry("mcpServers").or_insert_with(|| json!({}));
    if let Some(servers_obj) = servers.as_object_mut() {
        if servers_obj.contains_key("shire") {
            print_skipped("mcpServers.shire already configured in .mcp.json");
            return Ok(());
        }
        servers_obj.insert(
            "shire".into(),
            json!({
                "command": "shire",
                "args": serve_args,
            }),
        );
    } else {
        anyhow::bail!(
            ".mcp.json has 'mcpServers' as a non-object type. \
             Please fix it manually or delete the key and re-run `{reinit_cmd}`."
        );
    }

    let output = serde_json::to_string_pretty(&Value::Object(mcp))
        .context("Failed to serialize .mcp.json")?;
    let tmp_path = mcp_path.with_extension("json.tmp");
    fs::write(&tmp_path, format!("{output}\n"))
        .with_context(|| format!("Failed to write {}", tmp_path.display()))?;
    if let Err(e) = fs::rename(&tmp_path, &mcp_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e).with_context(|| {
            format!(
                "Failed to rename {} to {}",
                tmp_path.display(),
                mcp_path.display()
            )
        });
    }
    print_created("Added mcpServers.shire to .mcp.json");
    Ok(())
}

/// Patch a Claude Code settings JSON file to add hooks.PostToolUse only.
fn patch_claude_hooks(settings_path: &Path, display_path: &str, reinit_cmd: &str) -> Result<()> {
    let mut settings: Map<String, Value> = if settings_path.exists() {
        let content = fs::read_to_string(settings_path)
            .with_context(|| format!("Failed to read {}", settings_path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", settings_path.display()))?
    } else {
        Map::new()
    };

    let hooks = settings.entry("hooks").or_insert_with(|| json!({}));
    if let Some(hooks_obj) = hooks.as_object_mut() {
        let has_shire_hook = hooks_obj
            .get("PostToolUse")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().any(|entry| {
                    entry
                        .get("hooks")
                        .and_then(|h| h.as_array())
                        .map(|hooks| {
                            hooks.iter().any(|h| {
                                h.get("command")
                                    .and_then(|c| c.as_str())
                                    .is_some_and(|c| c.contains("shire rebuild"))
                            })
                        })
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);

        if has_shire_hook {
            print_skipped(&format!(
                "hooks.PostToolUse already configured in {display_path}"
            ));
            return Ok(());
        }

        let hook_entry = json!({
            "matcher": "Edit|Write|NotebookEdit|Bash",
            "hooks": [{ "type": "command", "command": "shire rebuild --stdin" }]
        });
        let post_tool_use = hooks_obj.entry("PostToolUse").or_insert_with(|| json!([]));
        if let Some(arr) = post_tool_use.as_array_mut() {
            arr.push(hook_entry);
        } else {
            anyhow::bail!(
                "{display_path} has 'hooks.PostToolUse' as a non-array type. \
                 Please fix it manually or delete the key and re-run `{reinit_cmd}`."
            );
        }
    } else {
        anyhow::bail!(
            "{display_path} has 'hooks' as a non-object type. \
             Please fix it manually or delete the key and re-run `{reinit_cmd}`."
        );
    }

    let output = serde_json::to_string_pretty(&Value::Object(settings))
        .context("Failed to serialize settings to JSON")?;
    let tmp_path = settings_path.with_extension("json.tmp");
    fs::write(&tmp_path, format!("{output}\n"))
        .with_context(|| format!("Failed to write {}", tmp_path.display()))?;
    if let Err(e) = fs::rename(&tmp_path, settings_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e).with_context(|| {
            format!(
                "Failed to rename {} to {}",
                tmp_path.display(),
                settings_path.display()
            )
        });
    }
    print_created(&format!("Added hooks.PostToolUse to {display_path}"));
    Ok(())
}

/// Patch ~/.claude.json to add mcpServers.shire (user-scoped MCP config).
fn patch_claude_json(path: &Path, serve_args: Value, reinit_cmd: &str) -> Result<()> {
    let mut config: Map<String, Value> = if path.exists() {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?
    } else {
        Map::new()
    };

    let servers = config.entry("mcpServers").or_insert_with(|| json!({}));
    if let Some(servers_obj) = servers.as_object_mut() {
        if servers_obj.contains_key("shire") {
            print_skipped("mcpServers.shire already configured in ~/.claude.json");
            return Ok(());
        }
        servers_obj.insert(
            "shire".into(),
            json!({
                "command": "shire",
                "args": serve_args,
            }),
        );
    } else {
        anyhow::bail!(
            "~/.claude.json has 'mcpServers' as a non-object type. \
             Please fix it manually or delete the key and re-run `{reinit_cmd}`."
        );
    }

    let output = serde_json::to_string_pretty(&Value::Object(config))
        .context("Failed to serialize ~/.claude.json")?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, format!("{output}\n"))
        .with_context(|| format!("Failed to write {}", tmp_path.display()))?;
    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e).with_context(|| {
            format!(
                "Failed to rename {} to {}",
                tmp_path.display(),
                path.display()
            )
        });
    }
    print_created("Added mcpServers.shire to ~/.claude.json");
    Ok(())
}

/// Write content to a file atomically via a temp file + rename.
fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, content)
        .with_context(|| format!("Failed to write {}", tmp_path.display()))?;
    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e).with_context(|| {
            format!(
                "Failed to rename {} to {}",
                tmp_path.display(),
                path.display()
            )
        });
    }
    Ok(())
}

/// Ensure `dir` is listed in `.gitignore` at the project root.
/// Creates the file if it doesn't exist, appends if the entry isn't already present.
fn ensure_gitignore(root: &Path, dir: &str) -> Result<()> {
    let gitignore_path = root.join(".gitignore");
    // Use anchored form `/{dir}/` so it only matches the root-level directory,
    // not directories of the same name nested deeper in the tree.
    let entry = format!("/{dir}/");
    if gitignore_path.exists() {
        let content = fs::read_to_string(&gitignore_path)
            .with_context(|| format!("Failed to read {}", gitignore_path.display()))?;
        // Accept any variant (anchored or legacy unanchored) as already-present.
        if content.lines().any(|line| {
            let t = line.trim();
            t == entry || t == format!("/{dir}") || t == dir || t == format!("{dir}/")
        }) {
            return Ok(());
        }
        let separator = if content.ends_with('\n') { "" } else { "\n" };
        atomic_write(&gitignore_path, &format!("{content}{separator}{entry}\n"))?;
        print_created(&format!("Added {entry} to .gitignore"));
    } else {
        atomic_write(&gitignore_path, &format!("{entry}\n"))?;
        print_created(&format!("Created .gitignore with {entry}"));
    }
    Ok(())
}

/// Write .claude/rules/shire.md with Shire usage guidance. When
/// `refs_enabled` is true, appends the cross-reference tool guidance so
/// users who opt into the experimental index discover the new tools.
///
/// If the file already exists and refs were just enabled for the first
/// time, upgrades the existing file in place by appending the refs section
/// — otherwise an early opt-in would leave users without discovery docs
/// for the new MCP tools.
fn write_rules_file(rules_dir: &Path, display_path: &str, refs_enabled: bool) -> Result<()> {
    fs::create_dir_all(rules_dir)
        .with_context(|| format!("Failed to create directory {}", rules_dir.display()))?;
    let rules_path = rules_dir.join("shire.md");
    if rules_path.exists() {
        // Upgrade path: file was created before the user opted into refs,
        // so the refs guidance section is missing. Append it in place.
        if refs_enabled {
            let existing = fs::read_to_string(&rules_path)
                .with_context(|| format!("Failed to read {}", rules_path.display()))?;
            if !existing.contains("symbol_references") {
                let sep = if existing.ends_with('\n') { "" } else { "\n" };
                let updated = format!("{existing}{sep}{RULES_CONTENT_REFS}");
                atomic_write(&rules_path, &updated)?;
                print_created(&format!(
                    "Updated {display_path} with cross-reference guidance"
                ));
                return Ok(());
            }
        }
        print_skipped(&format!("{display_path} already exists"));
        return Ok(());
    }
    let content = if refs_enabled {
        format!("{RULES_CONTENT}{RULES_CONTENT_REFS}")
    } else {
        RULES_CONTENT.to_string()
    };
    atomic_write(&rules_path, &content)?;
    print_created(&format!("Created {display_path}"));
    Ok(())
}

/// Append Shire guidance to ~/.claude/CLAUDE.md if not already present.
fn ensure_claude_md_line() -> Result<()> {
    let claude_dir = home_dir()?.join(".claude");
    ensure_claude_md_line_in(&claude_dir)
}

fn ensure_claude_md_line_in(claude_dir: &Path) -> Result<()> {
    let claude_md_path = claude_dir.join("CLAUDE.md");
    if claude_md_path.exists() {
        let content = fs::read_to_string(&claude_md_path)
            .with_context(|| format!("Failed to read {}", claude_md_path.display()))?;
        if content.contains(CLAUDE_MD_LINE) {
            print_skipped("~/.claude/CLAUDE.md already has Shire guidance");
            return Ok(());
        }
        let separator = if content.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        atomic_write(
            &claude_md_path,
            &format!("{content}{separator}{CLAUDE_MD_LINE}\n"),
        )?;
        print_created("Added Shire guidance to ~/.claude/CLAUDE.md");
    } else {
        fs::create_dir_all(claude_dir)?;
        atomic_write(&claude_md_path, &format!("{CLAUDE_MD_LINE}\n"))?;
        print_created("Created ~/.claude/CLAUDE.md with Shire guidance");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_creates_config_and_mcp() {
        let dir = tempfile::TempDir::new().unwrap();
        run_init(dir.path(), false, true).unwrap();

        // shire.toml created
        let config_path = dir.path().join("shire.toml");
        assert!(config_path.exists());
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("# Shire configuration"));

        // .mcp.json created with MCP server
        let mcp_path = dir.path().join(".mcp.json");
        assert!(mcp_path.exists());
        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
        let mcp = &parsed["mcpServers"]["shire"];
        assert_eq!(mcp["command"], "shire");
        assert_eq!(mcp["args"], json!(["serve"]));

        // .claude/settings.json created with hooks
        let settings_path = dir.path().join(".claude/settings.json");
        assert!(settings_path.exists());
        let settings: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert!(settings["hooks"]["PostToolUse"].is_array());

        // .claude/rules/shire.md created
        let rules_path = dir.path().join(".claude/rules/shire.md");
        assert!(rules_path.exists());
        let content = fs::read_to_string(&rules_path).unwrap();
        assert!(content.contains("use Shire MCP tools"));
    }

    #[test]
    fn test_init_skips_existing_config_but_adds_mcp() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("shire.toml");
        fs::write(&config_path, "existing").unwrap();
        run_init(dir.path(), false, true).unwrap();

        // shire.toml unchanged
        let content = fs::read_to_string(&config_path).unwrap();
        assert_eq!(content, "existing");

        // .mcp.json still created
        let mcp_path = dir.path().join(".mcp.json");
        assert!(mcp_path.exists());
    }

    #[test]
    fn test_init_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        run_init(dir.path(), false, true).unwrap();
        run_init(dir.path(), false, true).unwrap();

        let mcp_path = dir.path().join(".mcp.json");
        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
        assert!(parsed["mcpServers"]["shire"].is_object());

        let settings_path = dir.path().join(".claude/settings.json");
        let settings: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        let hooks = settings["hooks"]["PostToolUse"].as_array().unwrap();
        let shire_hooks: Vec<_> = hooks
            .iter()
            .filter(|e| {
                e.get("hooks")
                    .and_then(|h| h.as_array())
                    .map(|arr| {
                        arr.iter().any(|h| {
                            h.get("command")
                                .and_then(|c| c.as_str())
                                .is_some_and(|c| c.contains("shire rebuild"))
                        })
                    })
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(shire_hooks.len(), 1);
    }

    #[test]
    fn test_init_preserves_existing_mcp_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let mcp_path = dir.path().join(".mcp.json");
        let existing = json!({
            "mcpServers": { "other": { "command": "other" } }
        });
        fs::write(&mcp_path, serde_json::to_string_pretty(&existing).unwrap()).unwrap();

        run_init(dir.path(), false, true).unwrap();

        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
        assert!(parsed["mcpServers"]["other"].is_object());
        assert!(parsed["mcpServers"]["shire"].is_object());
    }

    #[test]
    fn test_init_preserves_existing_settings() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        let settings_path = claude_dir.join("settings.json");
        let existing = json!({
            "permissions": { "allow": ["Bash(git:*)"] }
        });
        fs::write(
            &settings_path,
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        run_init(dir.path(), false, true).unwrap();

        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert!(parsed["permissions"]["allow"].is_array());
        assert!(parsed["hooks"]["PostToolUse"].is_array());
    }

    #[test]
    fn test_init_global_creates_config_and_mcp() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");

        run_init_global_in(&claude_dir, &InitOptions::default_global()).unwrap();

        // Config file created with {repo} placeholder
        let config_path = claude_dir.join("shire.toml");
        assert!(config_path.exists());
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("{repo}"));

        // MCP config written to ~/.claude.json (parent of claude_dir)
        let claude_json = dir.path().join(".claude.json");
        assert!(claude_json.exists());
        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&claude_json).unwrap()).unwrap();
        let mcp = &parsed["mcpServers"]["shire"];
        assert_eq!(mcp["command"], "shire");
        assert_eq!(mcp["args"], json!(["serve"]));

        // Hooks written to ~/.claude/settings.json
        let settings_path = claude_dir.join("settings.json");
        assert!(settings_path.exists());
        let settings: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert!(settings["hooks"]["PostToolUse"].is_array());

        // ~/.claude/rules/shire.md created
        let rules_path = claude_dir.join("rules/shire.md");
        assert!(rules_path.exists());
        let content = fs::read_to_string(&rules_path).unwrap();
        assert!(content.contains("use Shire MCP tools"));
    }

    #[test]
    fn test_init_global_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");

        run_init_global_in(&claude_dir, &InitOptions::default_global()).unwrap();
        run_init_global_in(&claude_dir, &InitOptions::default_global()).unwrap();

        // MCP should still have shire
        let claude_json = dir.path().join(".claude.json");
        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&claude_json).unwrap()).unwrap();
        assert!(parsed["mcpServers"]["shire"].is_object());

        // Should still have exactly one shire hook entry
        let settings_path = claude_dir.join("settings.json");
        let settings: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        let hooks = settings["hooks"]["PostToolUse"].as_array().unwrap();
        let shire_hooks: Vec<_> = hooks
            .iter()
            .filter(|e| {
                e.get("hooks")
                    .and_then(|h| h.as_array())
                    .map(|arr| {
                        arr.iter().any(|h| {
                            h.get("command")
                                .and_then(|c| c.as_str())
                                .is_some_and(|c| c.contains("shire rebuild"))
                        })
                    })
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(shire_hooks.len(), 1);
    }

    #[test]
    fn test_init_global_preserves_existing_claude_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();

        // Write existing ~/.claude.json with other MCP servers
        let claude_json = dir.path().join(".claude.json");
        let existing = json!({
            "mcpServers": {
                "other-server": {
                    "command": "other",
                    "args": ["serve"]
                }
            },
            "customSetting": true
        });
        fs::write(
            &claude_json,
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        run_init_global_in(&claude_dir, &InitOptions::default_global()).unwrap();

        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&claude_json).unwrap()).unwrap();
        assert!(parsed["mcpServers"]["other-server"].is_object());
        assert!(parsed["mcpServers"]["shire"].is_object());
        assert_eq!(parsed["customSetting"], json!(true));
    }

    #[test]
    fn test_init_global_malformed_claude_json_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(dir.path().join(".claude.json"), "not valid json{{{").unwrap();

        let result = run_init_global_in(&claude_dir, &InitOptions::default_global());
        assert!(result.is_err());
        let err = result.unwrap_err();
        let chain: String = err
            .chain()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            chain.contains("Failed to parse"),
            "expected 'Failed to parse' in: {chain}"
        );
    }

    #[test]
    fn test_init_global_wrong_mcp_servers_type_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(
            dir.path().join(".claude.json"),
            json!({"mcpServers": "broken"}).to_string(),
        )
        .unwrap();

        let result = run_init_global_in(&claude_dir, &InitOptions::default_global());
        assert!(result.is_err());
        let err = result.unwrap_err();
        let chain: String = err
            .chain()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            chain.contains("non-object type"),
            "expected 'non-object type' in: {chain}"
        );
    }

    #[test]
    fn test_init_no_hook_creates_mcp_with_root_and_no_hook() {
        let dir = tempfile::TempDir::new().unwrap();
        run_init(dir.path(), true, true).unwrap();

        let mcp_path = dir.path().join(".mcp.json");
        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
        let mcp = &parsed["mcpServers"]["shire"];
        assert_eq!(mcp["command"], "shire");
        assert_eq!(mcp["args"], json!(["serve", "--root", "."]));
        // No settings.json should be created in no-hook mode
        let settings_path = dir.path().join(".claude/settings.json");
        assert!(
            !settings_path.exists(),
            "settings.json should not exist in no-hook mode"
        );
    }

    #[test]
    fn test_init_no_hook_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        run_init(dir.path(), true, true).unwrap();
        run_init(dir.path(), true, true).unwrap();

        let mcp_path = dir.path().join(".mcp.json");
        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
        assert!(parsed["mcpServers"]["shire"].is_object());
    }

    #[test]
    fn test_init_with_hook_still_installs_hook() {
        let dir = tempfile::TempDir::new().unwrap();
        run_init(dir.path(), false, true).unwrap();

        let mcp_path = dir.path().join(".mcp.json");
        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
        let mcp = &parsed["mcpServers"]["shire"];
        assert_eq!(mcp["args"], json!(["serve"]));

        let settings_path = dir.path().join(".claude/settings.json");
        let settings: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert!(settings["hooks"]["PostToolUse"].is_array());
        let hooks = settings["hooks"]["PostToolUse"].as_array().unwrap();
        assert!(!hooks.is_empty(), "PostToolUse hooks should be present");
    }

    #[test]
    fn test_generate_config_toml_local_defaults() {
        let opts = InitOptions::default_local();
        let toml = generate_config_toml(&opts, false);
        assert!(toml.contains("# Shire configuration"));
        assert!(toml.contains("db_path = \".shire/index.db\""));
        assert!(!toml.contains("[discovery]"));
    }

    #[test]
    fn test_generate_config_toml_global_defaults() {
        let opts = InitOptions::default_global();
        let toml = generate_config_toml(&opts, true);
        assert!(toml.contains("# Shire global configuration"));
        assert!(toml.contains("{repo}"));
        assert!(toml.contains("{worktree}"));
        assert!(!toml.contains("[discovery]"));
    }

    #[test]
    fn test_generate_config_toml_custom_db() {
        let opts = InitOptions {
            db_path: "/tmp/custom.db".into(),
            ..InitOptions::default_local()
        };
        let toml = generate_config_toml(&opts, false);
        assert!(toml.contains("db_path = \"/tmp/custom.db\""));
    }

    #[test]
    fn test_generate_config_toml_extra_excludes() {
        let opts = InitOptions {
            extra_excludes: vec!["generated".into(), "tmp".into()],
            ..InitOptions::default_local()
        };
        let toml = generate_config_toml(&opts, false);
        assert!(toml.contains("[discovery]"));
        assert!(toml.contains("\"node_modules\""));
        assert!(toml.contains("\"generated\""));
        assert!(toml.contains("\"tmp\""));
    }

    #[test]
    fn test_generate_config_toml_all_options() {
        let opts = InitOptions {
            use_hook: false,
            db_path: "/custom/path.db".into(),
            extra_excludes: vec!["gen".into()],
            refs_enabled: true,
            generate_rules: true,
            patch_claude_md: false,
            gitignore_db_dir: false,
            non_interactive: true,
        };
        let toml = generate_config_toml(&opts, false);
        assert!(toml.contains("db_path = \"/custom/path.db\""));
        assert!(toml.contains("[discovery]"));
        assert!(toml.contains("\"gen\""));
        assert!(toml.contains("[symbols]"));
        assert!(toml.contains("references_enabled = true"));
    }

    #[test]
    fn test_generate_config_toml_refs_disabled_no_symbols_section() {
        let opts = InitOptions {
            refs_enabled: false,
            ..InitOptions::default_local()
        };
        let toml = generate_config_toml(&opts, false);
        assert!(
            !toml.contains("[symbols]"),
            "opt-out should not emit [symbols] section: {}",
            toml
        );
    }

    #[test]
    fn test_write_rules_file_includes_refs_guidance_when_enabled() {
        let dir = tempfile::TempDir::new().unwrap();
        let rules_dir = dir.path().join(".claude/rules");
        write_rules_file(&rules_dir, ".claude/rules/shire.md", true).unwrap();

        let content = fs::read_to_string(rules_dir.join("shire.md")).unwrap();
        assert!(content.contains("symbol_references"));
        assert!(content.contains("symbol_callers"));
        assert!(content.contains("symbol_callees"));
        assert!(content.contains("reference_audit"));
    }

    #[test]
    fn test_write_rules_file_omits_refs_guidance_when_disabled() {
        let dir = tempfile::TempDir::new().unwrap();
        let rules_dir = dir.path().join(".claude/rules");
        write_rules_file(&rules_dir, ".claude/rules/shire.md", false).unwrap();

        let content = fs::read_to_string(rules_dir.join("shire.md")).unwrap();
        assert!(!content.contains("symbol_references"));
        assert!(!content.contains("symbol_callers"));
        assert!(!content.contains("symbol_callees"));
        assert!(!content.contains("reference_audit"));
    }

    #[test]
    fn test_write_rules_file_upgrades_existing_when_refs_enabled() {
        // Simulates: user ran `shire init` without refs, then re-runs it after
        // enabling refs. The existing shire.md must get the refs guidance
        // appended rather than being silently skipped.
        let dir = tempfile::TempDir::new().unwrap();
        let rules_dir = dir.path().join(".claude/rules");

        // First pass: refs off.
        write_rules_file(&rules_dir, ".claude/rules/shire.md", false).unwrap();
        let before = fs::read_to_string(rules_dir.join("shire.md")).unwrap();
        assert!(!before.contains("symbol_references"));

        // Second pass: refs on — should upgrade the existing file.
        write_rules_file(&rules_dir, ".claude/rules/shire.md", true).unwrap();
        let after = fs::read_to_string(rules_dir.join("shire.md")).unwrap();
        assert!(after.contains("symbol_references"));
        assert!(after.contains("symbol_callers"));
        assert!(after.contains("reference_audit"));
        // Original content preserved.
        assert!(after.contains("search_symbols"));
    }

    #[test]
    fn test_write_rules_file_idempotent_upgrade() {
        // Upgrade must be idempotent: running twice with refs_enabled must
        // not duplicate the refs section.
        let dir = tempfile::TempDir::new().unwrap();
        let rules_dir = dir.path().join(".claude/rules");
        write_rules_file(&rules_dir, ".claude/rules/shire.md", false).unwrap();
        write_rules_file(&rules_dir, ".claude/rules/shire.md", true).unwrap();
        let first = fs::read_to_string(rules_dir.join("shire.md")).unwrap();
        write_rules_file(&rules_dir, ".claude/rules/shire.md", true).unwrap();
        let second = fs::read_to_string(rules_dir.join("shire.md")).unwrap();
        assert_eq!(first, second, "second upgrade should be a no-op");
        assert_eq!(
            second
                .matches("## Cross-reference index (experimental)")
                .count(),
            1,
            "refs section must appear exactly once"
        );
    }

    #[test]
    fn test_init_rules_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        run_init(dir.path(), false, true).unwrap();

        // Modify the rules file
        let rules_path = dir.path().join(".claude/rules/shire.md");
        fs::write(&rules_path, "custom content").unwrap();

        // Re-run init — should not overwrite
        run_init(dir.path(), false, true).unwrap();
        let content = fs::read_to_string(&rules_path).unwrap();
        assert_eq!(content, "custom content");
    }

    // --- gitignore_dir_from_db_path ---

    #[test]
    fn test_gitignore_dir_simple() {
        assert_eq!(
            gitignore_dir_from_db_path(".shire/index.db").as_deref(),
            Some(".shire")
        );
        assert_eq!(
            gitignore_dir_from_db_path("build/shire.db").as_deref(),
            Some("build")
        );
    }

    #[test]
    fn test_gitignore_dir_nested() {
        // Full parent path, not just the first segment
        assert_eq!(
            gitignore_dir_from_db_path("src/db/index.db").as_deref(),
            Some("src/db")
        );
        assert_eq!(
            gitignore_dir_from_db_path("a/b/c/index.db").as_deref(),
            Some("a/b/c")
        );
    }

    #[test]
    fn test_gitignore_dir_with_leading_dot_slash() {
        // CurDir prefix is stripped
        assert_eq!(
            gitignore_dir_from_db_path("./build/shire.db").as_deref(),
            Some("build")
        );
        assert_eq!(
            gitignore_dir_from_db_path("./src/db/index.db").as_deref(),
            Some("src/db")
        );
    }

    #[test]
    fn test_gitignore_dir_bare_filename_is_none() {
        // No directory component — nothing to ignore
        assert_eq!(gitignore_dir_from_db_path("index.db"), None);
    }

    #[test]
    fn test_gitignore_dir_absolute_is_none() {
        assert_eq!(gitignore_dir_from_db_path("/abs/path/index.db"), None);
    }

    #[test]
    fn test_gitignore_dir_tilde_is_none() {
        assert_eq!(gitignore_dir_from_db_path("~/.claude/shire.db"), None);
    }

    // --- ensure_gitignore idempotency ---

    #[test]
    fn test_ensure_gitignore_creates_anchored_entry() {
        let dir = tempfile::TempDir::new().unwrap();
        ensure_gitignore(dir.path(), ".shire").unwrap();
        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(
            content.contains("/.shire/"),
            "should contain anchored entry"
        );
    }

    #[test]
    fn test_ensure_gitignore_idempotent_anchored() {
        let dir = tempfile::TempDir::new().unwrap();
        // Pre-existing anchored entry
        fs::write(dir.path().join(".gitignore"), "/.shire/\n").unwrap();
        ensure_gitignore(dir.path(), ".shire").unwrap();
        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(
            content.matches("/.shire/").count(),
            1,
            "should not duplicate"
        );
    }

    #[test]
    fn test_ensure_gitignore_idempotent_legacy_unanchored() {
        let dir = tempfile::TempDir::new().unwrap();
        for existing in &[".shire", ".shire/", "/.shire"] {
            let gitignore = dir.path().join(".gitignore");
            fs::write(&gitignore, format!("{existing}\n")).unwrap();
            ensure_gitignore(dir.path(), ".shire").unwrap();
            let content = fs::read_to_string(&gitignore).unwrap();
            assert!(
                !content.contains("/.shire/"),
                "should not add anchored entry when legacy variant '{existing}' present"
            );
        }
    }

    #[test]
    fn test_ensure_gitignore_nested_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        ensure_gitignore(dir.path(), "src/db").unwrap();
        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("/src/db/"));
    }

    #[test]
    fn test_ensure_gitignore_appends_without_trailing_newline() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join(".gitignore"), "node_modules").unwrap();
        ensure_gitignore(dir.path(), ".shire").unwrap();
        let content = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("node_modules\n/.shire/\n"));
    }

    // --- ensure_claude_md_line ---

    #[test]
    fn test_ensure_claude_md_creates_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        ensure_claude_md_line_in(&claude_dir).unwrap();
        let content = fs::read_to_string(claude_dir.join("CLAUDE.md")).unwrap();
        assert!(content.contains(CLAUDE_MD_LINE));
        assert!(content.ends_with('\n'));
    }

    #[test]
    fn test_ensure_claude_md_appends_to_existing() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(claude_dir.join("CLAUDE.md"), "# My Config\n").unwrap();
        ensure_claude_md_line_in(&claude_dir).unwrap();
        let content = fs::read_to_string(claude_dir.join("CLAUDE.md")).unwrap();
        assert!(content.starts_with("# My Config\n"));
        assert!(content.contains(CLAUDE_MD_LINE));
    }

    #[test]
    fn test_ensure_claude_md_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        ensure_claude_md_line_in(&claude_dir).unwrap();
        ensure_claude_md_line_in(&claude_dir).unwrap();
        let content = fs::read_to_string(claude_dir.join("CLAUDE.md")).unwrap();
        assert_eq!(content.matches(CLAUDE_MD_LINE).count(), 1);
    }

    #[test]
    fn test_ensure_claude_md_appends_without_trailing_newline() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(claude_dir.join("CLAUDE.md"), "# No newline").unwrap();
        ensure_claude_md_line_in(&claude_dir).unwrap();
        let content = fs::read_to_string(claude_dir.join("CLAUDE.md")).unwrap();
        assert!(content.contains("# No newline\n\n"));
        assert!(content.contains(CLAUDE_MD_LINE));
    }
}
