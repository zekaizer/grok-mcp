//! MCP tools (docs/tool_spec.md).

mod ask_grok;
mod auth_status;
mod job_status;
#[cfg(test)]
mod live_e2e;
mod research;
mod x_search;

use rmcp::handler::server::router::tool::ToolRouter;

use crate::GrokMcpServer;

/// Composed tool router.
pub fn router() -> ToolRouter<GrokMcpServer> {
    GrokMcpServer::auth_status_router()
        + GrokMcpServer::research_router()
        + GrokMcpServer::x_search_router()
        + GrokMcpServer::ask_grok_router()
        + GrokMcpServer::job_status_router()
}
