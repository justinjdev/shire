use anyhow::Result;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::SystemTime;

/// Compute hex-encoded SHA-256 of a file's contents.
pub fn hash_file(path: &Path) -> Result<String> {
    let content = std::fs::read(path)?;
    let digest = Sha256::digest(&content);
    Ok(format!("{:x}", digest))
}

/// Compute an aggregate SHA-256 hash of the file tree from walked files.
/// Collects (relative_path, size_bytes) tuples, sorts lexicographically by path,
/// and hashes the concatenation.
pub fn compute_file_tree_hash(files: &[(String, u64)]) -> String {
    let mut sorted: Vec<(&str, u64)> = files.iter().map(|(p, s)| (p.as_str(), *s)).collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));

    let mut hasher = Sha256::new();
    for (path, size) in &sorted {
        hasher.update(path.as_bytes());
        hasher.update(size.to_le_bytes());
    }
    let digest = hasher.finalize();
    format!("{:x}", digest)
}

/// Compute hex-encoded SHA-256 of a byte slice.
pub fn hash_bytes_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    format!("{:x}", digest)
}

/// Why a package's source tree looks like it may have changed since the last
/// per-file hash pass (or that it demonstrably has not).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceStaleness {
    /// Nothing on disk suggests a change — the per-file hash pass can be skipped.
    Unchanged,
    /// At least one source file has an mtime newer than the last hash pass.
    MtimeNewer,
    /// The set of source files on disk differs from the set the DB recorded
    /// (a rename, an addition, a deletion, or a file that failed to read
    /// during an earlier build).
    FileSetChanged,
}

