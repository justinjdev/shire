use anyhow::{Context, Result};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_PROJECT_CONFIG: &str = r#"# Shire configuration (optional — defaults work for most repos)

# Custom database location (default: .shire/index.db)
# db_path = ".shire/index.db"

# [discovery]
# manifests = ["package.json", "go.mod", "Cargo.toml", "pyproject.toml"]
# exclude = ["node_modules", "vendor", "dist", ".build", "target", "third_party"]

# [symbols]
# exclude_extensions = [".proto"]

# [[packages]]
# name = "legacy-auth"
# description = "Deprecated auth service"
"#;

const DEFAULT_GLOBAL_CONFIG: &str = r#"# Shire global configuration — shared across all repositories
# The {repo} placeholder is replaced with the repository directory name

db_path = "~/.claude/shire/{repo}/index.db"
"#;

pub fn run_init(root: &Path) -> Result<()> {
    // 1. Create shire.toml
    let config_path = root.join("shire.toml");
    if config_path.exists() {
        println!("shire.toml already exists, skipping");
    } else {
        fs::write(&config_path, DEFAULT_PROJECT_CONFIG)
            .with_context(|| format!("Failed to write {}", config_path.display()))?;
        println!("Created {}", config_path.display());
    }

    // 2. Patch .claude/settings.local.json
    let claude_dir = root.join(".claude");
    fs::create_dir_all(&claude_dir)
        .with_context(|| format!("Failed to create directory {}", claude_dir.display()))?;
    let settings_path = claude_dir.join("settings.local.json");
    patch_claude_settings(
        &settings_path,
        json!(["serve"]),
        ".claude/settings.local.json",
        "shire init",
    )
    .context("Failed to configure .claude/settings.local.json. shire.toml was created successfully. Fix the issue above and re-run `shire init`.")?;

    println!("\nNext: run `shire build` in this repo to create the index.");
    Ok(())
}

fn home_dir() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .context("HOME environment variable not set")
}

pub fn run_init_global() -> Result<()> {
    let claude_dir = home_dir()?.join(".claude");
    run_init_global_in(&claude_dir)
}

fn run_init_global_in(claude_dir: &Path) -> Result<()> {
    fs::create_dir_all(claude_dir)
        .with_context(|| format!("Failed to create directory {}", claude_dir.display()))?;

    // 1. Create shire.toml
    let config_path = claude_dir.join("shire.toml");
    if config_path.exists() {
        println!("~/.claude/shire.toml already exists, skipping");
    } else {
        fs::write(&config_path, DEFAULT_GLOBAL_CONFIG)
            .with_context(|| format!("Failed to write {}", config_path.display()))?;
        println!("Created ~/.claude/shire.toml");
    }

    // 2. Patch settings.json
    let settings_path = claude_dir.join("settings.json");
    patch_claude_settings(
        &settings_path,
        json!(["serve"]),
        "~/.claude/settings.json",
        "shire init --global",
    )
    .context("Failed to configure ~/.claude/settings.json. ~/.claude/shire.toml was created successfully. Fix the issue above and re-run `shire init --global`.")?;

    println!("\nNext: run `shire build` in each repo you want to index.");
    Ok(())
}

