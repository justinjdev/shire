use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn pid_path(root: &Path) -> PathBuf {
    root.join(".shire/watch.pid")
}

pub fn sock_path(root: &Path) -> PathBuf {
    root.join(".shire/watch.sock")
}

/// Read and parse `.shire/watch.pid`. Returns `None` if the file is missing or does not
/// contain a valid PID.
fn read_pid_file(root: &Path) -> Option<u32> {
    std::fs::read_to_string(pid_path(root))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Split a raw `/proc/<pid>/cmdline` buffer (NUL-separated arguments) into owned strings.
/// Pure and independent of the filesystem so it can be unit-tested directly.
fn parse_cmdline(raw: &[u8]) -> Vec<String> {
    raw.split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// Whether a parsed argv looks like `shire watch --foreground` (in either order, with
/// other flags/values interspersed) rather than some unrelated process that happens to
/// have reused the PID.
fn cmdline_looks_like_shire_watch(args: &[String]) -> bool {
    args.iter().any(|a| a == "watch") && args.iter().any(|a| a == "--foreground")
}

/// Parse the process state character out of a raw `/proc/<pid>/stat` line. The command
/// name field (2nd field, in parens) may itself contain spaces or parens, so we look for
/// the *last* `)` and take the first whitespace-separated token after it — matching the
/// documented `/proc/[pid]/stat` format (see `man 5 proc`).
fn parse_proc_stat_state(raw: &str) -> Option<char> {
    let close = raw.rfind(')')?;
    raw[close + 1..].split_whitespace().next()?.chars().next()
}

#[cfg(target_os = "linux")]
fn read_cmdline(pid: u32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    if raw.is_empty() {
        return None;
    }
    Some(parse_cmdline(&raw))
}

#[cfg(target_os = "linux")]
fn read_proc_state(pid: u32) -> Option<char> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_proc_stat_state(&raw)
}

/// Fallback for non-Linux Unixes (macOS): shell out to `ps` instead of reading `/proc`.
#[cfg(all(unix, not(target_os = "linux")))]
fn read_cmdline(pid: u32) -> Option<Vec<String>> {
    let out = Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.split_whitespace().map(str::to_string).collect())
}

