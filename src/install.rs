use anyhow::{Context, Result};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Result of registering with a single tool.
struct Registration {
    tool: &'static str,
    status: RegStatus,
}

enum RegStatus {
    Registered(String),
    Updated(String),
    AlreadyRegistered(String),
    NotFound,
    Failed(String),
}

pub fn run_install(dry_run: bool, force: bool) -> Result<()> {
    let binary_path = detect_binary_path()?;

    println!("shire {} — install", env!("CARGO_PKG_VERSION"));
    println!("Binary: {}", binary_path.display());
    if force {
        println!("Mode: force (overwrite existing registrations)");
    }
    println!();

    let mut results = Vec::new();

    // Claude Code — use `claude mcp add` CLI if available
    results.push(register_claude_code(&binary_path, dry_run, force));

    // Codex CLI — ~/.codex/config.toml
    results.push(register_codex(&binary_path, dry_run, force));

    // Cursor — ~/.cursor/mcp.json
    results.push(register_editor_mcp(
        &binary_path,
        "Cursor",
        &cursor_config_path(),
        "mcpServers",
        None,
        dry_run,
        force,
    ));

    // Windsurf — ~/.codeium/windsurf/mcp_config.json
    results.push(register_editor_mcp(
        &binary_path,
        "Windsurf",
        &windsurf_config_path(),
        "mcpServers",
        None,
        dry_run,
        force,
    ));

    // Gemini CLI — ~/.gemini/settings.json
    results.push(register_editor_mcp(
        &binary_path,
        "Gemini CLI",
        &gemini_config_path(),
        "mcpServers",
        None,
        dry_run,
        force,
    ));

    // VS Code — ~/Library/Application Support/Code/User/mcp.json
    results.push(register_editor_mcp(
        &binary_path,
        "VS Code",
        &vscode_config_path(),
        "servers",
        Some(json!({"type": "stdio", "command": binary_path.to_string_lossy()})),
        dry_run,
        force,
    ));

    // Zed — ~/.config/zed/settings.json
    results.push(register_editor_mcp(
        &binary_path,
        "Zed",
        &zed_config_path(),
        "context_servers",
        Some(json!({"source": "custom", "command": binary_path.to_string_lossy()})),
        dry_run,
        force,
    ));

    // Summary
    println!();
    println!("Summary:");
    for r in &results {
        match &r.status {
            RegStatus::Registered(path) => println!("  + {} — registered ({})", r.tool, path),
            RegStatus::Updated(path) => println!("  ~ {} — updated ({})", r.tool, path),
            RegStatus::AlreadyRegistered(path) => {
                println!("  = {} — already registered ({})", r.tool, path)
            }
            RegStatus::NotFound => println!("  - {} — not found, skipped", r.tool),
            RegStatus::Failed(err) => println!("  ! {} — failed: {}", r.tool, err),
        }
    }

    let registered = results
        .iter()
        .filter(|r| matches!(r.status, RegStatus::Registered(_) | RegStatus::Updated(_) | RegStatus::AlreadyRegistered(_)))
        .count();
    if registered > 0 {
        println!("\nRestart your editor/CLI to activate.");
    } else {
        println!("\nNo supported tools detected. Install one of: Claude Code, Cursor, Windsurf, VS Code, Zed, Gemini CLI, Codex CLI");
    }

    Ok(())
}

pub fn run_uninstall(dry_run: bool) -> Result<()> {
    println!("shire uninstall");
    println!();

    // Claude Code
    if let Some(claude_path) = find_cli("claude") {
        println!("[Claude Code] detected ({})", claude_path.display());
        if dry_run {
            println!("  [dry-run] Would run: claude mcp remove -s user shire");
        } else {
            let output = Command::new(&claude_path)
                .args(["mcp", "remove", "-s", "user", "shire"])
                .output();
            match output {
                Ok(o) if o.status.success() => println!("  Removed MCP registration"),
                Ok(o) => {
                    let err = String::from_utf8_lossy(&o.stderr);
                    println!("  Removal may have failed: {}", err.trim());
                }
                Err(e) => println!("  Removal failed: {}", e),
            }
        }
    }

    // Codex CLI
    remove_codex_mcp(dry_run);

    // JSON-based editors
    remove_editor_mcp("Cursor", &cursor_config_path(), "mcpServers", dry_run);
    remove_editor_mcp("Windsurf", &windsurf_config_path(), "mcpServers", dry_run);
    remove_editor_mcp("Gemini CLI", &gemini_config_path(), "mcpServers", dry_run);
    remove_editor_mcp("VS Code", &vscode_config_path(), "servers", dry_run);
    remove_editor_mcp("Zed", &zed_config_path(), "context_servers", dry_run);

    println!("\nDone. Binary and databases were NOT removed.");
    Ok(())
}

