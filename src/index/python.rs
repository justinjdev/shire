use super::manifest::{DepInfo, DepKind, ManifestParser, PackageInfo};
use anyhow::Result;
use std::path::Path;

pub struct PythonParser;

impl ManifestParser for PythonParser {
    fn filename(&self) -> &'static str {
        "pyproject.toml"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        let content = std::fs::read_to_string(manifest_path)?;
        let doc: toml::Value = toml::from_str(&content)?;

        let project = doc.get("project");

        let name = project
            .and_then(|p| p.get("name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| relative_dir.replace('/', "-"));

        let version = project
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let description = project
            .and_then(|p| p.get("description"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut dependencies = Vec::new();

        // Parse [project.dependencies] array
        if let Some(deps) = project
            .and_then(|p| p.get("dependencies"))
            .and_then(|v| v.as_array())
        {
            for dep in deps {
                if let Some(s) = dep.as_str() {
                    let (name, version_req) = parse_pep508(s);
                    dependencies.push(DepInfo {
                        name,
                        version_req,
                        dep_kind: DepKind::Runtime,
                    });
                }
            }
        }

        // Parse [project.optional-dependencies] groups as dev deps
        if let Some(opt_deps) = project
            .and_then(|p| p.get("optional-dependencies"))
            .and_then(|v| v.as_table())
        {
            for (_group, entries) in opt_deps {
                if let Some(arr) = entries.as_array() {
                    for dep in arr {
                        if let Some(s) = dep.as_str() {
                            let (name, version_req) = parse_pep508(s);
                            dependencies.push(DepInfo {
                                name,
                                version_req,
                                dep_kind: DepKind::Dev,
                            });
                        }
                    }
                }
            }
        }

        Ok(PackageInfo {
            name,
            path: relative_dir.to_string(),
            kind: "python",
            version,
            description,
            metadata: None,
            dependencies,
        })
    }
}

/// Parse a PEP 508 dependency string into (name, optional version_req).
///
/// Examples:
///   "requests>=2.28"      -> ("requests", Some(">=2.28"))
///   "torch>=2.0,<3.0"     -> ("torch", Some(">=2.0,<3.0"))
///   "numpy"               -> ("numpy", None)
///   "black[jupyter]>=23"  -> ("black", Some(">=23"))
fn parse_pep508(spec: &str) -> (String, Option<String>) {
    let spec = spec.trim();

    // Find where the name ends: first char that is not alphanumeric, hyphen, underscore, or dot
    let name_end = spec
        .find(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.')
        .unwrap_or(spec.len());

    let name = spec[..name_end].to_string();
    let rest = spec[name_end..].trim();

    // Strip extras like [jupyter]
    let rest = if rest.starts_with('[') {
        if let Some(close) = rest.find(']') {
            rest[close + 1..].trim()
        } else {
            rest
        }
    } else {
        rest
    };

    // Strip environment markers (everything after ";")
    let rest = if let Some(idx) = rest.find(';') {
        rest[..idx].trim()
    } else {
        rest
    };

    let version_req = if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    };

    (name, version_req)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_manifest(dir: &std::path::Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("pyproject.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_parse_pyproject_toml() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
[project]
name = "ml-pipeline"
version = "2.1.0"
description = "ML training pipeline"
dependencies = [
    "torch>=2.0",
    "numpy>=1.24,<2.0",
    "requests",
]

[project.optional-dependencies]
dev = [
    "pytest>=7.0",
    "black[jupyter]>=23",
]
lint = [
    "ruff>=0.1.0",
]
"#,
        );

        let parser = PythonParser;
        let info = parser.parse(&path, "packages/ml").unwrap();

        assert_eq!(info.name, "ml-pipeline");
        assert_eq!(info.version.as_deref(), Some("2.1.0"));
        assert_eq!(info.description.as_deref(), Some("ML training pipeline"));
        assert_eq!(info.kind, "python");
        assert_eq!(info.path, "packages/ml");

        let runtime: Vec<&DepInfo> = info
            .dependencies
            .iter()
            .filter(|d| matches!(d.dep_kind, DepKind::Runtime))
            .collect();
        assert_eq!(runtime.len(), 3);

        let torch = runtime.iter().find(|d| d.name == "torch").unwrap();
        assert_eq!(torch.version_req.as_deref(), Some(">=2.0"));

        let numpy = runtime.iter().find(|d| d.name == "numpy").unwrap();
        assert_eq!(numpy.version_req.as_deref(), Some(">=1.24,<2.0"));

        let requests = runtime.iter().find(|d| d.name == "requests").unwrap();
        assert_eq!(requests.version_req, None);

        let dev: Vec<&DepInfo> = info
            .dependencies
            .iter()
            .filter(|d| matches!(d.dep_kind, DepKind::Dev))
            .collect();
        assert_eq!(dev.len(), 3);

        let black = dev.iter().find(|d| d.name == "black").unwrap();
        assert_eq!(black.version_req.as_deref(), Some(">=23"));

        let ruff = dev.iter().find(|d| d.name == "ruff").unwrap();
        assert_eq!(ruff.version_req.as_deref(), Some(">=0.1.0"));
    }

    #[test]
    fn test_parse_minimal_pyproject() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
[project]
name = "tiny-lib"
version = "0.1.0"
"#,
        );

        let parser = PythonParser;
        let info = parser.parse(&path, "libs/tiny").unwrap();

        assert_eq!(info.name, "tiny-lib");
        assert_eq!(info.version.as_deref(), Some("0.1.0"));
        assert_eq!(info.description, None);
        assert!(info.dependencies.is_empty());
    }

    #[test]
    fn test_parse_no_project_falls_back_to_dir() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
[build-system]
requires = ["setuptools"]
"#,
        );

        let parser = PythonParser;
        let info = parser.parse(&path, "tools/build").unwrap();

        assert_eq!(info.name, "tools-build");
    }

    #[test]
    fn test_parse_pep508_variants() {
        assert_eq!(
            parse_pep508("requests>=2.28"),
            ("requests".to_string(), Some(">=2.28".to_string()))
        );
        assert_eq!(
            parse_pep508("numpy>=1.24,<2.0"),
            ("numpy".to_string(), Some(">=1.24,<2.0".to_string()))
        );
        assert_eq!(parse_pep508("flask"), ("flask".to_string(), None));
        assert_eq!(
            parse_pep508("black[jupyter]>=23"),
            ("black".to_string(), Some(">=23".to_string()))
        );
        assert_eq!(
            parse_pep508("typing-extensions>=4.0; python_version<\"3.11\""),
            (
                "typing-extensions".to_string(),
                Some(">=4.0".to_string())
            )
        );
    }
}
