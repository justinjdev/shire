use anyhow::{Context, Result};
use dialoguer::{Confirm, Input, Select};
use serde_json::{Map, Value, json};
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::config::default_exclude;

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
    pub rag_enabled: bool,
    /// When true, skip interactive prompts for existing files.
    pub non_interactive: bool,
}

impl InitOptions {
    pub fn default_local() -> Self {
        Self {
            use_hook: true,
            db_path: ".shire/index.db".into(),
            extra_excludes: Vec::new(),
            rag_enabled: false,
            non_interactive: true,
        }
    }

    pub fn default_global() -> Self {
        Self {
            use_hook: true,
            db_path: "~/.claude/shire/{repo}/index.db".into(),
            extra_excludes: Vec::new(),
            rag_enabled: false,
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

    // 3. Additional exclude directories
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

    // 4. Enable RAG vector search
    let rag_enabled = Confirm::new()
        .with_prompt("Enable RAG vector search?")
        .default(false)
        .interact()?;

    Ok(InitOptions {
        use_hook,
        db_path,
        extra_excludes,
        rag_enabled,
        non_interactive: false,
    })
}

pub fn generate_config_toml(opts: &InitOptions, global: bool) -> String {
    let mut lines = Vec::new();

    if global {
        lines.push("# Shire global configuration — shared across all repositories".into());
        lines.push("# The {repo} placeholder is replaced with the repository directory name".into());
    } else {
        lines.push("# Shire configuration".into());
    }
    lines.push(String::new());

    lines.push(format!("db_path = \"{}\"", escape_toml_string(&opts.db_path)));
    lines.push(String::new());

    if !opts.extra_excludes.is_empty() {
        let mut all_excludes = default_exclude();
        for ex in &opts.extra_excludes {
            if !all_excludes.contains(ex) {
                all_excludes.push(ex.clone());
            }
        }
        lines.push("[discovery]".into());
        let quoted: Vec<String> = all_excludes.iter().map(|e| format!("\"{}\"", escape_toml_string(e))).collect();
        lines.push(format!("exclude = [{}]", quoted.join(", ")));
        lines.push(String::new());
    }

    if opts.rag_enabled {
        lines.push("[rag]".into());
        lines.push("enabled = true".into());
        lines.push(String::new());
    }

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub fn run_init(root: &Path, no_hook: bool, yes: bool) -> Result<()> {
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
            println!("shire.toml already exists, skipping");
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
        println!(
            "{} {}",
            if config_exists { "Updated" } else { "Created" },
            config_path.display()
        );
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

    if opts.use_hook {
        println!("\nNext: run `shire build` in this repo to create the index.");
    } else {
        println!("\nOn-demand reindexing enabled. The MCP server will rebuild the index automatically when needed.");
        println!("No PostToolUse hook installed.");
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
            println!("~/.claude/shire.toml already exists, skipping");
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
        println!(
            "{} ~/.claude/shire.toml",
            if config_exists { "Updated" } else { "Created" }
        );
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
        patch_claude_hooks(&settings_path, "~/.claude/settings.json", "shire init --global")?;
    }

    if opts.use_hook {
        println!("\nNext: run `shire build` in each repo you want to index.");
    } else {
        println!("\nOn-demand reindexing enabled globally. The MCP server will rebuild the index automatically when needed.");
        println!("No PostToolUse hook installed.");
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

    let servers = mcp
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    if let Some(servers_obj) = servers.as_object_mut() {
        if servers_obj.contains_key("shire") {
            println!("mcpServers.shire already configured in .mcp.json, skipping");
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
            format!("Failed to rename {} to {}", tmp_path.display(), mcp_path.display())
        });
    }
    println!("Added mcpServers.shire to .mcp.json");
    Ok(())
}

/// Patch a Claude Code settings JSON file to add hooks.PostToolUse only.
fn patch_claude_hooks(
    settings_path: &Path,
    display_path: &str,
    reinit_cmd: &str,
) -> Result<()> {
    let mut settings: Map<String, Value> = if settings_path.exists() {
        let content = fs::read_to_string(settings_path)
            .with_context(|| format!("Failed to read {}", settings_path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", settings_path.display()))?
    } else {
        Map::new()
    };

    let hooks = settings
        .entry("hooks")
        .or_insert_with(|| json!({}));
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
            println!("hooks.PostToolUse shire rebuild already configured in {display_path}, skipping");
            return Ok(());
        }

        let hook_entry = json!({
            "matcher": "Edit|Write|NotebookEdit|Bash",
            "hooks": [{ "type": "command", "command": "shire rebuild --stdin" }]
        });
        let post_tool_use = hooks_obj
            .entry("PostToolUse")
            .or_insert_with(|| json!([]));
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
            format!("Failed to rename {} to {}", tmp_path.display(), settings_path.display())
        });
    }
    println!("Added hooks.PostToolUse shire rebuild to {display_path}");
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

    let servers = config
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    if let Some(servers_obj) = servers.as_object_mut() {
        if servers_obj.contains_key("shire") {
            println!("mcpServers.shire already configured in ~/.claude.json, skipping");
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
            format!("Failed to rename {} to {}", tmp_path.display(), path.display())
        });
    }
    println!("Added mcpServers.shire to ~/.claude.json");
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
        fs::write(&settings_path, serde_json::to_string_pretty(&existing).unwrap()).unwrap();

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
        fs::write(&claude_json, serde_json::to_string_pretty(&existing).unwrap()).unwrap();

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
        let chain: String = err.chain().map(|e| e.to_string()).collect::<Vec<_>>().join(" ");
        assert!(chain.contains("Failed to parse"), "expected 'Failed to parse' in: {chain}");
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
        let chain: String = err.chain().map(|e| e.to_string()).collect::<Vec<_>>().join(" ");
        assert!(chain.contains("non-object type"), "expected 'non-object type' in: {chain}");
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
        assert!(!settings_path.exists(), "settings.json should not exist in no-hook mode");
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
        assert!(!toml.contains("[rag]"));
    }

    #[test]
    fn test_generate_config_toml_global_defaults() {
        let opts = InitOptions::default_global();
        let toml = generate_config_toml(&opts, true);
        assert!(toml.contains("# Shire global configuration"));
        assert!(toml.contains("{repo}"));
        assert!(!toml.contains("[discovery]"));
        assert!(!toml.contains("[rag]"));
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
    fn test_generate_config_toml_rag_enabled() {
        let opts = InitOptions {
            rag_enabled: true,
            ..InitOptions::default_local()
        };
        let toml = generate_config_toml(&opts, false);
        assert!(toml.contains("[rag]"));
        assert!(toml.contains("enabled = true"));
    }

    #[test]
    fn test_generate_config_toml_all_options() {
        let opts = InitOptions {
            use_hook: false,
            db_path: "/custom/path.db".into(),
            extra_excludes: vec!["gen".into()],
            rag_enabled: true,
            non_interactive: true,
        };
        let toml = generate_config_toml(&opts, false);
        assert!(toml.contains("db_path = \"/custom/path.db\""));
        assert!(toml.contains("[discovery]"));
        assert!(toml.contains("\"gen\""));
        assert!(toml.contains("[rag]"));
        assert!(toml.contains("enabled = true"));
    }
}
