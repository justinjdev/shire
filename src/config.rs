// Stub - will be implemented by Task 5
use anyhow::Result;
use std::path::Path;

#[derive(Debug, Default)]
pub struct Config {
    pub discovery: DiscoveryConfig,
    pub packages: Vec<PackageOverride>,
}

#[derive(Debug)]
pub struct DiscoveryConfig {
    pub manifests: Vec<String>,
    pub exclude: Vec<String>,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            manifests: vec![
                "package.json".into(),
                "go.mod".into(),
                "Cargo.toml".into(),
                "pyproject.toml".into(),
            ],
            exclude: vec![
                "node_modules".into(),
                "vendor".into(),
                "dist".into(),
                ".build".into(),
                "target".into(),
                "third_party".into(),
            ],
        }
    }
}

#[derive(Debug)]
pub struct PackageOverride {
    pub name: String,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
}

pub fn load_config(_repo_root: &Path) -> Result<Config> {
    Ok(Config::default())
}