// --- Binary detection ---

fn detect_binary_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("Cannot detect binary path")?;
    fs::canonicalize(&exe).or(Ok(exe))
}

// --- CLI detection ---

fn find_cli(name: &str) -> Option<PathBuf> {
    if let Ok(p) = which::which(name) {
        return Some(p);
    }

    let home = home_dir().ok()?;
    let candidates = [
        PathBuf::from("/usr/local/bin").join(name),
        PathBuf::from("/opt/homebrew/bin").join(name),
        home.join(".npm/bin").join(name),
        home.join(".local/bin").join(name),
        home.join(".cargo/bin").join(name),
    ];

    candidates.into_iter().find(|c| c.exists())
}

fn home_dir() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .context("HOME environment variable not set")
}

// --- Claude Code ---

fn register_claude_code(binary_path: &Path, dry_run: bool, force: bool) -> Registration {
    let claude_path = match find_cli("claude") {
        Some(p) => p,
        None => {
            // Fall back to ~/.claude.json file patching
            return register_claude_code_file(binary_path, dry_run, force);
        }
    };

    println!("[Claude Code] detected ({})", claude_path.display());

    if dry_run {
        println!(
            "  [dry-run] Would run: claude mcp add --scope user shire -- {} serve --root .",
            binary_path.display()
        );
        return Registration {
            tool: "Claude Code",
            status: RegStatus::Registered("via CLI".into()),
        };
    }

    // Remove first (may not exist, that's OK)
    let _ = Command::new(&claude_path)
        .args(["mcp", "remove", "-s", "user", "shire"])
        .output();

    let output = Command::new(&claude_path)
        .args([
            "mcp",
            "add",
            "--scope",
            "user",
            "shire",
            "--",
            &binary_path.to_string_lossy(),
            "serve",
            "--root",
            ".",
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            println!("  MCP server registered (scope: user)");
            Registration {
                tool: "Claude Code",
                status: RegStatus::Registered("via claude mcp add".into()),
            }
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).to_string();
            println!("  MCP registration failed: {}", err.trim());
            Registration {
                tool: "Claude Code",
                status: RegStatus::Failed(err),
            }
        }
        Err(e) => {
            println!("  MCP registration failed: {}", e);
            Registration {
                tool: "Claude Code",
                status: RegStatus::Failed(e.to_string()),
            }
        }
    }
}

fn register_claude_code_file(binary_path: &Path, dry_run: bool, force: bool) -> Registration {
    let home = match home_dir() {
        Ok(h) => h,
        Err(_) => {
            return Registration {
                tool: "Claude Code",
                status: RegStatus::NotFound,
            }
        }
    };

    let claude_json = home.join(".claude.json");
    println!("[Claude Code] CLI not found, patching ~/.claude.json");

    if dry_run {
        println!("  [dry-run] Would upsert shire in ~/.claude.json");
        return Registration {
            tool: "Claude Code",
            status: RegStatus::Registered("~/.claude.json".into()),
        };
    }

    let entry = json!({
        "command": binary_path.to_string_lossy(),
        "args": ["serve", "--root", "."],
    });

    match upsert_json_mcp(&claude_json, "mcpServers", "shire", entry, force) {
        Ok(UpsertResult::Created) => {
            println!("  Added mcpServers.shire to ~/.claude.json");
            Registration {
                tool: "Claude Code",
                status: RegStatus::Registered("~/.claude.json".into()),
            }
        }
        Ok(UpsertResult::Updated) => {
            println!("  Updated mcpServers.shire in ~/.claude.json");
            Registration {
                tool: "Claude Code",
                status: RegStatus::Updated("~/.claude.json".into()),
            }
        }
        Ok(UpsertResult::AlreadyExists) => {
            println!("  mcpServers.shire already configured in ~/.claude.json");
            Registration {
                tool: "Claude Code",
                status: RegStatus::AlreadyRegistered("~/.claude.json".into()),
            }
        }
        Err(e) => Registration {
            tool: "Claude Code",
            status: RegStatus::Failed(e.to_string()),
        },
    }
}

