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
