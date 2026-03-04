use crate::db::queries;
use crate::mcp::BuildContext;
use rmcp::{
    handler::server::{router::tool::ToolRouter, tool::Parameters},
    model::*,
    schemars, tool, tool_router,
};
use rusqlite::Connection;
use serde::Deserialize;
use std::borrow::Cow;
use std::sync::Mutex;
use std::time::SystemTime;

pub struct ShireService {
    pub(crate) conn: Mutex<Connection>,
    pub tool_router: ToolRouter<ShireService>,
    build_ctx: Option<BuildContext>,
    last_indexed: Mutex<Option<SystemTime>>,
    #[cfg(feature = "rag")]
    rag_embedder: Option<crate::rag::embedder::Embedder>,
}

impl std::fmt::Debug for ShireService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("ShireService");
        d.field("conn", &self.conn);
        d.field("tool_router", &self.tool_router);
        d.field("build_ctx", &self.build_ctx.as_ref().map(|c| &c.repo_root));
        #[cfg(feature = "rag")]
        d.field("rag_embedder", &self.rag_embedder.as_ref().map(|_| "Embedder(...)"));
        d.finish()
    }
}

impl ShireService {
    pub fn new(conn: Connection, rag_config: &crate::config::RagConfig, build_ctx: Option<BuildContext>) -> Self {
        #[cfg(feature = "rag")]
        let rag_embedder = if rag_config.enabled {
            match crate::rag::embedder::Embedder::new(rag_config) {
                Ok(e) => {
                    // Verify vector table exists before enabling hybrid search
                    let table_exists = conn
                        .prepare("SELECT 1 FROM symbol_embeddings LIMIT 0")
                        .is_ok();
                    if !table_exists {
                        eprintln!(
                            "Warning: RAG enabled but symbol_embeddings table not found. \
                             Run `shire build` with [rag] enabled to generate embeddings."
                        );
                        None
                    } else {
                        Some(e)
                    }
                }
                Err(err) => {
                    eprintln!("Warning: RAG embedder init failed: {err}");
                    None
                }
            }
        } else {
            None
        };

        #[cfg(not(feature = "rag"))]
        let _ = rag_config;

        // Initialize last_indexed from DB metadata if available
        let last_indexed = Self::read_indexed_at(&conn);

        Self {
            conn: Mutex::new(conn),
            tool_router: Self::tool_router(),
            build_ctx,
            last_indexed: Mutex::new(last_indexed),
            #[cfg(feature = "rag")]
            rag_embedder,
        }
    }

    /// Read indexed_at from shire_meta and parse to SystemTime.
    fn read_indexed_at(conn: &Connection) -> Option<SystemTime> {
        let ts: String = conn
            .query_row(
                "SELECT value FROM shire_meta WHERE key = 'indexed_at'",
                [],
                |row| row.get(0),
            )
            .ok()?;
        let dt = chrono::DateTime::parse_from_rfc3339(&ts).ok()?;
        Some(SystemTime::from(dt))
    }

    /// Check if the index is stale by comparing .git/index mtime against last_indexed.
    fn is_stale(&self) -> bool {
        let ctx = match &self.build_ctx {
            Some(c) => c,
            None => return false, // read-only mode
        };

        let last = self.last_indexed.lock().ok().and_then(|g| *g);

        // No existing index — definitely stale
        if last.is_none() {
            return true;
        }
        let last = last.unwrap();

        // Check .git/index mtime
        let git_index = ctx.repo_root.join(".git/index");
        match std::fs::metadata(&git_index) {
            Ok(meta) => match meta.modified() {
                Ok(mtime) => mtime > last,
                Err(_) => false,
            },
            Err(_) => false, // no .git/index — can't determine staleness
        }
    }

