//! Whether shire is allowed to delete the file sitting at `db_path`.
//!
//! Two commands destroy that file: `shire clean` removes it on purpose, and
//! `shire build` removes a *corrupt* index and rebuilds from scratch (a
//! corrupt index is a derived artifact, never a source of truth). Both are
//! reachable from a repo-controlled `shire.toml`, whose `db_path` is shell-
//! expanded and deliberately not confined to the repo (the documented global
//! setup puts real databases under `~/.claude/shire/{repo}/{worktree}/`). A
//! hostile — or merely mistaken — `db_path` must therefore never get an
//! unrelated file deleted, whichever command is running.
//!
//! One implementation serves both, so the two can never disagree about what
//! they are allowed to remove.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// SQLite's on-disk file header — the first 16 bytes of every valid SQLite database
/// file (see the SQLite file format spec).
pub const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";

/// What [`classify_for_removal`] found at `db_path`.
#[derive(Debug, PartialEq, Eq)]
pub enum RemovalVerdict {
    /// Nothing there. Deleting is a no-op, not an error.
    Missing,
    /// Safe to delete: a shire-built index, or a file in a location shire
    /// itself manages (where a corrupt index is the only thing it can be).
    Allowed,
    /// Not a SQLite database at all (no file header) — and not in a location
    /// shire manages, so it is some other file entirely.
    NotSqlite,
    /// A real SQLite database, but not one shire built, in a location shire
    /// does not manage. A browser profile, someone's notes.db, …
    Foreign,
}

/// Open `path` refusing to follow a trailing symlink (O_NOFOLLOW on unix) and refusing
/// to block on a FIFO with no writer (O_NONBLOCK — opening a FIFO read-only can
/// otherwise hang forever waiting for a writer that will never arrive, a DoS via a
/// hostile `db_path`), returning `Ok(None)` for a missing file and `Err` for a symlink,
/// a non-regular file (FIFO, device, socket, directory), or any other open failure.
/// Centralizes this pattern for both the main db file and its `-wal`/`-shm` sidecars
/// (the sidecar paths are just as attacker-nameable as `db_path` itself, being derived
/// from it by string concatenation).
pub fn open_no_follow(path: &Path) -> Result<Option<std::fs::File>> {
    #[cfg(unix)]
    let opened = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
    };
    #[cfg(not(unix))]
    let opened = std::fs::File::open(path);

    let file = match opened {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        #[cfg(unix)]
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => anyhow::bail!(
            "{} is a symlink, not a plain file. Remove it by hand if that's intentional.",
            path.display()
        ),
        Err(e) => return Err(e).with_context(|| format!("Failed to open {}", path.display())),
    };

    // O_NONBLOCK only prevents the *open* from hanging on a FIFO; a FIFO that does
    // have a writer would still open successfully, so its type must be checked
    // explicitly. This also gives directories, devices, and sockets a clear refusal
    // instead of relying on read_exact() failing downstream for some of them.
    #[cfg(unix)]
    {
        let meta = file
            .metadata()
            .with_context(|| format!("Failed to stat {}", path.display()))?;
        if !meta.is_file() {
            let kind = describe_unix_file_type(&meta.file_type());
            anyhow::bail!(
                "{} is not a regular file ({kind}). Refusing to treat it as a database.",
                path.display()
            );
        }
    }

    Ok(Some(file))
}