/// Patch a Claude Code settings JSON file with mcpServers.shire and hooks.PostToolUse.
fn patch_claude_settings(
    settings_path: &Path,
    serve_args: Value,
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

    let mut changed = false;

    // Add mcpServers.shire
    let mcp_servers = settings
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    if let Some(servers) = mcp_servers.as_object_mut() {
        if servers.contains_key("shire") {
            println!("mcpServers.shire already configured in {display_path}, skipping");
        } else {
            servers.insert(
                "shire".into(),
                json!({
                    "command": "shire",
                    "args": serve_args,
                }),
            );
            println!("Added mcpServers.shire to {display_path}");
            changed = true;
        }
    } else {
        anyhow::bail!(
            "{display_path} has 'mcpServers' as a non-object type. \
             Please fix it manually or delete the key and re-run `{reinit_cmd}`."
        );
    }

    // Add hooks.PostToolUse for rebuild
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
        } else {
            let hook_entry = json!({
                "matcher": "Edit|Write|NotebookEdit|Bash",
                "hooks": [{ "type": "command", "command": "shire rebuild --stdin" }]
            });
            let post_tool_use = hooks_obj
                .entry("PostToolUse")
                .or_insert_with(|| json!([]));
            if let Some(arr) = post_tool_use.as_array_mut() {
                arr.push(hook_entry);
                println!("Added hooks.PostToolUse shire rebuild to {display_path}");
                changed = true;
            } else {
                anyhow::bail!(
                    "{display_path} has 'hooks.PostToolUse' as a non-array type. \
                     Please fix it manually or delete the key and re-run `{reinit_cmd}`."
                );
            }
        }
    } else {
        anyhow::bail!(
            "{display_path} has 'hooks' as a non-object type. \
             Please fix it manually or delete the key and re-run `{reinit_cmd}`."
        );
    }

    if changed {
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
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_creates_config_and_mcp() {
        let dir = tempfile::TempDir::new().unwrap();
        run_init(dir.path()).unwrap();

        // shire.toml created
        let config_path = dir.path().join("shire.toml");
        assert!(config_path.exists());
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("# Shire configuration"));

        // .claude/settings.local.json created with MCP and hooks
        let settings_path = dir.path().join(".claude/settings.local.json");
        assert!(settings_path.exists());
        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        let mcp = &parsed["mcpServers"]["shire"];
        assert_eq!(mcp["command"], "shire");
        assert_eq!(mcp["args"], json!(["serve"]));
        assert!(parsed["hooks"]["PostToolUse"].is_array());
    }

    #[test]
    fn test_init_skips_existing_config_but_adds_mcp() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("shire.toml");
        fs::write(&config_path, "existing").unwrap();
        run_init(dir.path()).unwrap();

        // shire.toml unchanged
        let content = fs::read_to_string(&config_path).unwrap();
        assert_eq!(content, "existing");

        // MCP config still created
        let settings_path = dir.path().join(".claude/settings.local.json");
        assert!(settings_path.exists());
    }

    #[test]
    fn test_init_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        run_init(dir.path()).unwrap();
        run_init(dir.path()).unwrap();

        let settings_path = dir.path().join(".claude/settings.local.json");
        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert!(parsed["mcpServers"]["shire"].is_object());
        let hooks = parsed["hooks"]["PostToolUse"].as_array().unwrap();
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
    fn test_init_preserves_existing_settings() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        let settings_path = claude_dir.join("settings.local.json");
        let existing = json!({
            "permissions": { "allow": ["Bash(git:*)"] },
            "mcpServers": { "other": { "command": "other" } }
        });
        fs::write(&settings_path, serde_json::to_string_pretty(&existing).unwrap()).unwrap();

        run_init(dir.path()).unwrap();

        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert!(parsed["mcpServers"]["other"].is_object());
        assert!(parsed["mcpServers"]["shire"].is_object());
        assert!(parsed["permissions"]["allow"].is_array());
    }

    #[test]
    fn test_init_global_creates_config_and_settings() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");

        run_init_global_in(&claude_dir).unwrap();

        // Config file created with {repo} placeholder
        let config_path = claude_dir.join("shire.toml");
        assert!(config_path.exists());
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("{repo}"));

        // Settings file created with mcpServers and hooks
        let settings_path = claude_dir.join("settings.json");
        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        let mcp = &parsed["mcpServers"]["shire"];
        assert_eq!(mcp["command"], "shire");
        assert_eq!(mcp["args"], json!(["serve"]));
        assert!(parsed["hooks"]["PostToolUse"].is_array());
    }

    #[test]
    fn test_init_global_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");

        run_init_global_in(&claude_dir).unwrap();
        run_init_global_in(&claude_dir).unwrap();

        // Should still have exactly one shire hook entry
        let settings_path = claude_dir.join("settings.json");
        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert!(parsed["mcpServers"]["shire"].is_object());
        let hooks = parsed["hooks"]["PostToolUse"].as_array().unwrap();
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
    fn test_init_global_preserves_existing_settings() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        let settings_path = claude_dir.join("settings.json");

        // Write existing settings with other MCP servers
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
            &settings_path,
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        run_init_global_in(&claude_dir).unwrap();

        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        // Existing server preserved
        assert!(parsed["mcpServers"]["other-server"].is_object());
        // Shire added
        assert!(parsed["mcpServers"]["shire"].is_object());
        // Custom setting preserved
        assert_eq!(parsed["customSetting"], json!(true));
    }

    #[test]
    fn test_init_global_malformed_settings_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(claude_dir.join("settings.json"), "not valid json{{{").unwrap();

        let result = run_init_global_in(&claude_dir);
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
            claude_dir.join("settings.json"),
            json!({"mcpServers": "broken"}).to_string(),
        )
        .unwrap();

        let result = run_init_global_in(&claude_dir);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let chain: String = err.chain().map(|e| e.to_string()).collect::<Vec<_>>().join(" ");
        assert!(chain.contains("non-object type"), "expected 'non-object type' in: {chain}");
    }
}
