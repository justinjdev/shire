use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Default, Clone)]
pub struct Config {
    #[serde(default)]
    pub db_path: Option<String>,
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub packages: Vec<PackageOverride>,
    #[serde(default)]
    pub symbols: SymbolsConfig,
    #[serde(default)]
    pub watch: WatchConfig,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct SymbolsConfig {
    #[serde(default)]
    pub exclude_extensions: Vec<String>,
}

fn default_debounce_ms() -> u64 {
    2000
}

#[derive(Debug, Deserialize, Clone)]
pub struct WatchConfig {
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce_ms: default_debounce_ms(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CustomDiscoveryRule {
    pub name: String,
    pub kind: String,
    pub requires: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    pub max_depth: Option<usize>,
    #[serde(default)]
    pub name_prefix: Option<String>,
    #[serde(default)]
    pub extensions: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DiscoveryConfig {
    #[serde(default = "default_manifests")]
    pub manifests: Vec<String>,
    #[serde(default = "default_exclude")]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub custom: Vec<CustomDiscoveryRule>,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            manifests: default_manifests(),
            exclude: default_exclude(),
            custom: Vec::new(),
        }
    }
}

fn default_manifests() -> Vec<String> {
    vec![
        "package.json".into(),
        "go.mod".into(),
        "go.work".into(),
        "Cargo.toml".into(),
        "pyproject.toml".into(),
        "pom.xml".into(),
        "build.gradle".into(),
        "build.gradle.kts".into(),
        "settings.gradle".into(),
        "settings.gradle.kts".into(),
        "cpanfile".into(),
        "Gemfile".into(),
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
        ".gradle".into(),
        "build".into(),
    ]
}

#[derive(Debug, Deserialize, Clone)]
pub struct PackageOverride {
    pub name: String,
    pub description: Option<String>,
}

/// Resolve the db_path with shell expansion (~, $ENV_VAR) and
/// `{repo}` placeholder substitution (replaced with repo directory name).
/// Falls back to `<repo_root>/.shire/index.db` if not set in config.
pub fn resolve_db_path(config: &Config, repo_root: &Path) -> Result<PathBuf> {
    match &config.db_path {
        Some(p) => {
            let expanded = shellexpand::full(p)
                .with_context(|| {
                    format!("Failed to expand db_path '{p}'. Check that all environment variables are set.")
                })?
                .into_owned();
            let repo_name = repo_name_from_path(repo_root)?;
            Ok(PathBuf::from(expanded.replace("{repo}", &repo_name)))
        }
        None => Ok(repo_root.join(".shire").join("index.db")),
    }
}

/// Extract the repo directory name from a path, for `{repo}` placeholder substitution.
fn repo_name_from_path(repo_root: &Path) -> Result<String> {
    repo_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .with_context(|| {
            format!(
                "Cannot determine repository name from '{}' for {{repo}} placeholder in db_path",
                repo_root.display()
            )
        })
}

pub fn load_config(repo_root: &Path) -> Result<Config> {
    load_config_from(None, repo_root)
}

pub fn load_config_from(config_path: Option<&Path>, repo_root: &Path) -> Result<Config> {
    let path = match config_path {
        Some(p) => {
            let raw = p.to_string_lossy();
            let expanded = shellexpand::full(&raw)
                .with_context(|| {
                    format!("Failed to expand config path '{raw}'. Check that all environment variables are set.")
                })?
                .into_owned();
            PathBuf::from(expanded)
        }
        None => repo_root.join("shire.toml"),
    };
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    } else if config_path.is_some() {
        anyhow::bail!("Config file not found: {}", path.display());
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
        assert_eq!(config.discovery.manifests.len(), 12);
        assert!(config.discovery.exclude.contains(&"node_modules".to_string()));
        assert!(config.discovery.exclude.contains(&".gradle".to_string()));
        assert!(config.discovery.exclude.contains(&"build".to_string()));
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
    fn test_parse_config_with_db_path() {
        let toml_str = r#"
db_path = "/tmp/custom-index.db"

[discovery]
manifests = ["package.json"]
exclude = ["vendor"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.db_path.as_deref(), Some("/tmp/custom-index.db"));
    }

    #[test]
    fn test_resolve_db_path_absolute() {
        let config = Config {
            db_path: Some("/tmp/custom.db".into()),
            ..Config::default()
        };
        let resolved = resolve_db_path(&config, Path::new("/repo")).unwrap();
        assert_eq!(resolved, PathBuf::from("/tmp/custom.db"));
    }

    #[test]
    fn test_resolve_db_path_tilde() {
        let config = Config {
            db_path: Some("~/.claude/shire/index.db".into()),
            ..Config::default()
        };
        let resolved = resolve_db_path(&config, Path::new("/repo")).unwrap();
        assert!(!resolved.to_str().unwrap().contains('~'));
        assert!(resolved.to_str().unwrap().ends_with("/.claude/shire/index.db"));
    }

    #[test]
    fn test_resolve_db_path_default() {
        let config = Config::default();
        let resolved = resolve_db_path(&config, Path::new("/repo")).unwrap();
        assert_eq!(resolved, PathBuf::from("/repo/.shire/index.db"));
    }

    #[test]
    fn test_resolve_db_path_env_var() {
        // SAFETY: test is single-threaded; no other thread reads this var.
        unsafe { std::env::set_var("SHIRE_TEST_DIR", "/tmp/shire-test") };
        let config = Config {
            db_path: Some("$SHIRE_TEST_DIR/index.db".into()),
            ..Config::default()
        };
        let resolved = resolve_db_path(&config, Path::new("/repo")).unwrap();
        assert_eq!(resolved, PathBuf::from("/tmp/shire-test/index.db"));
        unsafe { std::env::remove_var("SHIRE_TEST_DIR") };
    }

    #[test]
    fn test_resolve_db_path_undefined_env_var_errors() {
        let config = Config {
            db_path: Some("$SHIRE_NONEXISTENT_VAR_12345/index.db".into()),
            ..Config::default()
        };
        let result = resolve_db_path(&config, Path::new("/repo"));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Failed to expand db_path"));
    }

    #[test]
    fn test_resolve_db_path_repo_placeholder() {
        let config = Config {
            db_path: Some("~/.claude/shire/{repo}/index.db".into()),
            ..Config::default()
        };
        let resolved = resolve_db_path(&config, Path::new("/home/user/git/my-monorepo")).unwrap();
        assert!(resolved.to_str().unwrap().contains("/my-monorepo/"));
        assert!(resolved.to_str().unwrap().ends_with("/my-monorepo/index.db"));
    }

    #[test]
    fn test_resolve_db_path_root_path_errors() {
        let config = Config {
            db_path: Some("~/.claude/shire/{repo}/index.db".into()),
            ..Config::default()
        };
        let result = resolve_db_path(&config, Path::new("/"));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Cannot determine repository name"));
    }

    #[test]
    fn test_load_config_from_explicit_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg_path = dir.path().join("custom-shire.toml");
        std::fs::write(&cfg_path, "db_path = \"/tmp/test.db\"\n").unwrap();
        let config = load_config_from(Some(&cfg_path), dir.path()).unwrap();
        assert_eq!(config.db_path.as_deref(), Some("/tmp/test.db"));
    }

