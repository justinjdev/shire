use super::manifest::{DepInfo, DepKind, ManifestParser, PackageInfo};
use anyhow::{anyhow, Result};
use quick_xml::de::from_str;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Pom {
    #[serde(rename = "groupId")]
    group_id: Option<String>,
    #[serde(rename = "artifactId")]
    artifact_id: Option<String>,
    version: Option<String>,
    description: Option<String>,
    packaging: Option<String>,
    parent: Option<PomParent>,
    modules: Option<PomModules>,
    dependencies: Option<PomDependencies>,
    #[serde(rename = "dependencyManagement")]
    dependency_management: Option<PomDependencyManagement>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PomParent {
    #[serde(rename = "groupId")]
    group_id: Option<String>,
    #[serde(rename = "artifactId")]
    artifact_id: Option<String>,
    version: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PomModules {
    #[serde(rename = "module", default)]
    modules: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PomDependencies {
    #[serde(rename = "dependency", default)]
    dependencies: Vec<PomDependency>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PomDependency {
    #[serde(rename = "groupId")]
    group_id: Option<String>,
    #[serde(rename = "artifactId")]
    artifact_id: Option<String>,
    version: Option<String>,
    scope: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PomDependencyManagement {
    dependencies: Option<PomDependencies>,
}

/// Context collected from parent POMs for child resolution.
#[derive(Debug, Clone)]
pub struct MavenParentContext {
    pub group_id: Option<String>,
    pub version: Option<String>,
    /// Map of `groupId:artifactId` → version from `<dependencyManagement>`.
    pub managed_deps: HashMap<String, String>,
}

pub struct MavenParser;

impl ManifestParser for MavenParser {
    fn filename(&self) -> &'static str {
        "pom.xml"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        self.parse_with_parent_context(manifest_path, relative_dir, &HashMap::new())
    }
}

impl MavenParser {
    pub fn parse_with_parent_context(
        &self,
        manifest_path: &Path,
        relative_dir: &str,
        parent_context: &HashMap<String, MavenParentContext>,
    ) -> Result<PackageInfo> {
        let content = std::fs::read_to_string(manifest_path)?;
        let pom: Pom = from_str(&content)?;

        let artifact_id = pom
            .artifact_id
            .ok_or_else(|| anyhow!("No <artifactId> in pom.xml"))?;

        // Check if this is a parent/aggregator POM (has <modules> with pom packaging)
        let has_modules = pom
            .modules
            .as_ref()
            .map(|m| !m.modules.is_empty())
            .unwrap_or(false);
        let is_pom_packaging = pom
            .packaging
            .as_deref()
            .map(|p| p == "pom")
            .unwrap_or(false);

        if has_modules && is_pom_packaging {
            return Err(anyhow!("Parent/aggregator POM (has <modules> with pom packaging)"));
        }

        // Resolve groupId: own > parent in-repo > parent declared > fallback
        let parent_key = pom.parent.as_ref().map(|p| {
            format!(
                "{}:{}",
                p.group_id.as_deref().unwrap_or(""),
                p.artifact_id.as_deref().unwrap_or("")
            )
        });
        let resolved_parent = parent_key
            .as_ref()
            .and_then(|k| parent_context.get(k));

        let group_id = pom
            .group_id
            .or_else(|| resolved_parent.and_then(|p| p.group_id.clone()))
            .or_else(|| pom.parent.as_ref().and_then(|p| p.group_id.clone()));

        let version = pom
            .version
            .or_else(|| resolved_parent.and_then(|p| p.version.clone()))
            .or_else(|| pom.parent.as_ref().and_then(|p| p.version.clone()));

        let name = match &group_id {
            Some(gid) => format!("{}:{}", gid, artifact_id),
            None => {
                if relative_dir.is_empty() {
                    artifact_id.clone()
                } else {
                    relative_dir.replace('/', "-")
                }
            }
        };

        // Build managed deps map for resolving child dep versions
        let managed_deps = build_managed_deps(&pom.dependency_management, resolved_parent);

        // Extract dependencies
        let mut dependencies = Vec::new();
        if let Some(deps) = &pom.dependencies {
            for dep in &deps.dependencies {
                let dep_group = dep.group_id.as_deref().unwrap_or("");
                let dep_artifact = dep.artifact_id.as_deref().unwrap_or("");
                if dep_artifact.is_empty() {
                    continue;
                }
                let dep_name = format!("{}:{}", dep_group, dep_artifact);

                // Resolve version: own > dependencyManagement
                let version_req = dep
                    .version
                    .clone()
                    .or_else(|| managed_deps.get(&dep_name).cloned());

                let dep_kind = match dep.scope.as_deref() {
                    Some("test") => DepKind::Dev,
                    Some("provided") => DepKind::Peer,
                    _ => DepKind::Runtime, // compile (default), runtime, system
                };

                dependencies.push(DepInfo {
                    name: dep_name,
                    version_req,
                    dep_kind,
                });
            }
        }

        Ok(PackageInfo {
            name,
            path: relative_dir.to_string(),
            kind: "maven",
            version,
            description: pom.description,
            metadata: None,
            dependencies,
        })
    }
}

fn build_managed_deps(
    dep_mgmt: &Option<PomDependencyManagement>,
    parent_ctx: Option<&MavenParentContext>,
) -> HashMap<String, String> {
    let mut managed = HashMap::new();

    // Start with parent's managed deps
    if let Some(ctx) = parent_ctx {
        managed.extend(ctx.managed_deps.clone());
    }

    // Override with this POM's own dependencyManagement
    if let Some(mgmt) = dep_mgmt {
        if let Some(deps) = &mgmt.dependencies {
            for dep in &deps.dependencies {
                let group = dep.group_id.as_deref().unwrap_or("");
                let artifact = dep.artifact_id.as_deref().unwrap_or("");
                if let Some(ver) = &dep.version {
                    managed.insert(format!("{}:{}", group, artifact), ver.clone());
                }
            }
        }
    }

    managed
}

/// Collect parent POM context from all walked pom.xml files.
/// Returns a map of `groupId:artifactId` → MavenParentContext for parent/aggregator POMs.
pub fn collect_maven_parent_context(
    walked: &[super::WalkedManifest],
) -> HashMap<String, MavenParentContext> {
    let mut context = HashMap::new();

    for manifest in walked {
        let filename = manifest
            .abs_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("");

        if filename != "pom.xml" {
            continue;
        }

        let content = match std::fs::read_to_string(&manifest.abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let pom: Pom = match from_str(&content) {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Only collect context from POMs that have <modules>
        let has_modules = pom
            .modules
            .as_ref()
            .map(|m| !m.modules.is_empty())
            .unwrap_or(false);

        if !has_modules {
            continue;
        }

        let group_id = pom.group_id.clone();
        let artifact_id = match &pom.artifact_id {
            Some(a) => a.clone(),
            None => continue,
        };

        let key = format!(
            "{}:{}",
            group_id.as_deref().unwrap_or(""),
            artifact_id
        );

        let mut managed_deps = HashMap::new();
        if let Some(mgmt) = &pom.dependency_management {
            if let Some(deps) = &mgmt.dependencies {
                for dep in &deps.dependencies {
                    let g = dep.group_id.as_deref().unwrap_or("");
                    let a = dep.artifact_id.as_deref().unwrap_or("");
                    if let Some(ver) = &dep.version {
                        managed_deps.insert(format!("{}:{}", g, a), ver.clone());
                    }
                }
            }
        }

        context.insert(
            key,
            MavenParentContext {
                group_id,
                version: pom.version,
                managed_deps,
            },
        );
    }

    context
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_manifest(dir: &std::path::Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("pom.xml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_parse_standard_pom() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"<?xml version="1.0"?>
<project>
    <groupId>com.example</groupId>
    <artifactId>auth-service</artifactId>
    <version>1.2.0</version>
    <description>Authentication service</description>
    <dependencies>
        <dependency>
            <groupId>com.google.guava</groupId>
            <artifactId>guava</artifactId>
            <version>32.1</version>
        </dependency>
        <dependency>
            <groupId>junit</groupId>
            <artifactId>junit</artifactId>
            <version>4.13</version>
            <scope>test</scope>
        </dependency>
        <dependency>
            <groupId>javax.servlet</groupId>
            <artifactId>javax.servlet-api</artifactId>
            <version>4.0.1</version>
            <scope>provided</scope>
        </dependency>
    </dependencies>
</project>"#,
        );

        let parser = MavenParser;
        let info = parser.parse(&path, "services/auth").unwrap();

        assert_eq!(info.name, "com.example:auth-service");
        assert_eq!(info.version.as_deref(), Some("1.2.0"));
        assert_eq!(info.description.as_deref(), Some("Authentication service"));
        assert_eq!(info.kind, "maven");
        assert_eq!(info.path, "services/auth");

        assert_eq!(info.dependencies.len(), 3);

        let guava = info.dependencies.iter().find(|d| d.name == "com.google.guava:guava").unwrap();
        assert_eq!(guava.version_req.as_deref(), Some("32.1"));
        assert!(matches!(guava.dep_kind, DepKind::Runtime));

        let junit = info.dependencies.iter().find(|d| d.name == "junit:junit").unwrap();
        assert!(matches!(junit.dep_kind, DepKind::Dev));

        let servlet = info.dependencies.iter().find(|d| d.name.contains("servlet-api")).unwrap();
        assert!(matches!(servlet.dep_kind, DepKind::Peer));
    }

    #[test]
    fn test_parse_pom_no_artifact_id() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"<?xml version="1.0"?>
<project>
    <groupId>com.example</groupId>
</project>"#,
        );

        let parser = MavenParser;
        assert!(parser.parse(&path, "libs/core").is_err());
    }

    #[test]
    fn test_parse_parent_aggregator_pom_skipped() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"<?xml version="1.0"?>
<project>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
    <version>1.0.0</version>
    <packaging>pom</packaging>
    <modules>
        <module>auth</module>
        <module>billing</module>
    </modules>
</project>"#,
        );

        let parser = MavenParser;
        assert!(parser.parse(&path, "").is_err());
    }

    #[test]
    fn test_parse_pom_with_parent_context() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"<?xml version="1.0"?>
<project>
    <parent>
        <groupId>com.example</groupId>
        <artifactId>parent</artifactId>
        <version>2.0.0</version>
    </parent>
    <artifactId>child-service</artifactId>
    <dependencies>
        <dependency>
            <groupId>com.google.guava</groupId>
            <artifactId>guava</artifactId>
        </dependency>
    </dependencies>
</project>"#,
        );

        let mut parent_ctx = HashMap::new();
        let mut managed = HashMap::new();
        managed.insert("com.google.guava:guava".to_string(), "32.1".to_string());
        parent_ctx.insert(
            "com.example:parent".to_string(),
            MavenParentContext {
                group_id: Some("com.example".to_string()),
                version: Some("2.0.0".to_string()),
                managed_deps: managed,
            },
        );

        let parser = MavenParser;
        let info = parser
            .parse_with_parent_context(&path, "modules/child", &parent_ctx)
            .unwrap();

        // groupId inherited from parent
        assert_eq!(info.name, "com.example:child-service");
        // version inherited from parent
        assert_eq!(info.version.as_deref(), Some("2.0.0"));
        // dep version from dependencyManagement
        let guava = info.dependencies.iter().find(|d| d.name.contains("guava")).unwrap();
        assert_eq!(guava.version_req.as_deref(), Some("32.1"));
    }

    #[test]
    fn test_parse_pom_dep_missing_version() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"<?xml version="1.0"?>
<project>
    <groupId>com.example</groupId>
    <artifactId>my-app</artifactId>
    <version>1.0.0</version>
    <dependencies>
        <dependency>
            <groupId>org.unknown</groupId>
            <artifactId>mystery</artifactId>
        </dependency>
    </dependencies>
</project>"#,
        );

        let parser = MavenParser;
        let info = parser.parse(&path, "app").unwrap();
        let mystery = info.dependencies.iter().find(|d| d.name.contains("mystery")).unwrap();
        assert_eq!(mystery.version_req, None);
    }

    #[test]
    fn test_parse_pom_no_group_no_parent_falls_back() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"<?xml version="1.0"?>
<project>
    <artifactId>legacy-app</artifactId>
    <version>1.0.0</version>
</project>"#,
        );

        let parser = MavenParser;
        let info = parser.parse(&path, "tools/legacy").unwrap();
        assert_eq!(info.name, "tools-legacy");
    }
}
