pub mod prompts;
pub mod tools;

use crate::db;
use anyhow::Result;
use rmcp::{model::*, service::RequestContext, tool_handler, RoleServer, ServiceExt, ServerHandler};
use std::collections::HashMap;
use std::path::Path;

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
                "Shire indexes monorepo packages and their dependency graph. \
                 Use search_packages to find packages, package_dependencies/package_dependents \
                 to navigate the graph, and dependency_graph for transitive lookups. \
                 Use prompts for semantic codebase exploration: 'explore' a concept, \
                 'onboard' to get a repo overview, or 'impact-analysis' to understand blast radius."
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

pub async fn run_server(db_path: &Path) -> Result<()> {
    let conn = db::open_readonly(db_path)?;
    let service = tools::ShireService::new(conn);
    let server = service.serve(rmcp::transport::stdio()).await?;
    server.waiting().await?;
    Ok(())
}
