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
    pub docs: DocsConfig,
    #[serde(default)]
    pub watch: WatchConfig,
    #[serde(default)]
    pub serve: ServeConfig,
    #[serde(default)]
    pub rag: RagConfig,
    #[serde(default)]
    pub log: LogConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SymbolsConfig {
    #[serde(default)]
    pub exclude_extensions: Vec<String>,
    /// File name patterns to skip during symbol extraction.
    /// Supports suffix matches (e.g. "_generated.go") and prefix matches
    /// (e.g. "zz_generated." — note the trailing dot).
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
    /// Extract cross-references (call sites, type uses, imports, impl
    /// relationships) alongside symbol definitions. EXPERIMENTAL — off by
    /// default.
    ///
    /// When enabled, populates the `symbol_refs` table so the
    /// `symbol_references`, `symbol_callers`, and `symbol_callees` MCP tools
    /// can answer refactor-safety questions ("where is this used?", "who
    /// calls this?").
    ///
    /// Cost: DB grows substantially — roughly +30% on TS/JS repos and up to
    /// +150% on Go-heavy repos. Build time grows ~5-7%.
    /// Coverage: 8 tier-1 languages (Go, Python, Java, TypeScript, JavaScript,
    /// Perl, Ruby, Scala). `shire init` prompts for this option and marks
    /// it as experimental.
    ///
    /// Toggling this flag takes effect on the next build; changing it
    /// requires no manual migration — disabled builds wipe `symbol_refs`,
    /// re-enabled builds repopulate it on the next full rebuild
    /// (`shire build --force`).
    #[serde(default = "default_references_enabled")]
    pub references_enabled: bool,
    /// Maximum source file size in bytes for symbol extraction.
    /// Files larger than this are skipped with a warning. 0 = disabled (default).
    #[serde(default = "default_symbols_max_file_size")]
    pub max_file_size: u64,
    /// Maximum number of cross-references to collect per file.
    /// Caps memory for pathological inputs with millions of identifiers.
    /// 0 = no cap (unlimited).
    #[serde(default = "default_max_references_per_file")]
    pub max_references_per_file: usize,
}

impl Default for SymbolsConfig {
    fn default() -> Self {
        Self {
            exclude_extensions: Vec::new(),
            exclude_patterns: Vec::new(),
            references_enabled: default_references_enabled(),
            max_file_size: default_symbols_max_file_size(),
            max_references_per_file: default_max_references_per_file(),
        }
    }
}

fn default_references_enabled() -> bool {
    false
}

fn default_symbols_max_file_size() -> u64 {
    0 // 0 = disabled (no size limit); set to e.g. 2_097_152 for 2 MiB cap
}

fn default_max_references_per_file() -> usize {
    10_000
}

#[derive(Debug, Deserialize, Clone)]
pub struct DocsConfig {
    /// File extensions to index as documentation (e.g. ".md", ".rst", ".txt", ".adoc").
    #[serde(default = "default_doc_extensions")]
    pub extensions: Vec<String>,
    /// Maximum file size in bytes for doc content indexing. Files larger than this are truncated.
    #[serde(default = "default_doc_max_file_size")]
    pub max_file_size: u64,
}

impl Default for DocsConfig {
    fn default() -> Self {
        Self {
            extensions: default_doc_extensions(),
            max_file_size: default_doc_max_file_size(),
        }
    }
}

fn default_doc_extensions() -> Vec<String> {
    vec![".md".into(), ".rst".into(), ".txt".into(), ".adoc".into()]
}

fn default_doc_max_file_size() -> u64 {
    262_144 // 256 KB
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
#[derive(Default)]
pub struct RagConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cache_dir: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LogConfig {
    /// Log level: "error", "warn", "info", "debug", "trace". Default: "warn".
    /// Can be overridden by SHIRE_LOG env var.
    #[serde(default = "default_log_level")]
    pub level: String,
    /// Directory for log files. Default: ".shire/logs".
    /// Set to "" to disable file logging.
    #[serde(default = "default_log_dir")]
    pub dir: String,
    /// Number of days to retain log files. Default: 30.
    #[serde(default = "default_log_max_days")]
    pub max_days: u32,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            dir: default_log_dir(),
            max_days: default_log_max_days(),
        }
    }
}