#[cfg(unix)]
fn describe_unix_file_type(ft: &std::fs::FileType) -> &'static str {
    use std::os::unix::fs::FileTypeExt;
    if ft.is_dir() {
        "a directory"
    } else if ft.is_fifo() {
        "a FIFO"
    } else if ft.is_socket() {
        "a socket"
    } else if ft.is_char_device() {
        "a character device"
    } else if ft.is_block_device() {
        "a block device"
    } else {
        "not a regular file"
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

/// Canonicalize `dir` only if it is, itself, a real directory rather than a symlink
/// (`lstat`, not `stat`). Used for the two locations shire manages: a repo tracked in
/// git can commit `.shire` (not normally gitignored by shire itself) as a *symlink* to
/// anywhere — e.g. a browser profile directory — and naively canonicalizing
/// `root.join(".shire")` in that case would follow it, making "is this path under
/// `.shire`" trivially true for wherever the symlink points, defeating the whole
/// location check. Refusing to trust a symlinked `.shire`/`~/.claude/shire` at all
/// closes that: `starts_with` against a path that failed to resolve here can never
/// match.
fn canonical_managed_dir(dir: &Path) -> Option<PathBuf> {
    let meta = std::fs::symlink_metadata(dir).ok()?;
    if !meta.is_dir() {
        return None;
    }
    std::fs::canonicalize(dir).ok()
}

/// Is `db_path` somewhere shire itself manages — `<repo_root>/.shire/` or
/// `~/.claude/shire/`? Used only as a fallback for a database that fails the
/// `shire_meta` identity check because it's corrupt (which a real, crashed shire index
/// can be — `shire build` auto-cleans a corrupt DB it finds), not as a way to accept an
/// unidentified file from an arbitrary location.
///
/// `root` is the repository root when the caller knows it (`shire clean`, a build);
/// `None` restricts the check to the global location.
///
/// Checks the canonicalized *parent directory* of `db_path`, not `db_path` itself:
/// `O_NOFOLLOW` in `open_no_follow` only guards the final path component, so a symlink
/// planted at any ancestor directory (`<repo>/.shire/index.db` where `.shire` — or any
/// directory above it — is a symlink) would otherwise reach an arbitrary location
/// while still superficially "being under root". Canonicalizing the full parent
/// resolves every symlink along the way, so the comparison is against where the file
/// actually, physically lives.
pub fn is_in_managed_location(db_path: &Path, root: Option<&Path>) -> bool {
    let Some(parent) = db_path.parent() else {
        return false;
    };
    let Ok(canon_parent) = std::fs::canonicalize(parent) else {
        return false;
    };

    if let Some(root) = root
        && let Some(repo_shire) = canonical_managed_dir(&root.join(".shire"))
        && canon_parent.starts_with(&repo_shire)
    {
        return true;
    }

    if let Ok(home) = std::env::var("HOME")
        && let Some(claude_shire) =
            canonical_managed_dir(&PathBuf::from(home).join(".claude/shire"))
        && canon_parent.starts_with(&claude_shire)
    {
        return true;
    }

    false
}

/// Decide whether shire may delete the file at `db_path`.
///
/// Two checks gate the verdict, in order:
///
/// 1. The file must open (O_NOFOLLOW — never follow a symlink, never a FIFO or
///    other non-regular file) and start with the SQLite magic header, read from
///    that same handle to avoid a check-then-delete race. This alone only proves
///    *format*, not identity: `~/.mozilla/.../places.sqlite` or a browser's
///    `Login Data` file would pass it too.
/// 2. It must have a `shire_meta` table (queried via a strictly read-only rusqlite
///    connection) — the identifying mark every shire-built index carries. A real
///    shire database can fail this by being corrupt rather than foreign, so a file
///    that fails step 2 is still `Allowed` if — and only if — its canonical path is
///    under `<root>/.shire/` or `~/.claude/shire/`, the only places shire itself
///    ever creates one.
///
/// Errors only for a path that cannot be examined safely (a symlink, a FIFO, a
/// directory, an unreadable file); anything examinable gets a verdict.
pub fn classify_for_removal(db_path: &Path, root: Option<&Path>) -> Result<RemovalVerdict> {
    let Some(mut file) = open_no_follow(db_path)? else {
        return Ok(RemovalVerdict::Missing);
    };

    let is_sqlite = {
        use std::io::Read;
        let mut header = [0u8; 16];
        file.read_exact(&mut header).is_ok() && &header == SQLITE_HEADER
    };
    drop(file);

    // A file too short to hold a header is NOT treated as a truncated database:
    // reading it that way round is how an 8-byte secret at an attacker-named
    // `db_path` got deleted and replaced with an index (INDEX-2-2). The format
    // check comes first and admits nothing, whatever the location: a real
    // interrupted build writes the header before anything else, so a headerless
    // file is not an index shire made.
    if !is_sqlite {
        return Ok(RemovalVerdict::NotSqlite);
    }

    if looks_like_shire_db(db_path) || is_in_managed_location(db_path, root) {
        Ok(RemovalVerdict::Allowed)
    } else {
        Ok(RemovalVerdict::Foreign)
    }
}

/// Refuse, before anything is created beside it, a `db_path` that already holds
/// a file shire plainly did not write.
///
/// The build lock is taken *before* the database is opened (a builder that
/// loses the race must not have written anything), and its path is
/// `db_path` + ".lock" — so a repo-controlled `shire.toml` could get an empty
/// `.lock` file, and the directories above it, created next to an arbitrary
/// file, with [`classify_for_removal`] not running until much later
/// (INDEX-3-7). This is the cheap precondition that closes that: a non-empty
/// file without SQLite's header is not an index and never will be.
///
/// It also closes the other half of the same hole: a *valid* SQLite database
/// that shire did not build (a browser profile, someone's notes.db) was
/// adopted outright — shire wrote its schema into it, after which the file
/// carried a `shire_meta` table and `shire clean` would delete it as its own.
///
/// Deliberately narrow, so nothing that could be a shire index is refused:
/// * a missing `db_path` is fine — that is every first build;
/// * a zero-length file is fine — SQLite opens one as a brand-new empty
///   database, and an interrupted first build can leave one behind;
/// * so is a SQLite database with no objects at all, or one carrying the
///   `shire_meta` table (`create_schema` runs in one transaction, so a killed
///   first build leaves one of those two states and never a half-schema);
/// * a database that cannot be opened or queried — damaged, or locked by
///   another process mid-write — is left to the removal guard *only* inside a
///   location shire manages, where a damaged index is the only thing it can
///   be; anywhere else "could not tell" is refused rather than written into;
/// * a path that cannot be examined at all is refused: `db_path` must name a
///   regular file, since a symlink there would be followed by the open that
///   creates the index (and the removal guard refuses one anyway, so a
///   symlinked index could never be auto-repaired).
pub fn reject_unrelated_file_at_db_path(
    db_path: &Path,
    root: Option<&Path>,
    when: Inspection,
) -> Result<()> {
    reject_unrelated_file_within(db_path, root, when, crate::db::BUSY_TIMEOUT)
}

/// When in the build this check is running, which decides what "could not
/// inspect it" means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inspection {
    /// Before the build lock is taken. Another shire build may be holding the
    /// file — its write transactions run under `journal_mode=MEMORY` and block
    /// readers — so a database that cannot be inspected is not yet an answer:
    /// take the lock, which waits that build out, and ask again.
    BeforeBuildLock,
    /// Under the build lock, so no other shire build is running against this
    /// `db_path` and a database that still cannot be inspected is either
    /// damaged or held by something that is not shire.
    UnderBuildLock,
}

