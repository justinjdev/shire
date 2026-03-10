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

/// Installs shire MCP entries for supported CLIs and editors.
///
/// Attempts to register the current binary with supported tools (Claude Code, Codex CLI,
/// Cursor, Windsurf, Gemini CLI, VS Code, and Zed), printing progress and a concise summary.
///
/// Parameters:
/// - `dry_run`: when true, print planned actions without modifying files or invoking CLIs.
/// - `force`: when true, overwrite existing registrations when possible.
///
/// # Returns
///
/// `Ok(())` on success; an error indicates a failure to detect the current binary or to perform
/// filesystem/CLI operations required for installation.
///
/// # Examples
///
/// ```
/// // Run in dry-run mode to preview actions without making changes.
/// run_install(true, false).unwrap();
/// ```
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

/// Uninstalls MCP registrations previously added by shire.
///
/// When `dry_run` is true, this reports the removals that would be performed without modifying any files
/// or invoking external CLIs. Otherwise it attempts to remove registrations for supported tools:
/// Claude Code (via the `claude` CLI if found), Codex CLI, and JSON-based editor configs (Cursor,
/// Windsurf, Gemini CLI, VS Code, Zed).
///
/// # Parameters
///
/// - `dry_run`: If `true`, show planned removal actions without performing them.
///
/// # Returns
///
/// `Ok(())` on success, or an `Err` containing the underlying failure.
///
/// # Examples
///
/// ```
/// # use anyhow::Result;
/// # fn example() -> Result<()> {
/// // Show what would be removed without making changes
/// run_uninstall(true)?;
/// # Ok(())
/// # }
/// ```
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

/// Determine the filesystem path of the currently running executable.
///
/// Attempts to canonicalize the path returned by `std::env::current_exe()`. If canonicalization fails,
/// the original (non-canonical) executable path is returned.
///
/// # Errors
///
/// Returns an error if the current executable path cannot be determined.
///
/// # Examples
///
/// ```
/// // This example shows basic usage; in tests call from the same crate/module scope.
/// let path = crate::detect_binary_path().unwrap();
/// assert!(path.is_absolute() || path.exists());
/// ```
fn detect_binary_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("Cannot detect binary path")?;
    fs::canonicalize(&exe).or(Ok(exe))
}

// --- CLI detection ---

/// Locates an executable by name by checking the system PATH and common user/local install locations.
///
/// First attempts to find `name` using the system PATH. If that fails, checks these candidate locations:
/// `/usr/local/bin`, `/opt/homebrew/bin`, and the user's `~/.npm/bin`, `~/.local/bin`, and `~/.cargo/bin`.
///
/// # Examples
///
/// ```
/// use std::path::PathBuf;
/// // Attempt to find an executable; handle presence or absence.
/// match find_cli("example-cli") {
///     Some(path) => println!("Found at {}", path.display()),
///     None => println!("Not found"),
/// }
/// ```
///
/// # Returns
///
/// `Some(PathBuf)` with the resolved path to the executable if found, `None` otherwise.
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

/// Returns the current user's home directory as a PathBuf by reading the `HOME` environment variable.
///
/// # Errors
///
/// Returns an error with context "HOME environment variable not set" if the `HOME` environment variable is missing.
///
/// # Examples
///
/// ```
/// use std::path::PathBuf;
/// std::env::set_var("HOME", "/tmp/example_home");
/// let home = crate::home_dir().unwrap();
/// assert_eq!(home, PathBuf::from("/tmp/example_home"));
/// ```
fn home_dir() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .context("HOME environment variable not set")
}

// --- Claude Code ---

/// Registers the current binary as an MCP server for Claude Code.
///
/// Attempts to use the `claude` CLI to add a user-scoped MCP entry pointing to `binary_path`.
/// If the `claude` CLI is not found, falls back to updating the user's `~/.claude.json` via
/// `register_claude_code_file`. In dry-run mode, prints the planned command without executing it.
///
/// # Returns
///
/// A `Registration` describing the outcome of the registration attempt for Claude Code.
///
/// # Examples
///
/// ```
/// let reg = register_claude_code(std::path::Path::new("/usr/local/bin/shire"), true, false);
/// assert_eq!(reg.tool, "Claude Code");
/// ```
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

