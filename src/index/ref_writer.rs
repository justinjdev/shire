use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;

use crate::symbols::ReferenceInfo;

/// Strategy for handling cross-references during index builds.
///
/// Replaces the `references_enabled: bool` parameter that was threaded
/// through `phase_extract_symbols`, `phase_source_incremental`,
/// `single_pass_extract`, and per-file closures.
pub enum RefWriter {
    /// References disabled — skip extraction and DB writes.
    Disabled,
    /// References enabled — carries the file_path→file_id map for DB inserts.
    Enabled { file_ids: HashMap<String, i64> },
}

impl RefWriter {
    /// Create from config + DB state.
    pub fn new(conn: &Connection, enabled: bool) -> Result<Self> {
        if enabled {
            let file_ids = crate::db::queries::build_file_id_map(conn)?;
            Ok(Self::Enabled { file_ids })
        } else {
            Ok(Self::Disabled)
        }
    }

    /// Whether to skip reference extraction at the symbol layer.
    pub fn skip_references(&self) -> bool {
        matches!(self, Self::Disabled)
    }

    /// Insert references for a package. No-op when disabled.
    pub fn insert(
        &mut self,
        conn: &Connection,
        package: Option<&str>,
        refs: &[ReferenceInfo],
    ) -> Result<()> {
        if let Self::Enabled { file_ids } = self {
            crate::db::queries::batch_insert_references(conn, package, refs, file_ids)?;
        }
        Ok(())
    }
}
