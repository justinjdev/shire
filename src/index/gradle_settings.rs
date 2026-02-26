use anyhow::Result;
use regex::Regex;
use std::collections::HashSet;
use std::path::Path;

/// Parsed result from a settings.gradle or settings.gradle.kts file.
#[derive(Debug)]
pub struct GradleSettings {
    /// Directories of included subprojects (e.g., `:lib:core` → `lib/core`).
    pub include_dirs: HashSet<String>,
    /// The rootProject.name value, if set.
    pub root_project_name: Option<String>,
}

/// Parse a settings.gradle or settings.gradle.kts file.
pub fn parse_settings_gradle(path: &Path) -> Result<GradleSettings> {
    let content = std::fs::read_to_string(path)?;
    Ok(parse_settings_content(&content))
}

fn parse_settings_content(content: &str) -> GradleSettings {
    let mut include_dirs = HashSet::new();
    let root_project_name = extract_root_project_name(content);

    // Match include statements: include ':app', ':lib:core'
    // Also: include(":app", ":lib:core")
    let include_re = Regex::new(
        r#"(?m)^\s*include\s*[\(]?\s*((?:["'][^"']+["']\s*,?\s*)+)\s*[\)]?"#,
    )
    .unwrap();

    let project_re = Regex::new(r#"["']([^"']+)["']"#).unwrap();

    for cap in include_re.captures_iter(content) {
        let args = cap.get(1).unwrap().as_str();
        for proj_cap in project_re.captures_iter(args) {
            let project_path = proj_cap.get(1).unwrap().as_str();
            // Convert colon-separated to directory path: `:lib:core` → `lib/core`
            let dir_path = project_path
                .trim_start_matches(':')
                .replace(':', "/");
            if !dir_path.is_empty() {
                include_dirs.insert(dir_path);
            }
        }
    }

    GradleSettings {
        include_dirs,
        root_project_name,
    }
}

fn extract_root_project_name(content: &str) -> Option<String> {
    let re = Regex::new(r#"(?m)^\s*rootProject\s*\.\s*name\s*=\s*["']([^"']+)["']"#).ok()?;
    re.captures(content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_include_directives() {
        let content = r#"
rootProject.name = 'my-project'

include ':app', ':lib:core', ':lib:utils'
"#;

        let settings = parse_settings_content(content);
        assert_eq!(settings.include_dirs.len(), 3);
        assert!(settings.include_dirs.contains("app"));
        assert!(settings.include_dirs.contains("lib/core"));
        assert!(settings.include_dirs.contains("lib/utils"));
    }

    #[test]
    fn test_parse_root_project_name() {
        let content = r#"rootProject.name = "my-awesome-project""#;
        let settings = parse_settings_content(content);
        assert_eq!(
            settings.root_project_name.as_deref(),
            Some("my-awesome-project")
        );
    }

    #[test]
    fn test_parse_kts_style_include() {
        let content = r#"
rootProject.name = "my-project"

include(":app")
include(":lib:core", ":lib:utils")
"#;

        let settings = parse_settings_content(content);
        assert_eq!(settings.include_dirs.len(), 3);
        assert!(settings.include_dirs.contains("app"));
        assert!(settings.include_dirs.contains("lib/core"));
        assert!(settings.include_dirs.contains("lib/utils"));
        assert_eq!(settings.root_project_name.as_deref(), Some("my-project"));
    }

    #[test]
    fn test_parse_mixed_content() {
        let content = r#"
pluginManagement {
    repositories {
        gradlePluginPortal()
    }
}

rootProject.name = 'multi-mod'
include ':api', ':web'

dependencyResolutionManagement {
    versionCatalogs {
        create("libs") { from(files("gradle/libs.versions.toml")) }
    }
}
"#;

        let settings = parse_settings_content(content);
        assert_eq!(settings.include_dirs.len(), 2);
        assert!(settings.include_dirs.contains("api"));
        assert!(settings.include_dirs.contains("web"));
        assert_eq!(settings.root_project_name.as_deref(), Some("multi-mod"));
    }

    #[test]
    fn test_parse_no_includes() {
        let content = r#"rootProject.name = 'solo-project'"#;
        let settings = parse_settings_content(content);
        assert!(settings.include_dirs.is_empty());
        assert_eq!(settings.root_project_name.as_deref(), Some("solo-project"));
    }
}
