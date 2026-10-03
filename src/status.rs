//! `shire status`: a read-only snapshot of the index, for people and for
//! tools that poll it (the Claude Code mod, scripts, CI).
//!
//! Unlike the MCP `index_status` tool this never rebuilds and never writes:
//! the database is opened read-only, the build lock is only probed (see
//! [`crate::index::lock::is_held`]), and a missing index stays missing.
//!
//! "Read-only" includes not leaving SQLite's `-wal`/`-shm` sidecars behind:
//! the index is in WAL mode at rest, and SQLite creates both files when any
//! connection, even a read-only one, opens a WAL database, then cannot
//! remove them on close without write access. So an index nobody else has
//! open is read with `immutable=1` (see [`read_meta`]).

use anyhow::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::index::{FILE_WALK_KEY, LAST_BUILD_FAILURES_KEY};
use crate::watch::daemon::Liveness;

/// How long to wait on a database a build is writing. A build runs under
/// `journal_mode=MEMORY` and can hold its lock for seconds; status reports
/// "building" instead of waiting it out.
const STATUS_BUSY_TIMEOUT: Duration = Duration::from_millis(250);

/// The one-word summary. Ordered by what a reader should act on first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// No index at `db_path` yet.
    Missing,
    /// `db_path` is a symlink; shire refuses to use it.
    Refused,
    /// The file exists but could not be read as a shire index.
    Unreadable,
    /// A build holds the build lock right now.
    Building,
    /// The last build died part-way; the next build verifies and repairs it.
    Interrupted,
    /// Built, and no build running.
    Ok,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub packages: Option<i64>,
    pub symbols: Option<i64>,
    pub references: Option<i64>,
    pub files: Option<i64>,
    pub docs: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub shire_version: &'static str,
    pub root: PathBuf,
    /// `None` when it could not be resolved (a broken `shire.toml`).
    pub db_path: Option<PathBuf>,
    pub state: State,
    /// Why the index is `unreadable`, or why it could not be read mid-build.
    pub error: Option<String>,
    /// Main database file plus its `-wal` sidecar.
    pub db_size_bytes: Option<u64>,
    pub build_running: bool,
    pub indexed_at: Option<String>,
    pub build_duration_ms: Option<u64>,
    /// The commit the index was built at.
    pub git_commit: Option<String>,
    /// The working tree's `HEAD` now.
    pub head_commit: Option<String>,
    /// `None` when either side is unknown.
    pub head_matches: Option<bool>,
    pub counts: Counts,
    pub references_enabled: Option<bool>,
    /// `complete`, `partial` (unreadable paths) or `capped` (hit the file cap).
    pub file_walk: Option<String>,
    /// Packages the next build still owes a source re-check.
    pub pending_source_recheck: Vec<String>,
    /// Failures of the last completed build: `{kind, target, error}`.
    pub last_build_failures: Vec<serde_json::Value>,
    pub watch: Liveness,
}

/// `shire status`'s whole job: resolve the repo root and `db_path` the way
/// the other subcommands do, then [`collect`]. A failure to resolve them (a
/// malformed `shire.toml`, a `--root` that does not exist) is reported as an
/// `unreadable` status rather than an error, so a poller always gets JSON.
pub fn collect_for(root: Option<&Path>, db: Option<&Path>, config: Option<&Path>) -> Status {
    let given_root = root.map(Path::to_path_buf);
    let resolve = || -> Result<(PathBuf, PathBuf)> {
        let root = match root {
            Some(r) => std::fs::canonicalize(r)
                .map_err(|e| anyhow::anyhow!("cannot resolve --root {}: {e}", r.display()))?,
            None => crate::config::find_repo_root(&std::fs::canonicalize(".")?),
        };
        let db_path = match db {
            Some(p) => p.to_path_buf(),
            None => {
                let cfg = crate::config::load_config_from(config, &root)?;
                crate::config::resolve_db_path(&cfg, &root)?
            }
        };
        Ok((root, db_path))
    };
    match resolve() {
        Ok((root, db_path)) => collect(&root, &db_path),
        Err(e) => {
            let root = given_root
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_default();
            unresolved(root, format!("{e:#}"))
        }
    }
}