// --- Codex CLI ---

fn register_codex(binary_path: &Path, dry_run: bool, force: bool) -> Registration {
    let codex_path = find_cli("codex");
    if codex_path.is_none() {
        return Registration {
            tool: "Codex CLI",
            status: RegStatus::NotFound,
        };
    }

    let home = match home_dir() {
        Ok(h) => h,
        Err(e) => {
            return Registration {
                tool: "Codex CLI",
                status: RegStatus::Failed(e.to_string()),
            }
        }
    };

    let config_file = home.join(".codex/config.toml");
    println!(
        "[Codex CLI] detected ({})",
        codex_path.unwrap().display()
    );

    if dry_run {
        println!("  [dry-run] Would add MCP server to {}", config_file.display());
        return Registration {
            tool: "Codex CLI",
            status: RegStatus::Registered(config_file.display().to_string()),
        };
    }

    match upsert_codex_toml(&config_file, binary_path, force) {
        Ok(UpsertResult::Created) => {
            let display = config_file.display().to_string();
            println!("  MCP server registered: {}", display);
            Registration {
                tool: "Codex CLI",
                status: RegStatus::Registered(display),
            }
        }
        Ok(UpsertResult::Updated) => {
            let display = config_file.display().to_string();
            println!("  MCP server updated: {}", display);
            Registration {
                tool: "Codex CLI",
                status: RegStatus::Updated(display),
            }
        }
        Ok(UpsertResult::AlreadyExists) => {
            println!("  MCP server already configured in {}", config_file.display());
            Registration {
                tool: "Codex CLI",
                status: RegStatus::AlreadyRegistered(config_file.display().to_string()),
            }
        }
        Err(e) => Registration {
            tool: "Codex CLI",
            status: RegStatus::Failed(e.to_string()),
        },
    }
}

fn upsert_codex_toml(config_file: &Path, binary_path: &Path, force: bool) -> Result<UpsertResult> {
    let content = fs::read_to_string(config_file).unwrap_or_default();
    let mut doc: toml::Value = if content.is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", config_file.display()))?
    };

    let root = doc.as_table_mut().context("TOML root is not a table")?;

    // Ensure mcp_servers table exists
    if !root.contains_key("mcp_servers") {
        root.insert("mcp_servers".into(), toml::Value::Table(toml::map::Map::new()));
    }
    let mcp_servers = root
        .get_mut("mcp_servers")
        .and_then(|v| v.as_table_mut())
        .context("mcp_servers is not a table")?;

    let is_update = mcp_servers.contains_key("shire");
    if is_update && !force {
        return Ok(UpsertResult::AlreadyExists);
    }

    let mut shire_entry = toml::map::Map::new();
    shire_entry.insert("command".into(), toml::Value::String(binary_path.to_string_lossy().into()));
    shire_entry.insert(
        "args".into(),
        toml::Value::Array(vec![
            toml::Value::String("serve".into()),
            toml::Value::String("--root".into()),
            toml::Value::String(".".into()),
        ]),
    );
    mcp_servers.insert("shire".into(), toml::Value::Table(shire_entry));

    // Ensure directory exists
    if let Some(parent) = config_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    let output = toml::to_string_pretty(&doc).context("Failed to serialize TOML")?;

    // Atomic write via temp file
    let tmp_path = config_file.with_extension("toml.tmp");
    fs::write(&tmp_path, &output)
        .with_context(|| format!("Failed to write {}", tmp_path.display()))?;
    if let Err(e) = fs::rename(&tmp_path, config_file) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e).with_context(|| {
            format!("Failed to rename {} to {}", tmp_path.display(), config_file.display())
        });
    }

    Ok(if is_update {
        UpsertResult::Updated
    } else {
        UpsertResult::Created
    })
}

