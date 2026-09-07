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

/// Precomputed sets describing which files the indexer cares about, built once from
/// config at daemon startup and reused for every relevance check. Sourced from the same
/// config the indexer itself reads (`discovery.manifests`, `docs.extensions`,
/// `discovery.custom`, `discovery.exclude`, plus `walker::all_extensions()`) so this
/// filter can't drift from what a rebuild would actually pick up.
struct RelevanceFilter {
    manifest_names: HashSet<String>,
    source_exts: HashSet<String>,
    doc_exts: HashSet<String>,
    custom_names: HashSet<String>,
    custom_exts: HashSet<String>,
    exclude_dirs: HashSet<String>,
}

impl RelevanceFilter {
    fn from_config(config: &Config) -> Self {
        let manifest_names = config.discovery.manifests.iter().cloned().collect();
        let source_exts = walker::all_extensions()
            .into_iter()
            .map(str::to_string)
            .collect();
        // docs.extensions are stored with a leading dot (".md"); walker extensions and
        // Path::extension() are not, so normalize here.
        let doc_exts = config
            .docs
            .extensions
            .iter()
            .map(|e| e.trim_start_matches('.').to_string())
            .collect();

        let mut custom_names = HashSet::new();
        let mut custom_exts = HashSet::new();
        for rule in &config.discovery.custom {
            for name in &rule.requires {
                custom_names.insert(name.clone());
            }
            if let Some(exts) = &rule.extensions {
                for ext in exts {
                    custom_exts.insert(ext.trim_start_matches('.').to_string());
                }
            }
        }

        let exclude_dirs = config.discovery.exclude.iter().cloned().collect();

        Self {
            manifest_names,
            source_exts,
            doc_exts,
            custom_names,
            custom_exts,
            exclude_dirs,
        }
    }
}

/// Check whether a file is relevant to the index.
/// Must be inside the repo root, not under an excluded directory, AND be a manifest,
/// custom-discovery, source, doc, or shire config file.
fn is_relevant(path: &Path, root: &Path, filter: &RelevanceFilter) -> bool {
    // Must be inside the repo root
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };

    // Excluded directory anywhere in the path (matches how the indexer itself skips
    // vendored/build dirs via discovery.exclude — e.g. node_modules). Only ancestor
    // directory components are checked, not the filename itself — the indexer's own
    // WalkBuilder filter only excludes directory entries, so a file whose *basename*
    // happens to match an excluded directory name (e.g. a manifest literally named
    // "target") would otherwise be wrongly treated as irrelevant here.
    if rel.parent().is_some_and(|dir| {
        dir.components().any(|c| {
            c.as_os_str()
                .to_str()
                .is_some_and(|s| filter.exclude_dirs.contains(s))
        })
    }) {
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

    // Manifest file (package.json, go.mod, Cargo.toml, etc.) or a custom-discovery
    // marker filename (discovery.custom[].requires).
    if filter.manifest_names.contains(filename) || filter.custom_names.contains(filename) {
        return true;
    }

    // Source, doc, or custom-discovery extension.
    if let Some(ext) = path.extension().and_then(|e| e.to_str())
        && (filter.source_exts.contains(ext)
            || filter.doc_exts.contains(ext)
            || filter.custom_exts.contains(ext))
    {
        return true;
    }

    false
}

/// Send a rebuild signal to the daemon via UDS.
/// Graceful no-op if the daemon is not running or the socket doesn't exist — but warns
/// on stderr so a stale/dead daemon doesn't silently leave the index stale (the caller,
/// typically the PostToolUse hook, still gets exit 0 so the tool call itself never
/// fails).
pub fn send_rebuild(root: &Path, files: Vec<PathBuf>) -> Result<()> {
    let sock = daemon::sock_path(root);
    if !sock.exists() {
        eprintln!(
            "Warning: no watch daemon socket at {} — is `shire watch` running for this repo? \
             Run `shire watch --status` to check.",
            sock.display()
        );
        return Ok(());
    }
    if daemon::is_symlink(&sock) {
        // Refuse to connect through it — e.g. planted to point at another repo's live
        // socket — rather than reaching whatever daemon it actually names.
        eprintln!(
            "Warning: {} is a symlink; refusing to connect through it.",
            sock.display()
        );
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
        Err(e) => {
            eprintln!(
                "Warning: could not reach watch daemon at {} ({e}); the index may be stale.",
                sock.display()
            );
            Ok(())
        }
    }
}

