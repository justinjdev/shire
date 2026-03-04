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

        let query_text = params.query.as_deref().unwrap_or("");
        if query_text.is_empty() {
            return Ok(fts_results.to_vec());
        }

        // Embed the query
        let query_embeddings = embedder
            .embed(vec![query_text.to_string()])
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

        let limit = params.limit.unwrap_or(20) as usize;
        Ok(queries::rrf_merge(fts_results, &vec_symbols, limit))
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Search query
    pub query: String,
    /// Max results (default 20)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DepsParams {
    /// Package name
    pub name: String,
    /// Only return internal (in-repo) dependencies
    #[serde(default)]
    pub internal_only: bool,
    /// Traversal depth (default: direct only; >1 for transitive)
    pub depth: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DependentsParams {
    /// Package name
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListParams {
    /// Filter by package kind: "npm", "go", "cargo", "python", "maven", "gradle", "perl", "ruby"
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchSymbolsParams {
    /// Search query (omit to list all symbols in a package)
    pub query: Option<String>,
    /// Filter to a specific package
    pub package: Option<String>,
    /// Filter by symbol kind: "function", "class", "struct", "interface", "type", "enum", "trait", "method", "constant"
    pub kind: Option<String>,
    /// Max results (default 20)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetFileSymbolsParams {
    /// File path relative to repo root
    pub file_path: String,
    /// Filter by symbol kind: "function", "class", "struct", "interface", "type", "enum", "trait", "method", "constant"
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListPackageFilesParams {
    /// Package name
    pub package: String,
    /// Filter by file extension
    pub extension: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExploreParams {
    /// Concept to explore (e.g. "authentication", "error handling", "messaging interfaces")
    pub query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExplorePackageParams {
    /// Exact package name
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ImpactAnalysisParams {
    /// Package name to analyze impact for
    pub name: String,
}

#[tool_router]
impl ShireService {
    #[tool(description = "Search packages by name or description")]
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
        let limit = params.limit.unwrap_or(20);
        let results = queries::search_packages(&conn, &params.query, limit)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List a package's dependencies. Set depth>1 for transitive graph (returns edge list with different schema).")]
    fn package_dependencies(
        &self,
        Parameters(params): Parameters<DepsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        match params.depth {
            Some(n) if n > 1 => {
                let depth = n.min(20);
                let edges = queries::dependency_graph(&conn, &params.name, depth, params.internal_only)
                    .map_err(|e| Self::mcp_err(e.to_string()))?;
                let json = serde_json::to_string(&edges)
                    .map_err(|e| Self::mcp_err(e.to_string()))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            _ => {
                let results = queries::package_dependencies(&conn, &params.name, params.internal_only)
                    .map_err(|e| Self::mcp_err(e.to_string()))?;
                let json = serde_json::to_string(&results)
                    .map_err(|e| Self::mcp_err(e.to_string()))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
        }
    }

    #[tool(description = "Find all packages that depend on this package")]
    fn package_dependents(
        &self,
        Parameters(params): Parameters<DependentsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::package_dependents(&conn, &params.name)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List all indexed packages, optionally filtered by kind")]
    fn list_packages(
        &self,
        Parameters(params): Parameters<ListParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::list_packages(&conn, params.kind.as_deref())
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Search symbols by name or signature. Omit query with a package filter to list all symbols in that package.")]
    fn search_symbols(
        &self,
        Parameters(params): Parameters<SearchSymbolsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let limit = params.limit.unwrap_or(20);
        let query = params.query.as_deref().unwrap_or("").trim();
        if query.is_empty() {
            // No query: list all symbols in a package
            let pkg = match &params.package {
                Some(p) => p,
                None => return Ok(CallToolResult::success(vec![Content::text(
                    "Provide a query or a package filter",
                )])),
            };
            let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
            let results = queries::get_package_symbols(&conn, pkg, params.kind.as_deref())
                .map_err(|e| Self::mcp_err(e.to_string()))?;
            let json = serde_json::to_string(&results)
                .map_err(|e| Self::mcp_err(e.to_string()))?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;

        let fts_results = queries::search_symbols(
            &conn,
            query,
            params.package.as_deref(),
            params.kind.as_deref(),
            limit,
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;

        #[cfg(feature = "rag")]
        let results = if let Some(ref embedder) = self.rag_embedder {
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

        let json = serde_json::to_string(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List all symbols defined in a specific file")]
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
        let json = serde_json::to_string(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List all files in a package, optionally filtered by extension")]
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
        let json = serde_json::to_string(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Index build metadata: timestamp, git commit, counts")]
    fn index_status(&self) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let status = queries::index_status(&conn)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&status)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Semantic codebase exploration — search packages, symbols, and files for a concept. Returns a structured context map organized by package. Faster than Grep for broad searches.")]
    fn explore(
        &self,
        Parameters(params): Parameters<ExploreParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let mut args = std::collections::HashMap::new();
        args.insert("query".into(), params.query);
        let text = crate::mcp::prompts::call_prompt(&conn, "explore", &args)
            .map_err(|e| Self::mcp_err(e))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Deep dive into a specific package — metadata, dependencies, dependents, public API surface, and file tree")]
    fn explore_package(
        &self,
        Parameters(params): Parameters<ExplorePackageParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let mut args = std::collections::HashMap::new();
        args.insert("name".into(), params.name);
        let text = crate::mcp::prompts::call_prompt(&conn, "explore-package", &args)
            .map_err(|e| Self::mcp_err(e))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Analyze blast radius — what breaks if this package changes? Shows direct and transitive dependents")]
    fn impact_analysis(
        &self,
        Parameters(params): Parameters<ImpactAnalysisParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let mut args = std::collections::HashMap::new();
        args.insert("name".into(), params.name);
        let text = crate::mcp::prompts::call_prompt(&conn, "impact-analysis", &args)
            .map_err(|e| Self::mcp_err(e))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
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
