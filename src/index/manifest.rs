use anyhow::Result;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub name: String,
    pub path: String,
    pub kind: &'static str,
    pub version: Option<String>,
    pub description: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub dependencies: Vec<DepInfo>,
}

#[derive(Debug, Clone)]
pub struct DepInfo {
    pub name: String,
    pub version_req: Option<String>,
    pub dep_kind: DepKind,
}

#[derive(Debug, Clone, Copy)]
pub enum DepKind {
    Runtime,
    Dev,
    Peer,
    Build,
}

impl DepKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DepKind::Runtime => "runtime",
            DepKind::Dev => "dev",
            DepKind::Peer => "peer",
            DepKind::Build => "build",
        }
    }
}

pub trait ManifestParser {
    fn filename(&self) -> &'static str;
    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo>;
}

/// Marker for a manifest that legitimately declares no package of its own —
/// a Cargo virtual workspace root (`[workspace]` with no `[package]`), or a
/// Maven aggregator POM (`<packaging>pom</packaging>` with `<modules>`) —
/// as opposed to a genuine parse failure. A parser signals this by
/// returning `Err(NoPackageManifest(..).into())`; callers check for it with
/// [`is_no_package_marker`] to skip the manifest silently instead of
/// reporting a parse failure.
#[derive(Debug)]
pub struct NoPackageManifest(pub String);

impl std::fmt::Display for NoPackageManifest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for NoPackageManifest {}

/// True if `err` is (or wraps) a [`NoPackageManifest`] marker: this "error"
/// is actually an expected, package-less manifest and should be skipped
/// silently rather than reported as a parse failure.
pub fn is_no_package_marker(err: &anyhow::Error) -> bool {
    err.downcast_ref::<NoPackageManifest>().is_some()
}

/// Fallback package name for a manifest format that has no name field of its
/// own (e.g. a Gemfile or cpanfile).
///
/// For a root-level manifest (`relative_dir` is empty) the manifest's own
/// directory carries no useful basename, so the name is derived from the
/// repo root directory itself (the manifest's parent directory), falling
/// back to the literal `"root"` when that can't be determined. For a nested
/// manifest, it mirrors the convention used by the other parsers'
/// no-name fallback: `relative_dir` with path separators replaced by `-`.
pub fn fallback_name(manifest_path: &Path, relative_dir: &str) -> String {
    if relative_dir.is_empty() {
        manifest_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "root".to_string())
    } else {
        relative_dir.replace('/', "-")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_name_nested_dir_replaces_slashes() {
        let path = Path::new("/repo/services/api/Gemfile");
        assert_eq!(fallback_name(path, "services/api"), "services-api");
    }

    #[test]
    fn test_fallback_name_root_uses_parent_dir_name() {
        let path = Path::new("/repo/my-repo/Gemfile");
        assert_eq!(fallback_name(path, ""), "my-repo");
    }

    #[test]
    fn test_fallback_name_root_falls_back_to_root_literal_when_no_parent() {
        // A bare filename with no parent directory component.
        let path = Path::new("Gemfile");
        assert_eq!(fallback_name(path, ""), "root");
    }
}
