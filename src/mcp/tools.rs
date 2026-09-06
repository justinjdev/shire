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
    /// Serializes on-demand rebuilds. Tool calls arrive concurrently, and
    /// without this every one of them saw `is_stale() == true` and started
    /// its own `build_index_quiet` against the same SQLite file: the losers
    /// hit "database is locked", never swapped their connection, and
    /// answered with a bare -32603. Holders re-check staleness under the
    /// guard, so waiters see the winner's fresh index instead of rebuilding.
    rebuild_lock: Mutex<()>,
    /// When the last rebuild attempt failed. `last_indexed` is deliberately
    /// left alone on failure so a transient error is retried, but without
    /// this every waiter in the same burst would run its own full build
    /// while the failure persists.
    last_rebuild_failure: Mutex<Option<SystemTime>>,
    /// Number of index rebuilds this process has actually run. Used by the
    /// concurrency test to assert that N racing tool calls produce one build.
    rebuild_count: std::sync::atomic::AtomicU64,
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
        d.field(
            "rag_embedder",
            &self.rag_embedder.as_ref().map(|_| "Embedder(...)"),
        );
        d.finish()
    }
}

impl ShireService {
    pub fn new(
        conn: Connection,
        rag_config: &crate::config::RagConfig,
        build_ctx: Option<BuildContext>,
    ) -> Self {
        #[cfg(feature = "rag")]
        let rag_embedder = if rag_config.enabled {
            match crate::rag::embedder::Embedder::new(rag_config) {
                Ok(e) => {
                    // Verify vector table exists before enabling hybrid search
                    let table_check = conn.prepare("SELECT 1 FROM file_embeddings LIMIT 0");
                    match &table_check {
                        Ok(_) => {
                            tracing::info!("RAG hybrid search enabled");
                            Some(e)
                        }
                        Err(err) => {
                            tracing::warn!(%err,
                                "RAG enabled but file_embeddings table not accessible — \
                                 run `shire build` with [rag] enabled to generate embeddings");
                            None
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(%err, "RAG embedder init failed");
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
            rebuild_lock: Mutex::new(()),
            last_rebuild_failure: Mutex::new(None),
            rebuild_count: std::sync::atomic::AtomicU64::new(0),
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
    /// Includes a 2-second debounce to avoid redundant rebuilds during rapid tool calls.
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

        // Debounce: skip stale check if last rebuild completed within the debounce
        // window (default 5s, configurable via serve.debounce_s in shire.toml).
        // Prevents redundant rebuilds during rapid tool call bursts. No changes are
        // lost — the next check after the window expires triggers a rebuild that
        // reads current file state.
        let debounce_s = ctx.config.serve.debounce_s;
        if let Ok(elapsed) = last.elapsed()
            && elapsed < std::time::Duration::from_secs(debounce_s)
        {
            return false;
        }

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

    /// Number of rebuilds this service has run since start.
    #[cfg(test)]
    fn rebuild_count(&self) -> u64 {
        self.rebuild_count.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Rebuild the index if stale. No-op in read-only mode.
    ///
    /// The staleness check and the build happen together under
    /// `rebuild_lock`, so concurrent tool calls wait for the in-flight
    /// rebuild rather than starting their own.
    fn maybe_rebuild(&self) {
        // Cheap pre-check outside the lock: the common case is a warm index
        // where nothing is stale and nothing should serialize.
        if !self.is_stale() {
            return;
        }

        let ctx = match &self.build_ctx {
            Some(c) => c.clone(),
            None => return,
        };

        // A poisoned lock only means some other rebuild panicked; the guard
        // still gives us the mutual exclusion we need, so recover it.
        let _guard = match self.rebuild_lock.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        // Re-check under the guard: whoever held it before us may have
        // rebuilt and swapped in a fresh connection already.
        if !self.is_stale() {
            return;
        }

        // Back off after a failure for the same window the staleness check
        // debounces by, so a burst of calls against a repo that cannot build
        // does not turn into one full build per call.
        if let Ok(failed) = self.last_rebuild_failure.lock()
            && let Some(at) = *failed
            && at
                .elapsed()
                .is_ok_and(|e| e < std::time::Duration::from_secs(ctx.config.serve.debounce_s))
        {
            tracing::debug!("skipping rebuild: previous attempt failed recently");
            return;
        }

        tracing::info!("rebuilding index (stale)");
        self.rebuild_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        match crate::index::build_index_quiet(
            &ctx.repo_root,
            &ctx.config,
            false,
            Some(&ctx.db_path),
        ) {
            Ok(()) => {
                // Reopen connection read-only
                match crate::db::open_readonly(&ctx.db_path) {
                    Ok(new_conn) => match self.conn.lock() {
                        Ok(mut conn) => {
                            let now = Self::read_indexed_at(&new_conn)
                                .or_else(|| Some(SystemTime::now()));
                            *conn = new_conn;
                            if let Ok(mut li) = self.last_indexed.lock() {
                                *li = now;
                            }
                            if let Ok(mut failed) = self.last_rebuild_failure.lock() {
                                *failed = None;
                            }
                            tracing::info!("index rebuilt");
                        }
                        Err(e) => tracing::warn!(%e, "index rebuilt but failed to swap connection"),
                    },
                    Err(e) => {
                        // Prevent infinite rebuild loop: mark as indexed even if reopen fails
                        if let Ok(mut li) = self.last_indexed.lock() {
                            *li = Some(SystemTime::now());
                        }
                        tracing::warn!(%e, "failed to reopen index after rebuild");
                    }
                }
            }
            Err(e) => {
                if let Ok(mut failed) = self.last_rebuild_failure.lock() {
                    *failed = Some(SystemTime::now());
                }
                tracing::warn!(%e, "rebuild failed")
            }
        }
    }

    /// Resolve a caller-supplied `limit` against this tool's default and the
    /// query layer's hard ceiling. Every list-returning tool goes through
    /// this: a tool response is pasted verbatim into an LLM context window,
    /// so "no limit" is never an option.
    fn resolve_limit(requested: Option<u32>, default: u32) -> u32 {
        // `limit: 0` is a common client encoding for "no cap"; treat it as
        // "use the default" rather than silently returning a single row.
        requested
            .filter(|n| *n > 0)
            .unwrap_or(default)
            .min(queries::MAX_ROWS)
    }

    /// Serialize rows to JSON, appending a truncation notice when the result
    /// filled the limit exactly. Without the notice a capped list is
    /// indistinguishable from a complete one, and the model reasons about a
    /// package as if it had seen all of it.
    fn json_result<T: serde::Serialize>(
        rows: &[T],
        limit: u32,
        narrow_hint: &str,
    ) -> Result<CallToolResult, ErrorData> {
        let json = serde_json::to_string(rows).map_err(|e| Self::mcp_err(e.to_string()))?;
        let mut content = vec![Content::text(json)];
        if rows.len() as u32 >= limit {
            content.push(Content::text(format!(
                "Note: showing the first {limit} results (limit={limit}, max {max}). \
                 More may exist — {narrow_hint}.",
                max = queries::MAX_ROWS
            )));
        }
        Ok(CallToolResult::success(content))
    }

    pub(crate) fn mcp_err(detail: String) -> ErrorData {
        tracing::warn!(error = %detail, "MCP tool error");
        ErrorData {
            code: ErrorCode(-32603),
            message: Cow::from("Internal error — check server logs for details"),
            data: None,
        }
    }

    /// Early-return result for the three ref tools when the cross-reference
    /// index is disabled or was never populated. Without this, a refs-tool
    /// call against a refs-disabled DB returns `[]` silently — an LLM
    /// treats "no references" as "safe to delete/rename" and ships a
    /// broken refactor. We return an explicit actionable message instead.
    fn refs_disabled_result(conn: &Connection) -> Option<CallToolResult> {
        match crate::db::read_references_enabled(conn) {
            Some(true) => None,
            Some(false) => Some(CallToolResult::success(vec![Content::text(
                "Cross-reference index is disabled. Set `symbols.references_enabled = true` in \
                 shire.toml and run `shire build --force`, then retry this tool. \
                 (Feature is experimental and opt-in; defaults to off.)",
            )])),
            None => Some(CallToolResult::success(vec![Content::text(
                "Cross-reference index was never populated for this DB. Set \
                 `symbols.references_enabled = true` in shire.toml and run \
                 `shire build --force`, then retry this tool.",
            )])),
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

        // File-level vector search → find matching files → return their symbols
        let file_vec_results = crate::rag::storage::search_similar_files(conn, &query_vec, 20)
            .map_err(|e| Self::mcp_err(e.to_string()))?;

        if file_vec_results.is_empty() {
            return Ok(fts_results.to_vec());
        }

        let mut vec_symbols: Vec<queries::SymbolRow> = Vec::new();
        // Fetch up to 50 symbols per file to avoid missing matches after filtering
        let mut sym_stmt = conn.prepare(
            "SELECT name, kind, signature, package, file_path, line, visibility, parent_symbol, return_type, parameters \
             FROM symbols WHERE file_path = ?1 LIMIT 50"
        ).map_err(|e| Self::mcp_err(e.to_string()))?;

        for (file_id, _distance) in &file_vec_results {
            let file_path: String =
                match conn.query_row("SELECT path FROM files WHERE id = ?1", [file_id], |row| {
                    row.get(0)
                }) {
                    Ok(p) => p,
                    Err(_) => continue,
                };

            if let Ok(rows) = sym_stmt.query_map([&file_path], |row| {
                Ok(queries::SymbolRow {
                    name: row.get(0)?,
                    kind: row.get(1)?,
                    signature: row.get(2)?,
                    package: row.get(3)?,
                    file_path: row.get(4)?,
                    line: row.get(5)?,
                    visibility: row.get(6)?,
                    parent_symbol: row.get(7)?,
                    return_type: row.get(8)?,
                    parameters: row.get(9)?,
                })
            }) {
                for sym in rows.flatten() {
                    if params.package.as_ref().is_none_or(|p| sym.package == *p)
                        && params.kind.as_ref().is_none_or(|k| sym.kind == *k)
                    {
                        vec_symbols.push(sym);
                    }
                }
            }
        }

        if vec_symbols.is_empty() {
            return Ok(fts_results.to_vec());
        }

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
    /// Max results (default 100, max 200)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DependentsParams {
    /// Package name
    pub name: String,
    /// Max results (default 100, max 200)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListParams {
    /// Filter by package kind: "npm", "go", "cargo", "python", "maven", "gradle", "perl", "ruby"
    pub kind: Option<String>,
    /// Max results (default 100, max 200)
    pub limit: Option<u32>,
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
    /// Max results (default 100, max 200), in file order
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListPackageFilesParams {
    /// Package name
    pub package: String,
    /// Filter by file extension
    pub extension: Option<String>,
    /// Max results (default 100, max 200), in path order
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchFilesParams {
    /// Search query to find files by path or name
    pub query: String,
    /// Filter to files from a specific package
    pub package: Option<String>,
    /// Filter by file extension (e.g., "ts", "go", "rs")
    pub extension: Option<String>,
    /// Max results (default 20, max 200)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchDocsParams {
    /// Search query to find documentation by content, title, or path
    pub query: String,
    /// Filter to docs from a specific package
    pub package: Option<String>,
    /// Max results (default 20)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExploreParams {
    /// Concept to explore (e.g. "authentication", "error handling", "messaging interfaces")
    pub query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolRefsArgs {
    /// The symbol name to find references for
    pub name: String,
    /// Optional kind filter: "call", "type", "import", or "impl"
    #[serde(default)]
    pub kind: Option<String>,
    /// Optional package filter
    #[serde(default)]
    pub package: Option<String>,
    /// Max results (default 100, clamped to 1..=1000)
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolCallersArgs {
    /// The symbol being called
    pub name: String,
    /// Optional: restrict callers to this package
    #[serde(default)]
    pub package: Option<String>,
    /// Max results (default 100, clamped to 1..=1000)
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolCalleesArgs {
    /// The caller symbol (function/method name)
    pub name: String,
    /// Optional: restrict to this package
    #[serde(default)]
    pub package: Option<String>,
    /// Max results (default 100, clamped to 1..=1000)
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ChangeImpactArgs {
    /// The symbol name whose change impact to analyze
    pub name: String,
    /// Optional "home" package — the package that defines the symbol. When
    /// omitted, Shire looks it up from the symbols table. Provide this to
    /// disambiguate same-name symbols defined in multiple packages.
    #[serde(default)]
    pub package: Option<String>,
    /// Reverse-dependency BFS depth for transitive impact. Default 2, clamped
    /// 0..=10. Use 0 to skip transitive analysis entirely.
    #[serde(default)]
    pub transitive_depth: Option<u32>,
    /// Max results per bucket (default 100, clamped 1..=1000)
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SchemaConsumersArgs {
    /// Path to the schema file (e.g. "proto/user.proto")
    pub path: String,
    /// Max results (default 100, max 200)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GeneratedFromArgs {
    /// Path to the generated file (e.g. "gen/user.pb.go")
    pub path: String,
    /// Max results (default 100, max 200)
    pub limit: Option<u32>,
}

#[tool_router]
impl ShireService {
    #[tool(
        description = "Search packages by name or description. Use instead of Grep for finding packages."
    )]
    fn search_packages(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "search_packages", query = %params.query, limit = ?params.limit);
        self.maybe_rebuild();
        if params.query.trim().is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "Search query must not be empty",
            )]));
        }
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let limit = Self::resolve_limit(params.limit, 20);
        let results = queries::search_packages(&conn, &params.query, limit)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(
            &results,
            limit,
            "raise `limit` or use a more specific query",
        )
    }

    #[tool(
        description = "List a package's dependencies. Set depth>1 for transitive graph (returns edge list with different schema)."
    )]
    fn package_dependencies(
        &self,
        Parameters(params): Parameters<DepsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "package_dependencies", name = %params.name, depth = ?params.depth, internal_only = params.internal_only);
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        match params.depth {
            Some(n) if n > 1 => {
                let depth = n.min(20);
                let limit = Self::resolve_limit(params.limit, queries::DEFAULT_LIST_LIMIT);
                let mut edges =
                    queries::dependency_graph(&conn, &params.name, depth, params.internal_only)
                        .map_err(|e| Self::mcp_err(e.to_string()))?;
                // The graph walk is bounded only by its own MAX_EDGES; the
                // edge list goes into a context window like any other list.
                edges.truncate(limit as usize);
                Self::json_result(&edges, limit, "raise `limit` or lower `depth`")
            }
            _ => {
                let limit = Self::resolve_limit(params.limit, queries::DEFAULT_LIST_LIMIT);
                let results =
                    queries::package_dependencies(&conn, &params.name, params.internal_only, limit)
                        .map_err(|e| Self::mcp_err(e.to_string()))?;
                Self::json_result(&results, limit, "raise `limit`")
            }
        }
    }

    #[tool(description = "Find all packages that depend on this package")]
    fn package_dependents(
        &self,
        Parameters(params): Parameters<DependentsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "package_dependents", name = %params.name);
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let limit = Self::resolve_limit(params.limit, queries::DEFAULT_LIST_LIMIT);
        let results = queries::package_dependents(&conn, &params.name, limit)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&results, limit, "raise `limit`")
    }

    #[tool(description = "List all indexed packages, optionally filtered by kind")]
    fn list_packages(
        &self,
        Parameters(params): Parameters<ListParams>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "list_packages", kind = ?params.kind);
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let limit = Self::resolve_limit(params.limit, queries::DEFAULT_LIST_LIMIT);
        let results = queries::list_packages(&conn, params.kind.as_deref(), limit)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&results, limit, "raise `limit` or filter by `kind`")
    }

    #[tool(
        description = "Find functions, classes, types, methods by identifier or identifier prefix (not regex or substring). Every whitespace-separated token must match, by prefix and against identifier sub-tokens: 'handle' finds handleRequest, 'verify jwt' finds verifyJwtToken. Use instead of Grep for 'where is function X?'. Omit query with a package filter to list the start of that package in (file, line) order."
    )]
    fn search_symbols(
        &self,
        Parameters(params): Parameters<SearchSymbolsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "search_symbols", query = ?params.query, package = ?params.package, kind = ?params.kind, limit = ?params.limit);
        self.maybe_rebuild();
        let limit = Self::resolve_limit(params.limit, 20);
        let query = params.query.as_deref().unwrap_or("").trim();
        if query.is_empty() {
            // No query: list all symbols in a package
            let pkg = match &params.package {
                Some(p) => p,
                None => {
                    return Ok(CallToolResult::success(vec![Content::text(
                        "Provide a query or a package filter",
                    )]));
                }
            };
            let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
            let results = queries::get_package_symbols(&conn, pkg, params.kind.as_deref(), limit)
                .map_err(|e| Self::mcp_err(e.to_string()))?;
            // Ordered by (file_path, line), so a capped listing is the first
            // `limit` symbols of the alphabetically-first files — say so.
            return Self::json_result(
                &results,
                limit,
                "this is the start of the package in (file, line) order; \
                 narrow with `kind` or `get_file_symbols`, or raise `limit`",
            );
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
                    tracing::warn!(error = %e.message, "hybrid search failed, falling back to FTS-only");
                    fts_results
                }
            }
        } else {
            fts_results
        };

        #[cfg(not(feature = "rag"))]
        let results = fts_results;

        Self::json_result(
            &results,
            limit,
            "raise `limit` or use a more specific query",
        )
    }

    #[tool(
        description = "List all symbols defined in a specific file. Use instead of reading the file to understand its exports."
    )]
    fn get_file_symbols(
        &self,
        Parameters(params): Parameters<GetFileSymbolsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "get_file_symbols", file_path = %params.file_path, kind = ?params.kind);
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let limit = Self::resolve_limit(params.limit, queries::DEFAULT_LIST_LIMIT);
        let results =
            queries::get_file_symbols(&conn, &params.file_path, params.kind.as_deref(), limit)
                .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&results, limit, "raise `limit` or filter by `kind`")
    }

    #[tool(
        description = "List all files in a package, optionally filtered by extension. Use instead of Glob for listing package contents."
    )]
    fn list_package_files(
        &self,
        Parameters(params): Parameters<ListPackageFilesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "list_package_files", package = %params.package, extension = ?params.extension);
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let limit = Self::resolve_limit(params.limit, queries::DEFAULT_LIST_LIMIT);
        let results =
            queries::list_package_files(&conn, &params.package, params.extension.as_deref(), limit)
                .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&results, limit, "raise `limit` or filter by `extension`")
    }

    #[tool(description = "Index build metadata: timestamp, git commit, counts")]
    fn index_status(&self) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "index_status");
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let status = queries::index_status(&conn).map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&status).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Find files by path or name, matching each whitespace-separated token by prefix against the path's own tokens (path components are not split further). Use instead of Glob/find for locating files."
    )]
    fn search_files(
        &self,
        Parameters(params): Parameters<SearchFilesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "search_files", query = %params.query, package = ?params.package, extension = ?params.extension);
        self.maybe_rebuild();
        if params.query.trim().is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "Search query must not be empty",
            )]));
        }
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let limit = Self::resolve_limit(params.limit, 20);
        let results = queries::search_files(
            &conn,
            &params.query,
            params.package.as_deref(),
            params.extension.as_deref(),
            limit,
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(
            &results,
            limit,
            "raise `limit` or use a more specific query",
        )
    }

    #[tool(
        description = "Search documentation files by content, title, or path. Returns matching docs with relevant text snippets. Use for finding guides, READMEs, and written documentation."
    )]
    fn search_docs(
        &self,
        Parameters(params): Parameters<SearchDocsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "search_docs", query = %params.query, package = ?params.package, limit = ?params.limit);
        self.maybe_rebuild();
        if params.query.trim().is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "Search query must not be empty",
            )]));
        }
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let limit = Self::resolve_limit(params.limit, 20);
        let results = queries::search_docs(&conn, &params.query, params.package.as_deref(), limit)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(
            &results,
            limit,
            "raise `limit` or use a more specific query",
        )
    }

    #[tool(
        description = "Explore a concept across the codebase — searches packages, symbols, files, and documentation semantically. Use as the first tool when investigating unfamiliar code or broad topics like 'authentication' or 'error handling'. Returns a structured context map organized by package."
    )]
    fn explore(
        &self,
        Parameters(params): Parameters<ExploreParams>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "explore", query = %params.query);
        self.maybe_rebuild();
        if params.query.trim().is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "Search query must not be empty",
            )]));
        }
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let mut args = std::collections::HashMap::new();
        args.insert("query".into(), params.query);
        let text =
            crate::mcp::prompts::call_prompt(&conn, "explore", &args).map_err(|e| match e {
                crate::mcp::prompts::PromptError::InvalidParams(msg) => {
                    ErrorData::invalid_params(msg, None)
                }
                crate::mcp::prompts::PromptError::NotFound(msg) => {
                    ErrorData::resource_not_found(msg, None)
                }
                crate::mcp::prompts::PromptError::Internal(msg) => {
                    ErrorData::internal_error(msg, None)
                }
            })?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Find all references (call sites, type uses, imports, impl clauses) to a symbol by name. Use instead of Grep for 'who uses X?' — returns file, line, kind, and enclosing symbol. Note: matches by name only, so two symbols with the same name cannot be distinguished."
    )]
    fn symbol_references(
        &self,
        Parameters(args): Parameters<SymbolRefsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "symbol_references", name = %args.name);
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        if let Some(disabled) = Self::refs_disabled_result(&conn) {
            return Ok(disabled);
        }
        // Validate `kind` up front. Without this, a typo like "CALL" or
        // "cal" passes through to the SQL `AND r.kind = ?` and returns
        // zero rows — visually identical to "no matches", which hides the
        // error from the caller.
        if let Some(k) = args.kind.as_deref()
            && !matches!(k, "call" | "type" | "import" | "impl")
        {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "Unknown kind {k:?}. Valid kinds are: call, type, import, impl."
            ))]));
        }
        let limit = i64::from(args.limit.unwrap_or(100).clamp(1, 1000));
        let rows = queries::query_symbol_references(
            &conn,
            &args.name,
            args.kind.as_deref(),
            args.package.as_deref(),
            limit,
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&rows).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Find which symbols (functions, methods) call the named symbol. Returns the caller name, file, line of first call, and count of call sites. Navigates the call graph upward."
    )]
    fn symbol_callers(
        &self,
        Parameters(args): Parameters<SymbolCallersArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "symbol_callers", name = %args.name);
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        if let Some(disabled) = Self::refs_disabled_result(&conn) {
            return Ok(disabled);
        }
        let limit = i64::from(args.limit.unwrap_or(100).clamp(1, 1000));
        let rows = queries::query_symbol_callers(&conn, &args.name, args.package.as_deref(), limit)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&rows).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Find which symbols are called from inside the named function/method. Navigates the call graph downward."
    )]
    fn symbol_callees(
        &self,
        Parameters(args): Parameters<SymbolCalleesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "symbol_callees", name = %args.name);
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        if let Some(disabled) = Self::refs_disabled_result(&conn) {
            return Ok(disabled);
        }
        let limit = i64::from(args.limit.unwrap_or(100).clamp(1, 1000));
        let rows = queries::query_symbol_callees(&conn, &args.name, args.package.as_deref(), limit)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&rows).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Analyze the impact of changing a symbol. Combines the cross-reference index with the dependency graph to return: direct_impact (same-package refs), cross_package_impact (refs in other packages), and transitive_impact (packages that depend on affected packages via the reverse dep graph). Use before renaming, changing a signature, or deleting a symbol. Requires `symbols.references_enabled = true` (experimental). Same name-based-match caveat as symbol_references — pass `package` to disambiguate same-name symbols."
    )]
    fn change_impact(
        &self,
        Parameters(args): Parameters<ChangeImpactArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "change_impact", name = %args.name, package = ?args.package);
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        if let Some(disabled) = Self::refs_disabled_result(&conn) {
            return Ok(disabled);
        }
        let depth = args.transitive_depth.unwrap_or(2).min(10);
        let limit = i64::from(args.limit.unwrap_or(100).clamp(1, 1000));
        let impact =
            queries::change_impact(&conn, &args.name, args.package.as_deref(), depth, limit)
                .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&impact).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Find all files generated from a schema file (e.g. .proto). Returns generated file paths and their packages. Use to understand the blast radius of a schema change."
    )]
    fn schema_consumers(
        &self,
        Parameters(args): Parameters<SchemaConsumersArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "schema_consumers", path = %args.path);
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let limit = Self::resolve_limit(args.limit, queries::DEFAULT_LIST_LIMIT);
        let rows = queries::query_schema_consumers(&conn, &args.path, limit)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&rows, limit, "raise `limit`")
    }

    #[tool(
        description = "Find the source schema file that generated a given file. Use to trace a generated file (e.g. user.pb.go) back to its source proto."
    )]
    fn generated_from(
        &self,
        Parameters(args): Parameters<GeneratedFromArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "generated_from", path = %args.path);
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let limit = Self::resolve_limit(args.limit, queries::DEFAULT_LIST_LIMIT);
        let rows = queries::query_generated_from(&conn, &args.path, limit)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&rows, limit, "raise `limit`")
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
        assert!(
            !svc.is_stale(),
            "should not be stale when .git/index is older than last_indexed"
        );
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
        assert!(
            svc.is_stale(),
            "should be stale when .git/index is newer than last_indexed"
        );
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
        conn.execute_batch("CREATE TABLE shire_meta (key TEXT PRIMARY KEY, value TEXT);")
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
        assert!(
            result.is_none(),
            "should return None when shire_meta doesn't exist"
        );
    }

    /// When the `references_enabled` flag is absent (e.g. an index built
    /// before the flag was persisted), the ref tools must refuse to
    /// serve — otherwise an LLM sees `[]` and assumes "no callers" on a
    /// DB that never populated `symbol_refs`.
    #[test]
    fn test_refs_disabled_result_none_when_enabled() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("t.db");
        let conn = crate::db::open_or_create(&path, false).unwrap();
        crate::db::write_references_enabled(&conn, true).unwrap();
        assert!(
            ShireService::refs_disabled_result(&conn).is_none(),
            "enabled flag allows the tool to proceed"
        );
    }

    #[test]
    fn test_refs_disabled_result_some_when_disabled() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("t.db");
        let conn = crate::db::open_or_create(&path, false).unwrap();
        crate::db::write_references_enabled(&conn, false).unwrap();
        let r = ShireService::refs_disabled_result(&conn);
        assert!(r.is_some(), "disabled flag short-circuits the tool");
    }

    #[test]
    fn test_refs_disabled_result_some_when_unset() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("t.db");
        let conn = crate::db::open_or_create(&path, false).unwrap();
        // Flag was never written — simulates an old DB or an index that
        // predates this guard. Tools must still refuse to serve.
        let r = ShireService::refs_disabled_result(&conn);
        assert!(r.is_some(), "missing flag short-circuits the tool");
    }

    /// End-to-end test of the ref tool against a DB with refs enabled but
    /// no data — exercises the disabled-guard bypass and verifies the tool
    /// returns a JSON array (possibly empty) rather than silently
    /// swallowing a kind-filter typo. Covers D3.
    #[test]
    fn test_symbol_references_rejects_unknown_kind() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("t.db");
        {
            let conn = crate::db::open_or_create(&path, false).unwrap();
            crate::db::write_references_enabled(&conn, true).unwrap();
        }
        let conn = crate::db::open_or_create(&path, false).unwrap();
        let svc = ShireService::new(conn, &default_rag_config(), None);

        let args = SymbolRefsArgs {
            name: "foo".into(),
            kind: Some("CALL".into()), // wrong case — would silently return []
            package: None,
            limit: None,
        };
        let r = svc.symbol_references(Parameters(args)).unwrap();
        // The result carries the validation message, not an empty JSON array.
        let text = match &r.content.first().expect("content").raw {
            RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        assert!(
            text.contains("Unknown kind"),
            "expected validation error, got {text}"
        );
        assert!(
            text.contains("call, type, import, impl"),
            "should list the valid kinds"
        );
    }

    #[test]
    fn test_symbol_references_accepts_known_kind() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("t.db");
        {
            let conn = crate::db::open_or_create(&path, false).unwrap();
            crate::db::write_references_enabled(&conn, true).unwrap();
        }
        let conn = crate::db::open_or_create(&path, false).unwrap();
        let svc = ShireService::new(conn, &default_rag_config(), None);

        let args = SymbolRefsArgs {
            name: "foo".into(),
            kind: Some("call".into()),
            package: None,
            limit: None,
        };
        let r = svc.symbol_references(Parameters(args)).unwrap();
        let text = match &r.content.first().expect("content").raw {
            RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        // Empty DB → empty JSON array, not an error string.
        assert_eq!(text, "[]", "empty DB returns empty array");
    }

    #[test]
    fn test_change_impact_refs_disabled_message() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("t.db");
        {
            let conn = crate::db::open_or_create(&path, false).unwrap();
            crate::db::write_references_enabled(&conn, false).unwrap();
        }
        let conn = crate::db::open_or_create(&path, false).unwrap();
        let svc = ShireService::new(conn, &default_rag_config(), None);

        let args = ChangeImpactArgs {
            name: "foo".into(),
            package: None,
            transitive_depth: None,
            limit: None,
        };
        let r = svc.change_impact(Parameters(args)).unwrap();
        let text = match &r.content.first().expect("content").raw {
            RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        assert!(text.contains("Cross-reference index is disabled"));
    }

    /// End-to-end: wire a minimal symbol + refs + dep graph through the
    /// tool and verify the JSON carries the partitioning and summary fields.
    #[test]
    fn test_change_impact_happy_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("t.db");
        {
            let conn = crate::db::open_or_create(&path, false).unwrap();
            crate::db::write_references_enabled(&conn, true).unwrap();
            conn.execute(
                "INSERT INTO packages (name, path, kind) VALUES ('core','core','rust'),('dep','dep','rust'),('grand','grand','rust')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO dependencies (package, dependency, dep_kind, is_internal) VALUES ('dep','core','runtime',1),('grand','dep','runtime',1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO symbols (package, name, kind, file_path, line) VALUES ('core','foo','function','core/f.rs',1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO files (path, package, extension, size_bytes) VALUES ('core/x.rs','core','rs',0),('dep/y.rs','dep','rs',0)",
                [],
            )
            .unwrap();
            let core_id: i64 = conn
                .query_row("SELECT id FROM files WHERE path='core/x.rs'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            let dep_id: i64 = conn
                .query_row("SELECT id FROM files WHERE path='dep/y.rs'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            conn.execute(
                &format!(
                    "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) VALUES \
                     ('foo','call',{core_id},10,'core','bar'), \
                     ('foo','call',{dep_id},5,'dep','baz')"
                ),
                [],
            )
            .unwrap();
        }
        let conn = crate::db::open_or_create(&path, false).unwrap();
        let svc = ShireService::new(conn, &default_rag_config(), None);

        let args = ChangeImpactArgs {
            name: "foo".into(),
            package: None,
            transitive_depth: Some(2),
            limit: None,
        };
        let r = svc.change_impact(Parameters(args)).unwrap();
        let text = match &r.content.first().expect("content").raw {
            RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        };
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(v["symbol"], "foo");
        assert_eq!(v["home_package"], "core");
        assert_eq!(v["direct_impact"].as_array().unwrap().len(), 1);
        assert_eq!(v["cross_package_impact"].as_array().unwrap().len(), 1);
        assert_eq!(v["summary"]["direct_count"], 1);
        assert_eq!(v["summary"]["cross_package_count"], 1);
        // dep is directly affected; grand depends on dep → transitive.
        let trans = v["transitive_impact"].as_array().unwrap();
        assert_eq!(trans.len(), 1);
        assert_eq!(trans[0]["package"], "grand");
        assert_eq!(trans[0]["via"], "dep");
    }

    #[test]
    fn test_schema_consumers_empty_db() {
        let dir = tempfile::TempDir::new().unwrap();
        let conn = crate::db::open_or_create(&dir.path().join("t.db"), false).unwrap();
        let svc = ShireService::new(conn, &default_rag_config(), None);
        let args = SchemaConsumersArgs {
            path: "a.proto".into(),
            limit: None,
        };
        let r = svc.schema_consumers(Parameters(args)).unwrap();
        let text = match &r.content.first().expect("content").raw {
            RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text"),
        };
        assert_eq!(text, "[]");
    }

    #[test]
    fn test_mcp_err_redacts_details() {
        let err = ShireService::mcp_err(
            "SQLITE_ERROR: no such table: foo at /home/user/.shire/index.db".to_string(),
        );
        // The message returned to the caller must NOT contain the raw error
        assert!(!err.message.contains("SQLITE_ERROR"));
        assert!(!err.message.contains("/home/user"));
        assert!(!err.message.contains("foo"));
        assert_eq!(
            err.message,
            "Internal error \u{2014} check server logs for details"
        );
    }

    /// Text of the first content block of a tool result.
    fn result_text(r: &CallToolResult) -> String {
        match &r.content.first().expect("content").raw {
            RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text content"),
        }
    }

    /// A service over a real (on-disk) schema, with `n` symbols in one
    /// package spread over `n` files.
    fn service_with_symbols(dir: &std::path::Path, n: usize) -> ShireService {
        let path = dir.join("t.db");
        {
            let conn = crate::db::open_or_create(&path, false).unwrap();
            conn.execute(
                "INSERT INTO packages (name, path, kind) VALUES ('pkg', 'pkg', 'npm')",
                [],
            )
            .unwrap();
            for i in 0..n {
                conn.execute(
                    "INSERT INTO symbols (package, name, kind, file_path, line, name_tokens)
                     VALUES ('pkg', ?1, 'function', ?2, 1, 'handle thing')",
                    rusqlite::params![format!("handleThing{i}"), format!("pkg/src/f{i:03}.ts")],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO files (path, package, extension, size_bytes)
                     VALUES (?1, 'pkg', 'ts', 10)",
                    [format!("pkg/src/f{i:03}.ts")],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO symbols (package, name, kind, file_path, line)
                     VALUES ('pkg', ?1, 'function', 'pkg/src/big.ts', ?2)",
                    rusqlite::params![format!("sym{i}"), i as i64],
                )
                .unwrap();
            }
        }
        let conn = crate::db::open_or_create(&path, false).unwrap();
        ShireService::new(conn, &default_rag_config(), None)
    }

    /// MCP-1: `search_symbols` with a package filter and no query used to
    /// ignore `limit` entirely and serialize the whole package.
    #[test]
    fn test_search_symbols_package_listing_honors_limit() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = service_with_symbols(dir.path(), 300);

        let r = svc
            .search_symbols(Parameters(SearchSymbolsParams {
                query: None,
                package: Some("pkg".into()),
                kind: None,
                limit: Some(5),
            }))
            .unwrap();
        let rows: serde_json::Value = serde_json::from_str(&result_text(&r)).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 5, "limit must be honored");
        // …and the model must be told the list was cut.
        assert_eq!(r.content.len(), 2, "expected a truncation note");
        let note = match &r.content[1].raw {
            RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text"),
        };
        assert!(note.contains("first 5 results"), "got {note}");

        // Default (no limit given) is 20, not "everything".
        let r = svc
            .search_symbols(Parameters(SearchSymbolsParams {
                query: None,
                package: Some("pkg".into()),
                kind: None,
                limit: None,
            }))
            .unwrap();
        let rows: serde_json::Value = serde_json::from_str(&result_text(&r)).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 20);

        // The hard ceiling wins over an absurd request.
        let r = svc
            .search_symbols(Parameters(SearchSymbolsParams {
                query: None,
                package: Some("pkg".into()),
                kind: None,
                limit: Some(100_000),
            }))
            .unwrap();
        let rows: serde_json::Value = serde_json::from_str(&result_text(&r)).unwrap();
        assert_eq!(
            rows.as_array().unwrap().len(),
            queries::MAX_ROWS as usize,
            "capped at MAX_ROWS"
        );
    }

    /// MCP-5 / DB-5: the other list-returning tools are bounded too.
    #[test]
    fn test_list_tools_are_bounded() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = service_with_symbols(dir.path(), 300);

        let len = |r: &CallToolResult| -> usize {
            serde_json::from_str::<serde_json::Value>(&result_text(r))
                .unwrap()
                .as_array()
                .unwrap()
                .len()
        };

        let r = svc
            .list_package_files(Parameters(ListPackageFilesParams {
                package: "pkg".into(),
                extension: None,
                limit: None,
            }))
            .unwrap();
        assert_eq!(len(&r), queries::DEFAULT_LIST_LIMIT as usize);

        let r = svc
            .get_file_symbols(Parameters(GetFileSymbolsParams {
                file_path: "pkg/src/big.ts".into(),
                kind: None,
                limit: Some(7),
            }))
            .unwrap();
        assert_eq!(len(&r), 7);

        let r = svc
            .list_packages(Parameters(ListParams {
                kind: None,
                limit: Some(1),
            }))
            .unwrap();
        assert_eq!(len(&r), 1);

        let r = svc
            .search_files(Parameters(SearchFilesParams {
                query: "pkg".into(),
                package: None,
                extension: None,
                limit: Some(3),
            }))
            .unwrap();
        assert_eq!(len(&r), 3);

        let r = svc
            .search_symbols(Parameters(SearchSymbolsParams {
                query: Some("handle".into()),
                package: None,
                kind: None,
                limit: Some(4),
            }))
            .unwrap();
        assert_eq!(len(&r), 4, "prefix query still respects limit");
    }

    /// `limit: 0` is a common client encoding for "no cap"; it must not
    /// come back as a single row plus a "showing the first 1 results" note.
    #[test]
    fn test_zero_limit_falls_back_to_default() {
        assert_eq!(ShireService::resolve_limit(Some(0), 20), 20);
        assert_eq!(ShireService::resolve_limit(None, 20), 20);
        assert_eq!(ShireService::resolve_limit(Some(5), 20), 5);
        assert_eq!(
            ShireService::resolve_limit(Some(u32::MAX), 20),
            queries::MAX_ROWS
        );

        let dir = tempfile::TempDir::new().unwrap();
        let svc = service_with_symbols(dir.path(), 50);
        let r = svc
            .search_symbols(Parameters(SearchSymbolsParams {
                query: None,
                package: Some("pkg".into()),
                kind: None,
                limit: Some(0),
            }))
            .unwrap();
        let rows: serde_json::Value = serde_json::from_str(&result_text(&r)).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 20);
    }

    /// A short list gets no truncation note — the note must mean something.
    #[test]
    fn test_no_truncation_note_when_under_limit() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = service_with_symbols(dir.path(), 3);
        let r = svc
            .list_packages(Parameters(ListParams {
                kind: None,
                limit: None,
            }))
            .unwrap();
        assert_eq!(r.content.len(), 1, "no note for a complete list");
    }

    /// MCP-2: concurrent tool calls under `serve --root` all saw
    /// `is_stale() == true` and each started its own build against the same
    /// SQLite file; the losers failed with "database is locked" and answered
    /// -32603. The rebuild lock must collapse them into one build.
    #[test]
    fn test_concurrent_rebuilds_run_once() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"name":"p","version":"1.0.0"}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("src/a.ts"),
            "export function verifyJwtToken(): string { return \"t\"; }\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/index"), "x").unwrap();

        let svc = make_service_with_ctx(root.clone());
        assert!(svc.is_stale(), "no index yet");

        std::thread::scope(|scope| {
            for _ in 0..6 {
                scope.spawn(|| svc.maybe_rebuild());
            }
        });

        assert_eq!(
            svc.rebuild_count(),
            1,
            "six racing callers must produce exactly one build"
        );
        assert!(!svc.is_stale(), "index is fresh after the rebuild");

        // The winner's connection was swapped in, so every caller can query.
        let r = svc
            .search_symbols(Parameters(SearchSymbolsParams {
                query: Some("jwt".into()),
                package: None,
                kind: None,
                limit: None,
            }))
            .unwrap();
        let rows: serde_json::Value = serde_json::from_str(&result_text(&r)).unwrap();
        assert_eq!(
            rows.as_array().unwrap().len(),
            1,
            "sub-token search over the freshly built index"
        );
    }

    #[test]
    fn test_generated_from_empty_db() {
        let dir = tempfile::TempDir::new().unwrap();
        let conn = crate::db::open_or_create(&dir.path().join("t.db"), false).unwrap();
        let svc = ShireService::new(conn, &default_rag_config(), None);
        let args = GeneratedFromArgs {
            path: "a.pb.go".into(),
            limit: None,
        };
        let r = svc.generated_from(Parameters(args)).unwrap();
        let text = match &r.content.first().expect("content").raw {
            RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text"),
        };
        assert_eq!(text, "[]");
    }
}
