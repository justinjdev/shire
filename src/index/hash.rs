use crate::symbols::walker;
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Compute hex-encoded SHA-256 of a file's contents.
pub fn hash_file(path: &Path) -> Result<String> {
    let content = std::fs::read(path)?;
    let digest = Sha256::digest(&content);
    Ok(format!("{:x}", digest))
}

/// Compute an aggregate SHA-256 hash of all source files in a package directory.
/// Walks source files using the same walker as symbol extraction, hashes each file,
/// then hashes the concatenation of all individual hashes (in sorted-path order).
/// Returns SHA-256 of empty string if no source files are found.
pub fn compute_source_hash(repo_root: &Path, package_path: &str, package_kind: &str) -> Result<String> {
    let package_dir = repo_root.join(package_path);
    if !package_dir.is_dir() {
        let digest = Sha256::digest(b"");
        return Ok(format!("{:x}", digest));
    }

    let extensions = walker::extensions_for_kind(package_kind);
    let source_files = walker::walk_source_files(&package_dir, &extensions)?;

    if source_files.is_empty() {
        let digest = Sha256::digest(b"");
        return Ok(format!("{:x}", digest));
    }

    // Hash each file, concatenate hex hashes, then hash the concatenation
    let mut combined = String::new();
    for file_path in &source_files {
        let file_hash = hash_file(file_path)?;
        combined.push_str(&file_hash);
    }

    let digest = Sha256::digest(combined.as_bytes());
    Ok(format!("{:x}", digest))
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
    fn test_compute_source_hash_deterministic() {
        let dir = tempfile::TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "pub fn hello() {}").unwrap();
        std::fs::write(src.join("main.rs"), "fn main() {}").unwrap();

        let hash1 = compute_source_hash(dir.path(), "", "cargo").unwrap();
        let hash2 = compute_source_hash(dir.path(), "", "cargo").unwrap();
        assert_eq!(hash1, hash2);
        assert!(!hash1.is_empty());
    }

    #[test]
    fn test_compute_source_hash_changes_on_add() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn hello() {}").unwrap();

        let hash1 = compute_source_hash(dir.path(), "", "cargo").unwrap();

        std::fs::write(dir.path().join("util.rs"), "pub fn util() {}").unwrap();

        let hash2 = compute_source_hash(dir.path(), "", "cargo").unwrap();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_compute_source_hash_empty_dir() {
        let dir = tempfile::TempDir::new().unwrap();

        let hash1 = compute_source_hash(dir.path(), "", "cargo").unwrap();
        let hash2 = compute_source_hash(dir.path(), "", "cargo").unwrap();
        assert_eq!(hash1, hash2);
        // SHA-256 of empty string
        assert_eq!(hash1, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }
}
