use super::manifest::{DepInfo, DepKind, ManifestParser, PackageInfo};
use anyhow::Result;
use std::path::Path;

pub struct NpmParser;

impl ManifestParser for NpmParser {
    fn filename(&self) -> &'static str {
        "package.json"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        let content = std::fs::read_to_string(manifest_path)?;
        let json: serde_json::Value = serde_json::from_str(&content)?;

        let name = json["name"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| relative_dir.replace('/', "-"));

        let version = json["version"].as_str().map(|s| s.to_string());
        let description = json["description"].as_str().map(|s| s.to_string());

        let mut dependencies = Vec::new();

        extract_deps(&json, "dependencies", DepKind::Runtime, &mut dependencies);
        extract_deps(&json, "devDependencies", DepKind::Dev, &mut dependencies);
        extract_deps(&json, "peerDependencies", DepKind::Peer, &mut dependencies);

        Ok(PackageInfo {
            name,
            path: relative_dir.to_string(),
            kind: "npm",
            version,
            description,
            metadata: None,
            dependencies,
        })
    }
}

fn extract_deps(
    json: &serde_json::Value,
    section: &str,
    kind: DepKind,
    out: &mut Vec<DepInfo>,
) {
    if let Some(deps) = json[section].as_object() {
        for (name, ver) in deps {
            let version_req = ver.as_str().map(|s| strip_workspace_protocol(s).to_string());
            out.push(DepInfo {
                name: name.clone(),
                version_req,
                dep_kind: kind,
            });
        }
    }
}

/// Strip the `workspace:` prefix from npm workspace protocol versions.
fn strip_workspace_protocol(version: &str) -> &str {
    version.strip_prefix("workspace:").unwrap_or(version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_manifest(dir: &std::path::Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("package.json");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_parse_basic_package_json() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
                "name": "@scope/auth-service",
                "version": "1.2.3",
                "description": "Auth service",
                "dependencies": {
                    "express": "^4.18.0",
                    "jsonwebtoken": "^9.0.0"
                },
                "devDependencies": {
                    "jest": "^29.0.0"
                }
            }"#,
        );

        let parser = NpmParser;
        let info = parser.parse(&path, "services/auth").unwrap();

        assert_eq!(info.name, "@scope/auth-service");
        assert_eq!(info.version.as_deref(), Some("1.2.3"));
        assert_eq!(info.description.as_deref(), Some("Auth service"));
        assert_eq!(info.kind, "npm");
        assert_eq!(info.path, "services/auth");
        assert_eq!(info.dependencies.len(), 3);

        let runtime_deps: Vec<&str> = info
            .dependencies
            .iter()
            .filter(|d| matches!(d.dep_kind, DepKind::Runtime))
            .map(|d| d.name.as_str())
            .collect();
        assert!(runtime_deps.contains(&"express"));
        assert!(runtime_deps.contains(&"jsonwebtoken"));

        let dev_deps: Vec<&str> = info
            .dependencies
            .iter()
            .filter(|d| matches!(d.dep_kind, DepKind::Dev))
            .map(|d| d.name.as_str())
            .collect();
        assert!(dev_deps.contains(&"jest"));
    }

    #[test]
    fn test_parse_minimal_package_json() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(dir.path(), r#"{"name": "minimal"}"#);

        let parser = NpmParser;
        let info = parser.parse(&path, "packages/minimal").unwrap();

        assert_eq!(info.name, "minimal");
        assert_eq!(info.version, None);
        assert_eq!(info.description, None);
        assert!(info.dependencies.is_empty());
    }

    #[test]
    fn test_parse_workspace_protocol_versions() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
                "name": "app",
                "version": "1.0.0",
                "dependencies": {
                    "shared-utils": "workspace:*",
                    "shared-types": "workspace:^",
                    "shared-config": "workspace:~1.0.0"
                },
                "devDependencies": {
                    "test-helpers": "workspace:^2.0.0"
                }
            }"#,
        );

        let parser = NpmParser;
        let info = parser.parse(&path, "packages/app").unwrap();

        let find_dep = |name: &str| -> String {
            info.dependencies
                .iter()
                .find(|d| d.name == name)
                .unwrap()
                .version_req
                .clone()
                .unwrap()
        };

        assert_eq!(find_dep("shared-utils"), "*");
        assert_eq!(find_dep("shared-types"), "^");
        assert_eq!(find_dep("shared-config"), "~1.0.0");
        assert_eq!(find_dep("test-helpers"), "^2.0.0");
    }

    #[test]
    fn test_parse_no_name_falls_back_to_dir() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(dir.path(), r#"{"version": "1.0.0"}"#);

        let parser = NpmParser;
        let info = parser.parse(&path, "packages/unnamed").unwrap();

        assert_eq!(info.name, "packages-unnamed");
    }
}
