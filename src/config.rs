use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub packages: Vec<PackageOverride>,
}

#[derive(Debug, Deserialize)]
pub struct DiscoveryConfig {
    #[serde(default = "default_manifests")]
    pub manifests: Vec<String>,
    #[serde(default = "default_exclude")]
    pub exclude: Vec<String>,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            manifests: default_manifests(),
            exclude: default_exclude(),
        }
    }
}

fn default_manifests() -> Vec<String> {
    vec![
        "package.json".into(),
        "go.mod".into(),
        "Cargo.toml".into(),
        "pyproject.toml".into(),
    ]
}

fn default_exclude() -> Vec<String> {
    vec![
        "node_modules".into(),
        "vendor".into(),
        "dist".into(),
        ".build".into(),
        "target".into(),
        "third_party".into(),
        ".shire".into(),
    ]
}

#[derive(Debug, Deserialize)]
pub struct PackageOverride {
    pub name: String,
    pub description: Option<String>,
}

pub fn load_config(repo_root: &Path) -> Result<Config> {
    let config_path = repo_root.join("shire.toml");
    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    } else {
        Ok(Config::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.discovery.manifests.len(), 4);
        assert!(config.discovery.exclude.contains(&"node_modules".to_string()));
        assert!(config.packages.is_empty());
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[discovery]
manifests = ["package.json", "go.mod"]
exclude = ["vendor", "dist"]

[[packages]]
name = "legacy-auth"
description = "Deprecated auth service"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.discovery.manifests.len(), 2);
        assert_eq!(config.packages.len(), 1);
        assert_eq!(config.packages[0].name, "legacy-auth");
    }

    #[test]
    fn test_load_missing_config_returns_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = load_config(dir.path()).unwrap();
        assert_eq!(config.discovery.manifests.len(), 4);
    }
}
