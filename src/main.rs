use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod config;
mod db;
mod index;
mod mcp;
mod symbols;

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
    },
    /// Start the MCP server over stdio
    Serve {
        /// Path to the index database (defaults to .shire/index.db)
        #[arg(long)]
        db: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Build { root, force, db } => {
            let root = std::fs::canonicalize(&root)?;
            let config = config::load_config(&root)?;
            index::build_index(&root, &config, force, db.as_deref())
        }
        Commands::Serve { db } => {
            let db_path = db.unwrap_or_else(|| PathBuf::from(".shire/index.db"));
            if !db_path.exists() {
                anyhow::bail!(
                    "Index not found at {}. Run `shire build` first.",
                    db_path.display()
                );
            }
            mcp::run_server(&db_path).await
        }
    }
}
