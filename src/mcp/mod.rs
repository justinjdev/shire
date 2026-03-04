pub mod prompts;
pub mod tools;

use crate::db;
use anyhow::Result;
use rmcp::{model::*, service::RequestContext, tool_handler, RoleServer, ServiceExt, ServerHandler};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Build context for on-demand reindexing. When present, the MCP server
/// can rebuild the index before answering queries.
#[derive(Clone)]
pub struct BuildContext {
    pub repo_root: PathBuf,
    pub config: crate::config::Config,
    pub db_path: PathBuf,
}

#[tool_handler]
impl ServerHandler for tools::ShireService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .build(),
            server_info: Implementation {
                name: "shire".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            instructions: Some(
                "Shire is a pre-built search index for codebases. It indexes packages, symbols \
                 (functions, classes, types, methods), files, and the dependency graph into SQLite \
                 with FTS5 full-text search.\n\n\
                 ## When to use Shire\n\n\
                 **Default to Shire for codebase search.** Index lookups are faster than scanning files \
                 with Grep/Glob and return structured results (symbol kind, signature, file path, line number).\n\n\
                 - `search_symbols` — find functions, classes, types by name or signature. Use instead of Grep \
                 for \"where is function X defined?\" or \"what functions match pattern Y?\"\n\
                 - `search_packages` — find packages by name or description\n\
                 - `search_files` — find files by path or name\n\
                 - `get_file_symbols` — list all symbols in a file (functions, classes, types). Use to understand \
                 a file's exports without reading the entire file\n\
                 - `list_package_files` — list files in a package, optionally filtered by extension\n\
                 - `explore` — broad semantic search across packages, symbols, and files for a concept. \
                 Returns a structured context map. Use when exploring unfamiliar code\n\n\
                 ## Dependency graph (unique to Shire)\n\n\
                 - `package_dependencies` / `package_dependents` — navigate the dependency graph. \
                 Set depth>1 on package_dependencies for transitive graph\n\n\
                 ## When to fall back to Grep/Glob\n\n\
                 Use Grep when searching for literal strings, log messages, or content inside function bodies. \
                 Shire indexes symbol definitions, not implementations."
                    .into(),
            ),
        }
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<ListPromptsResult, ErrorData>> + Send + '_ {
        std::future::ready(Ok(ListPromptsResult {
            prompts: prompts::list(),
            next_cursor: None,
        }))
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<GetPromptResult, ErrorData>> + Send + '_ {
        let result = (|| {
            let conn = self.conn.lock().map_err(|e| tools::ShireService::mcp_err(e.to_string()))?;
            let args: HashMap<String, String> = request
                .arguments
                .unwrap_or_default()
                .into_iter()
                .map(|(k, v)| {
                    let s = match v {
                        serde_json::Value::String(s) => s,
                        other => other.to_string(),
                    };
                    (k, s)
                })
                .collect();
            prompts::handle(&conn, &request.name, &args)
                .map_err(|e| match e {
                    prompts::PromptError::InvalidParams(msg) => ErrorData::invalid_params(msg, None),
                    prompts::PromptError::NotFound(msg) => ErrorData::resource_not_found(msg, None),
                    prompts::PromptError::Internal(msg) => ErrorData::internal_error(msg, None),
                })
        })();

        std::future::ready(result)
    }
}

pub async fn run_server(db_path: &Path, rag_config: &crate::config::RagConfig, build_ctx: Option<BuildContext>) -> Result<()> {
    let conn = if db_path.exists() {
        db::open_readonly(db_path)?
    } else {
        // On-demand mode with no DB yet — create an in-memory placeholder.
        // The first tool call will trigger a build and reopen the real DB.
        rusqlite::Connection::open_in_memory()?
    };
    let service = tools::ShireService::new(conn, rag_config, build_ctx);
    let server = service.serve(rmcp::transport::stdio()).await?;
    server.waiting().await?;
    Ok(())
}
