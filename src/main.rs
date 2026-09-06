use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

use shire::config;
use shire::index;
use shire::init;
use shire::install;
use shire::logging;
use shire::mcp;
use shire::watch;

#[derive(Parser)]
#[command(
    name = "shire",
    version,
    about = "Monorepo package index and MCP server"
)]
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
        /// Print whether the daemon is running (pid, socket, liveness) and exit
        #[arg(long)]
        status: bool,
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
    /// Register shire as an MCP server with all detected AI tools
    Install {
        /// Show what would be done without making changes
        #[arg(long)]
        dry_run: bool,
        /// Overwrite existing registrations (useful after binary path changes)
        #[arg(long)]
        force: bool,
    },
    /// Remove shire MCP registration from all detected AI tools
    Uninstall {
        /// Show what would be done without making changes
        #[arg(long)]
        dry_run: bool,
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

/// Program entry point that parses command-line arguments and dispatches the selected subcommand.
///
/// This function drives the CLI behavior (build, serve, watch, init, install, uninstall, clean,
/// rebuild), performs path canonicalization and configuration resolution, and delegates work to
/// the corresponding modules. It returns an error if any subcommand encounters a failure.
///
/// # Examples
///
/// ```no_run
/// // Run the installed binary with the "build" subcommand:
/// use std::process::Command;
/// let _ = Command::new("shire").arg("build").status().unwrap();
/// ```
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build {
            root,
            force,
            db,
            config: cfg_path,
        } => {
            let root = std::fs::canonicalize(&root)?;
            let config = config::load_config_from(cfg_path.as_deref(), &root)?;
            let _sid = logging::init(&config.log, &root, "build");
            index::build_index(&root, &config, force, db.as_deref())
        }
        Commands::Serve {
            root,
            db,
            config: cfg_path,
        } => {
            let cwd = std::fs::canonicalize(".")?;
            let repo_root = root.as_ref().map(std::fs::canonicalize).transpose()?;
            let effective_root = repo_root.as_deref().unwrap_or(&cwd);
            let cfg = config::load_config_from(cfg_path.as_deref(), effective_root)?;
            let _sid = logging::init(&cfg.log, effective_root, "serve");
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
            mcp::run_server(&db_path, build_ctx).await
        }
        Commands::Watch {
            root,
            stop,
            status,
            foreground,
            db,
            config: cfg_path,
        } => {
            let root = std::fs::canonicalize(&root)?;
            if status {
                watch::daemon::print_status(&root);
                Ok(())
            } else if stop {
                watch::daemon::stop_daemon(&root)
            } else if foreground {
                let config = config::load_config_from(cfg_path.as_deref(), &root)?;
                let _sid = logging::init(&config.log, &root, "watch");
                watch::run_daemon(root, config, db).await
            } else {
                watch::daemon::start_daemon(&root, db.as_deref(), cfg_path.as_deref())
            }
        }
        Commands::Init {
            root,
            global,
            no_hook,
            yes,
        } => {
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
        Commands::Install { dry_run, force } => install::run_install(dry_run, force),
        Commands::Uninstall { dry_run } => install::run_uninstall(dry_run),
        Commands::Clean {
            root,
            db,
            config: cfg_path,
        } => {
            let root = std::fs::canonicalize(&root)?;

            // Stop the watch daemon if running. stop_daemon() itself waits (up to ~5s)
            // for the process to actually exit before touching its pid/socket files, so
            // a single is_running() check afterward is meaningful: if the daemon is
            // still alive at this point, it truly did not stop cleanly (previously the
            // pid file was deleted before the process had exited, which made this
            // check — and the bail below — unreachable, and let `clean` remove `.shire`
            // out from under a daemon that was still mid-rebuild).
            if watch::daemon::is_running(&root) {
                eprintln!("Stopping watch daemon...");
                watch::daemon::stop_daemon(&root)?;
                if watch::daemon::is_running(&root) {
                    anyhow::bail!("Watch daemon did not stop cleanly");
                }
            }

            // Resolve and remove the database file. db_path comes from a
            // repo-controlled shire.toml (or a global ~/.claude/shire.toml), so it must
            // not be deleted unconditionally — see remove_index_db().
            let db_path = if let Some(p) = db {
                p
            } else {
                let config = config::load_config_from(cfg_path.as_deref(), &root)?;
                config::resolve_db_path(&config, &root)?
            };
            remove_index_db(&db_path, &root)?;

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
                        let cwd = hook.cwd.unwrap_or(root);
                        // Claude Code's hook `cwd` is wherever the session was
                        // launched, which in a monorepo is often a package
                        // subdirectory rather than the repo root the watch daemon's
                        // socket lives under. Walk up to find it instead of using
                        // cwd verbatim, the way git/cargo resolve their root.
                        match std::fs::canonicalize(&cwd) {
                            Ok(canon) => config::find_repo_root(&canon),
                            Err(_) => cwd,
                        }
                    }
                    None => root,
                }
            } else {
                root
            };
            let root = std::fs::canonicalize(&root)?;

            // A `--file` argument may be relative (to wherever `shire rebuild` was
            // invoked); resolve it against the repo root rather than letting the
            // daemon's relevance filter silently treat it as outside the repo.
            let file: Vec<PathBuf> = file
                .into_iter()
                .map(|f| if f.is_relative() { root.join(f) } else { f })
                .collect();

            watch::send_rebuild(&root, file)
        }
    }
}

