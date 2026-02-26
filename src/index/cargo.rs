use super::manifest::{DepInfo, DepKind, ManifestParser, PackageInfo};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

pub struct CargoParser;

impl ManifestParser for CargoParser {
    fn filename(&self) -> &'static str {
        "Cargo.toml"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        let content = std::fs::read_to_string(manifest_path)?;
        let doc: toml::Value = toml::from_str(&content)?;

        let package = doc
            .get("package")
            .ok_or_else(|| anyhow::anyhow!("No [package] section"))?;

        let name = package
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| relative_dir.replace('/', "-"));

        let version = package
            .get("version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let description = package
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut dependencies = Vec::new();

        extract_deps(&doc, "dependencies", DepKind::Runtime, &mut dependencies);
        extract_deps(&doc, "dev-dependencies", DepKind::Dev, &mut dependencies);
        extract_deps(&doc, "build-dependencies", DepKind::Build, &mut dependencies);

        Ok(PackageInfo {
            name,
            path: relative_dir.to_string(),
            kind: "cargo",
            version,
            description,
            metadata: None,
            dependencies,
        })
    }
}

/// Read `[workspace.dependencies]` from a Cargo.toml and return a map of dep name → version.
pub fn collect_cargo_workspace_deps(path: &Path) -> Result<HashMap<String, String>> {
    let content = std::fs::read_to_string(path)?;
    let doc: toml::Value = toml::from_str(&content)?;

    let mut deps = HashMap::new();

    let ws_deps = doc
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(|d| d.as_table());

    if let Some(table) = ws_deps {
        for (name, value) in table {
            let version = match value {
                toml::Value::String(s) => Some(s.clone()),
                toml::Value::Table(t) => t
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                _ => None,
            };
            if let Some(v) = version {
                deps.insert(name.clone(), v);
            }
        }
    }

    Ok(deps)
}

impl CargoParser {
    /// Parse a Cargo.toml with optional workspace dependency context.
    /// When a dep uses `workspace = true`, its version is resolved from the workspace map.
    pub fn parse_with_workspace_deps(
        &self,
        manifest_path: &Path,
        relative_dir: &str,
        workspace_deps: &HashMap<String, String>,
    ) -> Result<PackageInfo> {
        let content = std::fs::read_to_string(manifest_path)?;
        let doc: toml::Value = toml::from_str(&content)?;

        let package = doc
            .get("package")
            .ok_or_else(|| anyhow::anyhow!("No [package] section"))?;

        let name = package
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| relative_dir.replace('/', "-"));

        let version = package
            .get("version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let description = package
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut dependencies = Vec::new();

        extract_deps_with_workspace(&doc, "dependencies", DepKind::Runtime, workspace_deps, &mut dependencies);
        extract_deps_with_workspace(&doc, "dev-dependencies", DepKind::Dev, workspace_deps, &mut dependencies);
        extract_deps_with_workspace(&doc, "build-dependencies", DepKind::Build, workspace_deps, &mut dependencies);

        Ok(PackageInfo {
            name,
            path: relative_dir.to_string(),
            kind: "cargo",
            version,
            description,
            metadata: None,
            dependencies,
        })
    }
}

fn extract_deps(doc: &toml::Value, section: &str, kind: DepKind, out: &mut Vec<DepInfo>) {
    extract_deps_with_workspace(doc, section, kind, &HashMap::new(), out);
}

