use super::manifest::{DepInfo, DepKind, ManifestParser, PackageInfo};
use anyhow::Result;
use std::path::Path;

pub struct CargoParser;

impl ManifestParser for CargoParser {
    fn filename(&self) -> &'static str {
        "Cargo.toml"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        let content = std::fs::read_to_string(manifest_path)?;
        let doc: toml::Value = toml::from_str(&content)?;

        let package = doc.get("package");

        let name = package
            .and_then(|p| p.get("name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| relative_dir.replace('/', "-"));

        let version = package
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let description = package
            .and_then(|p| p.get("description"))
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

fn extract_deps(doc: &toml::Value, section: &str, kind: DepKind, out: &mut Vec<DepInfo>) {
    let Some(table) = doc.get(section).and_then(|v| v.as_table()) else {
        return;
    };

    for (name, value) in table {
        let version_req = match value {
            toml::Value::String(s) => Some(s.clone()),
            toml::Value::Table(t) => t
                .get("version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
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
    fn test_parse_no_package_falls_back_to_dir() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
[dependencies]
serde = "1"
"#,
        );

        let parser = CargoParser;
        let info = parser.parse(&path, "crates/unnamed").unwrap();

        assert_eq!(info.name, "crates-unnamed");
        assert_eq!(info.dependencies.len(), 1);
    }
}
