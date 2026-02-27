pub mod daemon;
pub mod protocol;

use crate::config::Config;
use crate::index;
use crate::symbols::walker;
use anyhow::{Context, Result};
use protocol::RebuildMessage;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

/// Check whether a file is relevant to the index.
/// Must be inside the repo root AND be a manifest, source, or config file.
fn is_relevant(
    path: &Path,
    root: &Path,
    manifest_names: &HashSet<&str>,
    source_exts: &HashSet<&str>,
) -> bool {
    // Must be inside the repo root
    if !path.starts_with(root) {
        return false;
    }

    let filename = match path.file_name().and_then(|f| f.to_str()) {
        Some(f) => f,
        None => return false,
    };

    // shire config change
    if filename == "shire.toml" {
        return true;
    }

    // Manifest file (package.json, go.mod, Cargo.toml, etc.)
    if manifest_names.contains(filename) {
        return true;
    }

    // Source file with a tracked extension
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if source_exts.contains(ext) {
            return true;
        }
    }

    false
}

/// Send a rebuild signal to the daemon via UDS.
/// Graceful no-op if the daemon is not running or the socket doesn't exist.
pub fn send_rebuild(root: &Path, files: Vec<PathBuf>) -> Result<()> {
    let sock = daemon::sock_path(root);
    if !sock.exists() {
        return Ok(());
    }

    let msg = RebuildMessage { files };
    let mut payload = serde_json::to_string(&msg).context("failed to serialize rebuild message")?;
    payload.push('\n');

    // Use std::os::unix::net for a blocking connect + write (fire-and-forget)
    match std::os::unix::net::UnixStream::connect(&sock) {
        Ok(mut stream) => {
            use std::io::Write;
            let _ = stream.write_all(payload.as_bytes());
            Ok(())
        }
        Err(_) => Ok(()), // Daemon not listening, no-op
    }
}

/// Run the daemon event loop (called with --foreground).
/// Binds UDS, accepts rebuild signals, debounces, and runs build_index.
pub async fn run_daemon(
    root: PathBuf,
    config: Config,
    db_override: Option<PathBuf>,
) -> Result<()> {
    let sock = daemon::sock_path(&root);

    // Remove stale socket file before binding
    let _ = std::fs::remove_file(&sock);

    let listener = UnixListener::bind(&sock)
        .context("failed to bind Unix socket")?;

    let (tx, mut rx) = mpsc::unbounded_channel::<RebuildMessage>();

    eprintln!("[watch] daemon started, listening on {}", sock.display());

    // Spawn connection acceptor task
    let tx_clone = tx.clone();
    let accept_handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let tx = tx_clone.clone();
                    tokio::spawn(async move {
                        let reader = BufReader::new(stream);
                        let mut lines = reader.lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            match serde_json::from_str::<RebuildMessage>(&line) {
                                Ok(msg) => {
                                    let _ = tx.send(msg);
                                }
                                Err(e) => {
                                    eprintln!("[watch] invalid message: {e}");
                                }
                            }
                        }
                    });
                }
                Err(e) => {
                    eprintln!("[watch] accept error: {e}");
                }
            }
        }
    });

    // Debounce loop
    let debounce = std::time::Duration::from_millis(config.watch.debounce_ms);

    // Build relevance filter sets
    let manifest_names: HashSet<&str> = config
        .discovery
        .manifests
        .iter()
        .map(|s| s.as_str())
        .collect();
    let source_exts: HashSet<&str> = walker::all_extensions().into_iter().collect();

    // Set up signal handler for graceful shutdown
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    loop {
        // Wait for first signal or shutdown
        tokio::select! {
            Some(first_msg) = rx.recv() => {
                // Accumulate files across the debounce window
                let mut all_files: Vec<PathBuf> = first_msg.files;

                // Got a rebuild signal, start debounce window
                let deadline = tokio::time::Instant::now() + debounce;

                // Drain any additional signals during debounce window
                loop {
                    tokio::select! {
                        Some(msg) = rx.recv() => {
                            all_files.extend(msg.files);
                        }
                        _ = tokio::time::sleep_until(deadline) => {
                            break;
                        }
                    }
                }

                // If files were specified, check relevance before rebuilding.
                // Empty file list = unconditional rebuild (manual `shire rebuild`).
                if !all_files.is_empty() {
                    let dominated_by_irrelevant = all_files
                        .iter()
                        .all(|f| !is_relevant(f, &root, &manifest_names, &source_exts));
                    if dominated_by_irrelevant {
                        let names: Vec<_> = all_files
                            .iter()
                            .filter_map(|f| f.file_name().and_then(|n| n.to_str()))
                            .collect();
                        eprintln!("[watch] skipping rebuild — no relevant files: {}", names.join(", "));
                        continue;
                    }
                }

                // Run build
                let build_root = root.clone();
                let build_config = config.clone();
                let build_db = db_override.clone();

                eprintln!("[watch] triggering rebuild...");
                let result = tokio::task::spawn_blocking(move || {
                    index::build_index(
                        &build_root,
                        &build_config,
                        false,
                        build_db.as_deref(),
                    )
                })
                .await;

                match result {
                    Ok(Ok(())) => eprintln!("[watch] rebuild completed"),
                    Ok(Err(e)) => eprintln!("[watch] rebuild failed: {e}"),
                    Err(e) => eprintln!("[watch] rebuild task panicked: {e}"),
                }
            }
            _ = sigterm.recv() => {
                eprintln!("[watch] received SIGTERM, shutting down");
                break;
            }
            _ = sigint.recv() => {
                eprintln!("[watch] received SIGINT, shutting down");
                break;
            }
        }
    }

    // Cleanup
    accept_handle.abort();
    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(daemon::pid_path(&root));
    eprintln!("[watch] daemon stopped");
    Ok(())
}
