use anyhow::Result;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Compute hex-encoded SHA-256 of a file's contents.
pub fn hash_file(path: &Path) -> Result<String> {
    let content = std::fs::read(path)?;
    let digest = Sha256::digest(&content);
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
}