/// Inserts or updates a "shire" MCP entry in the user's ~/.claude.json to point at the provided binary.
///
/// If the user's home directory cannot be determined this returns a `Registration` with `RegStatus::NotFound`.
/// In dry-run mode the function reports the planned change and returns a `Registration` indicating the target path without writing.
/// The `force` flag controls whether an existing "shire" entry will be overwritten.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// // Dry-run example: does not modify the filesystem
/// let _ = crate::register_claude_code_file(Path::new("/usr/bin/shire"), true, false);
/// ```
///
/// # Returns
///
/// A `Registration` describing the result: created, updated, already registered, not found, or failed.
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

/// Register or update a "shire" MCP entry in the user's Codex CLI config.
///
/// Attempts to locate the `codex` CLI and then ensure the file `~/.codex/config.toml` contains an
/// [mcp_servers.shire] entry that invokes this binary with `serve --root .`. If the Codex CLI
/// executable is not found, returns `RegStatus::NotFound`. When `dry_run` is true the function
/// reports the intended config path without making changes. The `force` flag controls whether an
/// existing `shire` entry is overwritten.
///
/// # Examples
///
/// ```
/// // Example usage (may return NotFound if `codex` is not installed on the system):
/// use std::path::Path;
/// // call with dry_run=true to see planned changes without modifying files:
/// let _ = register_codex(Path::new("/path/to/binary"), true, false);
/// ```
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

/// Inserts or updates a `mcp_servers.shire` entry in a Codex TOML config file.
///
/// Creates the `mcp_servers` table if it does not exist, writes the `shire` table with
/// a `command` pointing to `binary_path` and `args = ["serve", "--root", "."]`,
/// and performs an atomic on-disk write (writes to a temporary file then renames).
/// If a `shire` entry already exists and `force` is false, the function returns `UpsertResult::AlreadyExists`.
/// Parent directories for `config_file` are created as needed.
///
/// # Returns
///
/// `UpsertResult::Created` if the `shire` entry was newly created, `UpsertResult::Updated` if an existing entry
/// was replaced, or `UpsertResult::AlreadyExists` if an entry existed and `force` was false.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// // Assume `upsert_codex_toml` and `UpsertResult` are available in scope.
/// let config = Path::new("/tmp/example_codex_config.toml");
/// let bin = Path::new("/usr/local/bin/shire-binary");
/// let res = upsert_codex_toml(config, bin, false).unwrap();
/// assert!(matches!(res, UpsertResult::Created | UpsertResult::Updated | UpsertResult::AlreadyExists));
/// ```
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

/// Remove the "shire" MCP entry from the Codex CLI configuration (~/.codex/config.toml) if it exists.
///
/// If the home directory cannot be determined, the config file cannot be read, or the file is not valid TOML,
/// the function returns without making changes. When `dry_run` is true, the function prints the planned removal
/// but does not modify any files. When removal occurs, the config is written atomically via a temporary file
/// and renamed into place; if the containing `mcp_servers` table becomes empty it is removed as well.
///
/// # Examples
///
/// ```
/// // Print what would be done without changing files.
/// remove_codex_mcp(true);
/// ```
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

/// Upserts an MCP "shire" entry into an editor's JSON configuration and returns a Registration describing the outcome.
///
/// If `config_path` is None, returns a `Registration` with `RegStatus::NotFound`. If `dry_run` is true, reports the planned upsert and returns `RegStatus::Registered` with the config path string. Uses `custom_entry` when provided; otherwise uses a default entry that runs the given `binary_path` with `serve --root .`. Calls `upsert_json_mcp` and maps its result to `RegStatus::Registered`/`Updated`/`AlreadyRegistered`, or `RegStatus::Failed` on error.
///
/// # Examples
///
/// ```
/// use std::path::{Path, PathBuf};
/// // assumes Registration and RegStatus are in scope
/// let cfg = Some(PathBuf::from("/tmp/mcp.json"));
/// let reg = register_editor_mcp(
///     Path::new("/usr/local/bin/shire"),
///     "Cursor",
///     &cfg,
///     "mcpServers",
///     None,
///     true,  // dry_run
///     false, // force
/// );
/// match reg.status {
///     RegStatus::Registered(p) => assert!(p.contains("/tmp/mcp.json")),
///     _ => panic!("expected Registered on dry-run"),
/// }
/// ```
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