/// Decide whether a package's per-file hash pass can be skipped, given the
/// source files found by one directory walk.
///
/// Two independent signals, because neither alone is sufficient:
///
/// * mtimes catch ordinary edits, but a rename (`mv a.ts b.ts`), a `cp -p`,
///   a tar/rsync restore or a `git checkout` all preserve mtimes;
/// * the on-disk path set compared against the paths the DB recorded catches
///   exactly those cases, costs nothing beyond the walk the caller already
///   did, and — crucially — makes it impossible for a package to be skipped
///   *forever*: any divergence between disk and DB forces the full hash pass,
///   which then rewrites both.
///
/// `source_files` must be the same file set the caller would hash (same
/// extension filter and skip patterns), otherwise the set comparison would
/// report a permanent, spurious difference.
pub fn package_source_staleness(
    repo_root: &Path,
    source_files: &[std::path::PathBuf],
    since: SystemTime,
    stored_file_hashes: &std::collections::HashMap<String, String>,
) -> SourceStaleness {
    // Paths are unique within a walk, so equal lengths + full containment
    // is set equality.
    if source_files.len() != stored_file_hashes.len() {
        return SourceStaleness::FileSetChanged;
    }
    for file_path in source_files {
        let relative = file_path
            .strip_prefix(repo_root)
            .unwrap_or(file_path)
            .to_string_lossy();
        if !stored_file_hashes.contains_key(relative.as_ref()) {
            return SourceStaleness::FileSetChanged;
        }
    }

    // Use a 1-second margin on the conservative side (subtract, never add)
    // to tolerate low-resolution filesystem timestamps.
    let margin = std::time::Duration::from_secs(1);
    let since_with_margin = since.checked_sub(margin).unwrap_or(since);

    for file_path in source_files {
        match std::fs::metadata(file_path).and_then(|m| m.modified()) {
            Ok(mtime) => {
                if mtime > since_with_margin {
                    return SourceStaleness::MtimeNewer;
                }
            }
            Err(_) => return SourceStaleness::MtimeNewer, // conservative: assume changed
        }
    }

    SourceStaleness::Unchanged
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_hash_known_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"hello world").unwrap();

        let hash = hash_file(&path).unwrap();
        // SHA-256 of "hello world"
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_hash_missing_file() {
        let result = hash_file(Path::new("/nonexistent/file.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn test_file_tree_hash_deterministic() {
        let files = vec![
            ("src/main.rs".to_string(), 100u64),
            ("src/lib.rs".to_string(), 200u64),
            ("README.md".to_string(), 50u64),
        ];
        let hash1 = compute_file_tree_hash(&files);
        let hash2 = compute_file_tree_hash(&files);
        assert_eq!(hash1, hash2);
        assert!(!hash1.is_empty());
    }

    #[test]
    fn test_file_tree_hash_order_independent() {
        let files_a = vec![
            ("src/main.rs".to_string(), 100u64),
            ("src/lib.rs".to_string(), 200u64),
        ];
        let files_b = vec![
            ("src/lib.rs".to_string(), 200u64),
            ("src/main.rs".to_string(), 100u64),
        ];
        assert_eq!(
            compute_file_tree_hash(&files_a),
            compute_file_tree_hash(&files_b)
        );
    }

    #[test]
    fn test_file_tree_hash_changes_on_addition() {
        let files_a = vec![("src/main.rs".to_string(), 100u64)];
        let files_b = vec![
            ("src/main.rs".to_string(), 100u64),
            ("src/lib.rs".to_string(), 200u64),
        ];
        assert_ne!(
            compute_file_tree_hash(&files_a),
            compute_file_tree_hash(&files_b)
        );
    }

    #[test]
    fn test_file_tree_hash_changes_on_size_change() {
        let files_a = vec![("src/main.rs".to_string(), 100u64)];
        let files_b = vec![("src/main.rs".to_string(), 101u64)];
        assert_ne!(
            compute_file_tree_hash(&files_a),
            compute_file_tree_hash(&files_b)
        );
    }

    #[test]
    fn test_file_tree_hash_empty() {
        let files: Vec<(String, u64)> = vec![];
        let hash = compute_file_tree_hash(&files);
        assert!(!hash.is_empty());
    }

    use std::collections::HashMap;
    use std::path::PathBuf;

    fn stored(paths: &[&str]) -> HashMap<String, String> {
        paths
            .iter()
            .map(|p| (p.to_string(), "hash".to_string()))
            .collect()
    }

    #[test]
    fn test_staleness_unchanged_when_old_and_set_matches() {
        let dir = tempfile::TempDir::new().unwrap();
        let f = dir.path().join("lib.rs");
        std::fs::write(&f, "pub fn hello() {}").unwrap();
        let future = SystemTime::now() + std::time::Duration::from_secs(60);
        assert_eq!(
            package_source_staleness(dir.path(), &[f], future, &stored(&["lib.rs"])),
            SourceStaleness::Unchanged
        );
    }

    #[test]
    fn test_staleness_detects_newer_mtime() {
        let dir = tempfile::TempDir::new().unwrap();
        let f = dir.path().join("lib.rs");
        std::fs::write(&f, "pub fn hello() {}").unwrap();
        let past = SystemTime::now() - std::time::Duration::from_secs(60);
        assert_eq!(
            package_source_staleness(dir.path(), &[f], past, &stored(&["lib.rs"])),
            SourceStaleness::MtimeNewer
        );
    }

    #[test]
    fn test_staleness_detects_rename_with_preserved_mtime() {
        // INDEX-3: `mv a.rs b.rs` keeps the file's mtime, so an mtime-only
        // pre-check skips the package forever. The path-set comparison must
        // catch it.
        let dir = tempfile::TempDir::new().unwrap();
        let renamed = dir.path().join("b.rs");
        std::fs::write(&renamed, "pub fn hello() {}").unwrap();
        let future = SystemTime::now() + std::time::Duration::from_secs(60);
        assert_eq!(
            package_source_staleness(dir.path(), &[renamed], future, &stored(&["a.rs"])),
            SourceStaleness::FileSetChanged
        );
    }

    #[test]
    fn test_staleness_detects_added_and_removed_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let a = dir.path().join("a.rs");
        std::fs::write(&a, "pub fn a() {}").unwrap();
        let future = SystemTime::now() + std::time::Duration::from_secs(60);
        // one extra file on disk
        assert_eq!(
            package_source_staleness(dir.path(), std::slice::from_ref(&a), future, &stored(&[])),
            SourceStaleness::FileSetChanged
        );
        // one extra file in the DB
        assert_eq!(
            package_source_staleness(dir.path(), &[a], future, &stored(&["a.rs", "b.rs"])),
            SourceStaleness::FileSetChanged
        );
    }

    #[test]
    fn test_staleness_empty_package_is_unchanged() {
        let dir = tempfile::TempDir::new().unwrap();
        let past = SystemTime::now() - std::time::Duration::from_secs(60);
        assert_eq!(
            package_source_staleness(dir.path(), &[], past, &HashMap::new()),
            SourceStaleness::Unchanged
        );
    }

    #[test]
    fn test_staleness_missing_file_is_conservative() {
        let dir = tempfile::TempDir::new().unwrap();
        let ghost: PathBuf = dir.path().join("ghost.rs");
        let future = SystemTime::now() + std::time::Duration::from_secs(60);
        // Set matches the DB but the file cannot be stat'd — never skip.
        assert_eq!(
            package_source_staleness(dir.path(), &[ghost], future, &stored(&["ghost.rs"])),
            SourceStaleness::MtimeNewer
        );
    }
}
