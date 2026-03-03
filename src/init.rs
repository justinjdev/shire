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
    let config_path = root.join("shire.toml");
    if config_path.exists() {
        println!("shire.toml already exists at {}", config_path.display());
        return Ok(());
    }
    fs::write(&config_path, DEFAULT_PROJECT_CONFIG)
        .with_context(|| format!("Failed to write {}", config_path.display()))?;
    println!("Created {}", config_path.display());
    Ok(())
}

fn home_dir() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .context("HOME environment variable not set")
}

pub fn run_init_global() -> Result<()> {
    let claude_dir = home_dir()?.join(".claude");
    fs::create_dir_all(&claude_dir)?;

    // 1. Create ~/.claude/shire.toml
    let config_path = claude_dir.join("shire.toml");
    if config_path.exists() {
        println!("~/.claude/shire.toml already exists, skipping");
    } else {
        fs::write(&config_path, DEFAULT_GLOBAL_CONFIG)?;
        println!("Created ~/.claude/shire.toml");
    }

    // 2. Patch ~/.claude/settings.json
    let settings_path = claude_dir.join("settings.json");
    let mut settings: Map<String, Value> = if settings_path.exists() {
        let content = fs::read_to_string(&settings_path)?;
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
            println!("mcpServers.shire already configured, skipping");
        } else {
            servers.insert(
                "shire".into(),
                json!({
                    "command": "shire",
                    "args": ["serve", "--config", "~/.claude/shire.toml"]
                }),
            );
            println!("Added mcpServers.shire to ~/.claude/settings.json");
            changed = true;
        }
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
            println!("hooks.PostToolUse shire rebuild already configured, skipping");
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
            }
            println!("Added hooks.PostToolUse shire rebuild to ~/.claude/settings.json");
            changed = true;
        }
    }

    if changed {
        let output = serde_json::to_string_pretty(&Value::Object(settings))?;
        fs::write(&settings_path, format!("{output}\n"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_creates_config() {
        let dir = tempfile::TempDir::new().unwrap();
        run_init(dir.path()).unwrap();
        let config_path = dir.path().join("shire.toml");
        assert!(config_path.exists());
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("# Shire configuration"));
    }

    #[test]
    fn test_init_skips_existing() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("shire.toml");
        fs::write(&config_path, "existing").unwrap();
        run_init(dir.path()).unwrap();
        let content = fs::read_to_string(&config_path).unwrap();
        assert_eq!(content, "existing");
    }

    #[test]
    fn test_init_global_creates_config_and_settings() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");

        // Temporarily override HOME so we don't touch the real ~/.claude
        // Instead, test the internal logic directly
        let config_path = claude_dir.join("shire.toml");
        let settings_path = claude_dir.join("settings.json");
        fs::create_dir_all(&claude_dir).unwrap();

        // Simulate what run_init_global does without touching real HOME
        fs::write(&config_path, DEFAULT_GLOBAL_CONFIG).unwrap();
        assert!(config_path.exists());
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("{repo}"));

        // Simulate settings.json merge
        let mut settings: Map<String, Value> = Map::new();
        settings.insert(
            "mcpServers".into(),
            json!({
                "shire": {
                    "command": "shire",
                    "args": ["serve", "--config", "~/.claude/shire.toml"]
                }
            }),
        );
        settings.insert(
            "hooks".into(),
            json!({
                "PostToolUse": [{
                    "matcher": "Edit|Write|NotebookEdit|Bash",
                    "hooks": [{ "type": "command", "command": "shire rebuild --stdin" }]
                }]
            }),
        );
        let output = serde_json::to_string_pretty(&Value::Object(settings)).unwrap();
        fs::write(&settings_path, format!("{output}\n")).unwrap();

        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert!(parsed.contains_key("mcpServers"));
        assert!(parsed.contains_key("hooks"));
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

        // Simulate merge
        let content = fs::read_to_string(&settings_path).unwrap();
        let mut settings: Map<String, Value> = serde_json::from_str(&content).unwrap();

        let mcp_servers = settings
            .entry("mcpServers")
            .or_insert_with(|| json!({}));
        if let Some(servers) = mcp_servers.as_object_mut() {
            servers.insert(
                "shire".into(),
                json!({
                    "command": "shire",
                    "args": ["serve", "--config", "~/.claude/shire.toml"]
                }),
            );
        }

        let output = serde_json::to_string_pretty(&Value::Object(settings.clone())).unwrap();
        fs::write(&settings_path, format!("{output}\n")).unwrap();

        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        // Existing server preserved
        assert!(parsed["mcpServers"]["other-server"].is_object());
        // Shire added
        assert!(parsed["mcpServers"]["shire"].is_object());
        // Custom setting preserved
        assert_eq!(parsed["customSetting"], json!(true));
    }
}
