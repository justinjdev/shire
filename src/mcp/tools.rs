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
}

impl std::fmt::Debug for ShireService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("ShireService");
        d.field("conn", &self.conn);
        d.field("tool_router", &self.tool_router);
        d.field("build_ctx", &self.build_ctx.as_ref().map(|c| &c.repo_root));
        d.finish()
    }
}

impl ShireService {
    pub fn new(conn: Connection, build_ctx: Option<BuildContext>) -> Self {
        // Initialize last_indexed from DB metadata if available
        let last_indexed = Self::read_indexed_at(&conn);

        Self {
            conn: Mutex::new(conn),
            tool_router: Self::tool_router(),
            build_ctx,
            last_indexed: Mutex::new(last_indexed),
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

    /// Decide whether to trigger an on-demand rebuild before answering.
    ///
    /// This is a *hint*, not a correctness oracle: the rebuild it triggers
    /// does its own mtime and content-hash comparisons and is cheap when
    /// nothing changed. So the bias is towards rebuilding — the Git index
    /// mtime moving is a strong signal, and "no Git index to look at"
    /// (a non-Git directory, a repository with nothing staged) means we
    /// cannot tell, which must not be mistaken for "nothing changed".
    ///
    /// The debounce keeps that bias from turning into a rebuild per tool
    /// call during a burst.
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

        // Resolve the real index file: in a linked worktree `.git` is a
        // file and the index lives under <main>/.git/worktrees/<id>/, so
        // stat'ing <root>/.git/index there always fails (INDEX-12).
        match crate::git::index_path(&ctx.repo_root) {
            Some(git_index) => match std::fs::metadata(&git_index).and_then(|m| m.modified()) {
                Ok(mtime) => mtime > last,
                // Unreadable index file — unknown, so assume stale.
                Err(_) => true,
            },
            // No Git index to compare against: unknown, not "unchanged".
            None => true,
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

        tracing::info!("rebuilding index (stale)");

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
            Err(e) => tracing::warn!(%e, "rebuild failed"),
        }
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
pub struct SearchFilesParams {
    /// Search query to find files by path or name
    pub query: String,
    /// Filter to files from a specific package
    pub package: Option<String>,
    /// Filter by file extension (e.g., "ts", "go", "rs")
    pub extension: Option<String>,
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
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GeneratedFromArgs {
    /// Path to the generated file (e.g. "gen/user.pb.go")
    pub path: String,
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
        let limit = params.limit.unwrap_or(20);
        let results = queries::search_packages(&conn, &params.query, limit)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&results).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
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
                let edges =
                    queries::dependency_graph(&conn, &params.name, depth, params.internal_only)
                        .map_err(|e| Self::mcp_err(e.to_string()))?;
                let json =
                    serde_json::to_string(&edges).map_err(|e| Self::mcp_err(e.to_string()))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            _ => {
                let results =
                    queries::package_dependencies(&conn, &params.name, params.internal_only)
                        .map_err(|e| Self::mcp_err(e.to_string()))?;
                let json =
                    serde_json::to_string(&results).map_err(|e| Self::mcp_err(e.to_string()))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
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
        let results = queries::package_dependents(&conn, &params.name)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&results).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List all indexed packages, optionally filtered by kind")]
    fn list_packages(
        &self,
        Parameters(params): Parameters<ListParams>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "list_packages", kind = ?params.kind);
        self.maybe_rebuild();
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::list_packages(&conn, params.kind.as_deref())
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&results).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Find functions, classes, types, methods by name or signature. Use instead of Grep for 'where is function X?' or 'what matches pattern Y?'. Omit query with a package filter to list all symbols in that package."
    )]
    fn search_symbols(
        &self,
        Parameters(params): Parameters<SearchSymbolsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        tracing::debug!(tool = "search_symbols", query = ?params.query, package = ?params.package, kind = ?params.kind, limit = ?params.limit);
        self.maybe_rebuild();
        let limit = params.limit.unwrap_or(20);
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
            let results = queries::get_package_symbols(&conn, pkg, params.kind.as_deref())
                .map_err(|e| Self::mcp_err(e.to_string()))?;
            let json = serde_json::to_string(&results).map_err(|e| Self::mcp_err(e.to_string()))?;
            return Ok(CallToolResult::success(vec![Content::text(json)]));
        }
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;