/// [`reject_unrelated_file_at_db_path`] with an explicit budget for waiting
/// out a writer, so the "locked database" path is testable in milliseconds.
fn reject_unrelated_file_within(
    db_path: &Path,
    root: Option<&Path>,
    when: Inspection,
    busy_timeout: std::time::Duration,
) -> Result<()> {
    // An error here is a path that cannot be examined safely — a symlink, a
    // FIFO, a directory. It must never read as "fine": `Connection::open`
    // follows a symlink, so accepting one had shire create `<db_path>.lock`
    // beside it and then write its schema into whatever it pointed at.
    let Some(mut file) = open_no_follow(db_path).with_context(|| {
        format!(
            "refusing to use {} as the index database",
            db_path.display()
        )
    })?
    else {
        // Genuinely missing: every first build.
        return Ok(());
    };
    let is_empty = file.metadata().map(|m| m.len() == 0).unwrap_or(false);
    if is_empty {
        return Ok(());
    }

    let is_sqlite = {
        use std::io::Read;
        let mut header = [0u8; 16];
        file.read_exact(&mut header).is_ok() && &header == SQLITE_HEADER
    };
    if !is_sqlite {
        anyhow::bail!(
            "refusing to use {} as the index database: there is already a file there \
             and it is not a SQLite database. Check shire.toml's db_path (or --db) — \
             shire will not overwrite a file it did not create",
            db_path.display()
        );
    }

    match peek_db_contents_within(db_path, busy_timeout) {
        DbContents::Shire => Ok(()),
        DbContents::Foreign => anyhow::bail!(
            "refusing to use {} as the index database: it is a SQLite database that \
             shire did not create — it holds tables of its own and no 'shire_meta'. \
             Check shire.toml's db_path (or --db) — shire will not write its schema \
             into a database it did not create",
            db_path.display()
        ),
        // A database shire cannot read is only safely *assumed* to be shire's
        // where shire is the one that puts databases: `<repo>/.shire/` or
        // `~/.claude/shire/`, where a damaged index is the only thing it can
        // be and `open_or_create_in_repo` rebuilds it. Anywhere else the file
        // belongs to whoever `db_path` names, and "could not tell" must not
        // resolve to "write the index into it".
        // Before the lock, "cannot inspect" is most likely a shire build
        // already running against this very database. Taking the lock waits
        // that out; the caller asks again once it holds it.
        DbContents::Unknown(_) if when == Inspection::BeforeBuildLock => Ok(()),
        DbContents::Unknown(reason) if !is_in_managed_location(db_path, root) => {
            anyhow::bail!(
                "refusing to use {} as the index database: it is a SQLite database \
                 that could not be inspected ({reason}) — it may be in use by another \
                 process, or damaged. shire will not write its schema into a database \
                 it cannot identify as its own; check shire.toml's db_path (or --db)",
                db_path.display()
            )
        }
        DbContents::Unknown(_) => Ok(()),
    }
}

