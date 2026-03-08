use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod config;
mod db;
mod git;
mod index;
mod init;
mod mcp;
mod rag;
mod symbols;
mod watch;

#[derive(Parser)]
#[command(name = "shire", about = "Monorepo package index and MCP server")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan the repository and build the package index
    Build {
        /// Root directory of the repository (defaults to current directory)
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Force a full rebuild, ignoring cached manifest hashes
        #[arg(long)]
        force: bool,
        /// Path to the index database (overrides shire.toml db_path)
        #[arg(long)]
        db: Option<PathBuf>,
        /// Path to config file (defaults to <root>/shire.toml)
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Start the MCP server over stdio
    Serve {
        /// Repository root for on-demand reindexing (enables auto-rebuild before queries)
        #[arg(long)]
        root: Option<PathBuf>,
        /// Path to the index database (defaults to .shire/index.db)
        #[arg(long)]
        db: Option<PathBuf>,
        /// Path to config file (defaults to ./shire.toml)
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Start the watch daemon for automatic index rebuilds
    Watch {
        /// Root directory of the repository (defaults to current directory)
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Stop the running daemon
        #[arg(long)]
        stop: bool,
        /// Run in foreground (used internally by the daemon)
        #[arg(long, hide = true)]
        foreground: bool,
        /// Path to the index database (overrides shire.toml db_path)
        #[arg(long)]
        db: Option<PathBuf>,
        /// Path to config file (defaults to <root>/shire.toml)
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Signal the watch daemon to rebuild the index
    Rebuild {
        /// Root directory of the repository (defaults to current directory)
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Specific file that changed (can be repeated)
        #[arg(long)]
        file: Vec<PathBuf>,
        /// Read Claude Code hook JSON from stdin to extract the changed file
        #[arg(long)]
        stdin: bool,
    },
    /// Initialize shire configuration
    Init {
        /// Root directory for the project config (defaults to current directory)
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Set up global config in ~/.claude/ for all projects
        #[arg(long)]
        global: bool,
        /// Use on-demand reindexing instead of PostToolUse hooks
        #[arg(long)]
        no_hook: bool,
        /// Skip interactive prompts and use defaults
        #[arg(long, short)]
        yes: bool,
    },
    /// Remove the index database and all shire artifacts for a project
    Clean {
        /// Root directory of the repository (defaults to current directory)
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Path to the index database (overrides shire.toml db_path)
        #[arg(long)]
        db: Option<PathBuf>,
        /// Path to config file (defaults to <root>/shire.toml)
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    #[cfg(feature = "rag")]
    if let Err(e) = rag::storage::load_extension() {
        eprintln!("Warning: {e}");
    }

    match cli.command {
        Commands::Build { root, force, db, config: cfg_path } => {
            let root = std::fs::canonicalize(&root)?;
            let config = config::load_config_from(cfg_path.as_deref(), &root)?;
            index::build_index(&root, &config, force, db.as_deref())
        }
        Commands::Serve { root, db, config: cfg_path } => {
            let cwd = std::fs::canonicalize(".")?;
            let repo_root = root.as_ref().map(|r| std::fs::canonicalize(r)).transpose()?;
            let effective_root = repo_root.as_deref().unwrap_or(&cwd);
            let cfg = config::load_config_from(cfg_path.as_deref(), effective_root)?;
            let db_path = if let Some(p) = db {
                p
            } else {
                config::resolve_db_path(&cfg, effective_root)?
            };
            let build_ctx = if let Some(ref repo_root) = repo_root {
                // On-demand mode: allow missing DB (first tool call triggers build)
                Some(mcp::BuildContext {
                    repo_root: repo_root.clone(),
                    config: cfg.clone(),
                    db_path: db_path.clone(),
                })
            } else {
                // Read-only mode: DB must exist
                if !db_path.exists() {
                    anyhow::bail!(
                        "Index not found at {}. Run `shire build` first.",
                        db_path.display()
                    );
                }
                None
            };
            mcp::run_server(&db_path, &cfg.rag, build_ctx).await
        }
        Commands::Watch {
            root,
            stop,
            foreground,
            db,
            config: cfg_path,
        } => {
            let root = std::fs::canonicalize(&root)?;
            if stop {
                watch::daemon::stop_daemon(&root)
            } else if foreground {
                let config = config::load_config_from(cfg_path.as_deref(), &root)?;
                watch::run_daemon(root, config, db).await
            } else {
                watch::daemon::start_daemon(&root, db.as_deref(), cfg_path.as_deref())
            }
        }
        Commands::Init { root, global, no_hook, yes } => {
            if global {
                init::run_init_global(no_hook, yes)
            } else {
                std::fs::create_dir_all(&root)
                    .with_context(|| format!("Failed to create directory {}", root.display()))?;
                let root = std::fs::canonicalize(&root)
                    .with_context(|| format!("Failed to resolve path {}", root.display()))?;
                init::run_init(&root, no_hook, yes)
            }
        }
        Commands::Clean { root, db, config: cfg_path } => {
            let root = std::fs::canonicalize(&root)?;
            let config = config::load_config_from(cfg_path.as_deref(), &root)?;

            // Stop the watch daemon if running
            if watch::daemon::is_running(&root) {
                eprintln!("Stopping watch daemon...");
                watch::daemon::stop_daemon(&root)?;
            }

            // Resolve and remove the database file
            let db_path = if let Some(p) = db {
                p
            } else {
                config::resolve_db_path(&config, &root)?
            };
            if db_path.exists() {
                std::fs::remove_file(&db_path)
                    .with_context(|| format!("Failed to remove database {}", db_path.display()))?;
                eprintln!("Removed {}", db_path.display());
                // Also remove WAL/SHM files that SQLite may leave behind
                for suffix in &["-wal", "-shm"] {
                    let mut p = db_path.as_os_str().to_owned();
                    p.push(suffix);
                    let _ = std::fs::remove_file(PathBuf::from(p));
                }
            }

            // Remove the .shire directory
            let shire_dir = root.join(".shire");
            if shire_dir.exists() {
                std::fs::remove_dir_all(&shire_dir)
                    .with_context(|| format!("Failed to remove {}", shire_dir.display()))?;
                eprintln!("Removed {}", shire_dir.display());
            }

            eprintln!("Clean complete.");
            Ok(())
        }
        Commands::Rebuild {
            root,
            mut file,
            stdin,
        } => {
            let root = if stdin {
                match watch::protocol::HookInput::from_stdin() {
                    Some(hook) if !hook.should_rebuild() => return Ok(()),
                    Some(hook) => {
                        if let Some(path) = hook.tool_input.file_path {
                            file.push(path);
                        } else if let Some(path) = hook.tool_input.notebook_path {
                            file.push(path);
                        }
                        // Use cwd from hook JSON as root (falls back to --root)
                        hook.cwd.unwrap_or(root)
                    }
                    None => root,
                }
            } else {
                root
            };
            let root = std::fs::canonicalize(&root)?;
            watch::send_rebuild(&root, file)
        }
    }
}
