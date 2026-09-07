use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn pid_path(root: &Path) -> PathBuf {
    root.join(".shire/watch.pid")
}

pub fn sock_path(root: &Path) -> PathBuf {
    root.join(".shire/watch.sock")
}

/// Whether `path` is itself a symlink (`lstat`, not `stat` — this must not follow it).
/// Used to refuse acting on `.shire/watch.pid` or `.shire/watch.sock` if either is a
/// symlink: a hostile or careless repo could plant one pointing at another repo's pid
/// file or live socket, and reading/connecting through it would let a command meant
/// for *this* repo affect that other one instead.
pub fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Read and parse `.shire/watch.pid`. Returns `None` if the file is missing, is itself
/// a symlink (never trusted), or does not contain a valid PID.
fn read_pid_file(root: &Path) -> Option<u32> {
    let path = pid_path(root);
    if is_symlink(&path) {
        return None;
    }
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Connect to `.shire/watch.sock` for `root`, refusing to follow it if the path is
/// itself a symlink (e.g. planted to point at another repo's live socket).
fn connect_if_not_symlink(root: &Path) -> Option<std::os::unix::net::UnixStream> {
    let sock = sock_path(root);
    if is_symlink(&sock) {
        return None;
    }
    std::os::unix::net::UnixStream::connect(&sock).ok()
}

/// Split a raw `/proc/<pid>/cmdline` buffer (NUL-separated arguments) into owned strings.
/// Pure and independent of the filesystem so it can be unit-tested directly.
#[cfg(any(target_os = "linux", test))]
fn parse_cmdline(raw: &[u8]) -> Vec<String> {
    raw.split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// Whether `args`'s first element (argv[0]) plausibly names shire's own binary. Reuses
/// `exe_path_is_shire`'s acceptance rule (naming heuristic, or same file as this
/// process's own `current_exe()`) so a legitimate non-`shire`-prefixed wrapper (e.g.
/// `myshire`) started by itself still passes when re-verified by that same binary —
/// `start_daemon` always spawns via `Command::new(current_exe())`, so argv[0] for a
/// genuine daemon is always the exact path that was used to invoke shire.
fn argv0_looks_like_shire(args: &[String]) -> bool {
    args.first().is_some_and(|a| exe_path_is_shire(a))
}

/// Whether `args` contains a `--root` flag whose value names the same repository root as
/// `root`, compared either verbatim or after canonicalizing the argument (the argument
/// itself might not exist any more or might not be canonicalizable, e.g. mid-deletion —
/// in that case only the verbatim comparison can match).
///
/// On Linux, `/proc/<pid>/cmdline` preserves the original NUL-separated argv, so the
/// `--root` value is always exactly one element of `args` regardless of what it
/// contains. The macOS fallback (`read_cmdline`) instead parses `ps -o command=`'s
/// space-joined text back into tokens via `split_whitespace()`, which loses the fact
/// that a value containing a space (e.g. `--root "/Users/me/my repo"`) was ever one
/// argument — it comes back as two consecutive tokens. To tolerate that without
/// requiring shell-level quoting information this doesn't have, for each `--root`
/// token this tries re-joining an increasing run of the tokens that follow it with a
/// single space, checking the joined string against `root` (verbatim, then
/// canonicalized) at every length rather than only ever the very next token. Matching
/// requires an exact equality at some length, never a prefix match, so joining too few
/// or too many tokens simply fails to match instead of falsely succeeding.
fn args_contain_root(args: &[String], root: &Path) -> bool {
    for (i, arg) in args.iter().enumerate() {
        if arg != "--root" {
            continue;
        }
        let mut joined = String::new();
        for token in &args[i + 1..] {
            if !joined.is_empty() {
                joined.push(' ');
            }
            joined.push_str(token);
            if Path::new(&joined) == root
                || std::fs::canonicalize(&joined)
                    .map(|c| c == root)
                    .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}

/// Whether a parsed argv looks like `shire watch --root <root> --foreground` (in any
/// order, with other flags/values interspersed) rather than some unrelated process that
/// happens to have reused the PID — or, worse, a same-uid tool whose binary name merely
/// satisfies the `shire`/`shire-*` naming heuristic and which happens to carry the bare
/// tokens "watch"/"--foreground" somewhere in its own arguments (WW-3-3). Requires
/// argv[0] to plausibly be shire's own binary AND a `--root` argument naming this exact
/// repository AND the two literal tokens `start_daemon` always passes.
fn cmdline_looks_like_shire_watch(args: &[String], root: &Path) -> bool {
    argv0_looks_like_shire(args)
        && args_contain_root(args, root)
        && args.iter().any(|a| a == "watch")
        && args.iter().any(|a| a == "--foreground")
}

/// Parse the process state character out of a raw `/proc/<pid>/stat` line. The command
/// name field (2nd field, in parens) may itself contain spaces or parens, so we look for
/// the *last* `)` and take the first whitespace-separated token after it — matching the
/// documented `/proc/[pid]/stat` format (see `man 5 proc`).
#[cfg(any(target_os = "linux", test))]
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

/// Whether an executable path plausibly belongs to shire's own binary. Two ways to
/// pass:
///
/// 1. Its basename is exactly `shire`, or starts with `shire-` or `shire.`
///    (`shire-v0.7`, `shire-0.6.2`, `shire.old` — the last from a manual
///    `mv shire shire.old && cp new shire`-style in-place upgrade, which a shell
///    completion or backup convention names with a `.` rather than a `-`) — covers a
///    versioned install, a release tarball's binary renamed on download, or a copy kept
///    side by side during an upgrade. Does not require the file to exist on disk, so it
///    still works after the on-disk binary was replaced (see the `" (deleted)"` handling
///    below). Deliberately requires a `-` or `.` separator rather than a bare prefix
///    match: without it, an unrelated binary whose name merely *starts with* "shire"
///    with no separator at all (e.g. `shireling` or `shirecheck`) would be misidentified
///    as shire's own daemon if its PID were ever reused for one after a crash/reboot.
///    Note this does NOT rule out a same-uid tool that happens to be named
///    `shire-<anything>`/`shire.<anything>` (e.g. `shire-metrics-exporter`) — that
///    residual case is left to the separate cmdline check in `check_pid_ownership`,
///    which additionally requires argv[0] to itself look like shire and a `--root`
///    argument naming this exact repository, alongside the literal argv tokens "watch"
///    and "--foreground" (WW-3-3).
/// 2. It resolves (after canonicalization) to the exact same file as this process's own
///    `std::env::current_exe()` — covers a wrapper name that doesn't start with `shire`
///    at all, so long as it truly is the same binary that would be spawned by
///    `start_daemon` (which always re-execs `current_exe()`).
///
/// Handles the Linux kernel's `readlink(/proc/<pid>/exe)` appending `" (deleted)"` when
/// the on-disk binary was replaced/removed after exec (e.g. an in-place upgrade of a
/// renamed install while the daemon kept running) — that's still shire, just an older
/// copy. The basename heuristic above already covers a `shire`/`shire-*`/`shire.*`-named
/// install; for a wrapper name that doesn't match it, the identity fallback below still
/// applies — see `canonical_dir_and_name` for why it compares directory+basename rather
/// than the full canonicalized path a live file would allow.
fn exe_path_is_shire(raw: &str) -> bool {
    let deleted = raw.ends_with(" (deleted)");
    let trimmed = raw.trim_end_matches(" (deleted)");
    let candidate = Path::new(trimmed);

    if let Some(name) = candidate.file_name().and_then(|f| f.to_str())
        && (name == "shire" || name.starts_with("shire-") || name.starts_with("shire."))
    {
        return true;
    }

    if deleted {
        return match std::env::current_exe() {
            Ok(cur) => same_install_location(candidate, &cur),
            Err(_) => false,
        };
    }

    match (
        candidate.canonicalize(),
        std::env::current_exe().and_then(|p| p.canonicalize()),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Canonicalize `path`'s parent directory (leaving the final path component alone) and
/// pair it with the basename, so two paths can still be compared for "same install
/// location" even when `path` itself no longer exists on disk — `Path::canonicalize`
/// requires the full path to exist, which always fails for a `readlink(/proc/<pid>/exe)`
/// result once the on-disk binary has been replaced or removed. Returns `None` if the
/// basename or the parent directory can't be resolved.
fn canonical_dir_and_name(path: &Path) -> Option<(PathBuf, std::ffi::OsString)> {
    let name = path.file_name()?.to_os_string();
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    Some((dir.canonicalize().ok()?, name))
}

/// Whether `deleted_link_target` — the pre-deletion path from a `"<path> (deleted)"`
/// `/proc/<pid>/exe` readlink, already stripped of that suffix — names the same install
/// location as `current_exe`: same canonicalized parent directory, same basename. Used
/// by `exe_path_is_shire`'s `" (deleted)"` branch to recognize an in-place upgrade (the
/// on-disk binary at a renamed install's path was replaced while the daemon kept
/// running) without requiring the old and new files to share an inode — a plain
/// `canonicalize()` of the full path always fails once the file is gone. Factored out as
/// a pure function of two paths (rather than reading `std::env::current_exe()` itself)
/// so it can be exercised in unit tests with synthetic paths: the scenario it exists for
/// can't otherwise be reproduced without actually replacing the test binary underneath
/// the running test process.
fn same_install_location(deleted_link_target: &Path, current_exe: &Path) -> bool {
    match (
        canonical_dir_and_name(deleted_link_target),
        canonical_dir_and_name(current_exe),
    ) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Result of checking a process's executable identity.
enum ExeCheck {
    /// The executable's path was read successfully (may carry a trailing
    /// `" (deleted)"` marker — see `exe_path_is_shire`).
    Path(String),
    /// Could not be determined — most commonly `/proc/<pid>/exe` returning EACCES
    /// because the process is owned by another user (reading it requires the same
    /// permission as a ptrace attach). Callers must not treat this the same as a
    /// confirmed mismatch: we simply don't know.
    Unverifiable,
}

#[cfg(target_os = "linux")]
fn read_exe_path(pid: u32) -> ExeCheck {
    match std::fs::read_link(format!("/proc/{pid}/exe")) {
        Ok(link) => match link.to_str() {
            Some(path) => ExeCheck::Path(path.to_string()),
            None => ExeCheck::Unverifiable,
        },
        Err(_) => ExeCheck::Unverifiable,
    }
}

/// Fallback for non-Linux Unixes (macOS): `ps -o comm=` gives the full executable path
/// there (unlike Linux's truncated `comm`).
#[cfg(all(unix, not(target_os = "linux")))]
fn read_exe_path(pid: u32) -> ExeCheck {
    let out = match Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
    {
        Ok(o) => o,
        Err(_) => return ExeCheck::Unverifiable,
    };
    if !out.status.success() {
        return ExeCheck::Unverifiable;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return ExeCheck::Unverifiable;
    }
    ExeCheck::Path(trimmed.to_string())
}

/// Result of checking whether a PID is safe to signal (and whether its state files are
/// safe to treat as stale and remove).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PidOwnership {
    /// Confirmed: live, non-zombie, executable is shire, cmdline matches
    /// `watch --foreground`.
    Owned,
    /// Confirmed NOT an owned shire watch daemon — cmdline doesn't match, it's a
    /// zombie, or the process no longer exists at all. Safe to treat as stale.
    NotShire,
    /// The cmdline matched, but the executable identity could not be verified
    /// (typically the process is owned by another user, so `/proc/<pid>/exe` is not
    /// readable). Must NOT be signalled, and — unlike `NotShire` — its pid/socket
    /// files must NOT be deleted either: we don't have enough information to call
    /// them stale, and the process may well be a live daemon another user started.
    Unverifiable,
}

/// Whether `pid` is safe to signal as a shire watch daemon, and (when it isn't) whether
/// its state files are safe to clean up. This is the ownership check used before
/// signalling a PID read from `.shire/watch.pid`: PIDs are reused aggressively after a
/// reboot, and the pid file can outlive the process it once named (crash, SIGKILL,
/// reboot), so `kill -0` alone is not enough to know it is safe to signal. The
/// executable check specifically closes a spoofing gap the cmdline check alone left
/// open: any process that happens to carry the bare argv tokens "watch" and
/// "--foreground" plus a same-uid `shire`/`shire-*`-named binary — e.g.
/// `some-other-daemon --foreground` with a subcommand named `watch`, or a deliberately
/// crafted invocation — would otherwise pass (WW-3-3; `root` is required so the argv's
/// `--root` value can be checked against the repository this call is actually for).
fn check_pid_ownership(pid: u32, root: &Path) -> PidOwnership {
    let Some(args) = read_cmdline(pid) else {
        return PidOwnership::NotShire;
    };
    let cmdline_matches = cmdline_looks_like_shire_watch(&args, root);
    if !cmdline_matches {
        return PidOwnership::NotShire;
    }
    let exe = read_exe_path(pid);
    let is_zombie = matches!(read_proc_state(pid), Some('Z'));
    ownership_from_checks(exe, is_zombie)
}

/// Pure decision logic for `check_pid_ownership`, factored out so the tri-state result
/// — including the `Unverifiable` case, which is otherwise only reachable via a real
/// cross-user process — can be exercised directly in unit tests. Assumes the cmdline
/// match has already been confirmed by the caller.
fn ownership_from_checks(exe: ExeCheck, is_zombie: bool) -> PidOwnership {
    match exe {
        ExeCheck::Unverifiable => PidOwnership::Unverifiable,
        ExeCheck::Path(path) if !exe_path_is_shire(&path) => PidOwnership::NotShire,
        ExeCheck::Path(_) if is_zombie => PidOwnership::NotShire,
        ExeCheck::Path(_) => PidOwnership::Owned,
    }
}

/// Check if the daemon is running for `root`.
///
/// Prefers a connect probe on the Unix socket — the strongest signal, since only a live
/// listener can accept connections (this also catches a daemon that died mid-startup
/// before writing anything else, e.g. a bind failure). Falls back to the PID file plus
/// an ownership + zombie check, for the brief window between the daemon process starting
/// and it finishing its bind. A PID whose ownership can't be verified (belongs to
/// another user) is conservatively treated as running, rather than assumed stale.
pub fn is_running(root: &Path) -> bool {
    if connect_if_not_symlink(root).is_some() {
        return true;
    }
    match read_pid_file(root) {
        Some(pid) => !matches!(check_pid_ownership(pid, root), PidOwnership::NotShire),
        None => false,
    }
}

/// What `stop_daemon` should do about a PID read from `.shire/watch.pid`, decided from
/// its ownership verdict plus whether *something* is actually listening on the socket
/// right now. Factored out from `stop_daemon` so this decision can be exercised
/// directly in unit tests with an injected ownership value and connect-probe result,
/// without spawning real processes or binding real sockets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopAction {
    /// Confirmed shire watch daemon: signal it, then remove its state files once it
    /// has actually exited.
    Signal,
    /// Confirmed NOT shire, and nothing is listening on the socket either: the pid
    /// file is genuinely stale (crash, reboot, PID reuse) and safe to remove without
    /// signalling anything.
    RemoveStaleFiles,
    /// Either ownership couldn't be verified (`Unverifiable`), or the PID looks
    /// unowned but the socket is still live (WW-3-2: a different shire binary, or the
    /// same install replaced in place while the daemon kept running, both make
    /// ownership say `NotShire` even though the daemon is demonstrably still up). In
    /// both cases: don't signal, and don't delete state that may belong to a live
    /// daemon this process simply can't positively identify.
    RefuseAndKeep,
}

/// Pure decision logic behind `StopAction` — see its variants for the reasoning.
fn decide_stop_action(ownership: PidOwnership, socket_live: bool) -> StopAction {
    match ownership {
        PidOwnership::Owned => StopAction::Signal,
        PidOwnership::Unverifiable => StopAction::RefuseAndKeep,
        PidOwnership::NotShire if socket_live => StopAction::RefuseAndKeep,
        PidOwnership::NotShire => StopAction::RemoveStaleFiles,
    }
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
/// wait times out, the PID file is left in place (so a caller's own `is_running()` check
/// still reports the daemon as running) and this returns an error — the daemon only
/// handles SIGTERM between rebuilds, so a slow/uninterruptible rebuild can easily outlast
/// the wait, and silently reporting success in that case (WATCHCLI-2-2) told the user the
/// opposite of what actually happened.
pub fn stop_daemon(root: &Path) -> Result<()> {
    let pid_file = pid_path(root);

    if is_symlink(&pid_file) {
        eprintln!(
            "Warning: {} is a symlink; refusing to read or act on it (it could point at \
             another repo's pid file).",
            pid_file.display()
        );
        return Ok(());
    }

    let contents = match std::fs::read_to_string(&pid_file) {
        Ok(c) => c,
        Err(_) => return Ok(()), // No PID file, nothing to stop
    };
    let pid = match contents.trim().parse::<u32>() {
        Ok(p) => p,
        Err(_) => {
            // Invalid (unparseable) PID file. There's no PID to check ownership of, but
            // that doesn't make the socket file safe to remove unconditionally: a
            // corrupted pid file next to a socket a real daemon is still listening on
            // (e.g. a crash or a concurrent writer truncated only the pid file) would
            // otherwise get its live socket deleted right along with the garbage pid
            // file. Probe it first, and keep it if something answers.
            let _ = std::fs::remove_file(&pid_file);
            if connect_if_not_symlink(root).is_none() {
                let _ = std::fs::remove_file(sock_path(root));
            } else {
                eprintln!(
                    "Warning: {} contained an unparseable pid, but something is still \
                     listening on {} — leaving the socket file in place.",
                    pid_file.display(),
                    sock_path(root).display()
                );
            }
            return Ok(());
        }
    };

    let ownership = check_pid_ownership(pid, root);
    // Probe the socket *before* touching any state: a `NotShire` verdict alone does not
    // mean the process is dead or unrelated — it can equally mean the daemon is alive
    // but couldn't be positively identified (a different shire binary verifying it, or
    // the same install path replaced in place while the daemon kept running — WW-3-2).
    // Deleting the pid/socket files of a demonstrably live listener is exactly the
    // WATCH-6 failure this whole check exists to prevent, so the socket being live
    // downgrades that case to the same "refuse and keep" treatment as `Unverifiable`.
    let socket_live = connect_if_not_symlink(root).is_some();

    match decide_stop_action(ownership, socket_live) {
        StopAction::RemoveStaleFiles => {
            // The PID either doesn't exist, is a zombie, or belongs to some unrelated
            // process (pid reuse after a reboot/crash is common — see WATCH-1), and
            // nothing is listening on the socket either, so it's genuinely stale.
            // Signalling it would risk killing something else entirely, so refuse; the
            // pid file is no longer trustworthy either way, so drop it (and any stale
            // socket) rather than leaving it to be misread by a future stop/clean.
            eprintln!(
                "Warning: PID {pid} in {} does not look like a shire watch daemon; not signalling it. Removing stale state.",
                pid_file.display()
            );
            let _ = std::fs::remove_file(&pid_file);
            let _ = std::fs::remove_file(sock_path(root));
            return Ok(());
        }
        StopAction::RefuseAndKeep if ownership == PidOwnership::Unverifiable => {
            // Unlike RemoveStaleFiles, this must NOT be treated as stale: the process
            // may well be a live, legitimate shire daemon started by another user, and
            // deleting its state files out from under it would orphan it and let a
            // later `shire watch` start a duplicate.
            eprintln!(
                "Warning: PID {pid} in {} appears to belong to another user (its executable \
                 could not be verified); not signalling it, and leaving its state files alone.",
                pid_file.display()
            );
            return Ok(());
        }
        StopAction::RefuseAndKeep => {
            // NotShire but the socket is live: same "don't touch its state" treatment
            // as Unverifiable, but reported as a *failure* rather than a quiet Ok(()) —
            // unlike Unverifiable (which may just be another user's legitimate daemon
            // we simply can't introspect), here the caller asked to stop a demonstrably
            // live listener and nothing was actually stopped. Returning Ok(()) would
            // let a `shire watch --stop && rm -rf .shire`-style script see rc=0 and
            // proceed to delete `.shire` out from under a daemon that is still running.
            anyhow::bail!(
                "PID {pid} in {} does not look like a shire watch daemon, but \
                 something is still listening on {} — it may be a live daemon this process \
                 simply could not positively identify (a different shire binary than the one \
                 that started it, or the same install replaced in place while it kept \
                 running). Not signalling it, and leaving its pid/socket files alone; if \
                 you're certain it isn't shire, stop it manually before removing those files.",
                pid_file.display(),
                sock_path(root).display()
            );
        }
        StopAction::Signal => {}
    }

    // Send SIGTERM
    let _ = Command::new("kill")
        .args([&pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    let mut exited = false;
    for _ in 0..50 {
        if !matches!(check_pid_ownership(pid, root), PidOwnership::Owned) {
            exited = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    if exited {
        let _ = std::fs::remove_file(&pid_file);
        let _ = std::fs::remove_file(sock_path(root));
        return Ok(());
    }

    anyhow::bail!(
        "Sent SIGTERM to watch daemon (pid {pid}), but it had not stopped after 5s. It may \
         be finishing an uninterruptible rebuild — the daemon only handles SIGTERM between \
         rebuilds. Its pid/socket files were left in place; retry `shire watch --stop` or \
         check `shire watch --status`."
    )
}

/// Print a human-readable status report for the watch daemon at `root`: PID file
/// contents, whether that PID looks like an owned, live shire daemon, and whether the
/// socket is actually accepting connections. Always succeeds (this is a diagnostic).
pub fn print_status(root: &Path) {
    let pid_file = pid_path(root);
    let sock = sock_path(root);

    if is_symlink(&pid_file) {
        println!("root:      {}", root.display());
        println!(
            "pid file:  {} (symlink — refusing to read it)",
            pid_file.display()
        );
        println!("socket:    {}", sock.display());
        println!("status:    cannot verify (pid file is a symlink)");
        return;
    }

    let pid = read_pid_file(root);
    let ownership = pid.map(|p| check_pid_ownership(p, root));
    let sock_is_symlink = is_symlink(&sock);
    let socket_live = !sock_is_symlink && std::os::unix::net::UnixStream::connect(&sock).is_ok();

    println!("root:      {}", root.display());
    println!("pid file:  {}", pid_file.display());
    match (pid, ownership) {
        (Some(p), Some(PidOwnership::Owned)) => {
            println!("pid:       {p} (looks like a shire watch daemon)")
        }
        (Some(p), Some(PidOwnership::Unverifiable)) => {
            println!("pid:       {p} (belongs to another user — cannot verify)")
        }
        (Some(p), _) => println!("pid:       {p} (does NOT look like a shire watch daemon)"),
        (None, _) => println!("pid:       (no pid file)"),
    }
    if sock_is_symlink {
        println!(
            "socket:    {} (symlink — refusing to connect through it)",
            sock.display()
        );
    } else {
        println!("socket:    {}", sock.display());
    }
    println!("listening: {}", if socket_live { "yes" } else { "no" });

    let status = match (socket_live, ownership) {
        (true, Some(PidOwnership::Owned)) => "running",
        (true, _) => {
            "a process is listening on the socket, but the pid file doesn't match a known shire daemon"
        }
        (false, Some(PidOwnership::Owned)) => {
            "pid looks like a shire watch daemon, but it is not answering on its socket (starting up, or stuck)"
        }
        (false, Some(PidOwnership::Unverifiable)) => {
            "pid belongs to another user and is not answering on its socket — cannot verify"
        }
        (false, _) => "not running",
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
        assert!(cmdline_looks_like_shire_watch(&args, Path::new("/repo")));
    }

    #[test]
    fn cmdline_rejects_unrelated_process() {
        let args: Vec<String> = ["/usr/bin/sleep", "300"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!cmdline_looks_like_shire_watch(&args, Path::new("/repo")));
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
        assert!(!cmdline_looks_like_shire_watch(&args, Path::new(".")));
    }

    #[test]
    fn cmdline_rejects_shire_named_binary_with_no_matching_root_flag() {
        // WW-3-3 repro: a foreign, same-uid tool named `shire-metrics` carries the bare
        // "watch"/"--foreground" tokens somewhere in its own argv (e.g. positional
        // arguments to an unrelated `-c` script), but never names this repo's --root —
        // it must not be mistaken for this repo's daemon.
        let args: Vec<String> = [
            "/tmp/bin/shire-metrics",
            "-c",
            "sleep 300; true",
            "fakearg0",
            "watch",
            "--foreground",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert!(!cmdline_looks_like_shire_watch(&args, Path::new("/repo")));
    }

    #[test]
    fn cmdline_rejects_a_root_flag_naming_a_different_repository() {
        let args: Vec<String> = [
            "/usr/bin/shire",
            "watch",
            "--root",
            "/other-repo",
            "--foreground",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert!(!cmdline_looks_like_shire_watch(&args, Path::new("/repo")));
    }

    #[test]
    fn args_contain_root_reconstructs_a_space_split_root_from_a_ps_style_command_line() {
        // Simulates the macOS fallback: read_cmdline() for non-Linux targets parses
        // `ps -o command=`'s single space-joined string back into tokens via
        // split_whitespace(), so a --root value that itself contains a space (a real,
        // common macOS path like "/Users/me/my repo") comes back as two consecutive
        // tokens instead of one.
        let fake_ps_line = "/usr/bin/shire watch --root /Users/me/my repo --foreground";
        let args: Vec<String> = fake_ps_line
            .split_whitespace()
            .map(str::to_string)
            .collect();
        assert!(args_contain_root(&args, Path::new("/Users/me/my repo")));
        assert!(cmdline_looks_like_shire_watch(
            &args,
            Path::new("/Users/me/my repo")
        ));
    }

    #[test]
    fn args_contain_root_does_not_match_a_different_root_via_the_reconstruction() {
        let fake_ps_line = "/usr/bin/shire watch --root /Users/me/my repo --foreground";
        let args: Vec<String> = fake_ps_line
            .split_whitespace()
            .map(str::to_string)
            .collect();
        // None of the joined prefixes of the tokens following --root ("/Users/me/my",
        // "/Users/me/my repo", "/Users/me/my repo --foreground", ...) equal this
        // unrelated root, so it must not match.
        assert!(!args_contain_root(&args, Path::new("/Users/me/other repo")));
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
    fn check_pid_ownership_not_shire_for_unrelated_process() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("failed to spawn sleep");
        assert_eq!(
            check_pid_ownership(child.id(), Path::new("/repo")),
            PidOwnership::NotShire
        );
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn check_pid_ownership_not_shire_for_dead_pid() {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("failed to spawn true");
        let pid = child.id();
        let _ = child.wait();
        // Give the kernel a moment to fully reap it in CI environments.
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_eq!(
            check_pid_ownership(pid, Path::new("/repo")),
            PidOwnership::NotShire
        );
    }

    #[test]
    fn exe_path_is_shire_accepts_shire_and_renamed_or_versioned_copies() {
        assert!(exe_path_is_shire("/usr/bin/shire"));
        assert!(exe_path_is_shire("shire"));
        // Linux appends this when the on-disk binary was replaced/removed after exec.
        assert!(exe_path_is_shire("/usr/bin/shire (deleted)"));
        // WATCHCLI-2-1: a versioned install, a renamed release tarball binary, or a
        // side-by-side upgrade copy must still be recognized as shire's own binary.
        assert!(exe_path_is_shire("/opt/shire-v0.7/shire-v0.7"));
        assert!(exe_path_is_shire("/opt/bin/shire-0.6.2"));
        // A manual `mv shire shire.old && cp new shire` in-place upgrade leaves the
        // still-running old daemon's exe link resolving to "shire.old" — a `.`
        // separator must be accepted just like the hyphenated form above.
        assert!(exe_path_is_shire("/opt/bin/shire.old"));
        assert!(!exe_path_is_shire("/usr/bin/python3"));
        assert!(!exe_path_is_shire("/bin/sleep"));
        assert!(!exe_path_is_shire(""));
        // An unrelated binary that merely *starts with* "shire" (no separator) must
        // not be misidentified as shire's own daemon — otherwise a reused PID from a
        // crashed daemon could later match a completely different long-running
        // process from an unrelated tool and get signalled as if it were shire.
        assert!(!exe_path_is_shire("/usr/bin/shireling"));
        assert!(!exe_path_is_shire("/opt/bin/shirecheck"));
    }

    #[test]
    fn exe_path_is_shire_accepts_a_wrapper_name_that_is_the_same_file_as_current_exe() {
        // A wrapper/alias that doesn't even start with "shire" is still recognized as
        // long as it resolves to the exact same on-disk file as this test binary's own
        // current_exe() — the fallback path start_daemon relies on implicitly, since it
        // always re-execs current_exe().
        //
        // Symlink to a name that deliberately does NOT start with "shire" at all: under
        // `cargo test`, current_exe() itself is `.../deps/shire-<hash>`, whose basename
        // *does* start with "shire-" and would satisfy the basename check on its own —
        // asserting on current_exe()'s path directly would pass without ever
        // exercising the canonicalize()/current_exe() identity fallback this test is
        // named for.
        let current = std::env::current_exe().unwrap();
        let dir = TempDir::new().unwrap();
        let wrapper = dir.path().join("totally-different-name");
        std::os::unix::fs::symlink(&current, &wrapper).unwrap();
        assert!(exe_path_is_shire(wrapper.to_str().unwrap()));
    }

    // --- same_install_location: the "(deleted)" exe-link identity fallback (WW-3-2) ---
    //
    // Neither path needs to exist on disk for this comparison — that's the whole
    // point: it exists specifically for the moment after the on-disk binary has
    // already been replaced or removed.

    #[test]
    fn same_install_location_true_for_matching_directory_and_basename() {
        let dir = TempDir::new().unwrap();
        let pre_deletion = dir.path().join("myshire");
        let current = dir.path().join("myshire");
        assert!(same_install_location(&pre_deletion, &current));
    }

    #[test]
    fn same_install_location_true_across_a_symlinked_directory() {
        // The "same install location" an in-place upgrade cares about is the real
        // directory, not the literal string path used to reach it (e.g. a
        // `/opt/current -> /opt/v3` convention).
        let real_dir = TempDir::new().unwrap();
        let alias_parent = TempDir::new().unwrap();
        let alias = alias_parent.path().join("current");
        std::os::unix::fs::symlink(real_dir.path(), &alias).unwrap();

        let via_alias = alias.join("myshire");
        let via_real = real_dir.path().join("myshire");
        assert!(same_install_location(&via_alias, &via_real));
    }

    #[test]
    fn same_install_location_false_for_different_basename() {
        let dir = TempDir::new().unwrap();
        assert!(!same_install_location(
            &dir.path().join("myshire"),
            &dir.path().join("othername")
        ));
    }

    #[test]
    fn same_install_location_false_for_different_directory() {
        let dir_a = TempDir::new().unwrap();
        let dir_b = TempDir::new().unwrap();
        assert!(!same_install_location(
            &dir_a.path().join("myshire"),
            &dir_b.path().join("myshire")
        ));
    }

    #[test]
    fn same_install_location_false_when_directory_does_not_exist() {
        assert!(!same_install_location(
            Path::new("/definitely/does/not/exist/myshire"),
            &std::env::current_exe().unwrap()
        ));
    }

    #[test]
    fn exe_path_is_shire_rejects_a_deleted_link_in_an_unrelated_directory() {
        // A deleted-marker exe link whose basename doesn't satisfy the naming rule and
        // whose directory doesn't match current_exe()'s must still be rejected — the
        // in-place-upgrade fallback recognizes "same location, replaced file", not
        // "any deleted binary".
        let other_dir = TempDir::new().unwrap();
        let elsewhere = format!(
            "{}/totally-unrelated-name (deleted)",
            other_dir.path().display()
        );
        assert!(!exe_path_is_shire(&elsewhere));
    }

    // --- ownership_from_checks: pure tri-state decision logic ---
    //
    // The Unverifiable case is only reachable in practice via a real process owned by
    // another uid (which needs privilege-dropping tooling not reliably available in
    // every CI environment), so it's tested here directly against the decision
    // function rather than via a live process.

    #[test]
    fn ownership_owned_when_exe_matches_and_not_zombie() {
        assert_eq!(
            ownership_from_checks(ExeCheck::Path("shire".into()), false),
            PidOwnership::Owned
        );
    }

    #[test]
    fn ownership_owned_for_a_versioned_or_renamed_shire_binary() {
        assert_eq!(
            ownership_from_checks(ExeCheck::Path("/opt/bin/shire-v0.7".into()), false),
            PidOwnership::Owned
        );
    }

    #[test]
    fn ownership_not_shire_when_exe_is_not_shire() {
        assert_eq!(
            ownership_from_checks(ExeCheck::Path("/usr/bin/python3".into()), false),
            PidOwnership::NotShire
        );
    }

    #[test]
    fn ownership_not_shire_for_zombie_even_with_matching_exe() {
        assert_eq!(
            ownership_from_checks(ExeCheck::Path("shire".into()), true),
            PidOwnership::NotShire
        );
    }

    #[test]
    fn ownership_unverifiable_when_exe_identity_cannot_be_read() {
        // Reproduces the round-2 finding: /proc/<pid>/exe returning EACCES (a daemon
        // owned by another uid) must not collapse into "not shire" — that led
        // stop_daemon to delete pid/socket state files out from under a live daemon
        // it simply couldn't verify.
        assert_eq!(
            ownership_from_checks(ExeCheck::Unverifiable, false),
            PidOwnership::Unverifiable
        );
        // Even a reported zombie state shouldn't override "can't verify" — if we
        // can't read the exe, we can't fully trust other cross-user /proc reads either;
        // this function receives is_zombie already resolved, but Unverifiable must win
        // outright regardless of that input.
        assert_eq!(
            ownership_from_checks(ExeCheck::Unverifiable, true),
            PidOwnership::Unverifiable
        );
    }

    #[test]
    fn check_pid_ownership_not_shire_for_spoofed_argv_on_a_foreign_executable() {
        // Reproduces a false-positive found in review: a process whose argv happens to
        // contain the bare tokens "watch" and "--foreground" (satisfying
        // cmdline_looks_like_shire_watch on its own) but whose actual executable is not
        // shire must still be rejected — otherwise any process launched with those two
        // tokens anywhere in its arguments could be killed as if it were the daemon.
        let mut child = std::process::Command::new("python3")
            .args([
                "-c",
                "import time; time.sleep(300)",
                "watch",
                "--foreground",
            ])
            .spawn()
            .expect("failed to spawn python3 (expected to be present on CI runners)");
        assert_eq!(
            check_pid_ownership(child.id(), Path::new("/repo")),
            PidOwnership::NotShire
        );
        let _ = child.kill();
        let _ = child.wait();
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

    // --- decide_stop_action: pure decision logic behind stop_daemon (WW-3-2) ---

    #[test]
    fn decide_stop_action_signals_when_owned_regardless_of_socket() {
        assert_eq!(
            decide_stop_action(PidOwnership::Owned, true),
            StopAction::Signal
        );
        assert_eq!(
            decide_stop_action(PidOwnership::Owned, false),
            StopAction::Signal
        );
    }

    #[test]
    fn decide_stop_action_removes_stale_files_when_not_shire_and_socket_is_dead() {
        assert_eq!(
            decide_stop_action(PidOwnership::NotShire, false),
            StopAction::RemoveStaleFiles
        );
    }

    #[test]
    fn decide_stop_action_refuses_when_not_shire_but_socket_is_still_live() {
        // WW-3-2: a `NotShire` verdict does not mean the process is dead or unrelated
        // — it can equally mean a live daemon this process just couldn't positively
        // identify (a different shire binary verifying it, or the same install
        // replaced in place while it kept running). Deleting its state files while
        // the socket is still live is exactly the failure this exists to prevent.
        assert_eq!(
            decide_stop_action(PidOwnership::NotShire, true),
            StopAction::RefuseAndKeep
        );
    }

    #[test]
    fn decide_stop_action_refuses_when_unverifiable_regardless_of_socket() {
        assert_eq!(
            decide_stop_action(PidOwnership::Unverifiable, true),
            StopAction::RefuseAndKeep
        );
        assert_eq!(
            decide_stop_action(PidOwnership::Unverifiable, false),
            StopAction::RefuseAndKeep
        );
    }

    #[test]
    fn stop_daemon_keeps_state_when_pid_is_not_shire_but_socket_is_still_live() {
        // Reproduces WW-3-2 end to end: the pid file names a process that fails
        // ownership verification, but something is still listening on the socket —
        // stop_daemon must not delete the pid/socket files in that case, unlike the
        // genuinely-stale case covered by `stop_daemon_refuses_to_signal_unrelated_process`.
        // It must also report this as a failure (not a quiet Ok(())), so a
        // `stop && rm -rf .shire`-style script doesn't proceed as if the daemon were
        // actually stopped.
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".shire")).unwrap();

        // A real, still-listening socket standing in for "a daemon is actually up".
        let listener = std::os::unix::net::UnixListener::bind(sock_path(dir.path())).unwrap();

        // An unrelated process for the pid file — ownership resolves to NotShire.
        let mut victim = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("failed to spawn sleep");
        std::fs::write(pid_path(dir.path()), victim.id().to_string()).unwrap();

        assert!(
            stop_daemon(dir.path()).is_err(),
            "stop_daemon must report failure when it can't verify a still-live socket"
        );

        assert!(
            victim.try_wait().unwrap().is_none(),
            "stop_daemon must not signal a pid it couldn't verify"
        );
        assert!(
            pid_path(dir.path()).exists(),
            "the pid file must be kept while the socket is still live"
        );
        assert!(
            sock_path(dir.path()).exists(),
            "the socket file must be kept while it is still live"
        );

        let _ = victim.kill();
        let _ = victim.wait();
        drop(listener);
    }

    #[test]
    fn stop_daemon_is_noop_with_no_pid_file() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".shire")).unwrap();
        assert!(stop_daemon(dir.path()).is_ok());
    }

    #[test]
    fn stop_daemon_keeps_a_live_socket_when_the_pid_file_is_unparseable() {
        // A corrupted/truncated pid file (e.g. a crash or a concurrent writer) next to
        // a socket a real daemon is still listening on must not have that socket
        // deleted just because the pid it's paired with couldn't be parsed.
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".shire")).unwrap();
        std::fs::write(pid_path(dir.path()), "not-a-pid").unwrap();
        let listener = std::os::unix::net::UnixListener::bind(sock_path(dir.path())).unwrap();

        assert!(stop_daemon(dir.path()).is_ok());

        assert!(
            !pid_path(dir.path()).exists(),
            "the unparseable pid file itself is still garbage and should be removed"
        );
        assert!(
            sock_path(dir.path()).exists(),
            "the socket must be kept while something is still listening on it"
        );
        drop(listener);
    }

    #[test]
    fn stop_daemon_removes_a_dead_socket_when_the_pid_file_is_unparseable() {
        // The existing, simpler case: a leftover socket *file* with nothing listening
        // on it (no live UnixListener bound) really is stale and safe to remove
        // alongside the unparseable pid file.
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".shire")).unwrap();
        std::fs::write(pid_path(dir.path()), "not-a-pid").unwrap();
        std::fs::write(sock_path(dir.path()), "").unwrap();

        assert!(stop_daemon(dir.path()).is_ok());

        assert!(!pid_path(dir.path()).exists());
        assert!(!sock_path(dir.path()).exists());
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

    // --- symlinked pid file / socket (round-2 finding) ---

    #[test]
    fn read_pid_file_ignores_a_symlinked_pid_file() {
        let dir_a = TempDir::new().unwrap();
        let dir_c = TempDir::new().unwrap();
        std::fs::create_dir_all(dir_a.path().join(".shire")).unwrap();
        std::fs::create_dir_all(dir_c.path().join(".shire")).unwrap();
        std::fs::write(pid_path(dir_a.path()), "12345").unwrap();
        std::os::unix::fs::symlink(pid_path(dir_a.path()), pid_path(dir_c.path())).unwrap();

        assert!(read_pid_file(dir_c.path()).is_none());
    }

    #[test]
    fn stop_daemon_refuses_a_symlinked_pid_file() {
        // Repo C's watch.pid symlinked to repo A's real, live pid file must not let
        // `shire watch --stop --root C` reach (and signal) A's daemon.
        let dir_a = TempDir::new().unwrap();
        let dir_c = TempDir::new().unwrap();
        std::fs::create_dir_all(dir_a.path().join(".shire")).unwrap();
        std::fs::create_dir_all(dir_c.path().join(".shire")).unwrap();

        let mut victim = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("failed to spawn sleep");
        std::fs::write(pid_path(dir_a.path()), victim.id().to_string()).unwrap();
        std::os::unix::fs::symlink(pid_path(dir_a.path()), pid_path(dir_c.path())).unwrap();

        stop_daemon(dir_c.path()).unwrap();

        assert!(
            victim.try_wait().unwrap().is_none(),
            "the process behind repo A's pid file must survive"
        );
        assert!(
            pid_path(dir_a.path()).exists(),
            "repo A's real pid file must be untouched"
        );
        assert!(
            is_symlink(&pid_path(dir_c.path())),
            "the symlink itself is left alone, not deleted as if it were stale state"
        );

        let _ = victim.kill();
        let _ = victim.wait();
    }

    #[test]
    fn connect_if_not_symlink_refuses_a_symlinked_socket() {
        let dir_a = TempDir::new().unwrap();
        let dir_c = TempDir::new().unwrap();
        std::fs::create_dir_all(dir_a.path().join(".shire")).unwrap();
        std::fs::create_dir_all(dir_c.path().join(".shire")).unwrap();

        let listener = std::os::unix::net::UnixListener::bind(sock_path(dir_a.path())).unwrap();
        std::os::unix::fs::symlink(sock_path(dir_a.path()), sock_path(dir_c.path())).unwrap();

        assert!(connect_if_not_symlink(dir_c.path()).is_none());
        drop(listener);
    }
}
