use super::manifest::{DepInfo, DepKind, ManifestParser, PackageInfo};
use anyhow::Result;
use regex::Regex;
use std::path::Path;

pub struct GradleParser;
pub struct GradleKtsParser;

impl ManifestParser for GradleParser {
    fn filename(&self) -> &'static str {
        "build.gradle"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        parse_gradle(manifest_path, relative_dir, &None)
    }
}

impl ManifestParser for GradleKtsParser {
    fn filename(&self) -> &'static str {
        "build.gradle.kts"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        parse_gradle(manifest_path, relative_dir, &None)
    }
}

/// Parsed settings.gradle context for project naming.
pub struct GradleSettingsContext {
    pub root_project_name: Option<String>,
}

pub fn parse_with_settings_context(
    manifest_path: &Path,
    relative_dir: &str,
    settings_ctx: &Option<GradleSettingsContext>,
) -> Result<PackageInfo> {
    parse_gradle(manifest_path, relative_dir, settings_ctx)
}

fn parse_gradle(
    manifest_path: &Path,
    relative_dir: &str,
    settings_ctx: &Option<GradleSettingsContext>,
) -> Result<PackageInfo> {
    let content = std::fs::read_to_string(manifest_path)?;

    let group = extract_property(&content, "group");
    let version = extract_property(&content, "version");

    // Determine project name: settings rootProject.name > directory > fallback
    let dir_name = if relative_dir.is_empty() {
        settings_ctx
            .as_ref()
            .and_then(|s| s.root_project_name.clone())
            .unwrap_or_else(|| "root".to_string())
    } else {
        relative_dir
            .rsplit_once('/')
            .map(|(_, name)| name.to_string())
            .unwrap_or_else(|| relative_dir.to_string())
    };

    let name = match &group {
        Some(g) => format!("{}:{}", g, dir_name),
        None => {
            if relative_dir.is_empty() {
                dir_name
            } else {
                relative_dir.replace('/', "-")
            }
        }
    };

    let dependencies = extract_dependencies(&content);

    Ok(PackageInfo {
        name,
        path: relative_dir.to_string(),
        kind: "gradle",
        version,
        description: None,
        metadata: None,
        dependencies,
    })
}

