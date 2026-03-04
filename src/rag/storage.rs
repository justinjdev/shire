use anyhow::Result;
use rusqlite::Connection;
use zerocopy::IntoBytes;

/// Embedding dimension for BAAI/bge-small-en-v1.5. Must match the model output.
pub const EMBEDDING_DIM: usize = 384;

/// Register the sqlite-vec extension globally so all future connections load it.
pub fn load_extension() -> Result<()> {
    // SAFETY: sqlite3_vec_init has the signature expected by sqlite3_auto_extension
    // (i.e. fn(db, err_msg, api) -> c_int). The transmute casts the typed function
    // pointer to the generic Option<unsafe extern "C" fn()> that the FFI expects.
    let rc = unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )))
    };
    if rc != 0 {
        anyhow::bail!(
            "Failed to register sqlite-vec extension (SQLite error code: {rc})"
        );
    }
    Ok(())
}

/// Create the vec0 virtual table for symbol embeddings (384-dim, cosine distance)
/// if it doesn't exist.
pub fn init_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS symbol_embeddings USING vec0(
            symbol_id INTEGER PRIMARY KEY,
            embedding float[{EMBEDDING_DIM}] distance_metric=cosine
        );"
    ))?;
    Ok(())
}

/// Insert embeddings within a transaction using a prepared statement loop.
/// Each entry is a (symbol_id, embedding_vector) pair.
pub fn insert_embeddings(conn: &Connection, embeddings: &[(i64, Vec<f32>)]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO symbol_embeddings (symbol_id, embedding) VALUES (?1, ?2)",
        )?;
        for (symbol_id, embedding) in embeddings {
            if embedding.len() != EMBEDDING_DIM {
                anyhow::bail!(
                    "Embedding dimension mismatch for symbol {symbol_id}: expected {EMBEDDING_DIM}, got {}",
                    embedding.len()
                );
            }
            let bytes: &[u8] = embedding.as_slice().as_bytes();
            stmt.execute(rusqlite::params![symbol_id, bytes])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Delete embeddings for the given symbol IDs.
pub fn delete_embeddings_for_symbols(conn: &Connection, symbol_ids: &[i64]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare("DELETE FROM symbol_embeddings WHERE symbol_id = ?1")?;
        for id in symbol_ids {
            stmt.execute([id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Search for the most similar embeddings using KNN cosine distance.
/// Returns a list of (symbol_id, distance) pairs ordered by distance ascending (most similar first).
pub fn search_similar(
    conn: &Connection,
    query_embedding: &[f32],
    limit: usize,
) -> Result<Vec<(i64, f64)>> {
    let bytes: &[u8] = query_embedding.as_bytes();
    let mut stmt = conn.prepare(
        "SELECT symbol_id, distance FROM symbol_embeddings
         WHERE embedding MATCH ?1
         ORDER BY distance
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![bytes, limit], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