    /// Rebuild the index if stale. No-op in read-only mode.
    fn maybe_rebuild(&self) {
        if !self.is_stale() {
            return;
        }

        let ctx = match &self.build_ctx {
            Some(c) => c.clone(),
            None => return,
        };

        eprintln!("[shire] Rebuilding index…");

        match crate::index::build_index_quiet(&ctx.repo_root, &ctx.config, false, Some(&ctx.db_path)) {
            Ok(()) => {
                // Reopen connection read-only
                match crate::db::open_readonly(&ctx.db_path) {
                    Ok(new_conn) => {
                        match self.conn.lock() {
                            Ok(mut conn) => {
                                let now = Self::read_indexed_at(&new_conn)
                                    .or_else(|| Some(SystemTime::now()));
                                *conn = new_conn;
                                if let Ok(mut li) = self.last_indexed.lock() {
                                    *li = now;
                                }
                                eprintln!("[shire] Index rebuilt");
                            }
                            Err(e) => eprintln!("[shire] Warning: index rebuilt but failed to swap connection: {e}"),
                        }
                    }
                    Err(e) => {
                        // Prevent infinite rebuild loop: mark as indexed even if reopen fails
                        if let Ok(mut li) = self.last_indexed.lock() {
                            *li = Some(SystemTime::now());
                        }
                        eprintln!("[shire] Warning: failed to reopen index after rebuild: {e}");
                    }
                }
            }
            Err(e) => eprintln!("[shire] Warning: rebuild failed: {e}"),
        }
    }

    pub(crate) fn mcp_err(msg: String) -> ErrorData {
        ErrorData {
            code: ErrorCode(-32603),
            message: Cow::from(msg),
            data: None,
        }
    }