fn remove_codex_mcp(dry_run: bool) {
    let home = match home_dir() {
        Ok(h) => h,
        Err(_) => return,
    };
    let config_file = home.join(".codex/config.toml");
    let content = match fs::read_to_string(&config_file) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut doc: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };

    let has_shire = doc
        .get("mcp_servers")
        .and_then(|s| s.get("shire"))
        .is_some();

    if !has_shire {
        return;
    }

    println!("[Codex CLI] config: {}", config_file.display());
    if dry_run {
        println!("  [dry-run] Would remove MCP section");
        return;
    }

    if let Some(mcp_servers) = doc.get_mut("mcp_servers").and_then(|s| s.as_table_mut()) {
        mcp_servers.remove("shire");
        if mcp_servers.is_empty() {
            if let Some(root) = doc.as_table_mut() {
                root.remove("mcp_servers");
            }
        }
    }

    let output = toml::to_string_pretty(&doc).unwrap_or_default();
    let tmp_path = config_file.with_extension("toml.tmp");
    if fs::write(&tmp_path, &output).is_ok() {
        let _ = fs::rename(&tmp_path, &config_file);
    }
    println!("  Removed MCP section");
}

// --- Generic JSON-based editor MCP registration ---

fn register_editor_mcp(
    binary_path: &Path,
    tool_name: &'static str,
    config_path: &Option<PathBuf>,
    servers_key: &str,
    custom_entry: Option<Value>,
    dry_run: bool,
    force: bool,
) -> Registration {
    let config_path = match config_path {
        Some(p) => p,
        None => {
            return Registration {
                tool: tool_name,
                status: RegStatus::NotFound,
            }
        }
    };

    println!("[{}] config: {}", tool_name, config_path.display());

    let entry = custom_entry.unwrap_or_else(|| {
        json!({
            "command": binary_path.to_string_lossy(),
            "args": ["serve", "--root", "."],
        })
    });

    if dry_run {
        println!("  [dry-run] Would upsert shire in {}", config_path.display());
        return Registration {
            tool: tool_name,
            status: RegStatus::Registered(config_path.display().to_string()),
        };
    }

    match upsert_json_mcp(config_path, servers_key, "shire", entry, force) {
        Ok(UpsertResult::Created) => {
            println!("  MCP server registered");
            Registration {
                tool: tool_name,
                status: RegStatus::Registered(config_path.display().to_string()),
            }
        }
        Ok(UpsertResult::Updated) => {
            println!("  MCP server updated");
            Registration {
                tool: tool_name,
                status: RegStatus::Updated(config_path.display().to_string()),
            }
        }
        Ok(UpsertResult::AlreadyExists) => {
            println!("  MCP server already configured");
            Registration {
                tool: tool_name,
                status: RegStatus::AlreadyRegistered(config_path.display().to_string()),
            }
        }
        Err(e) => {
            println!("  Failed: {}", e);
            Registration {
                tool: tool_name,
                status: RegStatus::Failed(e.to_string()),
            }
        }
    }
}

fn remove_editor_mcp(
    tool_name: &str,
    config_path: &Option<PathBuf>,
    servers_key: &str,
    dry_run: bool,
) {
    let config_path = match config_path {
        Some(p) => p,
        None => return,
    };

    let data = match fs::read_to_string(config_path) {
        Ok(d) => d,
        Err(_) => return,
    };

    let mut root: Map<String, Value> = match serde_json::from_str(&data) {
        Ok(m) => m,
        Err(_) => return,
    };

    let servers = match root.get_mut(servers_key).and_then(|v| v.as_object_mut()) {
        Some(s) => s,
        None => return,
    };

    if !servers.contains_key("shire") {
        return;
    }

    println!("[{}] config: {}", tool_name, config_path.display());

    if dry_run {
        println!("  [dry-run] Would remove shire");
        return;
    }

    servers.remove("shire");
    if let Ok(out) = serde_json::to_string_pretty(&Value::Object(root)) {
        let tmp_path = config_path.with_extension("json.tmp");
        if fs::write(&tmp_path, format!("{}\n", out)).is_ok() {
            let _ = fs::rename(&tmp_path, config_path);
        }
        println!("  Removed shire");
    }
}