/// Remove the "shire" MCP entry from an editor JSON configuration if present.
///
/// If `config_path` is `None` or the file cannot be read/parsed, the function returns
/// silently. If the specified `servers_key` does not contain an object or does not
/// have a `"shire"` entry, the function does nothing. When `dry_run` is true, the
/// function only prints what it would remove without writing changes.
///
/// # Parameters
///
/// - `tool_name`: human-readable tool identifier used in printed messages.
/// - `config_path`: path to the editor's JSON config file; if `None`, the function is a no-op.
/// - `servers_key`: top-level object key that holds server entries (e.g., "mcpServers").
/// - `dry_run`: when true, print the intended removal but do not modify the file.
///
/// # Examples
///
/// ```
/// // Best-effort example: calling with `None` is a safe no-op.
/// remove_editor_mcp("example", &None, "servers", true);
/// ```
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

/// Returns the expected path to Cursor's MCP configuration file inside the current user's home directory.
///
/// If the HOME environment variable cannot be determined, returns `None`.
///
/// # Examples
///
/// ```
/// if let Some(path) = cursor_config_path() {
///     assert!(path.ends_with(".cursor/mcp.json"));
/// }
/// ```
fn cursor_config_path() -> Option<PathBuf> {
    let home = home_dir().ok()?;
    Some(home.join(".cursor/mcp.json"))
}

/// Locate the Windsurf MCP JSON config file under the current user's home directory.
///
/// Returns `Some(PathBuf)` pointing to `HOME/.codeium/windsurf/mcp_config.json` when the `HOME`
/// environment variable is available, or `None` if the home directory cannot be determined.
///
/// # Examples
///
/// ```
/// let p = windsurf_config_path();
/// // If HOME is unset this will be `None`; otherwise the path ends with the expected suffix.
/// assert!(p.map_or(true, |pb| pb.ends_with(".codeium/windsurf/mcp_config.json")));
/// ```
fn windsurf_config_path() -> Option<PathBuf> {
    let home = home_dir().ok()?;
    Some(home.join(".codeium/windsurf/mcp_config.json"))
}

/// Get the path to the Gemini settings JSON in the user's home directory if available.
///
/// Returns `Some(PathBuf)` with the path to `.gemini/settings.json` inside the user's HOME
/// when the HOME environment variable is set and can be resolved, or `None` if HOME is unset.
///
/// # Examples
///
/// ```
/// if let Some(path) = gemini_config_path() {
///     assert!(path.ends_with(".gemini/settings.json"));
/// } else {
///     // HOME not set in this environment
/// }
/// ```
fn gemini_config_path() -> Option<PathBuf> {
    let home = home_dir().ok()?;
    Some(home.join(".gemini/settings.json"))
}

/// Returns the platform-specific path to VS Code's MCP configuration file.
///
/// On macOS this is `HOME/Library/Application Support/Code/User/mcp.json`.
/// On Linux this is `HOME/.config/Code/User/mcp.json`.
/// On other platforms returns `None`.
///
/// # Examples
///
/// ```
/// if let Some(path) = vscode_config_path() {
///     assert!(path.ends_with("mcp.json"));
/// }
/// ```
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

/// Locate the Zed editor MCP configuration file inside the current user's home directory.
///
/// Returns `None` if the `HOME` environment variable is not set.
///
/// # Examples
///
/// ```
/// if let Some(path) = zed_config_path() {
///     assert_eq!(path.file_name().and_then(|s| s.to_str()), Some("settings.json"));
/// }
/// ```
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

/// Inserts or updates a named entry under a JSON object key and writes the file atomically.
///
/// If the file does not exist, a new JSON object is created. Ensures `servers_key` exists
/// as an object and inserts `entry_value` under `entry_name`. If an entry already exists
/// and `force` is false, the function returns `UpsertResult::AlreadyExists`. Writes are
/// performed atomically by writing to a temporary file and renaming it into place.
///
/// # Examples
///
/// ```
/// use serde_json::json;
/// use std::path::Path;
/// // create a temp path for demonstration (in real code prefer tempfile crate)
/// let path = Path::new("/tmp/shire_example_mcp.json");
/// // ensure file is removed after example run (ignore errors)
/// let _ = std::fs::remove_file(path);
///
/// let entry = json!({
///     "command": path.to_string_lossy(),
///     "args": ["serve", "--root", "."]
/// });
///
/// // first call should create the file
/// let res = crate::install::upsert_json_mcp(path, "mcpServers", "shire", entry.clone(), false)
///     .expect("upsert failed");
/// assert!(matches!(res, crate::install::UpsertResult::Created));
///
/// // calling again without force returns AlreadyExists
/// let res2 = crate::install::upsert_json_mcp(path, "mcpServers", "shire", entry, false)
///     .expect("upsert failed");
/// assert!(matches!(res2, crate::install::UpsertResult::AlreadyExists));
/// ```
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
