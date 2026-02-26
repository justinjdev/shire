pub mod tools;

use crate::db;
use anyhow::Result;
use rmcp::{model::*, tool_handler, ServiceExt, ServerHandler};
use std::path::Path;

#[tool_handler]
impl ServerHandler for tools::ShireService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "shire".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            instructions: Some(
                "Shire indexes monorepo packages and their dependency graph. \
                 Use search_packages to find packages, package_dependencies/package_dependents \
                 to navigate the graph, and dependency_graph for transitive lookups."
                    .into(),
            ),
        }
    }
}

pub async fn run_server(db_path: &Path) -> Result<()> {
    let conn = db::open_readonly(db_path)?;
    let service = tools::ShireService::new(conn);
    let server = service.serve(rmcp::transport::stdio()).await?;
    server.waiting().await?;
    Ok(())
}