        let results = queries::search_symbols(
            &conn,
            query,
            params.package.as_deref(),
            params.kind.as_deref(),
            limit,
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;

        let json = serde_json::to_string(&results).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
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
        let results = queries::get_file_symbols(&conn, &params.file_path, params.kind.as_deref())
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&results).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
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
        let results =
            queries::list_package_files(&conn, &params.package, params.extension.as_deref())
                .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&results).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
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
        description = "Find files by path or name. Use instead of Glob/find for locating files. Useful for 'middleware', 'proto files', or files in a specific directory."
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
        let results = queries::search_files(
            &conn,
            &params.query,
            params.package.as_deref(),
            params.extension.as_deref(),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&results).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
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
        let limit = params.limit.unwrap_or(20);
        let results = queries::search_docs(&conn, &params.query, params.package.as_deref(), limit)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&results).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
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
        let rows = queries::query_schema_consumers(&conn, &args.path)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&rows).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
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
        let rows = queries::query_generated_from(&conn, &args.path)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string(&rows).map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn make_service_readonly() -> ShireService {
        let conn = Connection::open_in_memory().unwrap();
        ShireService::new(conn, None)
    }

    fn make_service_with_ctx(repo_root: std::path::PathBuf) -> ShireService {
        let db_path = repo_root.join(".shire/index.db");
        let conn = Connection::open_in_memory().unwrap();
        let build_ctx = BuildContext {
            repo_root,
            config: crate::config::Config::default(),
            db_path,
        };
        ShireService::new(conn, Some(build_ctx))
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
    fn test_is_stale_true_when_no_git_index() {
        // Without a Git index we cannot tell whether the tree moved, and
        // "unknown" must not be served as "fresh" — a non-Git repo root
        // would otherwise never see an on-demand rebuild again.
        let dir = tempfile::TempDir::new().unwrap();
        let svc = make_service_with_ctx(dir.path().to_path_buf());
        // Old enough to be outside the debounce window.
        *svc.last_indexed.lock().unwrap() = Some(SystemTime::now() - Duration::from_secs(60));
        assert!(
            svc.is_stale(),
            "unknown staleness must trigger a (cheap, self-checking) rebuild"
        );
    }

    #[test]
    fn test_is_stale_resolves_linked_worktree_index() {
        // INDEX-12: `.git` is a file in a linked worktree, so the index has
        // to be resolved through the gitdir pointer.
        let dir = tempfile::TempDir::new().unwrap();
        let main_repo = dir.path().join("main");
        let wt_git_dir = main_repo.join(".git").join("worktrees").join("feat");
        std::fs::create_dir_all(&wt_git_dir).unwrap();
        std::fs::write(wt_git_dir.join("index"), "dummy").unwrap();

        let wt = dir.path().join("feat");
        std::fs::create_dir(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}", wt_git_dir.display())).unwrap();

        let svc = make_service_with_ctx(wt.clone());
        *svc.last_indexed.lock().unwrap() = Some(SystemTime::now() - Duration::from_secs(60));
        assert!(svc.is_stale(), "a newer worktree index must read as stale");

        *svc.last_indexed.lock().unwrap() = Some(SystemTime::now() + Duration::from_secs(600));
        assert!(
            !svc.is_stale(),
            "an older worktree index must read as fresh — not always-stale"
        );
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
        let conn = crate::db::open_or_create(&path).unwrap();
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
        let conn = crate::db::open_or_create(&path).unwrap();
        crate::db::write_references_enabled(&conn, false).unwrap();
        let r = ShireService::refs_disabled_result(&conn);
        assert!(r.is_some(), "disabled flag short-circuits the tool");
    }

    #[test]
    fn test_refs_disabled_result_some_when_unset() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("t.db");
        let conn = crate::db::open_or_create(&path).unwrap();
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
            let conn = crate::db::open_or_create(&path).unwrap();
            crate::db::write_references_enabled(&conn, true).unwrap();
        }
        let conn = crate::db::open_or_create(&path).unwrap();
        let svc = ShireService::new(conn, None);

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
            let conn = crate::db::open_or_create(&path).unwrap();
            crate::db::write_references_enabled(&conn, true).unwrap();
        }
        let conn = crate::db::open_or_create(&path).unwrap();
        let svc = ShireService::new(conn, None);

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
            let conn = crate::db::open_or_create(&path).unwrap();
            crate::db::write_references_enabled(&conn, false).unwrap();
        }
        let conn = crate::db::open_or_create(&path).unwrap();
        let svc = ShireService::new(conn, None);

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
            let conn = crate::db::open_or_create(&path).unwrap();
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
        let conn = crate::db::open_or_create(&path).unwrap();
        let svc = ShireService::new(conn, None);

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
        let conn = crate::db::open_or_create(&dir.path().join("t.db")).unwrap();
        let svc = ShireService::new(conn, None);
        let args = SchemaConsumersArgs {
            path: "a.proto".into(),
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

    #[test]
    fn test_generated_from_empty_db() {
        let dir = tempfile::TempDir::new().unwrap();
        let conn = crate::db::open_or_create(&dir.path().join("t.db")).unwrap();
        let svc = ShireService::new(conn, None);
        let args = GeneratedFromArgs {
            path: "a.pb.go".into(),
        };
        let r = svc.generated_from(Parameters(args)).unwrap();
        let text = match &r.content.first().expect("content").raw {
            RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text"),
        };
        assert_eq!(text, "[]");
    }
}
