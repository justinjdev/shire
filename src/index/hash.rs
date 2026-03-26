use crate::symbols::walker;
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

/// Check if any source file in a package directory has been modified since the given timestamp.
/// Returns `true` if any file has a newer mtime (meaning hash computation is needed).
/// Returns `true` on any error (conservative fallback).
/// Short-circuits on the first newer file found.
pub fn has_newer_source_files(
    repo_root: &Path,
    package_path: &str,
    since: SystemTime,
) -> bool {
    let package_dir = repo_root.join(package_path);
    if !package_dir.is_dir() {
        return false;
    }

    let extensions = walker::all_extensions();
    let source_files = match walker::walk_source_files(&package_dir, &extensions) {
        Ok(files) => files,
        Err(_) => return true, // conservative: assume changed on error
    };

    // Use 1-second margin to handle low-resolution filesystem timestamps (e.g. HFS+)
    let margin = std::time::Duration::from_secs(1);
    let since_with_margin = since.checked_sub(margin).unwrap_or(since);

    for file_path in &source_files {
        match std::fs::metadata(file_path).and_then(|m| m.modified()) {
            Ok(mtime) => {
                if mtime > since_with_margin {
                    return true;
                }
            }
            Err(_) => return true, // conservative: assume changed on error
        }
    }

    false
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

    #[test]
    fn test_has_newer_source_files_no_newer() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn hello() {}").unwrap();

        // Use a timestamp in the future — no files should be newer
        let future = SystemTime::now() + std::time::Duration::from_secs(60);
        assert!(!has_newer_source_files(dir.path(), "", future));
    }

    #[test]
    fn test_has_newer_source_files_has_newer() {
        let dir = tempfile::TempDir::new().unwrap();

        // Use a timestamp in the past — file should be newer
        let past = SystemTime::now() - std::time::Duration::from_secs(60);
        std::fs::write(dir.path().join("lib.rs"), "pub fn hello() {}").unwrap();

        assert!(has_newer_source_files(dir.path(), "", past));
    }

    #[test]
    fn test_has_newer_source_files_empty_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let past = SystemTime::now() - std::time::Duration::from_secs(60);
        // No source files — nothing is newer
        assert!(!has_newer_source_files(dir.path(), "", past));
    }

    #[test]
    fn test_has_newer_source_files_nonexistent_dir() {
        let past = SystemTime::now() - std::time::Duration::from_secs(60);
        assert!(!has_newer_source_files(Path::new("/nonexistent/dir"), "", past));
    }

    #[test]
    fn test_has_newer_source_files_ignores_non_matching_extensions() {
        let dir = tempfile::TempDir::new().unwrap();
        // Write a .txt file (not a cargo source extension) — should be ignored
        std::fs::write(dir.path().join("readme.txt"), "hello").unwrap();

        let past = SystemTime::now() - std::time::Duration::from_secs(60);
        assert!(!has_newer_source_files(dir.path(), "", past));
    }
}