/// A status for a repo whose `db_path` could not even be worked out.
fn unresolved(root: PathBuf, error: String) -> Status {
    Status {
        shire_version: env!("CARGO_PKG_VERSION"),
        head_commit: crate::git::head_commit(&root),
        watch: crate::watch::daemon::liveness(&root),
        root,
        db_path: None,
        state: State::Unreadable,
        error: Some(error),
        db_size_bytes: None,
        build_running: false,
        indexed_at: None,
        build_duration_ms: None,
        git_commit: None,
        head_matches: None,
        counts: Counts::default(),
        references_enabled: None,
        file_walk: None,
        pending_source_recheck: Vec::new(),
        last_build_failures: Vec::new(),
    }
}

/// Gather the status of the index at `db_path` for the repo at `root`.
pub fn collect(root: &Path, db_path: &Path) -> Status {
    collect_with(root, db_path, crate::index::lock::is_held)
}

/// [`collect`], with the build-lock probe injectable so tests can stage a
/// build that starts while status is running.
fn collect_with(root: &Path, db_path: &Path, build_lock_held: impl Fn(&Path) -> bool) -> Status {
    let build_running = build_lock_held(db_path);
    let head_commit = crate::git::head_commit(root);
    let mut status = Status {
        shire_version: env!("CARGO_PKG_VERSION"),
        root: root.to_path_buf(),
        db_path: Some(db_path.to_path_buf()),
        state: State::Missing,
        error: None,
        db_size_bytes: None,
        build_running,
        indexed_at: None,
        build_duration_ms: None,
        git_commit: None,
        head_commit,
        head_matches: None,
        counts: Counts::default(),
        references_enabled: None,
        file_walk: None,
        pending_source_recheck: Vec::new(),
        last_build_failures: Vec::new(),
        watch: crate::watch::daemon::liveness(root),
    };

    match std::fs::symlink_metadata(db_path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            // EACCES on a parent directory is not "no index": a build would
            // fail too, so say what is actually wrong.
            status.state = State::Unreadable;
            status.error = Some(format!("cannot stat db_path: {e}"));
            return status;
        }
        Err(_) => {
            status.state = if build_running {
                State::Building
            } else {
                State::Missing
            };
            return status;
        }
        Ok(meta) if meta.file_type().is_symlink() => {
            status.state = State::Refused;
            status.error = Some("db_path is a symlink; shire refuses to use it".into());
            return status;
        }
        Ok(meta) if !meta.is_file() => {
            status.state = State::Unreadable;
            status.error = Some("db_path is not a regular file".into());
            return status;
        }
        Ok(meta) => {
            let wal = sidecar_len(db_path, "-wal");
            status.db_size_bytes = Some(meta.len() + wal);
        }
    }

    // With no sidecars on disk no WAL connection is open, and with the lock
    // free no build is writing: the main file is the whole database.
    let immutable = !build_running && !has_sidecars(db_path);
    match read_meta(db_path, immutable) {
        Ok(meta) => {
            let interrupted = status.apply(meta);
            status.state = if build_running {
                State::Building
            } else if interrupted {
                State::Interrupted
            } else {
                State::Ok
            };
        }
        Err(e) if build_running => {
            // Expected: a build under journal_mode=MEMORY locks readers out.
            status.state = State::Building;
            status.error = Some(format!("{e:#}"));
        }
        Err(e) => {
            status.state = State::Unreadable;
            status.error = Some(format!("{e:#}"));
        }
    }
    // A build that took the lock after the probe above sets its
    // `build_in_progress` marker at once, so it reads as interrupted, and it
    // can hold readers off long enough to read as unreadable. Probe again
    // before blaming the index.
    if matches!(status.state, State::Interrupted | State::Unreadable) && build_lock_held(db_path) {
        status.build_running = true;
        status.state = State::Building;
    }
    status.head_matches = match (&status.git_commit, &status.head_commit) {
        (Some(a), Some(b)) => Some(a == b),
        _ => None,
    };
    status
}

fn sidecar(db_path: &Path, suffix: &str) -> PathBuf {
    let mut p = db_path.as_os_str().to_os_string();
    p.push(suffix);
    PathBuf::from(p)
}

fn sidecar_len(db_path: &Path, suffix: &str) -> u64 {
    match std::fs::symlink_metadata(sidecar(db_path, suffix)) {
        Ok(m) if m.is_file() => m.len(),
        _ => 0,
    }
}