fn default_log_level() -> String {
    "warn".into()
}

fn default_log_dir() -> String {
    ".shire/logs".into()
}

fn default_log_max_days() -> u32 {
    30
}

fn default_debounce_ms() -> u64 {
    2000
}

fn default_serve_debounce_s() -> u64 {
    5
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServeConfig {
    /// Minimum seconds between rebuild checks during MCP tool calls.
    /// Prevents redundant rebuilds during rapid tool call bursts.
    #[serde(default = "default_serve_debounce_s")]
    pub debounce_s: u64,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            debounce_s: default_serve_debounce_s(),
        }
    }
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
#[allow(dead_code)]
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
        "flake.nix".into(),
    ]
}

pub(crate) fn default_exclude() -> Vec<String> {
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
/// placeholder substitution (`{repo}` → main repo name, `{worktree}` → worktree identifier).
/// Falls back to `<repo_root>/.shire/index.db` if not set in config.
pub fn resolve_db_path(config: &Config, repo_root: &Path) -> Result<PathBuf> {
    resolve_db_path_with_info(config, repo_root, &crate::git::worktree_info(repo_root))
}

/// Inner implementation that accepts pre-computed `WorktreeInfo` (for testability).
pub(crate) fn resolve_db_path_with_info(
    config: &Config,
    repo_root: &Path,
    info: &crate::git::WorktreeInfo,
) -> Result<PathBuf> {
    match &config.db_path {
        Some(p) => {
            if p.contains("{repo}") && info.repo_name == "unknown" {
                anyhow::bail!(
                    "Cannot determine repository name from '{}' for {{repo}} placeholder in db_path. \
                     Ensure the path is a valid directory.",
                    repo_root.display()
                );
            }
            let expanded = shellexpand::full(p)
                .with_context(|| {
                    format!("Failed to expand db_path '{p}'. Check that all environment variables are set.")
                })?
                .into_owned();
            let resolved = expanded
                .replace("{repo}", &info.repo_name)
                .replace("{worktree}", &info.worktree_name);
            let path = PathBuf::from(resolved);
            // Resolve relative paths against repo_root
            if path.is_relative() {
                Ok(repo_root.join(path))
            } else {
                Ok(path)
            }
        }
        None => Ok(repo_root.join(".shire").join("index.db")),
    }
}

/// Compute the seed DB path (main worktree's DB) for a linked worktree.
/// Returns `None` if:
/// - This is the main worktree (nothing to seed from)
/// - The current and main worktree resolve to the same DB path (shared DB)
pub(crate) fn seed_db_path(
    config: &Config,
    repo_root: &Path,
    info: &crate::git::WorktreeInfo,
) -> Result<Option<PathBuf>> {
    if !info.is_linked() {
        return Ok(None);
    }
    let main_info = crate::git::WorktreeInfo {
        repo_name: info.repo_name.clone(),
        worktree_name: crate::git::PRIMARY_WORKTREE_NAME.into(),
        main_root: None,
    };
    let main_repo_root = info.main_root.as_deref().unwrap_or(repo_root);
    let current_path = resolve_db_path_with_info(config, repo_root, info)?;
    let main_path = resolve_db_path_with_info(config, main_repo_root, &main_info)?;

    if current_path == main_path {
        Ok(None)
    } else {
        Ok(Some(main_path))
    }
}

#[allow(dead_code)]
pub fn load_config(repo_root: &Path) -> Result<Config> {
    load_config_from(None, repo_root)
}

pub fn load_config_from(config_path: Option<&Path>, repo_root: &Path) -> Result<Config> {
    if let Some(p) = config_path {
        // Explicit --config: must exist
        let raw = p.to_string_lossy();
        let expanded = shellexpand::full(&raw)
            .with_context(|| {
                format!("Failed to expand config path '{raw}'. Check that all environment variables are set.")
            })?
            .into_owned();
        let path = PathBuf::from(expanded);
        if !path.exists() {
            anyhow::bail!("Config file not found: {}", path.display());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config file {}", path.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file {}", path.display()))?;
        return Ok(config);
    }

    // Fallback chain: ./shire.toml → ~/.claude/shire.toml → defaults
    let local = repo_root.join("shire.toml");
    if local.exists() {
        let content = std::fs::read_to_string(&local)
            .with_context(|| format!("Failed to read config file {}", local.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file {}", local.display()))?;
        return Ok(config);
    }

    match std::env::var("HOME") {
        Ok(home) => {
            let global = PathBuf::from(home).join(".claude/shire.toml");
            if global.exists() {
                let content = std::fs::read_to_string(&global)
                    .with_context(|| format!("Failed to read config file {}", global.display()))?;
                let config: Config = toml::from_str(&content)
                    .with_context(|| format!("Failed to parse config file {}", global.display()))?;
                return Ok(config);
            }
        }
        Err(e) => {
            eprintln!(
                "Warning: could not read HOME environment variable ({e}), skipping global config fallback"
            );
        }
    }

    Ok(Config::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.discovery.manifests.len(), 13);
        assert!(
            config
                .discovery
                .exclude
                .contains(&"node_modules".to_string())
        );
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
    fn test_resolve_db_path_relative() {
        let config = Config {
            db_path: Some("tmp/index.db".into()),
            ..Config::default()
        };
        let resolved = resolve_db_path(&config, Path::new("/home/user/work/monorepo")).unwrap();
        assert_eq!(
            resolved,
            PathBuf::from("/home/user/work/monorepo/tmp/index.db")
        );
    }

    #[test]
    fn test_resolve_db_path_tilde() {
        let config = Config {
            db_path: Some("~/.claude/shire/index.db".into()),
            ..Config::default()
        };
        let resolved = resolve_db_path(&config, Path::new("/repo")).unwrap();
        assert!(!resolved.to_str().unwrap().contains('~'));
        assert!(
            resolved
                .to_str()
                .unwrap()
                .ends_with("/.claude/shire/index.db")
        );
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

    fn test_info(repo: &str, worktree: &str) -> crate::git::WorktreeInfo {
        crate::git::WorktreeInfo {
            repo_name: repo.into(),
            worktree_name: worktree.into(),
            main_root: None,
        }
    }

    fn test_linked_info(repo: &str, worktree: &str, main_root: &str) -> crate::git::WorktreeInfo {
        crate::git::WorktreeInfo {
            repo_name: repo.into(),
            worktree_name: worktree.into(),
            main_root: Some(PathBuf::from(main_root)),
        }
    }

    #[test]
    fn test_resolve_db_path_repo_placeholder() {
        let config = Config {
            db_path: Some("~/.claude/shire/{repo}/index.db".into()),
            ..Config::default()
        };
        let info = test_info("my-monorepo", crate::git::PRIMARY_WORKTREE_NAME);
        let resolved =
            resolve_db_path_with_info(&config, Path::new("/home/user/git/my-monorepo"), &info)
                .unwrap();
        assert!(resolved.to_str().unwrap().contains("/my-monorepo/"));
        assert!(
            resolved
                .to_str()
                .unwrap()
                .ends_with("/my-monorepo/index.db")
        );
    }

    #[test]
    fn test_resolve_db_path_worktree_placeholder() {
        let config = Config {
            db_path: Some("/tmp/shire/{repo}/{worktree}/index.db".into()),
            ..Config::default()
        };
        let info = test_info("my-repo", "feat-xyz");
        let resolved = resolve_db_path_with_info(&config, Path::new("/some/path"), &info).unwrap();
        assert_eq!(
            resolved,
            PathBuf::from("/tmp/shire/my-repo/feat-xyz/index.db")
        );
    }

    #[test]
    fn test_resolve_db_path_worktree_main() {
        let config = Config {
            db_path: Some("/tmp/shire/{repo}/{worktree}/index.db".into()),
            ..Config::default()
        };
        let info = test_info("my-repo", crate::git::PRIMARY_WORKTREE_NAME);
        let resolved = resolve_db_path_with_info(&config, Path::new("/some/path"), &info).unwrap();
        assert_eq!(
            resolved,
            PathBuf::from("/tmp/shire/my-repo/_primary/index.db")
        );
    }

    #[test]
    fn test_resolve_db_path_no_worktree_placeholder_shares_db() {
        let config = Config {
            db_path: Some("/tmp/shire/{repo}/index.db".into()),
            ..Config::default()
        };
        let info = test_info("my-repo", "feat-xyz");
        let resolved = resolve_db_path_with_info(&config, Path::new("/some/path"), &info).unwrap();
        assert_eq!(resolved, PathBuf::from("/tmp/shire/my-repo/index.db"));
    }

    #[test]
    fn test_resolve_db_path_unknown_repo_errors() {
        let config = Config {
            db_path: Some("/tmp/shire/{repo}/index.db".into()),
            ..Config::default()
        };
        let info = test_info("unknown", crate::git::PRIMARY_WORKTREE_NAME);
        let result = resolve_db_path_with_info(&config, Path::new("/"), &info);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Cannot determine repository name")
        );
    }

    #[test]
    fn test_seed_db_path_returns_main_for_linked_worktree() {
        let config = Config {
            db_path: Some("/tmp/shire/{repo}/{worktree}/index.db".into()),
            ..Config::default()
        };
        let info = test_linked_info("my-repo", "feat-xyz", "/main/repo");
        let seed = seed_db_path(&config, Path::new("/some/path"), &info).unwrap();
        assert_eq!(
            seed,
            Some(PathBuf::from("/tmp/shire/my-repo/_primary/index.db"))
        );
    }

    #[test]
    fn test_seed_db_path_none_for_main_worktree() {
        let config = Config {
            db_path: Some("/tmp/shire/{repo}/{worktree}/index.db".into()),
            ..Config::default()
        };
        let info = test_info("my-repo", crate::git::PRIMARY_WORKTREE_NAME);
        assert!(
            seed_db_path(&config, Path::new("/some/path"), &info)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn test_seed_db_path_none_without_worktree_placeholder() {
        let config = Config {
            db_path: Some("/tmp/shire/{repo}/index.db".into()),
            ..Config::default()
        };
        let info = test_linked_info("my-repo", "feat-xyz", "/main/repo");
        assert!(
            seed_db_path(&config, Path::new("/some/path"), &info)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn test_seed_db_path_seeds_for_default_config() {
        // Default db_path (None) resolves to repo_root/.shire/index.db, which differs
        // between the linked worktree and main worktree roots, so seeding should apply.
        let config = Config::default();
        let info = test_linked_info("my-repo", "feat-xyz", "/main/repo");
        let seed = seed_db_path(&config, Path::new("/some/path"), &info).unwrap();
        assert_eq!(seed, Some(PathBuf::from("/main/repo/.shire/index.db")));
    }

    #[test]
    fn test_seed_db_path_seeds_for_explicit_relative_path() {
        // Explicit relative db_path without {worktree} resolves against different
        // repo_roots for linked vs main worktrees, so seeding should apply.
        let config = Config {
            db_path: Some(".shire/index.db".into()),
            ..Config::default()
        };
        let info = test_linked_info("my-repo", "feat-xyz", "/main/repo");
        let seed = seed_db_path(&config, Path::new("/some/path"), &info).unwrap();
        assert_eq!(seed, Some(PathBuf::from("/main/repo/.shire/index.db")));
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
        assert_eq!(config.discovery.manifests.len(), 13);
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

    #[test]
    fn test_parse_config_with_rag() {
        let toml_str = r#"
[rag]
enabled = true
model = "text-embedding-3-small"
cache_dir = "/tmp/shire-rag"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.rag.enabled);
        assert_eq!(config.rag.model.as_deref(), Some("text-embedding-3-small"));
        assert_eq!(config.rag.cache_dir.as_deref(), Some("/tmp/shire-rag"));
    }

    #[test]
    fn test_rag_config_defaults_to_disabled() {
        let toml_str = r#"
[discovery]
manifests = ["package.json"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(!config.rag.enabled);
        assert!(config.rag.model.is_none());
        assert!(config.rag.cache_dir.is_none());
    }

    #[test]
    fn test_load_config_local_takes_precedence() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("shire.toml"),
            "db_path = \"/local/index.db\"\n",
        )
        .unwrap();
        let config = load_config_from(None, dir.path()).unwrap();
        assert_eq!(config.db_path.as_deref(), Some("/local/index.db"));
    }

    #[test]
    fn test_load_config_falls_back_to_global() {
        let home_dir = tempfile::TempDir::new().unwrap();
        let claude_dir = home_dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(
            claude_dir.join("shire.toml"),
            "db_path = \"/global/index.db\"\n",
        )
        .unwrap();

        unsafe { std::env::set_var("HOME", home_dir.path()) };
        let repo_dir = tempfile::TempDir::new().unwrap();
        let config = load_config_from(None, repo_dir.path()).unwrap();
        assert_eq!(config.db_path.as_deref(), Some("/global/index.db"));
        unsafe { std::env::remove_var("HOME") };
    }

    #[test]
    fn test_load_config_local_takes_precedence_over_global() {
        let home_dir = tempfile::TempDir::new().unwrap();
        let claude_dir = home_dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(
            claude_dir.join("shire.toml"),
            "db_path = \"/global/index.db\"\n",
        )
        .unwrap();

        unsafe { std::env::set_var("HOME", home_dir.path()) };
        let repo_dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            repo_dir.path().join("shire.toml"),
            "db_path = \"/local/index.db\"\n",
        )
        .unwrap();
        let config = load_config_from(None, repo_dir.path()).unwrap();
        assert_eq!(config.db_path.as_deref(), Some("/local/index.db"));
        unsafe { std::env::remove_var("HOME") };
    }

    #[test]
    fn test_load_config_no_config_returns_defaults() {
        // Use an explicit empty config file to avoid HOME-dependent fallback
        let dir = tempfile::TempDir::new().unwrap();
        let empty_config = dir.path().join("empty.toml");
        std::fs::write(&empty_config, "").unwrap();
        let config = load_config_from(Some(empty_config.as_path()), dir.path()).unwrap();
        assert!(config.db_path.is_none());
        assert_eq!(config.serve.debounce_s, 5);
        assert!(config.symbols.exclude_patterns.is_empty());
    }

    #[test]
    fn test_serve_config_defaults() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.serve.debounce_s, 5);
    }

    #[test]
    fn test_serve_config_custom() {
        let config: Config = toml::from_str("[serve]\ndebounce_s = 10\n").unwrap();
        assert_eq!(config.serve.debounce_s, 10);
    }

    #[test]
    fn test_exclude_patterns_config() {
        let config: Config =
            toml::from_str("[symbols]\nexclude_patterns = [\"_mock.go\", \"Generated.kt\"]\n")
                .unwrap();
        assert_eq!(config.symbols.exclude_patterns.len(), 2);
        assert!(
            config
                .symbols
                .exclude_patterns
                .contains(&"_mock.go".to_string())
        );
    }

    #[test]
    fn test_docs_config_defaults() {
        let config = Config::default();
        assert_eq!(config.docs.extensions, vec![".md", ".rst", ".txt", ".adoc"]);
        assert_eq!(config.docs.max_file_size, 262_144);
    }

    #[test]
    fn test_symbols_max_file_size_default() {
        let config = Config::default();
        assert_eq!(config.symbols.max_file_size, 0); // disabled by default
        assert_eq!(config.symbols.max_references_per_file, 10_000);
    }

    #[test]
    fn test_parse_symbols_max_file_size() {
        let toml_str = r#"
[symbols]
max_file_size = 4194304
max_references_per_file = 5000
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.symbols.max_file_size, 4_194_304);
        assert_eq!(config.symbols.max_references_per_file, 5_000);
    }

    #[test]
    fn test_parse_docs_config() {
        let toml_str = r#"
[docs]
extensions = [".md", ".mdx"]
max_file_size = 524288
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.docs.extensions, vec![".md", ".mdx"]);
        assert_eq!(config.docs.max_file_size, 524_288);
    }
}