/// SQLite's on-disk file header — the first 16 bytes of every valid SQLite database
/// file (see the SQLite file format spec).
const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";

/// Open `path` refusing to follow a trailing symlink (O_NOFOLLOW on unix), returning
/// `Ok(None)` for a missing file and `Err` for a symlink or any other open failure.
/// Centralizes the "don't follow a symlink" pattern used for both the main db file and
/// its `-wal`/`-shm` sidecars (the sidecar paths are just as attacker-nameable as
/// `db_path` itself, being derived from it by string concatenation).
fn open_no_follow(path: &Path) -> Result<Option<std::fs::File>> {
    #[cfg(unix)]
    let opened = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    };
    #[cfg(not(unix))]
    let opened = std::fs::File::open(path);

    match opened {
        Ok(f) => Ok(Some(f)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        #[cfg(unix)]
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => anyhow::bail!(
            "{} is a symlink, not a plain file. Remove it by hand if that's intentional.",
            path.display()
        ),
        Err(e) => Err(e).with_context(|| format!("Failed to open {}", path.display())),
    }
}

/// Remove `path` unless it's a symlink or missing. Errors (including "it's a symlink")
/// are swallowed with a warning: sidecar removal is best-effort and shouldn't turn a
/// successful main-file removal into a failed `shire clean`.
fn remove_sidecar(path: &Path) {
    match open_no_follow(path) {
        Ok(Some(f)) => {
            drop(f);
            let _ = std::fs::remove_file(path);
        }
        Ok(None) => {} // doesn't exist, nothing to do
        Err(e) => eprintln!("Warning: not removing {}: {e:#}", path.display()),
    }
}

/// Does `path` (an existing, real — non-symlink — SQLite database) have the
/// `shire_meta` table that every shire-built index database creates? Opened strictly
/// read-only via rusqlite so this can't create, write to, or lock the file.
fn looks_like_shire_db(path: &Path) -> bool {
    let conn = match rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(_) => return false,
    };
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'shire_meta'",
        [],
        |_| Ok(()),
    )
    .is_ok()
}