/// What a read-only peek inside an existing SQLite file at `db_path` found.
#[derive(Debug)]
enum DbContents {
    /// Shire's own: it has the `shire_meta` table every index carries, or it
    /// has no objects at all (a brand-new database, which is also what SQLite
    /// makes of a zero-byte file).
    Shire,
    /// It holds objects and no `shire_meta`: someone else's database.
    ///
    /// Deliberately strict — matching on shire's other table names would
    /// admit any database that happens to hold a `files` or `packages` table,
    /// and shire would then write `shire_meta` into it, after which
    /// `shire clean` would delete it as its own. The partly-created first
    /// build that reasoning was meant to protect is instead handled at the
    /// source: `db::create_schema` runs in one transaction, so it leaves
    /// either a complete schema or no objects.
    Foreign,
    /// It could not be opened or queried, with the reason: damaged, or locked
    /// by another process mid-write (a build runs under `journal_mode=MEMORY`,
    /// whose write transactions block readers).
    Unknown(String),
}

/// Peek inside an existing SQLite database, strictly read-only, to see whose
/// it is. Never creates, writes to, or locks the file.
fn peek_db_contents_within(db_path: &Path, busy_timeout: std::time::Duration) -> DbContents {
    let conn = match rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(conn) => conn,
        Err(e) => return DbContents::Unknown(e.to_string()),
    };
    // Wait out a writer rather than reading "busy" as "cannot tell": the
    // answer to that decides whether shire writes its schema into the file.
    if let Err(e) = conn.busy_timeout(busy_timeout) {
        return DbContents::Unknown(e.to_string());
    }

    let objects = match conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    }) {
        Ok(n) => n,
        Err(e) => return DbContents::Unknown(e.to_string()),
    };
    if objects == 0 {
        return DbContents::Shire;
    }

    match conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'shire_meta'",
        [],
        |_| Ok(()),
    ) {
        Ok(()) => DbContents::Shire,
        Err(rusqlite::Error::QueryReturnedNoRows) => DbContents::Foreign,
        Err(e) => DbContents::Unknown(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Most of these cases are about identity, not concurrency, so they check
    /// the pass that has to give a final answer.
    const UNDER_LOCK: Inspection = Inspection::UnderBuildLock;

    #[test]
    fn an_unrelated_file_at_db_path_is_refused_before_anything_is_created() {
        // INDEX-3-7: the build lock is `db_path` + ".lock" and is taken before
        // the database is opened, so this is the only thing standing between a
        // repo-controlled db_path and an empty file appearing next to an
        // arbitrary one.
        let dir = tempfile::TempDir::new().unwrap();
        let secret = dir.path().join("secret");
        std::fs::write(&secret, b"hunter2\n").unwrap();

        let err = reject_unrelated_file_at_db_path(&secret, None, UNDER_LOCK)
            .expect_err("a plain file is not a database and must be refused");
        assert!(
            format!("{err:#}").contains("not a SQLite database"),
            "got {err:#}"
        );
    }

    /// A valid SQLite database at `path` holding exactly `schema`.
    fn write_sqlite_with(path: &Path, schema: &str) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(schema).unwrap();
    }

    #[test]
    fn a_foreign_sqlite_database_at_db_path_is_refused() {
        // A valid SQLite database shire did not build used to be adopted
        // outright: the build wrote shire's schema into it, after which it
        // carried a `shire_meta` table and `shire clean` deleted it as shire's
        // own. A repo-controlled db_path can name any file on the machine.
        //
        // The verdict keys on `shire_meta` alone. Matching shire's other
        // table names instead would admit any database that happens to hold a
        // `files` or `packages` table of its own — including one sitting next
        // to tables that are nobody's business but their owner's.
        let dir = tempfile::TempDir::new().unwrap();
        for (name, schema) in [
            (
                "places.sqlite",
                "CREATE TABLE places (id INTEGER PRIMARY KEY, url TEXT);",
            ),
            ("app.db", "CREATE TABLE files (id INTEGER, blob BLOB);"),
            (
                "crm.db",
                "CREATE TABLE packages (id INTEGER, sku TEXT);
                 CREATE TABLE customer_secrets (id INTEGER, token TEXT);",
            ),
            (
                "views.db",
                "CREATE TABLE t (id INTEGER); CREATE VIEW symbols AS SELECT id FROM t;",
            ),
        ] {
            let foreign = dir.path().join(name);
            write_sqlite_with(&foreign, schema);

            let err = reject_unrelated_file_at_db_path(&foreign, None, UNDER_LOCK)
                .expect_err("someone else's database must not be written into");
            let err = format!("{err:#}");
            assert!(err.contains("shire did not create"), "{name}: got {err}");
        }
    }

    #[test]
    fn a_database_that_cannot_be_inspected_is_refused_outside_a_managed_location() {
        // A build holds an exclusive write lock for the length of a phase
        // (builds run under journal_mode=MEMORY, which blocks readers), so
        // "busy" is a real answer here — and reading it as "cannot tell, carry
        // on" wrote shire's whole index into whatever the file turned out to
        // be. Outside the two directories shire itself manages, a file it
        // cannot identify is not its to write into.
        let dir = tempfile::TempDir::new().unwrap();
        let foreign = dir.path().join("notes.db");
        write_sqlite_with(&foreign, "CREATE TABLE notes (id INTEGER, body TEXT);");

        let holder = rusqlite::Connection::open(&foreign).unwrap();
        holder.execute_batch("BEGIN EXCLUSIVE;").unwrap();

        let err =
            reject_unrelated_file_within(&foreign, None, UNDER_LOCK, Duration::from_millis(50))
                .expect_err("a database that cannot be inspected must not be written into");
        holder.execute_batch("ROLLBACK;").unwrap();
        let err = format!("{err:#}");
        assert!(
            err.contains("could not be inspected") && err.contains("in use by another process"),
            "the error must say why: {err}"
        );
    }

    #[test]
    fn a_database_that_cannot_be_inspected_is_not_yet_refused_before_the_build_lock() {
        // A shire build already running against this db_path holds it under
        // `journal_mode=MEMORY`, whose write transactions block readers.
        // Refusing on the first pass would make two builders on one db_path
        // fail rather than serialise; the build lock waits the peer out and
        // the second pass decides.
        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join("index.db");
        write_sqlite_with(
            &db,
            "CREATE TABLE shire_meta (key TEXT PRIMARY KEY, value TEXT);",
        );

        let holder = rusqlite::Connection::open(&db).unwrap();
        holder.execute_batch("BEGIN EXCLUSIVE;").unwrap();

        let before = reject_unrelated_file_within(
            &db,
            None,
            Inspection::BeforeBuildLock,
            Duration::from_millis(50),
        );
        let under = reject_unrelated_file_within(
            &db,
            None,
            Inspection::UnderBuildLock,
            Duration::from_millis(50),
        );
        holder.execute_batch("ROLLBACK;").unwrap();

        before.expect("the pass before the lock must defer, not refuse");
        under.expect_err("the pass under the lock is the one that decides");

        // And once the writer is gone the same file is recognised.
        reject_unrelated_file_at_db_path(&db, None, UNDER_LOCK)
            .expect("an unlocked shire index is shire's own");
    }

    #[test]
    fn a_symlink_at_db_path_is_refused_rather_than_followed() {
        // `Connection::open` follows a symlink, so treating an unexaminable
        // path as "fine" had shire create `<db_path>.lock` beside the link and
        // then write its schema into whatever it pointed at. db_path must name
        // a regular file — the removal guard refuses a symlink too, so a
        // symlinked index could never be auto-repaired either.
        let dir = tempfile::TempDir::new().unwrap();
        let foreign = dir.path().join("notes.db");
        write_sqlite_with(&foreign, "CREATE TABLE notes (id INTEGER, body TEXT);");
        let shire = dir.path().join("real-index.db");
        write_sqlite_with(
            &shire,
            "CREATE TABLE shire_meta (key TEXT PRIMARY KEY, value TEXT);",
        );

        for (name, target) in [("to-foreign.db", &foreign), ("to-shire.db", &shire)] {
            let link = dir.path().join(name);
            std::os::unix::fs::symlink(target, &link).unwrap();
            let err = reject_unrelated_file_at_db_path(&link, None, UNDER_LOCK)
                .expect_err("a symlink at db_path must be refused, not followed");
            let err = format!("{err:#}");
            assert!(
                err.contains("refusing to use") && err.contains("is a symlink"),
                "{name}: got {err}"
            );
        }
    }

    #[test]
    fn a_database_that_cannot_be_inspected_inside_a_managed_location_is_left_alone() {
        // Inside `<repo>/.shire/` a database shire cannot read is a damaged
        // index and nothing else, and `open_or_create_in_repo` rebuilds it.
        // That path must not start failing because the file was busy.
        let repo = tempfile::TempDir::new().unwrap();
        let shire_dir = repo.path().join(".shire");
        std::fs::create_dir_all(&shire_dir).unwrap();
        let db = shire_dir.join("index.db");
        write_sqlite_with(
            &db,
            "CREATE TABLE shire_meta (key TEXT PRIMARY KEY, value TEXT);",
        );

        let holder = rusqlite::Connection::open(&db).unwrap();
        holder.execute_batch("BEGIN EXCLUSIVE;").unwrap();

        let verdict = reject_unrelated_file_within(
            &db,
            Some(repo.path()),
            UNDER_LOCK,
            Duration::from_millis(50),
        );
        holder.execute_batch("ROLLBACK;").unwrap();
        verdict.expect("shire's own directory keeps the corrupt-index handling");

        // And so is a genuinely corrupt file there.
        let corrupt = shire_dir.join("other.db");
        corrupt_sqlite_like(&corrupt);
        reject_unrelated_file_at_db_path(&corrupt, Some(repo.path()), UNDER_LOCK)
            .expect("a damaged index in a managed location still reaches the removal guard");
    }

    #[test]
    fn a_corrupt_database_outside_a_managed_location_is_refused() {
        // The same rule seen from the other side: a file shire cannot read,
        // in a directory shire does not manage, is not assumed to be shire's.
        let dir = tempfile::TempDir::new().unwrap();
        let corrupt = dir.path().join("index.db");
        corrupt_sqlite_like(&corrupt);

        let err = reject_unrelated_file_at_db_path(&corrupt, None, UNDER_LOCK)
            .expect_err("an unreadable database outside shire's own directories is refused");
        assert!(
            format!("{err:#}").contains("could not be inspected"),
            "got {err:#}"
        );
    }

    #[test]
    fn a_missing_empty_or_sqlite_db_path_is_accepted() {
        // The three shapes a real db_path takes: never built yet, a zero-byte
        // file left by an interrupted first build (SQLite opens one as a new
        // empty database), and an actual index.
        let dir = tempfile::TempDir::new().unwrap();

        reject_unrelated_file_at_db_path(&dir.path().join("nope.db"), None, UNDER_LOCK).unwrap();

        let empty = dir.path().join("empty.db");
        std::fs::write(&empty, b"").unwrap();
        reject_unrelated_file_at_db_path(&empty, None, UNDER_LOCK).unwrap();

        // A SQLite database with no objects at all is what an interrupted
        // first build leaves, and is indistinguishable from a fresh one.
        let blank = dir.path().join("blank.db");
        rusqlite::Connection::open(&blank).unwrap();
        reject_unrelated_file_at_db_path(&blank, None, UNDER_LOCK)
            .expect("an empty SQLite database must still be adopted");

        // And shire's own index, obviously.
        let shire = dir.path().join("shire.db");
        let conn = rusqlite::Connection::open(&shire).unwrap();
        conn.execute_batch("CREATE TABLE shire_meta (key TEXT PRIMARY KEY, value TEXT);")
            .unwrap();
        drop(conn);
        reject_unrelated_file_at_db_path(&shire, None, UNDER_LOCK)
            .expect("shire's own index must be adopted");
    }

    fn corrupt_sqlite_like(path: &Path) {
        let mut content = SQLITE_HEADER.to_vec();
        content.extend_from_slice(b"rest of a fake but header-valid sqlite file");
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn short_file_outside_a_managed_location_is_not_removable() {
        // INDEX-2-2: an 8-byte file used to pass the guard ("shorter than a
        // header — a truncated DB") and get deleted by `shire build`.
        let dir = tempfile::TempDir::new().unwrap();
        let secret = dir.path().join("small_secret");
        std::fs::write(&secret, b"hunter2\n").unwrap();
        assert_eq!(
            classify_for_removal(&secret, Some(dir.path())).unwrap(),
            RemovalVerdict::NotSqlite
        );
    }

    #[test]
    fn empty_file_outside_a_managed_location_is_not_removable() {
        let dir = tempfile::TempDir::new().unwrap();
        let empty = dir.path().join("placeholder");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(
            classify_for_removal(&empty, Some(dir.path())).unwrap(),
            RemovalVerdict::NotSqlite
        );
    }

    #[test]
    fn corrupt_file_inside_repo_shire_dir_is_removable() {
        let root = tempfile::TempDir::new().unwrap();
        let shire_dir = root.path().join(".shire");
        std::fs::create_dir_all(&shire_dir).unwrap();
        let db = shire_dir.join("index.db");
        corrupt_sqlite_like(&db);
        assert_eq!(
            classify_for_removal(&db, Some(root.path())).unwrap(),
            RemovalVerdict::Allowed
        );
    }

    #[test]
    fn truncated_file_inside_repo_shire_dir_is_still_not_removable() {
        // The location fallback rescues a *corrupt index*, not any stub: a
        // build writes the SQLite header first, so a headerless file in
        // `.shire/` is something else and stays put.
        let root = tempfile::TempDir::new().unwrap();
        let shire_dir = root.path().join(".shire");
        std::fs::create_dir_all(&shire_dir).unwrap();
        let db = shire_dir.join("index.db");
        std::fs::write(&db, b"SQLite").unwrap();
        assert_eq!(
            classify_for_removal(&db, Some(root.path())).unwrap(),
            RemovalVerdict::NotSqlite
        );
    }

    #[test]
    fn foreign_sqlite_db_outside_a_managed_location_is_not_removable() {
        let dir = tempfile::TempDir::new().unwrap();
        let victim = dir.path().join("places.sqlite");
        let conn = rusqlite::Connection::open(&victim).unwrap();
        conn.execute_batch("CREATE TABLE places (id INTEGER PRIMARY KEY, url TEXT);")
            .unwrap();
        drop(conn);
        let root = tempfile::TempDir::new().unwrap();
        assert_eq!(
            classify_for_removal(&victim, Some(root.path())).unwrap(),
            RemovalVerdict::Foreign
        );
    }

    #[test]
    fn a_real_shire_db_is_removable_anywhere() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join("index.db");
        let conn = crate::db::open_or_create(&db).unwrap();
        drop(conn);
        let root = tempfile::TempDir::new().unwrap();
        assert_eq!(
            classify_for_removal(&db, Some(root.path())).unwrap(),
            RemovalVerdict::Allowed
        );
    }

    #[test]
    fn missing_file_is_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(
            classify_for_removal(&dir.path().join("nope.db"), Some(dir.path())).unwrap(),
            RemovalVerdict::Missing
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_is_refused_outright() {
        let dir = tempfile::TempDir::new().unwrap();
        let target = dir.path().join("real");
        std::fs::write(&target, b"x").unwrap();
        let link = dir.path().join("link.db");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(classify_for_removal(&link, Some(dir.path())).is_err());
    }
}
