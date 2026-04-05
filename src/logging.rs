use crate::config::LogConfig;
use std::path::{Path, PathBuf};
use tracing_appender::rolling;
use tracing_subscriber::EnvFilter;

/// Generate a short random session ID (8 hex chars).
fn session_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(std::process::id() as u64);
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    format!("{:08x}", hasher.finish() as u32)
}

/// Initialize the tracing subscriber with daily-rotating file logging.
///
/// Log level priority: SHIRE_LOG env var > config file > default ("warn").
/// If the log dir is empty or cannot be created, falls back to stderr logging.
/// Old log files beyond `max_days` are cleaned up on init.
///
/// Returns the session ID for correlation.
pub fn init(log_config: &LogConfig, repo_root: &Path, command: &str) -> String {
    let sid = session_id();
    let level = std::env::var("SHIRE_LOG").unwrap_or_else(|_| log_config.level.clone());
    let filter = EnvFilter::try_new(format!("shire={level}"))
        .unwrap_or_else(|_| EnvFilter::new("shire=warn"));

    if !log_config.dir.is_empty() {
        let log_dir = resolve_log_dir(&log_config.dir, repo_root);
        match std::fs::create_dir_all(&log_dir) {
            Ok(()) => {
                evict_old_logs(&log_dir, log_config.max_days.max(1));
                let file_appender = rolling::daily(&log_dir, "shire.log");
                tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_writer(file_appender)
                    .with_ansi(false)
                    .try_init()
                    .ok();
                tracing::info!(session = %sid, command, "shire session started");
                return sid;
            }
            Err(e) => {
                if command == "serve" {
                    // serve cannot fall back to stderr (would corrupt MCP stdio)
                    eprintln!(
                        "Error: could not create log directory {}: {e}. Logging disabled for serve.",
                        log_dir.display()
                    );
                } else {
                    eprintln!(
                        "Warning: could not create log directory {}: {e}",
                        log_dir.display()
                    );
                }
            }
        }
    } else if command == "serve" {
        eprintln!("Warning: log.dir is empty — logging disabled for serve");
    }

    // Fallback: stderr-only logging (skip for serve — stderr may interfere with MCP stdio)
    if command != "serve" {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_ansi(false)
            .try_init()
            .ok();
    }

    sid
}

fn resolve_log_dir(dir: &str, repo_root: &Path) -> PathBuf {
    let p = PathBuf::from(dir);
    if p.is_absolute() {
        p
    } else {
        repo_root.join(p)
    }
}

/// Remove log files older than `max_days` from the log directory.
fn evict_old_logs(log_dir: &Path, max_days: u32) {
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(max_days as u64 * 86400);

    let entries = match std::fs::read_dir(log_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.starts_with("shire.log") {
            continue;
        }
        if let Ok(meta) = path.metadata()
            && let Ok(modified) = meta.modified()
                && modified < cutoff {
                    let _ = std::fs::remove_file(&path);
                }
    }
}
