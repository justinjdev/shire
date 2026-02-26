// Stub - will be implemented by Tasks 3, 4, 6
pub mod cargo;
pub mod go;
pub mod manifest;
pub mod npm;
pub mod python;

use crate::config::Config;
use anyhow::Result;
use std::path::Path;

pub fn build_index(_repo_root: &Path, _config: &Config) -> Result<()> {
    todo!("Will be implemented by Task 6")
}