// --- Config paths ---

fn cursor_config_path() -> Option<PathBuf> {
    let home = home_dir().ok()?;
    Some(home.join(".cursor/mcp.json"))
}

fn windsurf_config_path() -> Option<PathBuf> {
    let home = home_dir().ok()?;
    Some(home.join(".codeium/windsurf/mcp_config.json"))
}

fn gemini_config_path() -> Option<PathBuf> {
    let home = home_dir().ok()?;
    Some(home.join(".gemini/settings.json"))
}

fn vscode_config_path() -> Option<PathBuf> {
    let home = home_dir().ok()?;
    #[cfg(target_os = "macos")]
    {
        Some(home.join("Library/Application Support/Code/User/mcp.json"))
    }
    #[cfg(target_os = "linux")]
    {
        Some(home.join(".config/Code/User/mcp.json"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

fn zed_config_path() -> Option<PathBuf> {
    let home = home_dir().ok()?;
    Some(home.join(".config/zed/settings.json"))
}

// --- JSON upsert helpers ---

enum UpsertResult {
    Created,
    Updated,
    AlreadyExists,
}

fn upsert_json_mcp(
    path: &Path,
    servers_key: &str,
    entry_name: &str,
    entry_value: Value,
    force: bool,
) -> Result<UpsertResult> {
    let mut root: Map<String, Value> = if path.exists() {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?
    } else {
        Map::new()
    };

    let servers = root
        .entry(servers_key)
        .or_insert_with(|| json!({}));

    let servers_obj = servers
        .as_object_mut()
        .context(format!("{} is not an object in {}", servers_key, path.display()))?;

    let is_update = servers_obj.contains_key(entry_name);
    if is_update && !force {
        return Ok(UpsertResult::AlreadyExists);
    }

    servers_obj.insert(entry_name.into(), entry_value);

    // Ensure parent dir exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    let output = serde_json::to_string_pretty(&Value::Object(root))
        .context("Failed to serialize JSON")?;

    // Atomic write via temp file
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, format!("{}\n", output))
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

    Ok(if is_update {
        UpsertResult::Updated
    } else {
        UpsertResult::Created
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_upsert_creates_new_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mcp.json");

        let result = upsert_json_mcp(
            &path,
            "mcpServers",
            "shire",
            json!({"command": "shire", "args": ["serve"]}),
            false,
        )
        .unwrap();
        assert!(matches!(result, UpsertResult::Created));

        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["mcpServers"]["shire"]["command"], "shire");
    }

    #[test]
    fn test_upsert_preserves_existing_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mcp.json");
        let existing = json!({"mcpServers": {"other": {"command": "other"}}});
        fs::write(&path, serde_json::to_string(&existing).unwrap()).unwrap();

        upsert_json_mcp(
            &path,
            "mcpServers",
            "shire",
            json!({"command": "shire"}),
            false,
        )
        .unwrap();

        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(parsed["mcpServers"]["other"].is_object());
        assert!(parsed["mcpServers"]["shire"].is_object());
    }

    #[test]
    fn test_upsert_idempotent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mcp.json");

        upsert_json_mcp(&path, "mcpServers", "shire", json!({"command": "shire"}), false).unwrap();

        let result =
            upsert_json_mcp(&path, "mcpServers", "shire", json!({"command": "shire2"}), false).unwrap();
        assert!(matches!(result, UpsertResult::AlreadyExists));

        // Original value preserved
        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["mcpServers"]["shire"]["command"], "shire");
    }

    #[test]
    fn test_upsert_different_servers_key() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");

        upsert_json_mcp(
            &path,
            "context_servers",
            "shire",
            json!({"source": "custom", "command": "shire"}),
            false,
        )
        .unwrap();

        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["context_servers"]["shire"]["source"], "custom");
    }

    #[test]
    fn test_upsert_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested/dir/mcp.json");

        upsert_json_mcp(&path, "mcpServers", "shire", json!({"command": "shire"}), false).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_upsert_errors_on_non_object_servers() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mcp.json");
        fs::write(&path, json!({"mcpServers": "broken"}).to_string()).unwrap();

        let result = upsert_json_mcp(&path, "mcpServers", "shire", json!({"command": "shire"}), false);
        assert!(result.is_err());
    }

    #[test]
    fn test_upsert_force_overwrites() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mcp.json");

        upsert_json_mcp(&path, "mcpServers", "shire", json!({"command": "/old/shire"}), false).unwrap();

        let result =
            upsert_json_mcp(&path, "mcpServers", "shire", json!({"command": "/new/shire"}), true).unwrap();
        assert!(matches!(result, UpsertResult::Updated));

        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["mcpServers"]["shire"]["command"], "/new/shire");
    }

    #[test]
    fn test_upsert_force_preserves_other_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mcp.json");
        let existing = json!({"mcpServers": {"shire": {"command": "/old"}, "other": {"command": "other"}}});
        fs::write(&path, serde_json::to_string(&existing).unwrap()).unwrap();

        upsert_json_mcp(&path, "mcpServers", "shire", json!({"command": "/new"}), true).unwrap();

        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["mcpServers"]["shire"]["command"], "/new");
        assert_eq!(parsed["mcpServers"]["other"]["command"], "other");
    }

    #[test]
    fn test_remove_editor_mcp_removes_entry() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mcp.json");
        let existing = json!({"mcpServers": {"shire": {"command": "shire"}, "other": {"command": "other"}}});
        fs::write(&path, serde_json::to_string_pretty(&existing).unwrap()).unwrap();

        remove_editor_mcp("Test", &Some(path.clone()), "mcpServers", false);

        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(!parsed["mcpServers"].as_object().unwrap().contains_key("shire"));
        assert!(parsed["mcpServers"]["other"].is_object());
    }

    #[test]
    fn test_upsert_codex_toml_creates_new_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".codex/config.toml");

        let result = upsert_codex_toml(&path, Path::new("/usr/local/bin/shire"), false).unwrap();
        assert!(matches!(result, UpsertResult::Created));

        let content = fs::read_to_string(&path).unwrap();
        let parsed: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(
            parsed["mcp_servers"]["shire"]["command"].as_str().unwrap(),
            "/usr/local/bin/shire"
        );
    }

    #[test]
    fn test_upsert_codex_toml_idempotent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        upsert_codex_toml(&path, Path::new("/usr/local/bin/shire"), false).unwrap();
        let result = upsert_codex_toml(&path, Path::new("/new/shire"), false).unwrap();
        assert!(matches!(result, UpsertResult::AlreadyExists));

        let content = fs::read_to_string(&path).unwrap();
        let parsed: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(
            parsed["mcp_servers"]["shire"]["command"].as_str().unwrap(),
            "/usr/local/bin/shire"
        );
    }

    #[test]
    fn test_upsert_codex_toml_force_overwrites() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        upsert_codex_toml(&path, Path::new("/old/shire"), false).unwrap();
        let result = upsert_codex_toml(&path, Path::new("/new/shire"), true).unwrap();
        assert!(matches!(result, UpsertResult::Updated));

        let content = fs::read_to_string(&path).unwrap();
        let parsed: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(
            parsed["mcp_servers"]["shire"]["command"].as_str().unwrap(),
            "/new/shire"
        );
    }

    #[test]
    fn test_upsert_codex_toml_preserves_other_config() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "model = \"o3\"\n\n[mcp_servers.other]\ncommand = \"other\"\n").unwrap();

        upsert_codex_toml(&path, Path::new("/usr/local/bin/shire"), false).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let parsed: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(parsed["model"].as_str().unwrap(), "o3");
        assert_eq!(
            parsed["mcp_servers"]["other"]["command"].as_str().unwrap(),
            "other"
        );
        assert_eq!(
            parsed["mcp_servers"]["shire"]["command"].as_str().unwrap(),
            "/usr/local/bin/shire"
        );
    }

    #[test]
    fn test_remove_editor_mcp_noop_when_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mcp.json");
        let existing = json!({"mcpServers": {"other": {"command": "other"}}});
        fs::write(&path, serde_json::to_string_pretty(&existing).unwrap()).unwrap();

        remove_editor_mcp("Test", &Some(path.clone()), "mcpServers", false);

        let parsed: Map<String, Value> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(parsed["mcpServers"]["other"].is_object());
    }
}