/// Is `db_path` somewhere shire itself manages — `<repo_root>/.shire/` or
/// `~/.claude/shire/`? Used only as a fallback for a database that fails the
/// `shire_meta` identity check because it's corrupt (which a real, crashed shire index
/// can be — `shire build` auto-cleans a corrupt DB it finds), not as a way to accept an
/// unidentified file from an arbitrary location.
fn is_in_managed_location(db_path: &Path, root: &Path) -> bool {
    let Ok(canon) = std::fs::canonicalize(db_path) else {
        return false;
    };

    if let Ok(repo_shire) = std::fs::canonicalize(root.join(".shire"))
        && canon.starts_with(&repo_shire)
    {
        return true;
    }

    if let Ok(home) = std::env::var("HOME")
        && let Ok(claude_shire) = std::fs::canonicalize(PathBuf::from(home).join(".claude/shire"))
        && canon.starts_with(&claude_shire)
    {
        return true;
    }

    false
}

/// Remove the index database file at `db_path` (plus its `-wal`/`-shm` sidecars), but
/// only after verifying it is really a shire-built SQLite database.
///
/// `db_path` is resolved from a repo-controlled `shire.toml` (with `~`/`$VAR` shell
/// expansion and no confinement to the repo root, since the documented global setup
/// puts real per-worktree databases under `~/.claude/shire/{repo}/{worktree}/`), so a
/// hostile repo could otherwise point it at an arbitrary file and have `shire clean`
/// delete it unconditionally. Two checks gate the delete, in order:
///
/// 1. The file must open (O_NOFOLLOW — never follow a symlink) and start with the
///    SQLite magic header, read from that same handle to avoid a check-then-delete
///    race. This alone only proves *format*, not identity: `~/.mozilla/.../places.sqlite`
///    or a browser's `Login Data` file would pass it too.
/// 2. It must have a `shire_meta` table (queried via a strictly read-only rusqlite
///    connection) — the identifying mark every shire-built index carries. A real shire
///    database can fail this by being corrupt rather than foreign (`shire build`
///    auto-cleans a corrupt DB it encounters), so a file that fails step 2 is still
///    removed if — and only if — its canonical path is under `<repo_root>/.shire/` or
///    `~/.claude/shire/`, the only places shire itself ever creates one.
///
/// A missing `db_path` is not an error (nothing to clean up).
fn remove_index_db(db_path: &Path, root: &Path) -> Result<()> {
    let Some(mut file) = open_no_follow(db_path)? else {
        return Ok(());
    };

    let is_sqlite = {
        use std::io::Read;
        let mut header = [0u8; 16];
        file.read_exact(&mut header).is_ok() && &header == SQLITE_HEADER
    };
    drop(file);

    if !is_sqlite {
        anyhow::bail!(
            "Refusing to remove {}: it does not look like a shire index database \
             (missing the SQLite file header). Check shire.toml's db_path before \
             removing this file by hand.",
            db_path.display()
        );
    }

    if !looks_like_shire_db(db_path) && !is_in_managed_location(db_path, root) {
        anyhow::bail!(
            "Refusing to remove {}: it's a SQLite database, but not one shire built \
             (no 'shire_meta' table) and it isn't under a location shire manages \
             (<repo>/.shire/ or ~/.claude/shire/). shire.toml's db_path may be \
             pointing somewhere unexpected — delete it by hand if that's intended.",
            db_path.display()
        );
    }

    std::fs::remove_file(db_path)
        .with_context(|| format!("Failed to remove database {}", db_path.display()))?;
    eprintln!("Removed {}", db_path.display());

    // Only remove WAL/SHM sidecars once the main file passed one of the checks above,
    // and only via the same "never follow a symlink" open each of them gets on their
    // own — they're just as attacker-nameable as db_path, being derived from it by
    // string concatenation.
    for suffix in &["-wal", "-shm"] {
        let mut p = db_path.as_os_str().to_owned();
        p.push(suffix);
        remove_sidecar(&PathBuf::from(p));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file that starts with the SQLite magic header but is otherwise garbage —
    /// passes the format check but fails both to open cleanly as SQLite in most cases
    /// and, even where it opens, to have a `shire_meta` table. Stands in for a
    /// genuinely corrupt shire index (which `shire build` auto-cleans when it finds
    /// one) as well as for "some other file that merely starts with the same magic
    /// bytes" — either way, identity can't be confirmed and the location fallback
    /// decides it.
    fn write_corrupt_sqlite_like_file(path: &Path) {
        let mut content = SQLITE_HEADER.to_vec();
        content.extend_from_slice(b"rest of a fake but header-valid sqlite file");
        std::fs::write(path, content).unwrap();
    }

    /// A real, valid SQLite database (openable and queryable) that simply isn't one
    /// shire built — no `shire_meta` table. Stands in for e.g. a browser profile
    /// database that happens to live at whatever path a hostile shire.toml named.
    fn write_foreign_sqlite_db(path: &Path) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch("CREATE TABLE places (id INTEGER PRIMARY KEY, url TEXT);")
            .unwrap();
    }

    /// Build a sidecar path the same way remove_index_db() does: appended directly to
    /// the full path string, not via Path::with_extension (e.g. "index.db" + "-wal" =>
    /// "index.db-wal", and "id_rsa" + "-wal" => "id_rsa-wal").
    fn sidecar(path: &Path, suffix: &str) -> PathBuf {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        PathBuf::from(p)
    }

    #[test]
    fn remove_index_db_removes_a_real_shire_db_outside_any_managed_location() {
        // Identity (shire_meta) alone is sufficient — location doesn't matter for a
        // database that actually is shire's own.
        let dir = tempfile::TempDir::new().unwrap();
        let other_root = tempfile::TempDir::new().unwrap(); // unrelated to db's location
        let db = dir.path().join("index.db");
        shire::db::open_or_create(&db).unwrap();
        std::fs::write(sidecar(&db, "-wal"), b"wal").unwrap();
        std::fs::write(sidecar(&db, "-shm"), b"shm").unwrap();

        remove_index_db(&db, other_root.path()).unwrap();

        assert!(!db.exists());
        assert!(!sidecar(&db, "-wal").exists());
        assert!(!sidecar(&db, "-shm").exists());
    }

    #[test]
    fn remove_index_db_refuses_a_foreign_sqlite_db_outside_repo() {
        // The core of the vulnerability this guards: a real, valid SQLite database
        // (e.g. a browser profile db) that a hostile shire.toml's db_path pointed at.
        // Passing the SQLite-header/format check must not be enough.
        let dir = tempfile::TempDir::new().unwrap();
        let repo_root = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(repo_root.path().join(".shire")).unwrap();
        let victim = dir.path().join("places.sqlite");
        write_foreign_sqlite_db(&victim);
        std::fs::write(sidecar(&victim, "-wal"), b"wal-sidecar").unwrap();
        std::fs::write(sidecar(&victim, "-shm"), b"shm-sidecar").unwrap();

        let result = remove_index_db(&victim, repo_root.path());

        assert!(result.is_err());
        assert!(
            victim.exists(),
            "foreign db outside any managed location must survive"
        );
        assert!(
            sidecar(&victim, "-wal").exists(),
            "sidecars must survive too"
        );
        assert!(sidecar(&victim, "-shm").exists());
    }

    #[test]
    fn remove_index_db_removes_a_corrupt_file_inside_repo_shire_dir() {
        // A real shire index can be corrupt (crash mid-write); shire build
        // auto-cleans one it finds, and `shire clean` must still be able to remove it
        // even though it fails the shire_meta identity check — but only because it's
        // in the location shire itself manages.
        let repo_root = tempfile::TempDir::new().unwrap();
        let shire_dir = repo_root.path().join(".shire");
        std::fs::create_dir_all(&shire_dir).unwrap();
        let db = shire_dir.join("index.db");
        write_corrupt_sqlite_like_file(&db);

        remove_index_db(&db, repo_root.path()).unwrap();

        assert!(!db.exists());
    }

    #[test]
    fn remove_index_db_refuses_a_corrupt_file_outside_managed_locations() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_root = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(repo_root.path().join(".shire")).unwrap();
        let victim = dir.path().join("index.db");
        write_corrupt_sqlite_like_file(&victim);

        let result = remove_index_db(&victim, repo_root.path());

        assert!(result.is_err());
        assert!(
            victim.exists(),
            "corrupt file outside any managed location must survive"
        );
    }

    #[test]
    fn remove_index_db_refuses_non_sqlite_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let victim = dir.path().join("id_rsa");
        std::fs::write(&victim, "PRIVATE KEY MATERIAL").unwrap();

        let result = remove_index_db(&victim, dir.path());

        assert!(result.is_err());
        assert!(victim.exists(), "non-sqlite target must be left untouched");
        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "PRIVATE KEY MATERIAL"
        );
    }

    #[test]
    fn remove_index_db_refuses_symlink_even_if_target_is_a_real_shire_db() {
        let dir = tempfile::TempDir::new().unwrap();
        let real_db = dir.path().join("real.db");
        shire::db::open_or_create(&real_db).unwrap();
        let link = dir.path().join("db_via_symlink");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_db, &link).unwrap();

        #[cfg(unix)]
        {
            let result = remove_index_db(&link, dir.path());
            assert!(result.is_err());
            assert!(link.exists(), "symlink must not be followed and removed");
            assert!(real_db.exists(), "symlink target must be untouched");
        }
    }

    #[test]
    fn remove_index_db_missing_file_is_not_an_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist.db");
        assert!(remove_index_db(&missing, dir.path()).is_ok());
    }

    #[test]
    fn remove_index_db_leaves_sidecars_when_main_file_is_refused() {
        let dir = tempfile::TempDir::new().unwrap();
        let victim = dir.path().join("id_rsa");
        std::fs::write(&victim, "PRIVATE KEY MATERIAL").unwrap();
        // An attacker-controlled db_path could also have coincidentally-named
        // "sidecar" files; those must survive a refusal on the main file too.
        std::fs::write(sidecar(&victim, "-wal"), b"unrelated").unwrap();

        let _ = remove_index_db(&victim, dir.path());

        assert!(sidecar(&victim, "-wal").exists());
    }

    #[test]
    fn remove_index_db_sidecar_symlink_is_not_followed() {
        // Sidecars are just as attacker-nameable as db_path (derived from it by string
        // concatenation), so a symlinked "-wal" sidecar must not be followed either —
        // it's refused and left in place entirely, the same way the main file is.
        //
        // Uses a corrupt-but-location-approved main file rather than a real WAL-mode
        // database: a genuine shire db would try to read its own "-wal" companion to
        // determine current state, and a symlinked/garbage "-wal" there would make
        // *opening the main file itself* fail — a real (if minor) side effect of this
        // attack, but a distraction from what this test is actually checking.
        let repo_root = tempfile::TempDir::new().unwrap();
        let shire_dir = repo_root.path().join(".shire");
        std::fs::create_dir_all(&shire_dir).unwrap();
        let db = shire_dir.join("index.db");
        write_corrupt_sqlite_like_file(&db);
        let wal_target = repo_root.path().join("unrelated-secret-file");
        std::fs::write(&wal_target, "secret").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&wal_target, sidecar(&db, "-wal")).unwrap();

        // The main file removal still succeeds — sidecar handling is best-effort and
        // must not turn a successful `shire clean` into a failed one.
        remove_index_db(&db, repo_root.path()).unwrap();

        assert!(!db.exists());
        #[cfg(unix)]
        {
            assert!(
                wal_target.exists(),
                "the symlink's target must never be touched"
            );
            assert!(
                sidecar(&db, "-wal").exists(),
                "a symlinked sidecar is refused and left alone, not followed"
            );
        }
    }
}
