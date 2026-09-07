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
        // A read-only connection never migrates, so an index written by an
        // older release keeps serving its old FTS tables: no error, just
        // silently missing prefix/sub-token matching. Say so once at startup.
        if !crate::db::schema_is_current(&conn) {
            tracing::warn!(
                "index was built by an older shire and has not been migrated — \
                 symbol search will miss prefix and sub-token matches until you \
                 run `shire build`"
            );
        }

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
    /// There is exactly one mechanism here: once the debounce window has
    /// elapsed since the last index, re-check the working tree by running
    /// the incremental build. The build is its own freshness oracle — it
    /// compares file-tree hashes, per-package mtimes and per-file content
    /// hashes — and costs tens to a couple of hundred milliseconds when
    /// nothing changed, so "run it and let it decide" is both correct and
    /// cheap.
    ///
    /// Nothing cheaper is consulted first. The obvious candidate, the Git
    /// index mtime, was exactly wrong for this job: an ordinary edit to a
    /// tracked file never touches `.git/index`, so gating on it froze the
    /// served index at server start for the whole life of the process
    /// (INDEX-2-1) — while a *non*-Git directory, where the signal is
    /// simply absent, rebuilt correctly.
    ///
    /// The debounce keeps this from turning into a rebuild per tool call
    /// during a burst.
    fn is_stale(&self) -> bool {
        let ctx = match &self.build_ctx {
            Some(c) => c,
            None => return false, // read-only mode
        };

        let last = self.last_indexed.lock().ok().and_then(|g| *g);

        // No existing index — definitely stale
        let Some(last) = last else {
            return true;
        };

        // Debounce: skip the re-check if the last index completed within the
        // debounce window (default 5s, configurable via serve.debounce_s in
        // shire.toml). Prevents redundant rebuilds during rapid tool call
        // bursts. No changes are lost — the first check after the window
        // expires runs a build that reads current file state.
        let debounce = std::time::Duration::from_secs(ctx.config.serve.debounce_s);
        match last.elapsed() {
            Ok(elapsed) => elapsed >= debounce,
            // `indexed_at` is in the future (clock skew, or a DB built on
            // another machine): the window cannot be measured, and "unknown"
            // must not be served as "fresh".
            Err(_) => true,
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

    /// How many rows to actually fetch for a caller-visible `limit`: one
    /// more, whose presence proves further rows exist. Without the probe row
    /// a complete list that happens to fill the limit exactly is
    /// indistinguishable from a capped one, and every such answer carried a
    /// false "More may exist". At `MAX_ROWS` the probe cannot be fetched (the
    /// query layer's ceiling absorbs it), so a result filling the ceiling is
    /// reported as truncated.
    fn probe_limit(limit: u32) -> u32 {
        limit.saturating_add(1).min(queries::MAX_ROWS)
    }

    /// Serialize rows fetched with [`Self::probe_limit`] to JSON, cut back to
    /// `limit`, telling the model when rows were left behind. Without that a
    /// capped list is indistinguishable from a complete one and the model
    /// reasons about a package as if it had seen all of it.
    ///
    /// A complete list serializes as the bare JSON array it always was. A
    /// truncated one serializes as `{"results": [...], "truncated": true,
    /// ...}` — one content block either way, so concatenating a tool result's
    /// text blocks still yields parseable JSON.
    ///
    /// `narrow_hint` says how to make the result *smaller* (a filter, a
    /// tighter query); the "raise `limit`" half of the advice is added here,
    /// and only when raising it can actually help — see
    /// [`Self::truncation_advice`].
    fn json_result<T: serde::Serialize>(
        rows: &[T],
        limit: u32,
        narrow_hint: &str,
    ) -> Result<CallToolResult, ErrorData> {
        // A probe row we actually saw proves more rows exist. At the ceiling
        // there is no room for one, so a full result only *may* have been
        // cut — say so rather than asserting a truncation we cannot see.
        let over_limit = rows.len() as u32 > limit;
        let at_ceiling = limit >= queries::MAX_ROWS && rows.len() as u32 >= limit;
        let truncated = over_limit || at_ceiling;
        let shown = &rows[..rows.len().min(limit as usize)];
        let json = if truncated {
            serde_json::to_string(&TruncatedList {
                results: shown,
                truncated: true,
                limit,
                max: queries::MAX_ROWS,
                note: format!(
                    "showing the first {limit} results (limit={limit}, max {max}). \
                     {more} — {advice}.",
                    max = queries::MAX_ROWS,
                    more = if over_limit {
                        "More exist"
                    } else {
                        "`limit` is at the ceiling, so more may exist"
                    },
                    advice = Self::truncation_advice(limit, narrow_hint)
                ),
            })
        } else {
            serde_json::to_string(shown)
        }
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// What to tell the model to do about a truncated result.
    ///
    /// "Raise `limit`" is only advice while there is headroom: at
    /// `MAX_ROWS` [`Self::resolve_limit`] clamps a bigger request straight
    /// back down, so a model that follows it spends a second call to receive
    /// the identical rows and the identical note. Past the ceiling the only
    /// way forward is to narrow.
    fn truncation_advice(limit: u32, narrow_hint: &str) -> String {
        let max = queries::MAX_ROWS;
        if limit >= max {
            if narrow_hint.is_empty() {
                format!("`limit` cannot go above {max}, so narrow the request instead")
            } else {
                format!("`limit` cannot go above {max}; {narrow_hint}")
            }
        } else if narrow_hint.is_empty() {
            format!("raise `limit` (max {max})")
        } else {
            format!("raise `limit` (max {max}) or {narrow_hint}")
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

/// Envelope for a list that was cut at `limit`. Only truncated results are
/// wrapped — a complete list stays the bare array clients already parse.
#[derive(serde::Serialize)]
struct TruncatedList<'a, T: serde::Serialize> {
    results: &'a [T],
    truncated: bool,
    limit: u32,
    max: u32,
    note: String,
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
    /// Filter by package kind: "npm", "go", "cargo", "python", "maven", "gradle", "perl", "ruby", "nix"
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
    /// Max results (default 100, ceiling 200; 0 means "use the default")
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
    /// Max results (default 100, ceiling 200; 0 means "use the default")
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
    /// Max results (default 100, ceiling 200; 0 means "use the default")
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
    /// Max results per bucket (default 100, ceiling 200; 0 means "use the default")
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
        let results = queries::search_packages(&conn, &params.query, Self::probe_limit(limit))
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&results, limit, "use a more specific query")
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
                edges.truncate(Self::probe_limit(limit) as usize);
                Self::json_result(&edges, limit, "lower `depth`")
            }
            _ => {
                let limit = Self::resolve_limit(params.limit, queries::DEFAULT_LIST_LIMIT);
                let results = queries::package_dependencies(
                    &conn,
                    &params.name,
                    params.internal_only,
                    Self::probe_limit(limit),
                )
                .map_err(|e| Self::mcp_err(e.to_string()))?;
                Self::json_result(&results, limit, "")
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
        let results = queries::package_dependents(&conn, &params.name, Self::probe_limit(limit))
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&results, limit, "")
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
        let results =
            queries::list_packages(&conn, params.kind.as_deref(), Self::probe_limit(limit))
                .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&results, limit, "filter by `kind`")
    }

    #[tool(
        description = "Find functions, classes, types, methods by identifier or identifier prefix (not regex or substring). Every whitespace-separated token must match, by prefix and against identifier sub-tokens: 'handle' finds handleRequest, 'verify jwt' finds verifyJwtToken. Matches the symbol name and its sub-tokens only, never signatures or file paths. Use instead of Grep for 'where is function X?'. Omit `query` with a `package` filter to list that package's symbols in (file, line) order, capped at `limit`."
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
            let results = queries::get_package_symbols(
                &conn,
                pkg,
                params.kind.as_deref(),
                Self::probe_limit(limit),
            )
            .map_err(|e| Self::mcp_err(e.to_string()))?;
            // Ordered by (file_path, line), so a capped listing is the first
            // `limit` symbols of the alphabetically-first files — say so.
            return Self::json_result(
                &results,
                limit,
                "this is the start of the package in (file, line) order; \
                 narrow with `kind` or `get_file_symbols`",
            );
        }
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;

        let results = queries::search_symbols(
            &conn,
            query,
            params.package.as_deref(),
            params.kind.as_deref(),
            Self::probe_limit(limit),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;

        Self::json_result(&results, limit, "use a more specific query")
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
        let results = queries::get_file_symbols(
            &conn,
            &params.file_path,
            params.kind.as_deref(),
            Self::probe_limit(limit),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&results, limit, "filter by `kind`")
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
        let results = queries::list_package_files(
            &conn,
            &params.package,
            params.extension.as_deref(),
            Self::probe_limit(limit),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&results, limit, "filter by `extension`")
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
            Self::probe_limit(limit),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&results, limit, "use a more specific query")
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
        let results = queries::search_docs(
            &conn,
            &params.query,
            params.package.as_deref(),
            Self::probe_limit(limit),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&results, limit, "use a more specific query")
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
        description = "Find all references (call sites, type uses, imports, impl clauses) to a symbol by name. Use instead of Grep for 'who uses X?' — returns file, line, kind, and the dot-qualified enclosing symbol. Note: matches by name only, so two symbols with the same name cannot be distinguished."
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
        let limit = Self::resolve_limit(args.limit, queries::DEFAULT_LIST_LIMIT);
        let rows = queries::query_symbol_references(
            &conn,
            &args.name,
            args.kind.as_deref(),
            args.package.as_deref(),
            i64::from(Self::probe_limit(limit)),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&rows, limit, "filter by `package`/`kind`")
    }

    #[tool(
        description = "Find which symbols (functions, methods) call the named symbol. Returns the caller name, file, line of first call, and count of call sites. Navigates the call graph upward. `caller_name` is dot-qualified for methods (`AuthService.login`) and can be passed straight back in as `name`: a qualified name with no exact match falls back to its last segment."
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
        let limit = Self::resolve_limit(args.limit, queries::DEFAULT_LIST_LIMIT);
        let rows = queries::query_symbol_callers(
            &conn,
            &args.name,
            args.package.as_deref(),
            i64::from(Self::probe_limit(limit)),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&rows, limit, "filter by `package`")
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
        let limit = Self::resolve_limit(args.limit, queries::DEFAULT_LIST_LIMIT);
        let rows = queries::query_symbol_callees(
            &conn,
            &args.name,
            args.package.as_deref(),
            i64::from(Self::probe_limit(limit)),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&rows, limit, "filter by `package`")
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
        let limit = Self::resolve_limit(args.limit, queries::DEFAULT_LIST_LIMIT);
        // Buckets are filled with the probe row too: `summary` proves whether
        // the two ref buckets were cut, but nothing counts the transitive
        // walk, which simply stops at the cap — so a complete list of exactly
        // `limit` packages would otherwise be reported as truncated.
        let mut impact = queries::change_impact(
            &conn,
            &args.name,
            args.package.as_deref(),
            depth,
            i64::from(Self::probe_limit(limit)),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        // The payload is an object, not a list, so the truncation marker goes
        // on it as extra fields.
        let over = |n: usize| n as u32 > limit || (limit >= queries::MAX_ROWS && n as u32 >= limit);
        let transitive_capped = over(impact.transitive_impact.len());
        let truncated = impact.summary.direct_count as u32 > limit
            || impact.summary.cross_package_count as u32 > limit
            || transitive_capped;
        impact.direct_impact.truncate(limit as usize);
        impact.cross_package_impact.truncate(limit as usize);
        impact.transitive_impact.truncate(limit as usize);
        // `direct_count`/`cross_package_count` are true totals; the
        // transitive count is not (the walk stops at the cap), so keep it
        // consistent with the rows actually returned — `truncated` is what
        // says more exist.
        impact.summary.transitive_package_count = impact.transitive_impact.len();
        let mut value = serde_json::to_value(&impact).map_err(|e| Self::mcp_err(e.to_string()))?;
        if let Some(obj) = value.as_object_mut()
            && truncated
        {
            obj.insert("truncated".into(), serde_json::Value::Bool(true));
            obj.insert("limit".into(), serde_json::Value::from(limit));
            obj.insert("max".into(), serde_json::Value::from(queries::MAX_ROWS));
            obj.insert(
                "note".into(),
                serde_json::Value::from(format!(
                    "each impact bucket is capped at {limit} rows (max {max}); \
                     `summary.direct_count` and `summary.cross_package_count` are true \
                     totals, but {transitive} — {advice}.",
                    max = queries::MAX_ROWS,
                    advice = Self::truncation_advice(limit, "pass `package`"),
                    transitive = if transitive_capped {
                        "the transitive walk stopped at the cap, so \
                         `summary.transitive_package_count` is a floor, \
                         not a total"
                    } else {
                        "`summary.transitive_package_count` counts only the rows returned"
                    }
                )),
            );
        }
        let json = serde_json::to_string(&value).map_err(|e| Self::mcp_err(e.to_string()))?;
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
        let rows = queries::query_schema_consumers(&conn, &args.path, Self::probe_limit(limit))
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&rows, limit, "")
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
        let rows = queries::query_generated_from(&conn, &args.path, Self::probe_limit(limit))
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Self::json_result(&rows, limit, "")
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
    fn test_is_stale_true_after_debounce_window_in_a_git_repo() {
        // INDEX-2-1: an ordinary working-tree edit never touches
        // `.git/index`, so a Git repository whose index file is old (or
        // never written) must still be re-checked once the debounce window
        // has passed. Gating on the Git index mtime froze `serve --root` at
        // the index it started with.
        let dir = tempfile::TempDir::new().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        // A Git index that is *older* than the last build: under the old
        // oracle this read as "nothing changed", forever.
        std::fs::write(git_dir.join("index"), "dummy").unwrap();

        let svc = make_service_with_ctx(dir.path().to_path_buf());
        *svc.last_indexed.lock().unwrap() = Some(SystemTime::now() - Duration::from_secs(60));

        assert!(
            svc.is_stale(),
            "past the debounce window the working tree must be re-checked, \
             whatever .git/index says"
        );
    }

    #[test]
    fn test_is_stale_true_when_no_git_index() {
        // A non-Git directory is treated exactly like a Git one: the build
        // itself is the freshness oracle.
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
    fn test_is_stale_false_inside_debounce_window() {
        // The one thing that suppresses a re-check: a build that finished
        // less than `serve.debounce_s` ago.
        let dir = tempfile::TempDir::new().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("index"), "dummy").unwrap();

        let svc = make_service_with_ctx(dir.path().to_path_buf());
        *svc.last_indexed.lock().unwrap() = Some(SystemTime::now());
        assert!(
            !svc.is_stale(),
            "a rebuild inside the debounce window must not trigger another"
        );
    }

    #[test]
    fn test_is_stale_true_when_indexed_at_is_in_the_future() {
        // Clock skew (or a DB built on another machine) makes the window
        // unmeasurable; "unknown" must not be served as "fresh".
        let dir = tempfile::TempDir::new().unwrap();
        let svc = make_service_with_ctx(dir.path().to_path_buf());
        *svc.last_indexed.lock().unwrap() = Some(SystemTime::now() + Duration::from_secs(600));
        assert!(svc.is_stale(), "an unmeasurable window must read as stale");
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

    /// A service whose index has `n` `target` call-refs, each from its own
    /// enclosing symbol, plus `n` calls made *by* `caller`.
    fn service_with_refs(dir: &std::path::Path, n: usize) -> ShireService {
        let path = dir.join("refs.db");
        {
            let conn = crate::db::open_or_create(&path).unwrap();
            crate::db::write_references_enabled(&conn, true).unwrap();
            conn.execute(
                "INSERT INTO packages (name, path, kind) VALUES ('p','p','rust')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO symbols (package, name, kind, file_path, line) \
                 VALUES ('p','target','function','p/t.rs',1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO files (path, package, extension, size_bytes) VALUES ('p/t.rs','p','rs',0)",
                [],
            )
            .unwrap();
            let file_id: i64 = conn
                .query_row("SELECT id FROM files WHERE path='p/t.rs'", [], |r| r.get(0))
                .unwrap();
            for i in 0..n {
                conn.execute(
                    "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) \
                     VALUES ('target','call',?1,?2,'p',?3)",
                    rusqlite::params![file_id, i as i64, format!("c{i}")],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) \
                     VALUES (?1,'call',?2,?3,'p','caller')",
                    rusqlite::params![format!("callee{i}"), file_id, 100 + i as i64],
                )
                .unwrap();
            }
        }
        let conn = crate::db::open_or_create(&path).unwrap();
        ShireService::new(conn, None)
    }

    /// The four reference tools rolled their own
    /// `limit.unwrap_or(100).clamp(1, 1000)` and returned a bare
    /// `Content::text`, so `limit: 0` answered "one caller" and a capped list
    /// read as the complete blast radius. They go through `resolve_limit` /
    /// `json_result` like every other list tool now.
    #[test]
    fn test_reference_tools_use_the_shared_limit_helpers() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = service_with_refs(dir.path(), 5);

        let refs = |limit: Option<u32>| {
            svc.symbol_references(Parameters(SymbolRefsArgs {
                name: "target".into(),
                kind: None,
                package: None,
                limit,
            }))
            .unwrap()
        };
        let callers = |limit: Option<u32>| {
            svc.symbol_callers(Parameters(SymbolCallersArgs {
                name: "target".into(),
                package: None,
                limit,
            }))
            .unwrap()
        };
        let callees = |limit: Option<u32>| {
            svc.symbol_callees(Parameters(SymbolCalleesArgs {
                name: "caller".into(),
                package: None,
                limit,
            }))
            .unwrap()
        };

        // `limit: 0` means "no cap" to plenty of clients; it used to clamp to
        // one row, i.e. "this symbol has exactly one reference".
        for r in [refs(Some(0)), callers(Some(0)), callees(Some(0))] {
            assert_eq!(result_rows(&r).len(), 5);
            assert_eq!(truncation_note(&r), None, "complete list, no note");
        }
        for r in [refs(None), callers(None), callees(None)] {
            assert_eq!(result_rows(&r).len(), 5);
        }

        // A capped list says so, in the same envelope as every other tool.
        for r in [refs(Some(2)), callers(Some(2)), callees(Some(2))] {
            assert_eq!(result_rows(&r).len(), 2);
            let note = truncation_note(&r).expect("truncation note");
            assert!(note.contains("first 2 results"), "got {note}");
            assert_eq!(r.content.len(), 1, "one parseable JSON block");
        }

        // Exactly-full complete lists stay unmarked.
        for r in [refs(Some(5)), callers(Some(5)), callees(Some(5))] {
            assert_eq!(result_rows(&r).len(), 5);
            assert_eq!(truncation_note(&r), None);
        }

        // The ref tools used to accept up to 1000 rows; they share the
        // MAX_ROWS ceiling now.
        assert_eq!(
            ShireService::resolve_limit(Some(1000), queries::DEFAULT_LIST_LIMIT),
            queries::MAX_ROWS
        );
    }

    /// `change_impact` returns an object rather than a list, so its
    /// truncation marker rides on the object; `summary` keeps the true
    /// counts either way.
    #[test]
    fn test_change_impact_marks_truncated_buckets() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = service_with_refs(dir.path(), 5);

        let call = |limit: Option<u32>| -> serde_json::Value {
            let r = svc
                .change_impact(Parameters(ChangeImpactArgs {
                    name: "target".into(),
                    package: None,
                    transitive_depth: Some(1),
                    limit,
                }))
                .unwrap();
            assert_eq!(r.content.len(), 1);
            serde_json::from_str(&result_text(&r)).expect("valid JSON")
        };

        let v = call(Some(2));
        assert_eq!(v["direct_impact"].as_array().unwrap().len(), 2);
        assert_eq!(v["summary"]["direct_count"], 5);
        assert_eq!(v["truncated"], serde_json::Value::Bool(true));
        assert!(v["note"].as_str().unwrap().contains("summary"));

        // Not truncated: no marker, and `limit: 0` is the tool default, not
        // a one-row answer.
        for v in [call(Some(5)), call(Some(0)), call(None)] {
            assert_eq!(v["direct_impact"].as_array().unwrap().len(), 5);
            assert_eq!(v["summary"]["direct_count"], 5);
            assert!(v.get("truncated").is_none(), "unexpected marker in {v}");
        }
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

    /// Rows of a list tool's result, whether it came back as the bare array
    /// (complete) or as the `{results, truncated, …}` envelope (truncated).
    fn result_rows(r: &CallToolResult) -> Vec<serde_json::Value> {
        let v: serde_json::Value = serde_json::from_str(&result_text(r)).unwrap();
        match v {
            serde_json::Value::Array(rows) => rows,
            serde_json::Value::Object(ref o) => o
                .get("results")
                .and_then(|r| r.as_array())
                .unwrap_or_else(|| panic!("no results array in {v}"))
                .clone(),
            other => panic!("unexpected result shape: {other}"),
        }
    }

    /// The truncation note of a list tool's result, when it has one.
    fn truncation_note(r: &CallToolResult) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(&result_text(r)).unwrap();
        let note = v.get("note")?.as_str()?.to_string();
        assert_eq!(v.get("truncated"), Some(&serde_json::Value::Bool(true)));
        Some(note)
    }

    /// A service over a real (on-disk) schema, with `n` symbols in one
    /// package spread over `n` files.
    fn service_with_symbols(dir: &std::path::Path, n: usize) -> ShireService {
        let path = dir.join("t.db");
        {
            let conn = crate::db::open_or_create(&path).unwrap();
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
        let conn = crate::db::open_or_create(&path).unwrap();
        ShireService::new(conn, None)
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
        assert_eq!(result_rows(&r).len(), 5, "limit must be honored");
        // …and the model must be told the list was cut, inside the one JSON
        // block, so the whole tool output stays parseable.
        assert_eq!(r.content.len(), 1, "the note rides in the JSON envelope");
        let note = truncation_note(&r).expect("expected a truncation note");
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
        assert_eq!(result_rows(&r).len(), 20);

        // The hard ceiling wins over an absurd request.
        let r = svc
            .search_symbols(Parameters(SearchSymbolsParams {
                query: None,
                package: Some("pkg".into()),
                kind: None,
                limit: Some(100_000),
            }))
            .unwrap();
        assert_eq!(
            result_rows(&r).len(),
            queries::MAX_ROWS as usize,
            "capped at MAX_ROWS"
        );
    }

    /// MCP-5 / DB-5: the other list-returning tools are bounded too.
    #[test]
    fn test_list_tools_are_bounded() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = service_with_symbols(dir.path(), 300);

        let len = |r: &CallToolResult| -> usize { result_rows(r).len() };

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
        assert_eq!(result_rows(&r).len(), 20);
    }

    /// The note must not assert a truncation the probe row never proved. At
    /// `MAX_ROWS` there is no room to fetch the probe, so a result filling the
    /// ceiling is flagged — but "more exist" would be a guess, and it points
    /// the model at a `limit` it cannot raise.
    #[test]
    fn test_ceiling_note_says_more_may_exist() {
        let dir = tempfile::TempDir::new().unwrap();
        let svc = service_with_symbols(dir.path(), queries::MAX_ROWS as usize);

        // Exactly MAX_ROWS files, asked for MAX_ROWS: the probe cannot be
        // fetched, so the cut is unproven.
        let r = svc
            .list_package_files(Parameters(ListPackageFilesParams {
                package: "pkg".into(),
                extension: None,
                limit: Some(queries::MAX_ROWS),
            }))
            .unwrap();
        assert_eq!(result_rows(&r).len(), queries::MAX_ROWS as usize);
        let note = truncation_note(&r).expect("ceiling is still flagged");
        assert!(note.contains("may exist"), "got {note}");
        // …and it must not send the model back for an identical second call:
        // `resolve_limit` clamps anything above the ceiling straight down.
        assert!(
            !note.contains("raise `limit`"),
            "advice at the ceiling must not be to raise it: {note}"
        );

        // Below the ceiling the probe row is real proof.
        let r = svc
            .list_package_files(Parameters(ListPackageFilesParams {
                package: "pkg".into(),
                extension: None,
                limit: Some(10),
            }))
            .unwrap();
        let note = truncation_note(&r).expect("truncated");
        assert!(note.contains("More exist"), "got {note}");
        assert!(note.contains("raise `limit`"), "got {note}");
    }

    /// A complete list gets no truncation note — the note must mean
    /// something, including when the list happens to fill the limit exactly.
    #[test]
    fn test_no_truncation_note_when_the_list_is_complete() {
        let dir = tempfile::TempDir::new().unwrap();
        // One package, two symbols.
        let svc = service_with_symbols(dir.path(), 1);
        let r = svc
            .list_packages(Parameters(ListParams {
                kind: None,
                limit: None,
            }))
            .unwrap();
        assert_eq!(r.content.len(), 1, "no note for a complete list");
        assert_eq!(result_text(&r).chars().next(), Some('['), "bare array");

        // Exactly-full and complete: 2 symbols, limit 2. The old
        // `rows.len() >= limit` test called this truncated.
        let r = svc
            .search_symbols(Parameters(SearchSymbolsParams {
                query: None,
                package: Some("pkg".into()),
                kind: None,
                limit: Some(2),
            }))
            .unwrap();
        assert_eq!(result_rows(&r).len(), 2);
        assert_eq!(truncation_note(&r), None, "complete list, no note");

        // One more row exists → the note, and the whole output is still one
        // parseable JSON document.
        let r = svc
            .search_symbols(Parameters(SearchSymbolsParams {
                query: None,
                package: Some("pkg".into()),
                kind: None,
                limit: Some(1),
            }))
            .unwrap();
        assert_eq!(result_rows(&r).len(), 1);
        assert!(
            truncation_note(&r)
                .expect("truncated")
                .contains("first 1 results")
        );
        assert_eq!(r.content.len(), 1);
        serde_json::from_str::<serde_json::Value>(
            &r.content
                .iter()
                .map(|c| match &c.raw {
                    RawContent::Text(t) => t.text.clone(),
                    _ => panic!("expected text"),
                })
                .collect::<String>(),
        )
        .expect("concatenated content blocks must parse as JSON");
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
        let conn = crate::db::open_or_create(&dir.path().join("t.db")).unwrap();
        let svc = ShireService::new(conn, None);
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