fn has_sidecars(db_path: &Path) -> bool {
    ["-wal", "-shm"]
        .iter()
        .any(|s| std::fs::symlink_metadata(sidecar(db_path, s)).is_ok())
}

/// Every `shire_meta` value `shire status` reports.
#[derive(Debug, Default)]
struct Meta {
    values: std::collections::HashMap<String, String>,
    pending_source_recheck: Vec<String>,
}

/// Read the metadata. `immutable` opens the file with SQLite's `immutable=1`,
/// which takes no locks and never touches `-wal`/`-shm`: correct only while
/// nothing else has the database open for writing, which the caller
/// establishes (and re-checks afterwards, since a build may start meanwhile).
fn read_meta(db_path: &Path, immutable: bool) -> Result<Meta> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = match db_path.to_str().filter(|_| immutable) {
        Some(path) => Connection::open_with_flags(
            format!("file:{}?immutable=1", uri_escape(path)),
            flags | OpenFlags::SQLITE_OPEN_URI,
        )?,
        // A path that is not UTF-8 cannot go in a URI; read it normally.
        None => Connection::open_with_flags(db_path, flags)?,
    };
    conn.busy_timeout(STATUS_BUSY_TIMEOUT)?;
    conn.execute_batch("PRAGMA query_only=ON;")?;
    read_meta_from(&conn)
}

