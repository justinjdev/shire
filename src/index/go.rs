use super::manifest::{DepInfo, DepKind, ManifestParser, PackageInfo};
use anyhow::Result;
use std::path::Path;

pub struct GoParser;

impl ManifestParser for GoParser {
    fn filename(&self) -> &'static str {
        "go.mod"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        let content = std::fs::read_to_string(manifest_path)?;

        let mut module_path: Option<String> = None;
        let mut go_version: Option<String> = None;
        let mut dependencies = Vec::new();

        let mut in_require_block = false;

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("module ") {
                module_path = Some(trimmed.strip_prefix("module ").unwrap().trim().to_string());
                continue;
            }

            if trimmed.starts_with("go ") {
                go_version = Some(trimmed.strip_prefix("go ").unwrap().trim().to_string());
                continue;
            }

            // Multi-line require block
            if trimmed == "require (" {
                in_require_block = true;
                continue;
            }

            if in_require_block {
                if trimmed == ")" {
                    in_require_block = false;
                    continue;
                }

                if let Some(dep) = parse_require_line(trimmed) {
                    dependencies.push(dep);
                }
                continue;
            }

            // Single-line require: `require pkg version`
            if trimmed.starts_with("require ") {
                let rest = trimmed.strip_prefix("require ").unwrap().trim();
                if let Some(dep) = parse_require_line(rest) {
                    dependencies.push(dep);
                }
            }
        }

        let name = module_path
            .as_deref()
            .and_then(|p| p.rsplit('/').next())
            .map(|s| s.to_string())
            .unwrap_or_else(|| relative_dir.replace('/', "-"));

        let description = module_path.clone();

        Ok(PackageInfo {
            name,
            path: relative_dir.to_string(),
            kind: "go",
            version: go_version,
            description,
            metadata: None,
            dependencies,
        })
    }
}

fn parse_require_line(line: &str) -> Option<DepInfo> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("//") {
        return None;
    }

    // Remove inline comments
    let line = if let Some(idx) = line.find("//") {
        line[..idx].trim()
    } else {
        line
    };

    let mut parts = line.split_whitespace();
    let name = parts.next()?;
    let version = parts.next().map(|s| s.to_string());

    Some(DepInfo {
        name: name.to_string(),
        version_req: version,
        dep_kind: DepKind::Runtime,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_manifest(dir: &std::path::Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("go.mod");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_parse_go_mod() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"module github.com/company/api-gateway

go 1.21

require (
	github.com/gin-gonic/gin v1.9.1
	github.com/go-sql-driver/mysql v1.7.1
	go.uber.org/zap v1.26.0
)

require github.com/stretchr/testify v1.8.4
"#,
        );

        let parser = GoParser;
        let info = parser.parse(&path, "services/gateway").unwrap();

        assert_eq!(info.name, "api-gateway");
        assert_eq!(info.version.as_deref(), Some("1.21"));
        assert_eq!(
            info.description.as_deref(),
            Some("github.com/company/api-gateway")
        );
        assert_eq!(info.kind, "go");
        assert_eq!(info.path, "services/gateway");
        assert_eq!(info.dependencies.len(), 4);

        let dep_names: Vec<&str> = info.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(dep_names.contains(&"github.com/gin-gonic/gin"));
        assert!(dep_names.contains(&"github.com/go-sql-driver/mysql"));
        assert!(dep_names.contains(&"go.uber.org/zap"));
        assert!(dep_names.contains(&"github.com/stretchr/testify"));

        let gin = info
            .dependencies
            .iter()
            .find(|d| d.name == "github.com/gin-gonic/gin")
            .unwrap();
        assert_eq!(gin.version_req.as_deref(), Some("v1.9.1"));
    }

    #[test]
    fn test_parse_minimal_go_mod() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"module github.com/user/simple

go 1.22
"#,
        );

        let parser = GoParser;
        let info = parser.parse(&path, "simple").unwrap();

        assert_eq!(info.name, "simple");
        assert!(info.dependencies.is_empty());
    }

    #[test]
    fn test_parse_no_module_falls_back_to_dir() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(dir.path(), "go 1.21\n");

        let parser = GoParser;
        let info = parser.parse(&path, "services/unknown").unwrap();

        assert_eq!(info.name, "services-unknown");
    }

    #[test]
    fn test_parse_require_with_comments() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"module github.com/user/proj

go 1.21

require (
	// indirect dependency
	github.com/pkg/errors v0.9.1
	golang.org/x/sync v0.5.0 // indirect
)
"#,
        );

        let parser = GoParser;
        let info = parser.parse(&path, "proj").unwrap();

        assert_eq!(info.dependencies.len(), 2);
        let sync = info
            .dependencies
            .iter()
            .find(|d| d.name == "golang.org/x/sync")
            .unwrap();
        assert_eq!(sync.version_req.as_deref(), Some("v0.5.0"));
    }
}