#[cfg(all(unix, not(target_os = "linux")))]
fn read_proc_state(pid: u32) -> Option<char> {
    let out = Command::new("ps")
        .args(["-o", "state=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.trim().chars().next()
}

/// Whether `pid` is currently a live (non-zombie) process whose command line looks like
/// a `shire watch --foreground` daemon. This is the ownership check used before signalling
/// a PID read from `.shire/watch.pid`: PIDs are reused aggressively after a reboot, and the
/// pid file can outlive the process it once named (crash, SIGKILL, reboot), so `kill -0`
/// alone is not enough to know it is safe to signal.
fn is_shire_watch_pid(pid: u32) -> bool {
    match read_cmdline(pid) {
        Some(args) if cmdline_looks_like_shire_watch(&args) => {
            !matches!(read_proc_state(pid), Some('Z'))
        }
        _ => false,
    }
}

/// Check if the daemon is running for `root`.
///
/// Prefers a connect probe on the Unix socket — the strongest signal, since only a live
/// listener can accept connections (this also catches a daemon that died mid-startup
/// before writing anything else, e.g. a bind failure). Falls back to the PID file plus
/// an ownership + zombie check, for the brief window between the daemon process starting
/// and it finishing its bind.
pub fn is_running(root: &Path) -> bool {
    if std::os::unix::net::UnixStream::connect(sock_path(root)).is_ok() {
        return true;
    }
    read_pid_file(root).is_some_and(is_shire_watch_pid)
}

/// Start the daemon by re-exec'ing this binary with `watch --foreground`.
/// Idempotent: returns Ok(()) if already running.
pub fn start_daemon(root: &Path, db: Option<&Path>, config: Option<&Path>) -> Result<()> {
    if is_running(root) {
        return Ok(());
    }

    // Clean up stale state files
    let _ = std::fs::remove_file(pid_path(root));
    let _ = std::fs::remove_file(sock_path(root));

    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let mut cmd = Command::new(exe);
    cmd.arg("watch").arg("--root").arg(root).arg("--foreground");

    if let Some(db_path) = db {
        cmd.arg("--db").arg(db_path);
    }

    if let Some(cfg_path) = config {
        cmd.arg("--config").arg(cfg_path);
    }

    let shire_dir = root.join(".shire");
    let _ = std::fs::create_dir_all(&shire_dir);
    let stderr_log = shire_dir.join("watch-stderr.log");
    let stderr_target = match std::fs::File::create(&stderr_log) {
        Ok(file) => std::process::Stdio::from(file),
        Err(e) => {
            eprintln!(
                "Warning: failed to open {}: {e}; daemon stderr will be discarded",
                stderr_log.display()
            );
            std::process::Stdio::null()
        }
    };

    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr_target);

    let mut child = cmd.spawn().context("failed to spawn watch daemon")?;

    // Give the daemon a brief moment to fail fast (bind errors — e.g. a socket path
    // over SUN_LEN — surface here) before we report success and leave a PID file
    // pointing at a process that no longer exists.
    std::thread::sleep(std::time::Duration::from_millis(300));
    match child.try_wait() {
        Ok(Some(status)) => {
            anyhow::bail!(
                "Watch daemon exited immediately ({status}). See {} for details.",
                stderr_log.display()
            );
        }
        Ok(None) => {
            // Still running after the grace period — looks healthy.
        }
        Err(e) => {
            tracing::warn!(%e, "failed to check daemon status after spawn");
        }
    }

    // Write PID file
    std::fs::write(pid_path(root), child.id().to_string()).context("failed to write PID file")?;

    Ok(())
}

/// Stop the daemon by sending SIGTERM and cleaning up state files.
/// Idempotent: returns Ok(()) if not running.
///
/// Waits (up to ~5s) for the process to actually exit before removing the PID/socket
/// files. Deleting the PID file immediately after sending the signal made `is_running()`
/// return `false` on the very next check regardless of whether the daemon had actually
/// exited, which made callers' "did it really stop?" checks meaningless (WATCH-6) and let
/// `shire clean` race the daemon's own shutdown. If the process is still alive when the
/// wait times out, the PID file is left in place so a caller's own `is_running()` check
/// still reports the daemon as running.
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

    if !is_shire_watch_pid(pid) {
        // The PID either doesn't exist, is a zombie, or belongs to some unrelated
        // process (pid reuse after a reboot/crash is common — see WATCH-1). Signalling
        // it would risk killing something else entirely, so refuse; the pid file is no
        // longer trustworthy either way, so drop it (and any stale socket) rather than
        // leaving it to be misread by a future stop/clean.
        eprintln!(
            "Warning: PID {pid} in {} does not look like a shire watch daemon; not signalling it. Removing stale state.",
            pid_file.display()
        );
        let _ = std::fs::remove_file(&pid_file);
        let _ = std::fs::remove_file(sock_path(root));
        return Ok(());
    }

    // Send SIGTERM
    let _ = Command::new("kill")
        .args([&pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    let mut exited = false;
    for _ in 0..50 {
        if !is_shire_watch_pid(pid) {
            exited = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    if exited {
        let _ = std::fs::remove_file(&pid_file);
        let _ = std::fs::remove_file(sock_path(root));
    }

    Ok(())
}

/// Print a human-readable status report for the watch daemon at `root`: PID file
/// contents, whether that PID looks like an owned, live shire daemon, and whether the
/// socket is actually accepting connections. Always succeeds (this is a diagnostic).
pub fn print_status(root: &Path) {
    let pid_file = pid_path(root);
    let sock = sock_path(root);
    let pid = read_pid_file(root);
    let pid_owned = pid.is_some_and(is_shire_watch_pid);
    let socket_live = std::os::unix::net::UnixStream::connect(&sock).is_ok();

    println!("root:      {}", root.display());
    println!("pid file:  {}", pid_file.display());
    match pid {
        Some(p) if pid_owned => println!("pid:       {p} (looks like a shire watch daemon)"),
        Some(p) => println!("pid:       {p} (does NOT look like a shire watch daemon)"),
        None => println!("pid:       (no pid file)"),
    }
    println!("socket:    {}", sock.display());
    println!("listening: {}", if socket_live { "yes" } else { "no" });

    let status = match (socket_live, pid_owned) {
        (true, true) => "running",
        (true, false) => {
            "a process is listening on the socket, but the pid file doesn't match a known shire daemon"
        }
        (false, true) => {
            "pid looks like a shire watch daemon, but it is not answering on its socket (starting up, or stuck)"
        }
        (false, false) => "not running",
    };
    println!("status:    {status}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    // --- pure helpers: cmdline / /proc/stat parsing ---

    #[test]
    fn parse_cmdline_splits_on_nul() {
        let raw = b"/usr/bin/shire\0watch\0--root\0/repo\0--foreground\0";
        assert_eq!(
            parse_cmdline(raw),
            vec!["/usr/bin/shire", "watch", "--root", "/repo", "--foreground"]
        );
    }

    #[test]
    fn parse_cmdline_empty_is_empty() {
        assert!(parse_cmdline(b"").is_empty());
    }

    #[test]
    fn cmdline_recognizes_shire_watch_daemon() {
        let args: Vec<String> = ["/usr/bin/shire", "watch", "--root", "/repo", "--foreground"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(cmdline_looks_like_shire_watch(&args));
    }

    #[test]
    fn cmdline_rejects_unrelated_process() {
        let args: Vec<String> = ["/usr/bin/sleep", "300"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!cmdline_looks_like_shire_watch(&args));
    }

    #[test]
    fn cmdline_rejects_missing_foreground_flag() {
        // `shire watch --root .` without --foreground is the *launcher*, not the daemon
        // itself, and re-launching it isn't idempotent the way signalling the real
        // daemon is — it should not be treated as an ownership match.
        let args: Vec<String> = ["/usr/bin/shire", "watch", "--root", "."]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!cmdline_looks_like_shire_watch(&args));
    }

    #[test]
    fn proc_stat_state_parses_simple_comm() {
        assert_eq!(
            parse_proc_stat_state("1234 (shire) S 1 1234 1234 0 -1 4194304"),
            Some('S')
        );
    }

    #[test]
    fn proc_stat_state_parses_zombie() {
        assert_eq!(
            parse_proc_stat_state("2209 (shire) Z 1 2209 2209 0 -1 4194432"),
            Some('Z')
        );
    }

    #[test]
    fn proc_stat_state_handles_parens_in_comm() {
        // comm can itself contain ')' (e.g. a renamed process); state parsing must use
        // the *last* ')' in the line, not the first.
        assert_eq!(
            parse_proc_stat_state("99 (weird)comm) R 1 99 99 0 -1 0"),
            Some('R')
        );
    }

    // --- is_shire_watch_pid / is_running / stop_daemon against a real spawned process ---
    //
    // These spawn a real `sleep` child to get a genuine, valid PID without depending on
    // `/proc` layout details beyond what the parsers above already cover directly.

    #[test]
    fn is_shire_watch_pid_false_for_unrelated_process() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("failed to spawn sleep");
        assert!(!is_shire_watch_pid(child.id()));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn is_shire_watch_pid_false_for_dead_pid() {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("failed to spawn true");
        let pid = child.id();
        let _ = child.wait();
        // Give the kernel a moment to fully reap it in CI environments.
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!is_shire_watch_pid(pid));
    }

    #[test]
    fn stop_daemon_refuses_to_signal_unrelated_process() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".shire")).unwrap();

        let mut victim = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("failed to spawn sleep");
        let victim_pid = victim.id();

        let mut f = std::fs::File::create(pid_path(dir.path())).unwrap();
        write!(f, "{victim_pid}").unwrap();
        drop(f);

        stop_daemon(dir.path()).expect("stop_daemon should not error");

        // The victim must still be alive — stop_daemon must not have signalled it.
        assert!(
            victim.try_wait().unwrap().is_none(),
            "stop_daemon killed an unrelated process"
        );
        // The stale pid file should have been removed so it isn't misread again.
        assert!(!pid_path(dir.path()).exists());

        let _ = victim.kill();
        let _ = victim.wait();
    }

    #[test]
    fn stop_daemon_is_noop_with_no_pid_file() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".shire")).unwrap();
        assert!(stop_daemon(dir.path()).is_ok());
    }

    #[test]
    fn is_running_false_with_no_state() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".shire")).unwrap();
        assert!(!is_running(dir.path()));
    }

    #[test]
    fn is_running_false_for_stale_unrelated_pid() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".shire")).unwrap();

        let mut victim = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("failed to spawn sleep");

        std::fs::write(pid_path(dir.path()), victim.id().to_string()).unwrap();
        assert!(!is_running(dir.path()));

        let _ = victim.kill();
        let _ = victim.wait();
    }
}