/// Run the daemon event loop (called with --foreground).
/// Binds UDS, accepts rebuild signals, debounces, and runs build_index.
pub async fn run_daemon(root: PathBuf, config: Config, db_override: Option<PathBuf>) -> Result<()> {
    let sock = daemon::sock_path(&root);

    // Remove stale socket file before binding
    let _ = std::fs::remove_file(&sock);

    let listener = UnixListener::bind(&sock).context("failed to bind Unix socket")?;

    let (tx, mut rx) = mpsc::unbounded_channel::<RebuildMessage>();

    tracing::info!(socket = %sock.display(), "daemon started");

    // Spawn connection acceptor task
    let tx_clone = tx.clone();
    let accept_handle = tokio::spawn(async move {
        let mut consecutive_errors: u32 = 0;
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    consecutive_errors = 0;
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
                                    tracing::warn!(%e, "invalid message received");
                                }
                            }
                        }
                    });
                }
                Err(e) => {
                    consecutive_errors = consecutive_errors.saturating_add(1);
                    // Errors like EMFILE/ENFILE don't clear on their own between
                    // calls, so retrying immediately turns this into a hot spin loop
                    // (millions of log lines and hundreds of MB written to
                    // .shire/logs within seconds — WATCH-3). Back off exponentially
                    // (capped at 1s) and rate-limit the log line.
                    if consecutive_errors <= 3 || consecutive_errors.is_multiple_of(200) {
                        tracing::error!(%e, consecutive_errors, "socket accept error");
                    }
                    let backoff_ms = 10u64.saturating_mul(1u64 << consecutive_errors.min(7));
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms.min(1000)))
                        .await;
                }
            }
        }
    });

    // Debounce loop
    let debounce = std::time::Duration::from_millis(config.watch.debounce_ms);
    let filter = RelevanceFilter::from_config(&config);

    // Set up signal handler for graceful shutdown
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    loop {
        // Wait for first signal or shutdown
        tokio::select! {
            Some(first_msg) = rx.recv() => {
                // Accumulate unique files across the debounce window
                let mut file_set: HashSet<PathBuf> = first_msg.files.into_iter().collect();

                // Got a rebuild signal, start debounce window
                tracing::debug!(debounce_ms = config.watch.debounce_ms, "debounce window started");
                let deadline = tokio::time::Instant::now() + debounce;

                // Drain any additional signals during debounce window. Also listens
                // for shutdown here so SIGTERM/SIGINT during the debounce wait isn't
                // deferred behind a rebuild that hasn't even started yet (WATCH-6) —
                // once a rebuild is already running via spawn_blocking it can't be
                // cancelled mid-flight, but this at least avoids compounding the delay.
                let mut shutting_down = false;
                loop {
                    tokio::select! {
                        Some(msg) = rx.recv() => {
                            file_set.extend(msg.files);
                        }
                        _ = tokio::time::sleep_until(deadline) => {
                            break;
                        }
                        _ = sigterm.recv() => {
                            tracing::info!("received SIGTERM during debounce, shutting down");
                            shutting_down = true;
                            break;
                        }
                        _ = sigint.recv() => {
                            tracing::info!("received SIGINT during debounce, shutting down");
                            shutting_down = true;
                            break;
                        }
                    }
                }

                if shutting_down {
                    break;
                }

                let all_files: Vec<PathBuf> = file_set.into_iter().collect();

                // If files were specified, check relevance before rebuilding.
                // Empty file list = unconditional rebuild (manual `shire rebuild`).
                if !all_files.is_empty() {
                    let dominated_by_irrelevant = all_files
                        .iter()
                        .all(|f| !is_relevant(f, &root, &filter));
                    if dominated_by_irrelevant {
                        let names: Vec<_> = all_files
                            .iter()
                            .filter_map(|f| f.file_name().and_then(|n| n.to_str()))
                            .collect();
                        tracing::debug!(files = %names.join(", "), "skipping rebuild — no relevant files");
                        continue;
                    }
                }

                // Run build
                let build_root = root.clone();
                let build_config = config.clone();
                let build_db = db_override.clone();

                tracing::info!("triggering rebuild");
                let result = tokio::task::spawn_blocking(move || {
                    index::build_index_quiet(
                        &build_root,
                        &build_config,
                        false,
                        build_db.as_deref(),
                    )
                })
                .await;

                match result {
                    Ok(Ok(())) => tracing::info!("rebuild completed"),
                    Ok(Err(e)) => tracing::error!(%e, "rebuild failed"),
                    Err(e) => tracing::error!(%e, "rebuild task panicked"),
                }
            }
            _ = sigterm.recv() => {
                tracing::info!("received SIGTERM, shutting down");
                break;
            }
            _ = sigint.recv() => {
                tracing::info!("received SIGINT, shutting down");
                break;
            }
        }
    }

    // Cleanup
    accept_handle.abort();
    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(daemon::pid_path(&root));
    tracing::info!("daemon stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(config: &Config) -> RelevanceFilter {
        RelevanceFilter::from_config(config)
    }

    #[test]
    fn manifest_file_is_relevant() {
        let config = Config::default();
        let root = Path::new("/repo");
        assert!(is_relevant(
            Path::new("/repo/pkg/package.json"),
            root,
            &filter(&config)
        ));
        assert!(is_relevant(
            Path::new("/repo/Cargo.toml"),
            root,
            &filter(&config)
        ));
    }

    #[test]
    fn source_extension_is_relevant() {
        let config = Config::default();
        let root = Path::new("/repo");
        assert!(is_relevant(
            Path::new("/repo/pkg/src/main.rs"),
            root,
            &filter(&config)
        ));
        assert!(is_relevant(
            Path::new("/repo/pkg/index.ts"),
            root,
            &filter(&config)
        ));
    }

    #[test]
    fn doc_extension_is_relevant() {
        // WATCH-7: docs.extensions (default .md/.rst/.txt/.adoc) were previously
        // ignored entirely, so README edits never triggered a rebuild even though
        // `shire build` indexes them.
        let config = Config::default();
        let root = Path::new("/repo");
        assert!(is_relevant(
            Path::new("/repo/README.md"),
            root,
            &filter(&config)
        ));
        assert!(is_relevant(
            Path::new("/repo/pkg/notes.txt"),
            root,
            &filter(&config)
        ));
    }

    #[test]
    fn shire_toml_is_always_relevant() {
        let config = Config::default();
        let root = Path::new("/repo");
        assert!(is_relevant(
            Path::new("/repo/shire.toml"),
            root,
            &filter(&config)
        ));
    }

    #[test]
    fn unrelated_extension_is_not_relevant() {
        let config = Config::default();
        let root = Path::new("/repo");
        assert!(!is_relevant(
            Path::new("/repo/pkg/image.png"),
            root,
            &filter(&config)
        ));
    }

    #[test]
    fn path_outside_root_is_not_relevant() {
        let config = Config::default();
        let root = Path::new("/repo");
        assert!(!is_relevant(
            Path::new("/elsewhere/main.rs"),
            root,
            &filter(&config)
        ));
    }

    #[test]
    fn excluded_directory_is_not_relevant() {
        // WATCH-7: discovery.exclude (default includes node_modules) was previously
        // ignored, so an edit under a vendored dir triggered a full rebuild pass.
        let config = Config::default();
        let root = Path::new("/repo");
        assert!(!is_relevant(
            Path::new("/repo/node_modules/junk/a.js"),
            root,
            &filter(&config)
        ));
        assert!(!is_relevant(
            Path::new("/repo/vendor/pkg/mod.go"),
            root,
            &filter(&config)
        ));
    }

    #[test]
    fn excluded_directory_check_does_not_match_the_filename_itself() {
        // The indexer's own walk only excludes directory *entries*, not files whose
        // basename happens to collide with an excluded directory name — a source file
        // literally named "target" (default_exclude includes "target") must still be
        // treated normally, not silently dropped.
        let config = Config::default();
        let root = Path::new("/repo");
        assert!(is_relevant(
            Path::new("/repo/src/target.rs"),
            root,
            &filter(&config)
        ));
    }

    #[test]
    fn custom_discovery_marker_and_extension_are_relevant() {
        let toml_str = r#"
[[discovery.custom]]
name = "bazel"
kind = "bazel"
requires = ["BUILD.bazel"]
extensions = [".bzl"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        let root = Path::new("/repo");
        assert!(is_relevant(
            Path::new("/repo/pkg/BUILD.bazel"),
            root,
            &filter(&config)
        ));
        assert!(is_relevant(
            Path::new("/repo/pkg/rules.bzl"),
            root,
            &filter(&config)
        ));
        assert!(!is_relevant(
            Path::new("/repo/pkg/other.custom"),
            root,
            &filter(&config)
        ));
    }

    #[test]
    fn relative_path_is_not_matched_directly() {
        // is_relevant() itself expects absolute paths under an absolute root; a
        // relative path never matches strip_prefix against an absolute root and would
        // otherwise be silently treated as "outside the repo" (WATCH-7). Resolving a
        // relative `--file` argument against the repo root is the CLI's job (see
        // `Commands::Rebuild` in main.rs), not this function's — this test documents
        // that invariant so a future caller doesn't assume relative paths "just work".
        let config = Config::default();
        let root = Path::new("/repo");
        assert!(!is_relevant(
            Path::new("pkg/index.js"),
            root,
            &filter(&config)
        ));
        // The resolved, absolute equivalent is relevant.
        assert!(is_relevant(
            Path::new("/repo/pkg/index.js"),
            root,
            &filter(&config)
        ));
    }
}
