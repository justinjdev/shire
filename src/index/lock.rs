//! One builder at a time, across processes.
//!
//! shire is explicitly a multi-process design — a watch daemon, ad-hoc
//! `shire build`s, `serve --root`'s on-demand rebuilds, several worktrees
//! possibly sharing one `db_path` — and SQLite's 5s `busy_timeout` makes a
//! second builder *wait out* a lock and proceed rather than fail, so two
//! builds routinely overlap.
//!
//! That is not merely wasteful. `is_full_build` is read from
//! `manifest_hashes` early in phase 2, and a full build inserts symbols
//! without deleting the package's existing rows (the table is known to be
//! empty). Two builders that both pass phase 2 against a fresh database both
//! take that path, and every symbol ends up in the table twice — with the
//! FTS index rebuilt over the duplicates, every search returning each symbol
//! twice, and both builds exiting 0 (INDEX-2-4).
//!
//! An advisory `flock` on `<db_path>.lock`, held for the whole build, is
//! what actually serializes them: it is process-wide, released by the kernel
//! if the builder is killed, and works across the machine rather than within
//! one process the way `ShireService::rebuild_lock` does.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a builder waits for a competing builder before giving up. Same
/// budget as [`crate::db::BUSY_TIMEOUT`], so waiting for the lock and waiting
/// for the SQLite writer behave alike.
const LOCK_TIMEOUT: Duration = crate::db::BUSY_TIMEOUT;

/// How often to retry while waiting. `flock` can block natively, but that
/// gives no way to time out, and a builder that hangs forever behind a stuck
/// peer is worse than one that reports the conflict.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// What to do when another build already holds the lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockWait {
    /// Wait up to [`LOCK_TIMEOUT`], then fail — the CLI, where a human or CI
    /// asked for a build and must be told if it did not happen.
    Wait,
    /// Give up immediately and let the caller skip the build — a long-running
    /// process (the MCP server, the watch daemon) whose next trigger will come
    /// round again anyway, and whose whole purpose is served by the build that
    /// is already running.
    Skip,
}

/// Holds the build lock for as long as it is alive. The kernel releases the
/// `flock` when the file descriptor closes, including on a crash, so there is
/// no stale lock to clean up (the empty lock *file* is left behind, the way
/// the `-wal`/`-shm` sidecars are).
#[derive(Debug)]
pub struct BuildLock {
    _file: std::fs::File,
    path: PathBuf,
}

impl BuildLock {
    /// The lock file this guard holds.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The lock file that guards builds against `db_path`.
pub fn lock_path(db_path: &Path) -> PathBuf {
    let mut p = db_path.as_os_str().to_os_string();
    p.push(".lock");
    PathBuf::from(p)
}

/// Take the build lock for `db_path`.
///
/// `Ok(Some(guard))` means the build may proceed; `Ok(None)` means another
/// build holds the lock and the caller asked to [`LockWait::Skip`]. `Err` is
/// either a lock held past the timeout under [`LockWait::Wait`], or a lock
/// file that could not be created.
pub fn acquire(db_path: &Path, wait: LockWait) -> Result<Option<BuildLock>> {
    let path = lock_path(db_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("Failed to open build lock {}", path.display()))?;

    let deadline = Instant::now() + LOCK_TIMEOUT;
    loop {
        match try_lock(&file) {
            Ok(true) => {
                return Ok(Some(BuildLock { _file: file, path }));
            }
            Ok(false) => {
                if wait == LockWait::Skip {
                    tracing::info!(
                        lock = %path.display(),
                        "another shire build is already running — skipping this one"
                    );
                    return Ok(None);
                }
                if Instant::now() >= deadline {
                    anyhow::bail!(
                        "another shire build is already running (lock held on {}). \
                         Wait for it to finish, or remove the lock file if no build \
                         is running.",
                        path.display()
                    );
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("Failed to take the build lock {}", path.display()));
            }
        }
    }
}

/// `Ok(true)` when the exclusive lock was taken, `Ok(false)` when someone
/// else holds it.
#[cfg(unix)]
fn try_lock(file: &std::fs::File) -> Result<bool> {
    use std::os::unix::io::AsRawFd;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(code) if code == libc::EWOULDBLOCK || code == libc::EINTR => Ok(false),
        _ => Err(err.into()),
    }
}

#[cfg(not(unix))]
fn try_lock(_file: &std::fs::File) -> Result<bool> {
    // No advisory locking outside unix; shire has no Windows target.
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_path_is_the_db_path_plus_lock() {
        assert_eq!(
            lock_path(Path::new("/repo/.shire/index.db")),
            Path::new("/repo/.shire/index.db.lock")
        );
    }

    #[test]
    fn a_second_builder_skips_while_the_lock_is_held() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join(".shire").join("index.db");

        let held = acquire(&db, LockWait::Wait)
            .unwrap()
            .expect("first builder");
        // A separate open file description, i.e. what a second process gets.
        assert!(
            acquire(&db, LockWait::Skip).unwrap().is_none(),
            "a second builder must not run concurrently"
        );

        drop(held);
        assert!(
            acquire(&db, LockWait::Skip).unwrap().is_some(),
            "the lock must be released when the guard is dropped"
        );
    }

    #[test]
    fn a_waiting_builder_fails_rather_than_hanging_forever() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join("index.db");
        let _held = acquire(&db, LockWait::Wait).unwrap().unwrap();

        let start = Instant::now();
        let err = acquire(&db, LockWait::Wait)
            .expect_err("a lock held past the timeout must be reported, not waited on forever");

        assert!(
            start.elapsed() >= LOCK_TIMEOUT,
            "it must actually wait first"
        );
        assert!(
            format!("{err:#}").contains("another shire build is already running"),
            "the error must name the cause: {err:#}"
        );
    }

    #[test]
    fn a_waiting_builder_proceeds_once_the_holder_finishes() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join("index.db");
        let held = acquire(&db, LockWait::Wait).unwrap().unwrap();

        let db2 = db.clone();
        let waiter = std::thread::spawn(move || acquire(&db2, LockWait::Wait));
        std::thread::sleep(Duration::from_millis(150));
        drop(held);

        let got = waiter.join().unwrap().unwrap();
        assert!(got.is_some(), "the waiter must take the lock, not time out");
    }
}
