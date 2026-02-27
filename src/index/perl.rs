use super::manifest::{DepInfo, DepKind, ManifestParser, PackageInfo};
use anyhow::Result;
use regex::Regex;
use std::path::Path;

pub struct CpanfileParser;

impl ManifestParser for CpanfileParser {
    fn filename(&self) -> &'static str {
        "cpanfile"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        let content = std::fs::read_to_string(manifest_path)?;

        let mut dependencies = Vec::new();
        parse_cpanfile(&content, &mut dependencies);

        Ok(PackageInfo {
            name: relative_dir.to_string(),
            path: relative_dir.to_string(),
            kind: "perl",
            version: None,
            description: None,
            metadata: None,
            dependencies,
        })
    }
}

/// Parse cpanfile content, extracting requires directives and on 'test' blocks.
fn parse_cpanfile(content: &str, out: &mut Vec<DepInfo>) {
    let requires_re =
        Regex::new(r#"^\s*requires\s+'([^']+)'(?:\s*,\s*'([^']*)')?\s*;"#).unwrap();
    let test_block_start_re = Regex::new(r#"^\s*on\s+'test'\s*=>\s*sub\s*\{"#).unwrap();

    let mut in_test_block = false;
    let mut brace_depth: usize = 0;

    for line in content.lines() {
        if in_test_block {
            // Track brace depth to detect end of block
            for ch in line.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => {
                        if brace_depth > 0 {
                            brace_depth -= 1;
                        }
                        if brace_depth == 0 {
                            in_test_block = false;
                        }
                    }
                    _ => {}
                }
            }

            if let Some(caps) = requires_re.captures(line) {
                let name = caps[1].to_string();
                let version_req = caps.get(2).map(|m| m.as_str().to_string());
                out.push(DepInfo {
                    name,
                    version_req,
                    dep_kind: DepKind::Dev,
                });
            }
        } else if test_block_start_re.is_match(line) {
            in_test_block = true;
            brace_depth = 0;
            // Count braces on the opening line itself
            for ch in line.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => {
                        if brace_depth > 0 {
                            brace_depth -= 1;
                        }
                        if brace_depth == 0 {
                            in_test_block = false;
                        }
                    }
                    _ => {}
                }
            }

            // Check if the opening line itself has a requires
            if let Some(caps) = requires_re.captures(line) {
                let name = caps[1].to_string();
                let version_req = caps.get(2).map(|m| m.as_str().to_string());
                out.push(DepInfo {
                    name,
                    version_req,
                    dep_kind: DepKind::Dev,
                });
            }
        } else if let Some(caps) = requires_re.captures(line) {
            let name = caps[1].to_string();
            let version_req = caps.get(2).map(|m| m.as_str().to_string());
            out.push(DepInfo {
                name,
                version_req,
                dep_kind: DepKind::Runtime,
            });
        }
        // Silently skip unparseable lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_manifest(dir: &std::path::Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("cpanfile");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_parse_simple_deps() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            "requires 'Moose', '2.2009';\nrequires 'DBI', '1.643';\n",
        );

        let parser = CpanfileParser;
        let info = parser.parse(&path, "lib/myapp").unwrap();

        assert_eq!(info.name, "lib/myapp");
        assert_eq!(info.kind, "perl");
        assert_eq!(info.path, "lib/myapp");
        assert_eq!(info.dependencies.len(), 2);

        assert_eq!(info.dependencies[0].name, "Moose");
        assert_eq!(info.dependencies[0].version_req.as_deref(), Some("2.2009"));
        assert!(matches!(info.dependencies[0].dep_kind, DepKind::Runtime));

        assert_eq!(info.dependencies[1].name, "DBI");
        assert_eq!(info.dependencies[1].version_req.as_deref(), Some("1.643"));
        assert!(matches!(info.dependencies[1].dep_kind, DepKind::Runtime));
    }

    #[test]
    fn test_parse_versioned_deps() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            "requires 'JSON::XS', '4.03';\nrequires 'Try::Tiny', '0.31';\n",
        );

        let parser = CpanfileParser;
        let info = parser.parse(&path, "services/api").unwrap();

        assert_eq!(info.dependencies.len(), 2);
        assert_eq!(info.dependencies[0].name, "JSON::XS");
        assert_eq!(info.dependencies[0].version_req.as_deref(), Some("4.03"));
        assert_eq!(info.dependencies[1].name, "Try::Tiny");
        assert_eq!(info.dependencies[1].version_req.as_deref(), Some("0.31"));
    }

    #[test]
    fn test_parse_unversioned_deps() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            "requires 'File::Slurp';\nrequires 'Data::Dumper';\n",
        );

        let parser = CpanfileParser;
        let info = parser.parse(&path, "scripts").unwrap();

        assert_eq!(info.dependencies.len(), 2);

        assert_eq!(info.dependencies[0].name, "File::Slurp");
        assert!(info.dependencies[0].version_req.is_none());
        assert!(matches!(info.dependencies[0].dep_kind, DepKind::Runtime));

        assert_eq!(info.dependencies[1].name, "Data::Dumper");
        assert!(info.dependencies[1].version_req.is_none());
        assert!(matches!(info.dependencies[1].dep_kind, DepKind::Runtime));
    }

    #[test]
    fn test_parse_test_deps() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"requires 'Moose', '2.2009';

on 'test' => sub {
    requires 'Test::More', '1.302195';
    requires 'Test::Exception';
};
"#,
        );

        let parser = CpanfileParser;
        let info = parser.parse(&path, "lib/myapp").unwrap();

        assert_eq!(info.dependencies.len(), 3);

        // Runtime dep
        assert_eq!(info.dependencies[0].name, "Moose");
        assert!(matches!(info.dependencies[0].dep_kind, DepKind::Runtime));

        // Dev/test deps
        assert_eq!(info.dependencies[1].name, "Test::More");
        assert_eq!(
            info.dependencies[1].version_req.as_deref(),
            Some("1.302195")
        );
        assert!(matches!(info.dependencies[1].dep_kind, DepKind::Dev));

        assert_eq!(info.dependencies[2].name, "Test::Exception");
        assert!(info.dependencies[2].version_req.is_none());
        assert!(matches!(info.dependencies[2].dep_kind, DepKind::Dev));
    }

    #[test]
    fn test_parse_skips_unparseable_lines() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"# This is a comment
requires 'Moose', '2.2009';
some random garbage line
feature 'sqlite', 'SQLite support' => sub {
    requires 'DBD::SQLite';
};
requires 'DBI';
"#,
        );

        let parser = CpanfileParser;
        let info = parser.parse(&path, "lib/myapp").unwrap();

        // Should parse Moose, DBD::SQLite (inside a sub block), and DBI
        // The comment and garbage line are silently skipped
        let names: Vec<&str> = info.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"Moose"));
        assert!(names.contains(&"DBI"));
    }

    #[test]
    fn test_parse_empty_cpanfile() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(dir.path(), "# just a comment\n");

        let parser = CpanfileParser;
        let info = parser.parse(&path, "empty-project").unwrap();

        assert_eq!(info.name, "empty-project");
        assert_eq!(info.kind, "perl");
        assert!(info.dependencies.is_empty());
    }
}
