use super::manifest::{DepInfo, DepKind, ManifestParser, PackageInfo};
use anyhow::Result;
use regex::Regex;
use std::path::Path;

pub struct RubyParser;

impl ManifestParser for RubyParser {
    fn filename(&self) -> &'static str {
        "Gemfile"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        let content = std::fs::read_to_string(manifest_path)?;
        let dependencies = parse_gemfile(&content);

        Ok(PackageInfo {
            name: relative_dir.to_string(),
            path: relative_dir.to_string(),
            kind: "ruby",
            version: None,
            description: None,
            metadata: None,
            dependencies,
        })
    }
}

/// Parse a Gemfile and extract gem dependencies.
///
/// Handles top-level `gem` declarations as Runtime deps and gems inside
/// `group :test do ... end` or `group :development do ... end` blocks as Dev deps.
fn parse_gemfile(content: &str) -> Vec<DepInfo> {
    let gem_re = Regex::new(r#"^\s*gem\s+['"]([^'"]+)['"](?:\s*,\s*['"]([^'"]+)['"])?"#).unwrap();
    let group_start_re =
        Regex::new(r#"^\s*group\s+(?::(?:test|development)\b|.*:(?:test|development)\b)"#).unwrap();
    let end_re = Regex::new(r"^\s*end\b").unwrap();

    let mut deps = Vec::new();
    let mut in_dev_group = false;
    let mut group_depth = 0;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments and blank lines
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if in_dev_group {
            if end_re.is_match(line) {
                group_depth -= 1;
                if group_depth == 0 {
                    in_dev_group = false;
                }
                continue;
            }

            // Track nested blocks within the group
            if trimmed.ends_with(" do") || trimmed.ends_with(" do |") {
                group_depth += 1;
            }

            if let Some(caps) = gem_re.captures(line) {
                let name = caps.get(1).unwrap().as_str().to_string();
                let version_req = caps.get(2).map(|m| m.as_str().to_string());
                deps.push(DepInfo {
                    name,
                    version_req,
                    dep_kind: DepKind::Dev,
                });
            }
            continue;
        }

        if group_start_re.is_match(line) {
            in_dev_group = true;
            group_depth = 1;
            continue;
        }

        if let Some(caps) = gem_re.captures(line) {
            let name = caps.get(1).unwrap().as_str().to_string();
            let version_req = caps.get(2).map(|m| m.as_str().to_string());
            deps.push(DepInfo {
                name,
                version_req,
                dep_kind: DepKind::Runtime,
            });
        }
        // Silently skip unparseable lines
    }

    deps
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_manifest(dir: &std::path::Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("Gemfile");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_parse_simple_gemfile() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
source 'https://rubygems.org'

gem 'rails', '~> 7.0'
gem 'pg', '~> 1.4'
gem 'puma', '~> 6.0'
"#,
        );

        let parser = RubyParser;
        let info = parser.parse(&path, "services/api").unwrap();

        assert_eq!(info.name, "services/api");
        assert_eq!(info.kind, "ruby");
        assert_eq!(info.path, "services/api");
        assert_eq!(info.version, None);
        assert_eq!(info.dependencies.len(), 3);

        let names: Vec<&str> = info.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"rails"));
        assert!(names.contains(&"pg"));
        assert!(names.contains(&"puma"));

        assert!(info
            .dependencies
            .iter()
            .all(|d| matches!(d.dep_kind, DepKind::Runtime)));
    }

    #[test]
    fn test_parse_versioned_deps() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
gem 'rails', '~> 7.0'
gem 'sidekiq', '>= 6.0'
"#,
        );

        let parser = RubyParser;
        let info = parser.parse(&path, "app").unwrap();

        let rails = info
            .dependencies
            .iter()
            .find(|d| d.name == "rails")
            .unwrap();
        assert_eq!(rails.version_req.as_deref(), Some("~> 7.0"));

        let sidekiq = info
            .dependencies
            .iter()
            .find(|d| d.name == "sidekiq")
            .unwrap();
        assert_eq!(sidekiq.version_req.as_deref(), Some(">= 6.0"));
    }

    #[test]
    fn test_parse_unversioned_deps() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
gem 'bootsnap'
gem 'dotenv'
"#,
        );

        let parser = RubyParser;
        let info = parser.parse(&path, "app").unwrap();

        assert_eq!(info.dependencies.len(), 2);
        assert!(info.dependencies.iter().all(|d| d.version_req.is_none()));

        let bootsnap = info
            .dependencies
            .iter()
            .find(|d| d.name == "bootsnap")
            .unwrap();
        assert!(matches!(bootsnap.dep_kind, DepKind::Runtime));
    }

    #[test]
    fn test_parse_group_deps() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
source 'https://rubygems.org'

gem 'rails', '~> 7.0'

group :development do
  gem 'pry'
  gem 'better_errors', '~> 2.9'
end

group :test do
  gem 'rspec-rails', '~> 6.0'
  gem 'factory_bot_rails'
end

gem 'puma', '~> 6.0'
"#,
        );

        let parser = RubyParser;
        let info = parser.parse(&path, "services/web").unwrap();

        let runtime: Vec<&DepInfo> = info
            .dependencies
            .iter()
            .filter(|d| matches!(d.dep_kind, DepKind::Runtime))
            .collect();
        assert_eq!(runtime.len(), 2);
        assert!(runtime.iter().any(|d| d.name == "rails"));
        assert!(runtime.iter().any(|d| d.name == "puma"));

        let dev: Vec<&DepInfo> = info
            .dependencies
            .iter()
            .filter(|d| matches!(d.dep_kind, DepKind::Dev))
            .collect();
        assert_eq!(dev.len(), 4);
        assert!(dev.iter().any(|d| d.name == "pry"));
        assert!(dev.iter().any(|d| d.name == "better_errors"));
        assert!(dev.iter().any(|d| d.name == "rspec-rails"));
        assert!(dev.iter().any(|d| d.name == "factory_bot_rails"));

        // Verify version on dev dep
        let rspec = dev.iter().find(|d| d.name == "rspec-rails").unwrap();
        assert_eq!(rspec.version_req.as_deref(), Some("~> 6.0"));
    }

    #[test]
    fn test_skip_unparseable_lines() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"
source 'https://rubygems.org'
ruby '3.2.0'

# This is a comment
gem 'rails', '~> 7.0'

git_source(:github) { |repo| "https://github.com/#{repo}.git" }

gem 'puma'
"#,
        );

        let parser = RubyParser;
        let info = parser.parse(&path, "app").unwrap();

        // Only the two gem lines should be parsed
        assert_eq!(info.dependencies.len(), 2);
        let names: Vec<&str> = info.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"rails"));
        assert!(names.contains(&"puma"));
    }
}
