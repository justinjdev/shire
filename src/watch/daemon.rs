use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn pid_path(root: &Path) -> PathBuf {
    root.join(".shire/watch.pid")
}

pub fn sock_path(root: &Path) -> PathBuf {
    root.join(".shire/watch.sock")
}

fn log_path(root: &Path) -> PathBuf {
    root.join(".shire/watch.log")
}

/// Check if the daemon is running by reading the PID file and sending signal 0.
pub fn is_running(root: &Path) -> bool {
    let pid_file = pid_path(root);
    let Ok(contents) = std::fs::read_to_string(&pid_file) else {
        return false;
    };
    let Ok(pid) = contents.trim().parse::<u32>() else {
        return false;
    };
    // kill -0 checks process existence without sending a signal
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Start the daemon by re-exec'ing this binary with `watch --foreground`.
/// Idempotent: returns Ok(()) if already running.
pub fn start_daemon(root: &Path, db: Option<&Path>) -> Result<()> {
    if is_running(root) {
        return Ok(());
    }

    // Clean up stale state files
    let _ = std::fs::remove_file(pid_path(root));
    let _ = std::fs::remove_file(sock_path(root));

    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let log_file = std::fs::File::create(log_path(root))
        .context("failed to create watch.log")?;

    let mut cmd = Command::new(exe);
    cmd.arg("watch")
        .arg("--root")
        .arg(root)
        .arg("--foreground");

    if let Some(db_path) = db {
        cmd.arg("--db").arg(db_path);
    }

    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(log_file);

    let child = cmd.spawn().context("failed to spawn watch daemon")?;

    // Write PID file
    std::fs::write(pid_path(root), child.id().to_string())
        .context("failed to write PID file")?;

    Ok(())
}

/// Stop the daemon by sending SIGTERM and cleaning up state files.
/// Idempotent: returns Ok(()) if not running.
pub fn stop_daemon(root: &Path) -> Result<()> {
    let pid_file = pid_path(root);
    let contents = match std::fs::read_to_string(&pid_file) {
        Ok(c) => c,
        Err(_) => return Ok(()), // No PID file, nothing to stop
    };
    let pid = match contents.trim().parse::<u32>() {
        Ok(p) => p,
        Err(_) => {
            // Invalid PID file, clean up
            let _ = std::fs::remove_file(&pid_file);
            let _ = std::fs::remove_file(sock_path(root));
            return Ok(());
        }
    };

    // Send SIGTERM
    let _ = Command::new("kill")
        .args([&pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    // Clean up state files
    let _ = std::fs::remove_file(&pid_file);
    let _ = std::fs::remove_file(sock_path(root));

    Ok(())
}
