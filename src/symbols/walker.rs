use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    ".build",
    "vendor",
    "test",
    "tests",
    "__tests__",
    "__pycache__",
];

const SKIP_SUFFIXES: &[&str] = &[
    ".generated.ts",
    ".generated.js",
    ".pb.go",
    "_test.go",
    ".d.ts",
];

const SKIP_FILES: &[&str] = &["build.rs"];

/// Return the source file extensions to scan for a given package kind.
pub fn extensions_for_kind(kind: &str) -> Vec<&'static str> {
    match kind {
        "npm" => vec!["ts", "tsx", "js", "jsx"],
        "go" => vec!["go"],
        "cargo" => vec!["rs"],
        "python" => vec!["py"],
        "maven" | "gradle" => vec!["java", "kt"],
        _ => vec![],
    }
}

/// Walk a directory and collect source files matching the given extensions,
/// skipping excluded directories and generated/test files.
pub fn walk_source_files(dir: &Path, extensions: &[&str]) -> Result<Vec<PathBuf>> {
    let ext_set: HashSet<&str> = extensions.iter().copied().collect();
    let exclude_set: HashSet<&str> = EXCLUDED_DIRS.iter().copied().collect();

    let mut files = Vec::new();

    for entry in WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| {
            if e.file_type().is_dir() {
                let name = e.file_name().to_str().unwrap_or("");
                // Skip hidden dirs (except the root)
                if name.starts_with('.') && e.depth() > 0 {
                    return false;
                }
                return !exclude_set.contains(name);
            }
            true
        })
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        if !ext_set.contains(ext) {
            continue;
        }

        let filename = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("");

        // Skip known generated/test file patterns
        if SKIP_FILES.contains(&filename) {
            continue;
        }

        if SKIP_SUFFIXES.iter().any(|suffix| filename.ends_with(suffix)) {
            continue;
        }

        files.push(path.to_path_buf());
    }

    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_extensions_for_kind() {
        assert_eq!(extensions_for_kind("npm"), vec!["ts", "tsx", "js", "jsx"]);
        assert_eq!(extensions_for_kind("go"), vec!["go"]);
        assert_eq!(extensions_for_kind("cargo"), vec!["rs"]);
        assert_eq!(extensions_for_kind("python"), vec!["py"]);
        assert_eq!(extensions_for_kind("maven"), vec!["java", "kt"]);
        assert_eq!(extensions_for_kind("gradle"), vec!["java", "kt"]);
        assert!(extensions_for_kind("unknown").is_empty());
    }

    #[test]
    fn test_walk_source_files_finds_matching() {
        let dir = tempfile::TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("main.rs"), "fn main() {}").unwrap();
        fs::write(src.join("lib.rs"), "pub fn hello() {}").unwrap();
        fs::write(src.join("readme.md"), "# Hello").unwrap();

        let files = walk_source_files(dir.path(), &["rs"]).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|f| f.extension().unwrap() == "rs"));
    }

    #[test]
    fn test_walk_skips_excluded_dirs() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}").unwrap();

        fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        fs::write(dir.path().join("target/debug/build.rs"), "fn b() {}").unwrap();

        fs::create_dir_all(dir.path().join("node_modules/foo")).unwrap();
        fs::write(dir.path().join("node_modules/foo/index.js"), "").unwrap();

        let rs_files = walk_source_files(dir.path(), &["rs"]).unwrap();
        assert_eq!(rs_files.len(), 1);
        assert!(rs_files[0].ends_with("src/lib.rs"));

        let js_files = walk_source_files(dir.path(), &["js"]).unwrap();
        assert!(js_files.is_empty());
    }

    #[test]
    fn test_walk_skips_generated_files() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("handler.go"), "package main").unwrap();
        fs::write(dir.path().join("types.pb.go"), "// generated").unwrap();
        fs::write(dir.path().join("handler_test.go"), "package main").unwrap();

        let files = walk_source_files(dir.path(), &["go"]).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("handler.go"));
    }

    #[test]
    fn test_walk_skips_build_rs() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("lib.rs"), "pub fn a() {}").unwrap();
        fs::write(dir.path().join("build.rs"), "fn main() {}").unwrap();

        let files = walk_source_files(dir.path(), &["rs"]).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("lib.rs"));
    }
}
