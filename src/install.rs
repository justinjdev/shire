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
    AlreadyRegistered(String),
    NotFound,
    Failed(String),
}

pub fn run_install(dry_run: bool) -> Result<()> {
    let binary_path = detect_binary_path()?;

    println!("shire install");
    println!("Binary: {}", binary_path.display());
    println!();

    let mut results = Vec::new();

    // Claude Code — use `claude mcp add` CLI if available
    results.push(register_claude_code(&binary_path, dry_run));

    // Codex CLI — ~/.codex/config.toml
    results.push(register_codex(&binary_path, dry_run));

    // Cursor — ~/.cursor/mcp.json
    results.push(register_editor_mcp(
        &binary_path,
        "Cursor",
        &cursor_config_path(),
        "mcpServers",
        None,
        dry_run,
    ));

    // Windsurf — ~/.codeium/windsurf/mcp_config.json
    results.push(register_editor_mcp(
        &binary_path,
        "Windsurf",
        &windsurf_config_path(),
        "mcpServers",
        None,
        dry_run,
    ));

    // Gemini CLI — ~/.gemini/settings.json
    results.push(register_editor_mcp(
        &binary_path,
        "Gemini CLI",
        &gemini_config_path(),
        "mcpServers",
        None,
        dry_run,
    ));

    // VS Code — ~/Library/Application Support/Code/User/mcp.json
    results.push(register_editor_mcp(
        &binary_path,
        "VS Code",
        &vscode_config_path(),
        "servers",
        Some(json!({"type": "stdio", "command": binary_path.to_string_lossy()})),
        dry_run,
    ));

    // Zed — ~/.config/zed/settings.json
    results.push(register_editor_mcp(
        &binary_path,
        "Zed",
        &zed_config_path(),
        "context_servers",
        Some(json!({"source": "custom", "command": binary_path.to_string_lossy()})),
        dry_run,
    ));

    // Summary
    println!();
    println!("Summary:");
    for r in &results {
        match &r.status {
            RegStatus::Registered(path) => println!("  + {} — registered ({})", r.tool, path),
            RegStatus::AlreadyRegistered(path) => {
                println!("  = {} — already registered ({})", r.tool, path)
            }
            RegStatus::NotFound => println!("  - {} — not found, skipped", r.tool),
            RegStatus::Failed(err) => println!("  ! {} — failed: {}", r.tool, err),
        }
    }

    let registered = results
        .iter()
        .filter(|r| matches!(r.status, RegStatus::Registered(_) | RegStatus::AlreadyRegistered(_)))
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
            let _ = Command::new(&claude_path)
                .args(["mcp", "remove", "-s", "user", "shire"])
                .output();
            println!("  Removed MCP registration");
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

fn register_claude_code(binary_path: &Path, dry_run: bool) -> Registration {
    let claude_path = match find_cli("claude") {
        Some(p) => p,
        None => {
            // Fall back to ~/.claude.json file patching
            return register_claude_code_file(binary_path, dry_run);
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

fn register_claude_code_file(binary_path: &Path, dry_run: bool) -> Registration {
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

    match upsert_json_mcp(
        &claude_json,
        "mcpServers",
        "shire",
        json!({
            "command": binary_path.to_string_lossy(),
            "args": ["serve", "--root", "."],
        }),
    ) {
        Ok(UpsertResult::Created) => {
            println!("  Added mcpServers.shire to ~/.claude.json");
            Registration {
                tool: "Claude Code",
                status: RegStatus::Registered("~/.claude.json".into()),
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

fn register_codex(binary_path: &Path, dry_run: bool) -> Registration {
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

    let section_header = "[mcp_servers.shire]";
    let mcp_section = format!(
        "\n{}\ncommand = \"{}\"\nargs = [\"serve\", \"--root\", \".\"]\n",
        section_header,
        binary_path.display()
    );

    if dry_run {
        println!("  [dry-run] Would add MCP server to {}", config_file.display());
        return Registration {
            tool: "Codex CLI",
            status: RegStatus::Registered(config_file.display().to_string()),
        };
    }

    // Read existing or empty
    let content = fs::read_to_string(&config_file).unwrap_or_default();

    if content.contains(section_header) {
        println!("  MCP server already configured in {}", config_file.display());
        return Registration {
            tool: "Codex CLI",
            status: RegStatus::AlreadyRegistered(config_file.display().to_string()),
        };
    }

    // Ensure directory exists
    if let Some(parent) = config_file.parent() {
        let _ = fs::create_dir_all(parent);
    }

    match fs::write(&config_file, format!("{}{}", content, mcp_section)) {
        Ok(()) => {
            println!("  MCP server registered: {}", config_file.display());
            Registration {
                tool: "Codex CLI",
                status: RegStatus::Registered(config_file.display().to_string()),
            }
        }
        Err(e) => Registration {
            tool: "Codex CLI",
            status: RegStatus::Failed(e.to_string()),
        },
    }
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

    let section_header = "[mcp_servers.shire]";
    if let Some(idx) = content.find(section_header) {
        println!("[Codex CLI] config: {}", config_file.display());
        if dry_run {
            println!("  [dry-run] Would remove MCP section");
            return;
        }

        let rest = &content[idx + section_header.len()..];
        let end_idx = rest.find("\n[").map(|i| idx + section_header.len() + i + 1);
        let new_content = match end_idx {
            Some(end) => format!(
                "{}{}",
                content[..idx].trim_end_matches('\n'),
                &content[end..]
            ),
            None => content[..idx].trim_end_matches('\n').to_string(),
        };

        let _ = fs::write(&config_file, new_content);
        println!("  Removed MCP section");
    }
}

// --- Generic JSON-based editor MCP registration ---

fn register_editor_mcp(
    binary_path: &Path,
    tool_name: &'static str,
    config_path: &Option<PathBuf>,
    servers_key: &str,
    custom_entry: Option<Value>,
    dry_run: bool,
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

    match upsert_json_mcp(config_path, servers_key, "shire", entry) {
        Ok(UpsertResult::Created) => {
            println!("  MCP server registered");
            Registration {
                tool: tool_name,
                status: RegStatus::Registered(config_path.display().to_string()),
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
        let _ = fs::write(config_path, format!("{}\n", out));
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
    AlreadyExists,
}

fn upsert_json_mcp(
    path: &Path,
    servers_key: &str,
    entry_name: &str,
    entry_value: Value,
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

    if servers_obj.contains_key(entry_name) {
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

    Ok(UpsertResult::Created)
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

        upsert_json_mcp(&path, "mcpServers", "shire", json!({"command": "shire"})).unwrap();

        let result =
            upsert_json_mcp(&path, "mcpServers", "shire", json!({"command": "shire2"})).unwrap();
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

        upsert_json_mcp(&path, "mcpServers", "shire", json!({"command": "shire"})).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_upsert_errors_on_non_object_servers() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mcp.json");
        fs::write(&path, json!({"mcpServers": "broken"}).to_string()).unwrap();

        let result = upsert_json_mcp(&path, "mcpServers", "shire", json!({"command": "shire"}));
        assert!(result.is_err());
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
