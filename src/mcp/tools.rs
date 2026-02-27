use crate::db::queries;
use rmcp::{
    handler::server::{router::tool::ToolRouter, tool::Parameters},
    model::*,
    schemars, tool, tool_router,
};
use rusqlite::Connection;
use serde::Deserialize;
use std::borrow::Cow;
use std::sync::Mutex;

#[derive(Debug)]
pub struct ShireService {
    pub(crate) conn: Mutex<Connection>,
    pub tool_router: ToolRouter<ShireService>,
}

impl ShireService {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn: Mutex::new(conn),
            tool_router: Self::tool_router(),
        }
    }

    pub(crate) fn mcp_err(msg: String) -> ErrorData {
        ErrorData {
            code: ErrorCode(-32603),
            message: Cow::from(msg),
            data: None,
        }
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
        if params.query.trim().is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "Search query must not be empty",
            )]));
        }
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let results = queries::search_symbols(
            &conn,
            &params.query,
            params.package.as_deref(),
            params.kind.as_deref(),
        )
        .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List all symbols in a package. Useful for understanding a package's public API — its exported functions, classes, types, and methods.")]
    fn get_package_symbols(
        &self,
        Parameters(params): Parameters<GetPackageSymbolsParams>,
    ) -> Result<CallToolResult, ErrorData> {
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
        let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
        let status = queries::index_status(&conn)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        let json = serde_json::to_string_pretty(&status)
            .map_err(|e| Self::mcp_err(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}