/// Percent-encode everything a SQLite URI path could misread (`?`, `#`, `%`
/// and anything outside the unreserved set), keeping `/`.
fn uri_escape(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn read_meta_from(conn: &Connection) -> Result<Meta> {
    let has_meta: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'shire_meta'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if has_meta.is_none() {
        anyhow::bail!("not a shire index (no 'shire_meta' table)");
    }
    let mut stmt = conn.prepare("SELECT key, value FROM shire_meta")?;
    let values = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut pending: Vec<String> = crate::index::read_pending_source_recheck(conn)
        .into_iter()
        .collect();
    pending.sort();
    Ok(Meta {
        values,
        pending_source_recheck: pending,
    })
}

impl Status {
    /// Copy `meta` in; returns whether the `build_in_progress` marker is set.
    fn apply(&mut self, meta: Meta) -> bool {
        let get = |k: &str| meta.values.get(k).cloned();
        let num = |k: &str| get(k).and_then(|v| v.parse::<i64>().ok());
        self.indexed_at = get("indexed_at");
        self.build_duration_ms = get("total_duration_ms").and_then(|v| v.parse().ok());
        self.git_commit = get("git_commit");
        self.counts = Counts {
            packages: num("package_count"),
            symbols: num("symbol_count"),
            references: num("reference_count"),
            files: num("file_count"),
            docs: num("doc_count"),
        };
        self.references_enabled = match get("references_enabled").as_deref() {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ => None,
        };
        self.file_walk = get(FILE_WALK_KEY);
        self.last_build_failures = get(LAST_BUILD_FAILURES_KEY)
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        let interrupted = get("build_in_progress").as_deref() == Some("1");
        self.pending_source_recheck = meta.pending_source_recheck;
        interrupted
    }
}

/// The human-readable form `shire status` prints without `--json`.
pub fn render_text(s: &Status) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let opt = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".into());
    let num = |v: Option<i64>| v.map_or("-".into(), |n| n.to_string());
    let state = serde_json::to_value(s.state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let _ = writeln!(out, "state:      {state}");
    if let Some(e) = &s.error {
        let _ = writeln!(out, "error:      {e}");
    }
    let _ = writeln!(out, "root:       {}", s.root.display());
    let db = s
        .db_path
        .as_ref()
        .map_or("-".into(), |p| p.display().to_string());
    let _ = writeln!(out, "db:         {db}");
    if let Some(n) = s.db_size_bytes {
        let _ = writeln!(out, "size:       {n} bytes");
    }
    let _ = writeln!(out, "indexed at: {}", opt(&s.indexed_at));
    if let Some(ms) = s.build_duration_ms {
        let _ = writeln!(out, "build took: {ms} ms");
    }
    let commit = match (&s.git_commit, s.head_matches) {
        (Some(c), Some(false)) => format!("{c} (HEAD has moved since)"),
        (Some(c), _) => c.clone(),
        (None, _) => "-".into(),
    };
    let _ = writeln!(out, "commit:     {commit}");
    let c = &s.counts;
    let _ = writeln!(
        out,
        "counts:     {} packages, {} symbols, {} references, {} files, {} docs",
        num(c.packages),
        num(c.symbols),
        num(c.references),
        num(c.files),
        num(c.docs)
    );
    if let Some(walk) = &s.file_walk
        && walk != "complete"
    {
        let _ = writeln!(out, "file walk:  {walk}");
    }
    if !s.pending_source_recheck.is_empty() {
        let _ = writeln!(
            out,
            "pending:    {} package(s) owe a source re-check: {}",
            s.pending_source_recheck.len(),
            s.pending_source_recheck.join(", ")
        );
    }
    if !s.last_build_failures.is_empty() {
        let _ = writeln!(out, "failures:   {}", s.last_build_failures.len());
        for f in &s.last_build_failures {
            let field = |k: &str| f.get(k).and_then(|v| v.as_str()).unwrap_or("?");
            let _ = writeln!(
                out,
                "  [{}] {}: {}",
                field("kind"),
                field("target"),
                field("error")
            );
        }
    }
    let w = &s.watch;
    let watch = match (w.running, w.listening, w.pid) {
        (false, _, _) => "not running".to_string(),
        (true, true, Some(pid)) => format!("running (pid {pid})"),
        (true, true, None) => "running".to_string(),
        (true, false, _) => "starting (not listening yet)".to_string(),
    };
    let _ = writeln!(out, "watch:      {watch}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built_index(dir: &Path) -> PathBuf {
        let db = dir.join(".shire").join("index.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = Connection::open(&db).unwrap();
        crate::db::create_schema_for_test(&conn);
        for (k, v) in [
            ("indexed_at", "2026-10-02T12:00:00Z"),
            ("package_count", "3"),
            ("symbol_count", "42"),
            ("total_duration_ms", "180"),
            ("git_commit", "abc123"),
            ("references_enabled", "true"),
            (FILE_WALK_KEY, "complete"),
            (
                LAST_BUILD_FAILURES_KEY,
                r#"[{"kind":"extract","target":"pkg-a","error":"unreadable"}]"#,
            ),
            ("pending_source_recheck", r#"["pkg-b","pkg-a"]"#),
        ] {
            conn.execute(
                "INSERT OR REPLACE INTO shire_meta (key, value) VALUES (?1, ?2)",
                [k, v],
            )
            .unwrap();
        }
        db
    }

    #[test]
    fn missing_index_is_missing_and_creates_nothing() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join(".shire").join("index.db");
        let s = collect(dir.path(), &db);
        assert_eq!(s.state, State::Missing);
        assert!(!dir.path().join(".shire").exists(), "status must not write");
    }

    #[test]
    fn reads_the_build_metadata() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = built_index(dir.path());
        let s = collect(dir.path(), &db);
        assert_eq!(s.state, State::Ok, "{:?}", s.error);
        assert_eq!(s.counts.packages, Some(3));
        assert_eq!(s.counts.symbols, Some(42));
        assert_eq!(s.counts.references, None);
        assert_eq!(s.build_duration_ms, Some(180));
        assert_eq!(s.git_commit.as_deref(), Some("abc123"));
        assert_eq!(s.references_enabled, Some(true));
        assert_eq!(s.file_walk.as_deref(), Some("complete"));
        assert_eq!(s.pending_source_recheck, ["pkg-a", "pkg-b"]);
        assert_eq!(s.last_build_failures.len(), 1);
        assert_eq!(s.last_build_failures[0]["target"], "pkg-a");
        assert!(s.db_size_bytes.unwrap() > 0);
        assert!(!s.build_running);
        assert!(!s.watch.running);
    }

    #[test]
    fn an_unfinished_build_marker_reads_as_interrupted() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = built_index(dir.path());
        let conn = Connection::open(&db).unwrap();
        crate::db::set_build_in_progress(&conn, true).unwrap();
        drop(conn);
        assert_eq!(collect(dir.path(), &db).state, State::Interrupted);
    }

    #[test]
    fn a_held_build_lock_reads_as_building() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = built_index(dir.path());
        let _lock = crate::index::lock::acquire(&db, crate::index::lock::LockWait::Skip)
            .unwrap()
            .unwrap();
        let s = collect(dir.path(), &db);
        assert_eq!(s.state, State::Building);
        assert!(s.build_running);
        assert_eq!(
            s.counts.symbols,
            Some(42),
            "last build's counts still shown"
        );
    }

    #[test]
    fn a_foreign_sqlite_file_is_unreadable() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join("notes.db");
        Connection::open(&db)
            .unwrap()
            .execute_batch("CREATE TABLE notes (body TEXT);")
            .unwrap();
        let s = collect(dir.path(), &db);
        assert_eq!(s.state, State::Unreadable);
        assert!(s.error.unwrap().contains("shire_meta"));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_db_path_is_refused() {
        let dir = tempfile::TempDir::new().unwrap();
        let real = built_index(dir.path());
        let link = dir.path().join("link.db");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(collect(dir.path(), &link).state, State::Refused);
    }

    #[test]
    fn head_matches_compares_the_indexed_commit_with_head() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = built_index(dir.path());
        let mut s = collect(dir.path(), &db);
        // No git repo in the temp dir: unknown, not a mismatch.
        assert_eq!(s.head_commit, None);
        assert_eq!(s.head_matches, None);
        s.head_commit = Some("def456".into());
        s.head_matches = Some(false);
        assert!(render_text(&s).contains("HEAD has moved since"));
    }

    #[test]
    fn a_broken_config_is_reported_not_raised() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("shire.toml"), "db_path = [not toml").unwrap();
        let s = collect_for(Some(dir.path()), None, None);
        assert_eq!(s.state, State::Unreadable);
        assert_eq!(s.db_path, None);
        assert!(s.error.is_some());
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["db_path"], serde_json::Value::Null);
    }

    #[test]
    fn a_missing_root_is_reported_not_raised() {
        let dir = tempfile::TempDir::new().unwrap();
        let gone = dir.path().join("nope");
        let s = collect_for(Some(&gone), None, None);
        assert_eq!(s.state, State::Unreadable);
        assert!(s.error.unwrap().contains("--root"));
    }

    #[cfg(unix)]
    #[test]
    fn an_unstattable_db_path_is_unreadable_not_missing() {
        // A file used as a directory component: ENOTDIR, not ENOENT.
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, "").unwrap();
        let s = collect(dir.path(), &file.join("index.db"));
        assert_eq!(s.state, State::Unreadable);
        assert!(s.error.unwrap().contains("cannot stat db_path"));
    }

    #[test]
    fn reading_a_wal_index_leaves_no_sidecars() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = built_index(dir.path());
        let conn = Connection::open(&db).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        drop(conn); // the last connection checkpoints and removes the sidecars
        assert!(!has_sidecars(&db));

        let s = collect(dir.path(), &db);
        assert_eq!(s.state, State::Ok, "{:?}", s.error);
        assert_eq!(s.counts.symbols, Some(42));
        assert!(!has_sidecars(&db), "status must not create -wal/-shm");
    }

    #[test]
    fn a_wal_index_another_connection_has_open_is_still_read() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = built_index(dir.path());
        let conn = Connection::open(&db).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute(
            "UPDATE shire_meta SET value = '43' WHERE key = 'symbol_count'",
            [],
        )
        .unwrap();
        // The update is still in the WAL, which an immutable read would miss.
        assert!(has_sidecars(&db));
        assert_eq!(collect(dir.path(), &db).counts.symbols, Some(43));
    }

    #[test]
    fn a_build_starting_mid_status_reads_as_building_not_interrupted() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = built_index(dir.path());
        let conn = Connection::open(&db).unwrap();
        crate::db::set_build_in_progress(&conn, true).unwrap();
        drop(conn);
        // Free at the first probe, held by the time the marker is read.
        let probes = std::cell::Cell::new(0);
        let s = collect_with(dir.path(), &db, |_| {
            probes.set(probes.get() + 1);
            probes.get() > 1
        });
        assert_eq!(s.state, State::Building);
        assert!(s.build_running);
    }

    #[test]
    fn uri_escape_keeps_slashes_and_escapes_the_rest() {
        assert_eq!(uri_escape("/a b/c?d#e%f.db"), "/a%20b/c%3Fd%23e%25f.db");
    }

    #[test]
    fn json_uses_snake_case_states() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = built_index(dir.path());
        let v = serde_json::to_value(collect(dir.path(), &db)).unwrap();
        assert_eq!(v["state"], "ok");
        assert_eq!(v["counts"]["packages"], 3);
        assert_eq!(v["watch"]["running"], false);
    }
}
