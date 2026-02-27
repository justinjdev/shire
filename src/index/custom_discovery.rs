use crate::config::CustomDiscoveryRule;
use anyhow::Result;
use std::collections::HashSet;
use std::path::Path;
use walkdir::WalkDir;

use super::manifest::PackageInfo;

/// A discovered package from a custom rule match.
struct CustomMatch {
    relative_dir: String,
    rule_name: String,
    kind: String,
    name_prefix: Option<String>,
}

/// Run all custom discovery rules against the repo, returning PackageInfo entries.
/// Directories already discovered by manifest parsers (in `known_paths`) are skipped.
pub fn discover_custom_packages(
    repo_root: &Path,
    rules: &[CustomDiscoveryRule],
    global_excludes: &HashSet<String>,
    known_paths: &HashSet<String>,
) -> Result<Vec<PackageInfo>> {
    let mut packages = Vec::new();
    let mut all_matched: HashSet<String> = HashSet::new();

    for rule in rules {
        let matches = run_rule(repo_root, rule, global_excludes, known_paths, &all_matched)?;
        for m in matches {
            all_matched.insert(m.relative_dir.clone());

            let name = match &m.name_prefix {
                Some(prefix) if !m.relative_dir.is_empty() => {
                    format!("{}{}", prefix, m.relative_dir)
                }
                Some(prefix) => prefix.trim_end_matches(':').to_string(),
                None if m.relative_dir.is_empty() => m.rule_name.clone(),
                None => m.relative_dir.clone(),
            };

            packages.push(PackageInfo {
                name,
                path: m.relative_dir,
                kind: string_to_static_str(&m.kind),
                version: None,
                description: None,
                metadata: Some(serde_json::json!({"custom_rule": m.rule_name})),
                dependencies: Vec::new(),
            });
        }
    }

    Ok(packages)
}

/// Leak a String into a &'static str. Used to satisfy PackageInfo's &'static str kind.
fn string_to_static_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// Run a single custom discovery rule, returning matched directories.
fn run_rule(
    repo_root: &Path,
    rule: &CustomDiscoveryRule,
    global_excludes: &HashSet<String>,
    known_paths: &HashSet<String>,
    already_matched: &HashSet<String>,
) -> Result<Vec<CustomMatch>> {
    let combined_excludes: HashSet<&str> = global_excludes
        .iter()
        .chain(rule.exclude.iter())
        .map(|s| s.as_str())
        .collect();

    let patterns: Vec<glob::Pattern> = rule
        .requires
        .iter()
        .map(|p| glob::Pattern::new(p))
        .collect::<Result<Vec<_>, _>>()?;

    if patterns.is_empty() {
        return Ok(Vec::new());
    }

    let search_roots: Vec<&str> = if rule.paths.is_empty() {
        vec![""]
    } else {
        rule.paths.iter().map(|s| s.trim_end_matches('/')).collect()
    };

    let mut matches = Vec::new();
    let mut matched_dirs: HashSet<String> = HashSet::new();

    for search_root in &search_roots {
        let abs_root = if search_root.is_empty() {
            repo_root.to_path_buf()
        } else {
            repo_root.join(search_root)
        };

        if !abs_root.is_dir() {
            continue;
        }

        let mut walker = WalkDir::new(&abs_root);
        if let Some(max_d) = rule.max_depth {
            walker = walker.max_depth(max_d);
        }

        for entry in walker.into_iter().filter_entry(|e| {
            if !e.file_type().is_dir() {
                return true;
            }
            let name = e.file_name().to_str().unwrap_or("");
            // Skip hidden dirs (except the root)
            if name.starts_with('.') && e.depth() > 0 {
                return false;
            }
            !combined_excludes.contains(name)
        }) {
            let entry = entry?;
            if !entry.file_type().is_dir() {
                continue;
            }

            let abs_dir = entry.path();
            let relative_dir = abs_dir
                .strip_prefix(repo_root)
                .unwrap_or(abs_dir)
                .to_string_lossy()
                .to_string();

            // Skip if already discovered by manifest parsers
            if known_paths.contains(&relative_dir) {
                continue;
            }

            // Skip if already matched by a previous rule or earlier in this rule
            if already_matched.contains(&relative_dir) || matched_dirs.contains(&relative_dir) {
                continue;
            }

            // Skip if this is a subdirectory of an already-matched dir
            if is_child_of_matched(&relative_dir, &matched_dirs) {
                continue;
            }

            // Check if all requires patterns match at least one file in this directory
            if check_requires(abs_dir, &patterns) {
                matched_dirs.insert(relative_dir.clone());
                matches.push(CustomMatch {
                    relative_dir,
                    rule_name: rule.name.clone(),
                    kind: rule.kind.clone(),
                    name_prefix: rule.name_prefix.clone(),
                });
            }
        }
    }

    Ok(matches)
}