fn extract_deps_with_workspace(
    doc: &toml::Value,
    section: &str,
    kind: DepKind,
    workspace_deps: &HashMap<String, String>,
    out: &mut Vec<DepInfo>,
) {
    let Some(table) = doc.get(section).and_then(|v| v.as_table()) else {
        return;
    };

    for (name, value) in table {
        let version_req = match value {
            toml::Value::String(s) => Some(s.clone()),
            toml::Value::Table(t) => {
                if t.get("workspace")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    workspace_deps.get(name).cloned()
                } else {
                    t.get("version")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                }
            }
            _ => None,
        };

        out.push(DepInfo {
            name: name.clone(),
            version_req,
            dep_kind: kind,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_manifest(dir: &std::path::Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("Cargo.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_parse_cargo_toml() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
[package]
name = "my-service"
version = "0.3.1"
description = "A cool service"

[dependencies]
tokio = { version = "1", features = ["full"] }
serde = "1"
anyhow = "1"

[dev-dependencies]
tempfile = "3"

[build-dependencies]
cc = "1"
"#,
        );

        let parser = CargoParser;
        let info = parser.parse(&path, "crates/my-service").unwrap();

        assert_eq!(info.name, "my-service");
        assert_eq!(info.version.as_deref(), Some("0.3.1"));
        assert_eq!(info.description.as_deref(), Some("A cool service"));
        assert_eq!(info.kind, "cargo");
        assert_eq!(info.path, "crates/my-service");
        assert_eq!(info.dependencies.len(), 5);

        let runtime: Vec<&str> = info
            .dependencies
            .iter()
            .filter(|d| matches!(d.dep_kind, DepKind::Runtime))
            .map(|d| d.name.as_str())
            .collect();
        assert!(runtime.contains(&"tokio"));
        assert!(runtime.contains(&"serde"));
        assert!(runtime.contains(&"anyhow"));

        let tokio_dep = info
            .dependencies
            .iter()
            .find(|d| d.name == "tokio")
            .unwrap();
        assert_eq!(tokio_dep.version_req.as_deref(), Some("1"));

        let dev: Vec<&str> = info
            .dependencies
            .iter()
            .filter(|d| matches!(d.dep_kind, DepKind::Dev))
            .map(|d| d.name.as_str())
            .collect();
        assert!(dev.contains(&"tempfile"));

        let build: Vec<&str> = info
            .dependencies
            .iter()
            .filter(|d| matches!(d.dep_kind, DepKind::Build))
            .map(|d| d.name.as_str())
            .collect();
        assert!(build.contains(&"cc"));
    }

    #[test]
    fn test_parse_minimal_cargo_toml() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
[package]
name = "tiny"
version = "0.1.0"
edition = "2021"
"#,
        );

        let parser = CargoParser;
        let info = parser.parse(&path, "tiny").unwrap();

        assert_eq!(info.name, "tiny");
        assert_eq!(info.version.as_deref(), Some("0.1.0"));
        assert_eq!(info.description, None);
        assert!(info.dependencies.is_empty());
    }

    #[test]
    fn test_parse_workspace_inherited_deps() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
[package]
name = "member-crate"
version = "0.1.0"

[dependencies]
tokio = { workspace = true }
serde = { workspace = true, features = ["derive"] }
anyhow = "1"
"#,
        );

        let mut workspace_deps = HashMap::new();
        workspace_deps.insert("tokio".to_string(), "1.35".to_string());
        workspace_deps.insert("serde".to_string(), "1.0".to_string());

        let parser = CargoParser;
        let info = parser
            .parse_with_workspace_deps(&path, "crates/member", &workspace_deps)
            .unwrap();

        assert_eq!(info.name, "member-crate");

        let tokio_dep = info.dependencies.iter().find(|d| d.name == "tokio").unwrap();
        assert_eq!(tokio_dep.version_req.as_deref(), Some("1.35"));

        let serde_dep = info.dependencies.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde_dep.version_req.as_deref(), Some("1.0"));

        let anyhow_dep = info.dependencies.iter().find(|d| d.name == "anyhow").unwrap();
        assert_eq!(anyhow_dep.version_req.as_deref(), Some("1"));
    }

    #[test]
    fn test_parse_workspace_dep_not_in_map() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
[package]
name = "orphan-crate"
version = "0.1.0"

[dependencies]
missing-dep = { workspace = true }
"#,
        );

        let workspace_deps = HashMap::new();

        let parser = CargoParser;
        let info = parser
            .parse_with_workspace_deps(&path, "crates/orphan", &workspace_deps)
            .unwrap();

        let dep = info.dependencies.iter().find(|d| d.name == "missing-dep").unwrap();
        assert_eq!(dep.version_req, None);
    }

    #[test]
    fn test_collect_workspace_deps() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
[workspace]
members = ["crates/*"]

[workspace.dependencies]
tokio = { version = "1.35", features = ["full"] }
serde = "1.0"
anyhow = { version = "1" }
"#,
        );

        let deps = collect_cargo_workspace_deps(&path).unwrap();
        assert_eq!(deps.get("tokio").unwrap(), "1.35");
        assert_eq!(deps.get("serde").unwrap(), "1.0");
        assert_eq!(deps.get("anyhow").unwrap(), "1");
    }

    #[test]
    fn test_parse_no_package_returns_error() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
[dependencies]
serde = "1"
"#,
        );

        let parser = CargoParser;
        let result = parser.parse(&path, "crates/unnamed");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("[package]"));
    }
}
