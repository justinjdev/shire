use anyhow::Result;
use std::path::Path;

/// Parse a `go.work` file and extract `use` directive paths.
/// Handles both single-line `use ./dir` and multi-line `use ( ... )` syntax.
pub fn parse_go_work(path: &Path) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(path)?;
    let mut dirs = Vec::new();
    let mut in_use_block = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "use (" {
            in_use_block = true;
            continue;
        }

        if in_use_block {
            if trimmed == ")" {
                in_use_block = false;
                continue;
            }

            if !trimmed.is_empty() && !trimmed.starts_with("//") {
                let dir = trimmed.split("//").next().unwrap().trim();
                if !dir.is_empty() {
                    dirs.push(normalize_use_dir(dir));
                }
            }
            continue;
        }

        // Single-line: `use ./dir`
        if trimmed.starts_with("use ") && !trimmed.contains('(') {
            let rest = trimmed.strip_prefix("use ").unwrap().trim();
            let dir = rest.split("//").next().unwrap().trim();
            if !dir.is_empty() {
                dirs.push(normalize_use_dir(dir));
            }
        }
    }

    Ok(dirs)
}

/// Normalize a use directive path: strip leading `./` to get a relative dir.
fn normalize_use_dir(dir: &str) -> String {
    dir.strip_prefix("./").unwrap_or(dir).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_go_work(dir: &std::path::Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("go.work");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_parse_go_work_multiline() {
        let dir = TempDir::new().unwrap();
        let path = write_go_work(
            dir.path(),
            r#"go 1.22

use (
    ./services/auth
    ./services/gateway
    ./packages/shared
)
"#,
        );

        let dirs = parse_go_work(&path).unwrap();
        assert_eq!(dirs, vec!["services/auth", "services/gateway", "packages/shared"]);
    }

    #[test]
    fn test_parse_go_work_single_line() {
        let dir = TempDir::new().unwrap();
        let path = write_go_work(
            dir.path(),
            r#"go 1.22

use ./services/api
"#,
        );

        let dirs = parse_go_work(&path).unwrap();
        assert_eq!(dirs, vec!["services/api"]);
    }

    #[test]
    fn test_parse_go_work_with_comments() {
        let dir = TempDir::new().unwrap();
        let path = write_go_work(
            dir.path(),
            r#"go 1.22

use (
    // this is a comment
    ./services/auth
    ./services/gateway // inline comment
)
"#,
        );

        let dirs = parse_go_work(&path).unwrap();
        assert_eq!(dirs, vec!["services/auth", "services/gateway"]);
    }

    #[test]
    fn test_parse_go_work_no_dot_prefix() {
        let dir = TempDir::new().unwrap();
        let path = write_go_work(
            dir.path(),
            r#"go 1.22

use (
    services/auth
)
"#,
        );

        let dirs = parse_go_work(&path).unwrap();
        assert_eq!(dirs, vec!["services/auth"]);
    }
}