    /// Merge FTS5 and vector search results using Reciprocal Rank Fusion (RRF).
    #[cfg(feature = "rag")]
    fn hybrid_search(
        conn: &Connection,
        embedder: &crate::rag::embedder::Embedder,
        params: &SearchSymbolsParams,
        fts_results: &[queries::SymbolRow],
    ) -> Result<Vec<queries::SymbolRow>, ErrorData> {
        use std::collections::HashMap;

        // Embed the query
        let query_embeddings = embedder
            .embed(vec![params.query.clone()])
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let query_vec = query_embeddings
            .into_iter()
            .next()
            .ok_or_else(|| Self::mcp_err("No embedding returned".into()))?;

        // Vector search
        let vec_results = crate::rag::storage::search_similar(conn, &query_vec, 50)
            .map_err(|e| Self::mcp_err(e.to_string()))?;

        if vec_results.is_empty() {
            return Ok(fts_results.to_vec());
        }

        // Fetch symbol rows for vector results; rank order preserved by iterating vec_results below
        let vec_ids: Vec<i64> = vec_results.iter().map(|(id, _)| *id).collect();
        let id_symbol_pairs = queries::get_symbols_by_ids(conn, &vec_ids)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let id_to_symbol: HashMap<i64, queries::SymbolRow> =
            id_symbol_pairs.into_iter().collect();

        // Build filtered vector result list in rank order
        let vec_symbols: Vec<queries::SymbolRow> = vec_results
            .iter()
            .filter_map(|(symbol_id, _)| {
                let sym = id_to_symbol.get(symbol_id)?;
                if let Some(ref pkg) = params.package {
                    if sym.package != *pkg {
                        return None;
                    }
                }
                if let Some(ref kind) = params.kind {
                    if sym.kind != *kind {
                        return None;
                    }
                }
                Some(sym.clone())
            })
            .collect();

        Ok(queries::rrf_merge(fts_results, &vec_symbols, 50))
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Search query to find packages by name or description
    pub query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetPackageParams {
    /// Exact package name
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DepsParams {
    /// Package name to look up dependencies for
    pub name: String,
    /// If true, only return dependencies that are also packages in this repo
    #[serde(default)]
    pub internal_only: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DependentsParams {
    /// Package name to find dependents of
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GraphParams {
    /// Root package to start the graph from
    pub name: String,
    /// Maximum depth to traverse (default 3)
    #[serde(default = "default_depth")]
    pub depth: u32,
    /// If true, only follow internal dependencies
    #[serde(default)]
    pub internal_only: bool,
}

fn default_depth() -> u32 {
    3
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListParams {
    /// Filter by package kind: "npm", "go", "cargo", "python"
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchSymbolsParams {
    /// Search query to find symbols by name or signature
    pub query: String,
    /// Filter to symbols from a specific package
    pub package: Option<String>,
    /// Filter by symbol kind: "function", "class", "struct", "interface", "type", "enum", "trait", "method", "constant"
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetPackageSymbolsParams {
    /// Exact package name to get symbols for
    pub package: String,
    /// Filter by symbol kind: "function", "class", "struct", "interface", "type", "enum", "trait", "method", "constant"
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetSymbolParams {
    /// Exact symbol name to look up
    pub name: String,
    /// Filter to a specific package
    pub package: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetFileSymbolsParams {
    /// File path relative to repo root (e.g., "services/auth/src/auth.ts")
    pub file_path: String,
    /// Filter by symbol kind: "function", "class", "struct", "interface", "type", "enum", "trait", "method", "constant"
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchFilesParams {
    /// Search query to find files by path or name
    pub query: String,
    /// Filter to files from a specific package
    pub package: Option<String>,
    /// Filter by file extension (e.g., "ts", "go", "rs")
    pub extension: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListPackageFilesParams {
    /// Exact package name to list files for
    pub package: String,
    /// Filter by file extension (e.g., "ts", "go", "rs")
    pub extension: Option<String>,
}

#[tool_router]
impl ShireService {
    #[tool(description = "Search packages by name or description using full-text search")]
    fn search_packages(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        if params.query.trim().is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "Search query must not be empty",
            )]));
        }
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::search_packages(&conn, &params.query)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Get full details for a specific package by exact name")]
    fn get_package(
        &self,
        Parameters(params): Parameters<GetPackageParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let result = queries::get_package(&conn, &params.name)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        match result {
            Some(pkg) => {
                let json = serde_json::to_string_pretty(&pkg)
                    .map_err(|e| Self::mcp_err(e.to_string()))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "Package '{}' not found",
                params.name
            ))])),
        }
    }

    #[tool(description = "List what a package depends on. Set internal_only=true to see only dependencies that are other packages in this repo.")]
    fn package_dependencies(
        &self,
        Parameters(params): Parameters<DepsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::package_dependencies(&conn, &params.name, params.internal_only)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Find all packages that depend on this package (reverse dependency lookup)")]
    fn package_dependents(
        &self,
        Parameters(params): Parameters<DependentsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::package_dependents(&conn, &params.name)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Get the transitive dependency graph starting from a package. Returns a list of edges. Set internal_only=true to only follow dependencies within this repo.")]
    fn dependency_graph(
        &self,
        Parameters(mut params): Parameters<GraphParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        params.depth = params.depth.min(20);
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let edges = queries::dependency_graph(&conn, &params.name, params.depth, params.internal_only)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&edges)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List all indexed packages, optionally filtered by kind (npm, go, cargo, python)")]
    fn list_packages(
        &self,
        Parameters(params): Parameters<ListParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::list_packages(&conn, params.kind.as_deref())
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Search symbols (functions, classes, types, etc.) by name or signature using full-text search. Returns matching symbols with file location, signature, parameters, and return type.")]
    fn search_symbols(
        &self,
        Parameters(params): Parameters<SearchSymbolsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        if params.query.trim().is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "Search query must not be empty",
            )]));
        }
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;

        let fts_results = queries::search_symbols(
            &conn,
            &params.query,
            params.package.as_deref(),
            params.kind.as_deref(),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;

        #[cfg(feature = "rag")]
        let results = if let Some(ref embedder) = self.rag_embedder {
            // Falls back to FTS-only on hybrid search error
            match Self::hybrid_search(&conn, embedder, &params, &fts_results) {
                Ok(merged) => merged,
                Err(e) => {
                    eprintln!("Warning: hybrid search failed, falling back to FTS-only: {}", e.message);
                    fts_results
                }
            }
        } else {
            fts_results
        };

        #[cfg(not(feature = "rag"))]
        let results = fts_results;

        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List all symbols in a package. Useful for understanding a package's public API — its exported functions, classes, types, and methods.")]
    fn get_package_symbols(
        &self,
        Parameters(params): Parameters<GetPackageSymbolsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::get_package_symbols(
            &conn,
            &params.package,
            params.kind.as_deref(),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Get details for a specific symbol by exact name. Returns all symbols matching that name across packages, with file location, signature, parameters, and return type.")]
    fn get_symbol(
        &self,
        Parameters(params): Parameters<GetSymbolParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::get_symbol(
            &conn,
            &params.name,
            params.package.as_deref(),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List all symbols defined in a specific file. Useful for understanding what a file exports — its functions, classes, types, and methods.")]
    fn get_file_symbols(
        &self,
        Parameters(params): Parameters<GetFileSymbolsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::get_file_symbols(
            &conn,
            &params.file_path,
            params.kind.as_deref(),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Search files by path or name using full-text search. Useful for finding files like 'middleware', 'proto files', or files in a specific directory.")]
    fn search_files(
        &self,
        Parameters(params): Parameters<SearchFilesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        if params.query.trim().is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "Search query must not be empty",
            )]));
        }
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::search_files(
            &conn,
            &params.query,
            params.package.as_deref(),
            params.extension.as_deref(),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List all files belonging to a specific package. Optionally filter by file extension.")]
    fn list_package_files(
        &self,
        Parameters(params): Parameters<ListPackageFilesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::list_package_files(
            &conn,
            &params.package,
            params.extension.as_deref(),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Get index status: when it was built, git commit, package/symbol/file counts, and build duration in milliseconds")]
    fn index_status(&self) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let status = queries::index_status(&conn)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&status)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn default_rag_config() -> crate::config::RagConfig {
        crate::config::RagConfig::default()
    }

    fn make_service_readonly() -> ShireService {
        let conn = Connection::open_in_memory().unwrap();
        ShireService::new(conn, &default_rag_config(), None)
    }

    fn make_service_with_ctx(repo_root: std::path::PathBuf) -> ShireService {
        let db_path = repo_root.join(".shire/index.db");
        let conn = Connection::open_in_memory().unwrap();
        let build_ctx = BuildContext {
            repo_root,
            config: crate::config::Config::default(),
            db_path,
        };
        ShireService::new(conn, &default_rag_config(), Some(build_ctx))
    }

    #[test]
    fn test_is_stale_false_when_readonly() {
        let svc = make_service_readonly();
        assert!(!svc.is_stale(), "read-only mode should never be stale");
    }

    #[test]
    fn test_is_stale_true_when_no_last_indexed() {
        // build_ctx present but last_indexed is None (no DB yet) → stale
        let dir = tempfile::TempDir::new().unwrap();
        let svc = make_service_with_ctx(dir.path().to_path_buf());
        // last_indexed is None because in-memory DB has no shire_meta
        assert!(svc.is_stale(), "should be stale when no last_indexed");
    }

    #[test]
    fn test_is_stale_false_when_no_git_index() {
        let dir = tempfile::TempDir::new().unwrap();
        // No .git/index in temp dir
        let svc = make_service_with_ctx(dir.path().to_path_buf());
        // Set last_indexed to some time so we skip the "no DB" early return
        *svc.last_indexed.lock().unwrap() = Some(SystemTime::now());
        assert!(!svc.is_stale(), "should not be stale when no .git/index");
    }

    #[test]
    fn test_is_stale_false_when_git_index_older() {
        let dir = tempfile::TempDir::new().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        let git_index = git_dir.join("index");
        std::fs::write(&git_index, "dummy").unwrap();
        // Set last_indexed to future so git index is "older"
        let svc = make_service_with_ctx(dir.path().to_path_buf());
        *svc.last_indexed.lock().unwrap() = Some(SystemTime::now() + Duration::from_secs(60));
        assert!(!svc.is_stale(), "should not be stale when .git/index is older than last_indexed");
    }

    #[test]
    fn test_is_stale_true_when_git_index_newer() {
        let dir = tempfile::TempDir::new().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        let git_index = git_dir.join("index");
        std::fs::write(&git_index, "dummy").unwrap();
        // Set last_indexed to past so git index is "newer"
        let svc = make_service_with_ctx(dir.path().to_path_buf());
        *svc.last_indexed.lock().unwrap() = Some(SystemTime::now() - Duration::from_secs(60));
        assert!(svc.is_stale(), "should be stale when .git/index is newer than last_indexed");
    }

    #[test]
    fn test_maybe_rebuild_noop_when_readonly() {
        let svc = make_service_readonly();
        // Should not panic or error — just a no-op
        svc.maybe_rebuild();
    }

    #[test]
    fn test_read_indexed_at_parses_db_timestamp() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE shire_meta (key TEXT PRIMARY KEY, value TEXT);",
        )
        .unwrap();
        let ts = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO shire_meta (key, value) VALUES ('indexed_at', ?1)",
            [&ts],
        )
        .unwrap();
        let result = ShireService::read_indexed_at(&conn);
        assert!(result.is_some(), "should parse indexed_at from shire_meta");
    }

    #[test]
    fn test_read_indexed_at_none_when_no_table() {
        let conn = Connection::open_in_memory().unwrap();
        let result = ShireService::read_indexed_at(&conn);
        assert!(result.is_none(), "should return None when shire_meta doesn't exist");
    }
}