fn extract_property(content: &str, prop: &str) -> Option<String> {
    // Match: group = "com.example" or group = 'com.example' or group "com.example"
    let pattern = format!(
        r#"(?m)^\s*{}\s*[=]?\s*["']([^"']+)["']"#,
        regex::escape(prop)
    );
    let re = Regex::new(&pattern).ok()?;
    re.captures(content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

fn extract_dependencies(content: &str) -> Vec<DepInfo> {
    let mut deps = Vec::new();

    // Match string notation: configuration("group:name:version") or configuration 'group:name:version'
    // Use [ \t]* instead of \s* to avoid matching across newlines
    let string_dep_re = Regex::new(
        r#"(?m)^[ \t]*(testImplementation|testRuntimeOnly|testCompileOnly|compileOnly|runtimeOnly|implementation|api)\b[ \t]*[\(]?[ \t]*["']([^"']+)["'][ \t]*[\)]?"#,
    )
    .unwrap();

    for cap in string_dep_re.captures_iter(content) {
        let config = cap.get(1).unwrap().as_str();
        let dep_string = cap.get(2).unwrap().as_str();

        let dep_kind = map_config_to_dep_kind(config);

        // Parse group:name:version or group:name
        let parts: Vec<&str> = dep_string.splitn(3, ':').collect();
        let (dep_name, version_req) = match parts.len() {
            3 => (
                format!("{}:{}", parts[0], parts[1]),
                Some(parts[2].to_string()),
            ),
            2 => (format!("{}:{}", parts[0], parts[1]), None),
            _ => (dep_string.to_string(), None),
        };

        deps.push(DepInfo {
            name: dep_name,
            version_req,
            dep_kind,
        });
    }

    // Match project dependencies: configuration(project(":path")) or configuration project(':path')
    let project_dep_re = Regex::new(
        r#"(?m)^[ \t]*(testImplementation|testRuntimeOnly|testCompileOnly|compileOnly|runtimeOnly|implementation|api)\b[ \t]*[\(]?[ \t]*project[ \t]*\([ \t]*["']:?([^"')]+)["'][ \t]*\)[ \t]*[\)]?"#,
    )
    .unwrap();

    for cap in project_dep_re.captures_iter(content) {
        let config = cap.get(1).unwrap().as_str();
        let project_path = cap.get(2).unwrap().as_str();
        let dep_kind = map_config_to_dep_kind(config);

        deps.push(DepInfo {
            name: format!(":{}", project_path.trim_start_matches(':')),
            version_req: None,
            dep_kind,
        });
    }

    deps
}

fn map_config_to_dep_kind(config: &str) -> DepKind {
    match config {
        "testImplementation" | "testRuntimeOnly" => DepKind::Dev,
        "compileOnly" | "testCompileOnly" => DepKind::Peer,
        _ => DepKind::Runtime, // implementation, api, runtimeOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_gradle(dir: &std::path::Path, filename: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(filename);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_parse_build_gradle_with_group() {
        let dir = TempDir::new().unwrap();
        let path = write_gradle(
            dir.path(),
            "build.gradle",
            r#"
group = 'com.example'
version = '1.0.0'

dependencies {
    implementation 'com.google.guava:guava:32.1'
    testImplementation 'junit:junit:4.13'
}
"#,
        );

        let parser = GradleParser;
        let info = parser.parse(&path, "services/auth").unwrap();

        assert_eq!(info.name, "com.example:auth");
        assert_eq!(info.version.as_deref(), Some("1.0.0"));
        assert_eq!(info.kind, "gradle");
        assert_eq!(info.dependencies.len(), 2);

        let guava = info.dependencies.iter().find(|d| d.name.contains("guava")).unwrap();
        assert_eq!(guava.version_req.as_deref(), Some("32.1"));
        assert!(matches!(guava.dep_kind, DepKind::Runtime));

        let junit = info.dependencies.iter().find(|d| d.name.contains("junit")).unwrap();
        assert!(matches!(junit.dep_kind, DepKind::Dev));
    }

    #[test]
    fn test_parse_build_gradle_kts() {
        let dir = TempDir::new().unwrap();
        let path = write_gradle(
            dir.path(),
            "build.gradle.kts",
            r#"
group = "com.example"
version = "2.0.0"

dependencies {
    implementation("org.jetbrains.kotlin:kotlin-stdlib:1.9.0")
    compileOnly("javax.servlet:javax.servlet-api:4.0.1")
}
"#,
        );

        let parser = GradleKtsParser;
        let info = parser.parse(&path, "libs/core").unwrap();

        assert_eq!(info.name, "com.example:core");
        assert_eq!(info.version.as_deref(), Some("2.0.0"));
        assert_eq!(info.dependencies.len(), 2);

        let kotlin = info.dependencies.iter().find(|d| d.name.contains("kotlin-stdlib")).unwrap();
        assert!(matches!(kotlin.dep_kind, DepKind::Runtime));

        let servlet = info.dependencies.iter().find(|d| d.name.contains("servlet")).unwrap();
        assert!(matches!(servlet.dep_kind, DepKind::Peer));
    }

    #[test]
    fn test_parse_gradle_project_deps() {
        let dir = TempDir::new().unwrap();
        let path = write_gradle(
            dir.path(),
            "build.gradle",
            r#"
group = 'com.example'
version = '1.0.0'

dependencies {
    implementation project(':shared:utils')
    testImplementation project(':test-helpers')
}
"#,
        );

        let parser = GradleParser;
        let info = parser.parse(&path, "app").unwrap();

        let shared = info.dependencies.iter().find(|d| d.name == ":shared:utils").unwrap();
        assert_eq!(shared.version_req, None);
        assert!(matches!(shared.dep_kind, DepKind::Runtime));

        let test_helpers = info.dependencies.iter().find(|d| d.name == ":test-helpers").unwrap();
        assert!(matches!(test_helpers.dep_kind, DepKind::Dev));
    }

    #[test]
    fn test_parse_gradle_no_group_fallback() {
        let dir = TempDir::new().unwrap();
        let path = write_gradle(
            dir.path(),
            "build.gradle",
            r#"
version = '1.0.0'

dependencies {
    implementation 'com.google.guava:guava:32.1'
}
"#,
        );

        let parser = GradleParser;
        let info = parser.parse(&path, "tools/scanner").unwrap();

        assert_eq!(info.name, "tools-scanner");
    }

    #[test]
    fn test_parse_gradle_dep_without_version() {
        let dir = TempDir::new().unwrap();
        let path = write_gradle(
            dir.path(),
            "build.gradle",
            r#"
group = 'com.example'

dependencies {
    implementation 'com.google.guava:guava'
}
"#,
        );

        let parser = GradleParser;
        let info = parser.parse(&path, "app").unwrap();

        let guava = info.dependencies.iter().find(|d| d.name.contains("guava")).unwrap();
        assert_eq!(guava.version_req, None);
    }

    #[test]
    fn test_parse_gradle_all_configurations() {
        let dir = TempDir::new().unwrap();
        let path = write_gradle(
            dir.path(),
            "build.gradle",
            r#"
group = 'com.example'

dependencies {
    implementation 'a:impl:1'
    api 'a:api:1'
    runtimeOnly 'a:runtime:1'
    testImplementation 'a:test-impl:1'
    testRuntimeOnly 'a:test-runtime:1'
    compileOnly 'a:compile-only:1'
    testCompileOnly 'a:test-compile-only:1'
}
"#,
        );

        let parser = GradleParser;
        let info = parser.parse(&path, "app").unwrap();

        assert_eq!(info.dependencies.len(), 7);

        let runtime_deps: Vec<_> = info.dependencies.iter().filter(|d| matches!(d.dep_kind, DepKind::Runtime)).collect();
        assert_eq!(runtime_deps.len(), 3); // implementation, api, runtimeOnly

        let dev_deps: Vec<_> = info.dependencies.iter().filter(|d| matches!(d.dep_kind, DepKind::Dev)).collect();
        assert_eq!(dev_deps.len(), 2); // testImplementation, testRuntimeOnly

        let peer_deps: Vec<_> = info.dependencies.iter().filter(|d| matches!(d.dep_kind, DepKind::Peer)).collect();
        assert_eq!(peer_deps.len(), 2); // compileOnly, testCompileOnly
    }

    #[test]
    fn test_parse_gradle_with_settings_context() {
        let dir = TempDir::new().unwrap();
        let path = write_gradle(
            dir.path(),
            "build.gradle",
            r#"
group = 'com.example'
version = '1.0.0'
"#,
        );

        let ctx = Some(GradleSettingsContext {
            root_project_name: Some("my-project".to_string()),
        });

        let info = parse_with_settings_context(&path, "", &ctx).unwrap();
        assert_eq!(info.name, "com.example:my-project");
    }
}
