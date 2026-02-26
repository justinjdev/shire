// Stub - will be implemented by Tasks 3, 4, 6
pub mod manifest;
pub mod npm;

use crate::config::Config;
use anyhow::Result;
use std::path::Path;

pub fn build_index(_repo_root: &Path, _config: &Config) -> Result<()> {
    todo!("Will be implemented by Task 6")
}
