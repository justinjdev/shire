pub mod prompts;
pub mod tools;

use crate::db;
use anyhow::Result;
use rmcp::{
    RoleServer, ServerHandler, ServiceExt, model::*, service::RequestContext, tool_handler,
};
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
                "Shire is a pre-built search index for this codebase. Use Shire tools instead of \
                 Grep/Glob for codebase search — they return structured results (symbol kind, signature, \
                 file path, line number) instantly from an index, no file scanning needed.\n\n\
                 ## Tool selection guide\n\n\
                 | Task | Use this | Instead of |\n\
                 |------|----------|------------|\n\
                 | Find a function/class/type | `search_symbols` | Grep |\n\
                 | Find a file by name | `search_files` | Glob/find |\n\
                 | Find a package | `search_packages` | Grep |\n\
                 | Search documentation | `search_docs` | Grep/reading docs |\n\
                 | List symbols in a file | `get_file_symbols` | Reading the file |\n\
                 | List files in a package | `list_package_files` | Glob |\n\
                 | Explore a concept | `explore` | Grep |\n\
                 | Check what depends on X | `package_dependents` | Grep for imports |\n\
                 | Check what X depends on | `package_dependencies` | Reading manifests |\n\n\
                 ## Fall back to Grep/Glob when\n\n\
                 - Searching for literal strings, log messages, or error text inside function bodies\n\
                 - Shire indexes definitions, not implementations — use Grep for content within functions\n\
                 - You need regex or substring matching: search tools match identifiers by prefix \
                 (and by camelCase/snake_case sub-token for symbol names), so `handle` finds \
                 `handleRequest` but `andleRequ` finds nothing"
                    .into(),
            ),
        }
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<ListPromptsResult, ErrorData>> + Send + '_
    {
        std::future::ready(Ok(ListPromptsResult {
            prompts: prompts::list(),
            next_cursor: None,
        }))
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<GetPromptResult, ErrorData>> + Send + '_
    {
        let result = (|| {
            let conn = self
                .conn
                .lock()
                .map_err(|e| tools::ShireService::mcp_err(e.to_string()))?;
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
            prompts::handle(&conn, &request.name, &args).map_err(|e| match e {
                prompts::PromptError::InvalidParams(msg) => ErrorData::invalid_params(msg, None),
                prompts::PromptError::NotFound(msg) => ErrorData::resource_not_found(msg, None),
                prompts::PromptError::Internal(msg) => ErrorData::internal_error(msg, None),
            })
        })();

        std::future::ready(result)
    }
}

pub async fn run_server(db_path: &Path, build_ctx: Option<BuildContext>) -> Result<()> {
    let conn = if db_path.exists() {
        db::open_readonly(db_path)?
    } else {
        // On-demand mode with no DB yet — create an in-memory placeholder.
        // The first tool call will trigger a build and reopen the real DB.
        rusqlite::Connection::open_in_memory()?
    };
    let service = tools::ShireService::new(conn, build_ctx);
    let server = service.serve(rmcp::transport::stdio()).await?;
    server.waiting().await?;
    Ok(())
}