    #[test]
    fn test_load_config_from_missing_explicit_path_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = load_config_from(Some(Path::new("/nonexistent/shire.toml")), dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_config_with_symbols() {
        let toml_str = r#"
[symbols]
exclude_extensions = [".proto", ".pl"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.symbols.exclude_extensions, vec![".proto", ".pl"]);
    }

    #[test]
    fn test_load_missing_config_returns_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = load_config(dir.path()).unwrap();
        assert_eq!(config.discovery.manifests.len(), 12);
    }

    #[test]
    fn test_parse_custom_discovery_rules() {
        let toml_str = r#"
[[discovery.custom]]
name = "go-apps"
kind = "go"
requires = ["main.go", "ownership.yml"]
paths = ["services/", "cmd/"]
exclude = ["testdata"]
max_depth = 3
name_prefix = "go:"

[[discovery.custom]]
name = "proto-packages"
kind = "proto"
requires = ["*.proto", "buf.yaml"]
paths = ["proto/"]
max_depth = 4
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.discovery.custom.len(), 2);

        let go = &config.discovery.custom[0];
        assert_eq!(go.name, "go-apps");
        assert_eq!(go.kind, "go");
        assert_eq!(go.requires, vec!["main.go", "ownership.yml"]);
        assert_eq!(go.paths, vec!["services/", "cmd/"]);
        assert_eq!(go.exclude, vec!["testdata"]);
        assert_eq!(go.max_depth, Some(3));
        assert_eq!(go.name_prefix.as_deref(), Some("go:"));

        let proto = &config.discovery.custom[1];
        assert_eq!(proto.name, "proto-packages");
        assert_eq!(proto.kind, "proto");
        assert_eq!(proto.requires, vec!["*.proto", "buf.yaml"]);
        assert!(proto.exclude.is_empty());
        assert!(proto.name_prefix.is_none());
    }

    #[test]
    fn test_no_custom_rules_default() {
        let config = Config::default();
        assert!(config.discovery.custom.is_empty());
    }

    #[test]
    fn test_custom_rule_minimal_fields() {
        let toml_str = r#"
[[discovery.custom]]
name = "apps"
kind = "go"
requires = ["main.go"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.discovery.custom.len(), 1);
        let rule = &config.discovery.custom[0];
        assert!(rule.paths.is_empty());
        assert!(rule.exclude.is_empty());
        assert!(rule.max_depth.is_none());
        assert!(rule.name_prefix.is_none());
        assert!(rule.extensions.is_none());
    }
}