/// Check if `dir` is a child of any directory in `matched_dirs`.
fn is_child_of_matched(dir: &str, matched_dirs: &HashSet<String>) -> bool {
    for matched in matched_dirs {
        if matched.is_empty() {
            return true; // Root matched — everything is a child
        }
        if dir.starts_with(&format!("{}/", matched)) {
            return true;
        }
    }
    false
}

/// Check if ALL require patterns match at least one file in the directory.
fn check_requires(dir: &Path, patterns: &[glob::Pattern]) -> bool {
    let entries: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map_or(false, |ft| ft.is_file()))
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            .collect(),
        Err(_) => return false,
    };

    patterns.iter().all(|pattern| {
        entries.iter().any(|filename| pattern.matches(filename))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_test_repo(dir: &Path) {
        // services/auth with main.go + ownership.yml
        let auth = dir.join("services/auth");
        fs::create_dir_all(&auth).unwrap();
        fs::write(auth.join("main.go"), "package main").unwrap();
        fs::write(auth.join("ownership.yml"), "team: auth").unwrap();

        // services/gateway with main.go + ownership.yml
        let gw = dir.join("services/gateway");
        fs::create_dir_all(&gw).unwrap();
        fs::write(gw.join("main.go"), "package main").unwrap();
        fs::write(gw.join("ownership.yml"), "team: gateway").unwrap();

        // services/gateway/internal — should NOT match (nested)
        let internal = gw.join("internal");
        fs::create_dir_all(&internal).unwrap();
        fs::write(internal.join("main.go"), "package main").unwrap();
        fs::write(internal.join("ownership.yml"), "team: gw").unwrap();

        // libs/shared — has ownership.yml but NO main.go
        let shared = dir.join("libs/shared");
        fs::create_dir_all(&shared).unwrap();
        fs::write(shared.join("ownership.yml"), "team: platform").unwrap();
        fs::write(shared.join("lib.go"), "package shared").unwrap();

        // proto/user — has *.proto + buf.yaml
        let proto = dir.join("proto/user");
        fs::create_dir_all(&proto).unwrap();
        fs::write(proto.join("user.proto"), "syntax = \"proto3\";").unwrap();
        fs::write(proto.join("buf.yaml"), "version: v1").unwrap();

        // vendor/ — should be excluded
        let vendor = dir.join("vendor/pkg");
        fs::create_dir_all(&vendor).unwrap();
        fs::write(vendor.join("main.go"), "package main").unwrap();
        fs::write(vendor.join("ownership.yml"), "team: vendor").unwrap();
    }

    fn go_rule() -> CustomDiscoveryRule {
        CustomDiscoveryRule {
            name: "go-apps".to_string(),
            kind: "go".to_string(),
            requires: vec!["main.go".to_string(), "ownership.yml".to_string()],
            paths: vec!["services/".to_string()],
            exclude: vec![],
            max_depth: Some(3),
            name_prefix: Some("go:".to_string()),
            extensions: None,
        }
    }

    fn proto_rule() -> CustomDiscoveryRule {
        CustomDiscoveryRule {
            name: "proto-packages".to_string(),
            kind: "proto".to_string(),
            requires: vec!["*.proto".to_string(), "buf.yaml".to_string()],
            paths: vec!["proto/".to_string()],
            exclude: vec![],
            max_depth: Some(4),
            name_prefix: None,
            extensions: None,
        }
    }

    #[test]
    fn test_rule_matches_exact_filenames() {
        let dir = tempfile::TempDir::new().unwrap();
        setup_test_repo(dir.path());

        let global_excludes: HashSet<String> =
            ["vendor", "node_modules"].iter().map(|s| s.to_string()).collect();
        let known_paths = HashSet::new();

        let packages = discover_custom_packages(
            dir.path(),
            &[go_rule()],
            &global_excludes,
            &known_paths,
        )
        .unwrap();

        let names: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"go:services/auth"), "got: {:?}", names);
        assert!(names.contains(&"go:services/gateway"), "got: {:?}", names);
        assert_eq!(packages.len(), 2, "got: {:?}", names);
    }

    #[test]
    fn test_rule_matches_glob_patterns() {
        let dir = tempfile::TempDir::new().unwrap();
        setup_test_repo(dir.path());

        let global_excludes = HashSet::new();
        let known_paths = HashSet::new();

        let packages = discover_custom_packages(
            dir.path(),
            &[proto_rule()],
            &global_excludes,
            &known_paths,
        )
        .unwrap();

        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "proto/user");
        assert_eq!(packages[0].kind, "proto");
    }

    #[test]
    fn test_missing_required_files_no_match() {
        let dir = tempfile::TempDir::new().unwrap();
        setup_test_repo(dir.path());

        // libs/shared has ownership.yml but no main.go
        let rule = CustomDiscoveryRule {
            name: "go-apps".to_string(),
            kind: "go".to_string(),
            requires: vec!["main.go".to_string(), "ownership.yml".to_string()],
            paths: vec!["libs/".to_string()],
            exclude: vec![],
            max_depth: Some(3),
            name_prefix: None,
            extensions: None,
        };

        let packages = discover_custom_packages(
            dir.path(),
            &[rule],
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();

        assert!(packages.is_empty());
    }

    #[test]
    fn test_nested_match_prevention() {
        let dir = tempfile::TempDir::new().unwrap();
        setup_test_repo(dir.path());

        // Search all of services/ — gateway should match but gateway/internal should not
        let mut rule = go_rule();
        rule.max_depth = Some(5);

        let packages = discover_custom_packages(
            dir.path(),
            &[rule],
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();

        let paths: Vec<&str> = packages.iter().map(|p| p.path.as_str()).collect();
        assert!(paths.contains(&"services/gateway"), "got: {:?}", paths);
        assert!(
            !paths.contains(&"services/gateway/internal"),
            "nested dir should not match, got: {:?}",
            paths
        );
    }

    #[test]
    fn test_name_prefix_applied() {
        let dir = tempfile::TempDir::new().unwrap();
        setup_test_repo(dir.path());

        let packages = discover_custom_packages(
            dir.path(),
            &[go_rule()],
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();

        for pkg in &packages {
            assert!(
                pkg.name.starts_with("go:"),
                "expected prefix, got: {}",
                pkg.name
            );
        }
    }

    #[test]
    fn test_path_scoping() {
        let dir = tempfile::TempDir::new().unwrap();
        setup_test_repo(dir.path());

        // Put a matching dir outside of services/
        let extra = dir.path().join("extra/app");
        fs::create_dir_all(&extra).unwrap();
        fs::write(extra.join("main.go"), "package main").unwrap();
        fs::write(extra.join("ownership.yml"), "team: extra").unwrap();

        let packages = discover_custom_packages(
            dir.path(),
            &[go_rule()], // paths = ["services/"]
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();

        let paths: Vec<&str> = packages.iter().map(|p| p.path.as_str()).collect();
        assert!(!paths.contains(&"extra/app"), "out-of-scope dir should not match");
    }

    #[test]
    fn test_max_depth_limiting() {
        let dir = tempfile::TempDir::new().unwrap();

        // Create a deeply nested match
        let deep = dir.path().join("services/a/b/c/d/app");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("main.go"), "package main").unwrap();
        fs::write(deep.join("ownership.yml"), "team: deep").unwrap();

        let mut rule = go_rule();
        rule.max_depth = Some(2); // Only search 2 levels deep from services/

        let packages = discover_custom_packages(
            dir.path(),
            &[rule],
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();

        assert!(packages.is_empty(), "deep dir should be beyond max_depth");
    }

    #[test]
    fn test_rule_specific_excludes() {
        let dir = tempfile::TempDir::new().unwrap();

        let auth = dir.path().join("services/auth");
        fs::create_dir_all(&auth).unwrap();
        fs::write(auth.join("main.go"), "package main").unwrap();
        fs::write(auth.join("ownership.yml"), "team: auth").unwrap();

        let testdata = dir.path().join("services/testdata");
        fs::create_dir_all(&testdata).unwrap();
        fs::write(testdata.join("main.go"), "package main").unwrap();
        fs::write(testdata.join("ownership.yml"), "team: test").unwrap();

        let mut rule = go_rule();
        rule.exclude = vec!["testdata".to_string()];

        let packages = discover_custom_packages(
            dir.path(),
            &[rule],
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();

        let paths: Vec<&str> = packages.iter().map(|p| p.path.as_str()).collect();
        assert!(paths.contains(&"services/auth"));
        assert!(!paths.contains(&"services/testdata"));
    }

    #[test]
    fn test_known_paths_skipped() {
        let dir = tempfile::TempDir::new().unwrap();
        setup_test_repo(dir.path());

        let mut known = HashSet::new();
        known.insert("services/auth".to_string());

        let packages = discover_custom_packages(
            dir.path(),
            &[go_rule()],
            &HashSet::new(),
            &known,
        )
        .unwrap();

        let paths: Vec<&str> = packages.iter().map(|p| p.path.as_str()).collect();
        assert!(!paths.contains(&"services/auth"), "known path should be skipped");
        assert!(paths.contains(&"services/gateway"));
    }

    #[test]
    fn test_no_rules_no_discovery() {
        let dir = tempfile::TempDir::new().unwrap();
        setup_test_repo(dir.path());

        let packages = discover_custom_packages(
            dir.path(),
            &[],
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();

        assert!(packages.is_empty());
    }
}
