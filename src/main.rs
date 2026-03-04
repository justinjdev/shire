use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod config;
mod db;
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
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    #[cfg(feature = "rag")]
    rag::storage::load_extension();

    match cli.command {
        Commands::Build { root, force, db, config: cfg_path } => {
            let root = std::fs::canonicalize(&root)?;
            let config = config::load_config_from(cfg_path.as_deref(), &root)?;
            index::build_index(&root, &config, force, db.as_deref())
        }
        Commands::Serve { db, config: cfg_path } => {
            let root = std::fs::canonicalize(".")?;
            let cfg = config::load_config_from(cfg_path.as_deref(), &root)?;
            let db_path = if let Some(p) = db {
                p
            } else {
                config::resolve_db_path(&cfg, &root)?
            };
            if !db_path.exists() {
                anyhow::bail!(
                    "Index not found at {}. Run `shire build` first.",
                    db_path.display()
                );
            }
            mcp::run_server(&db_path, &cfg.rag).await
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
        Commands::Init { root, global } => {
            if global {
                init::run_init_global()
            } else {
                let root = std::fs::canonicalize(&root)?;
                init::run_init(&root)
            }
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
