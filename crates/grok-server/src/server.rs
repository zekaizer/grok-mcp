//! MCP server handler and tool registration.

use std::path::PathBuf;
use std::sync::Arc;

use grok_client::GrokClient;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool_handler};

use crate::jobs::JobStore;
use crate::tools;

/// Shared state for tool handlers.
#[derive(Clone)]
pub struct GrokMcpServer {
    pub(crate) auth_file: Option<PathBuf>,
    pub(crate) client: Arc<GrokClient>,
    pub(crate) jobs: JobStore,
    tool_router: ToolRouter<Self>,
}

impl GrokMcpServer {
    /// Build a server with the full tool set.
    pub fn new(auth_file: Option<PathBuf>, client: GrokClient) -> Self {
        Self {
            auth_file,
            client: Arc::new(client),
            jobs: JobStore::new(),
            tool_router: tools::router(),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for GrokMcpServer {
    fn get_info(&self) -> ServerInfo {
        let version = env!("CARGO_PKG_VERSION");
        let mut info = ServerInfo::default();
        info.server_info.name = "grok-mcp".into();
        info.server_info.version = version.into();
        info.server_info.title = Some(format!("grok-mcp v{version}"));
        info.server_info.description = Some(format!(
            "grok-mcp v{version} — SuperGrok / xAI Responses MCP: live X (x.com) with digest \
             or full-post evidence (no host x.com fetch needed), web/news research, offline ask_grok \
             (tools: research, x_search, ask_grok, job_status, auth_status)"
        ));
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(format!(
            "grok-mcp v{version} offloads to xAI Grok (SuperGrok).\n\
             Routing: X posts/tweets/x.com discourse → x_search (do not use host built-in search alone; hosts often cannot open x.com).\n\
             Exact wording/quotes → x_search with result=evidence (or both). Digest/sentiment → result=digest (default).\n\
             Web/news/multi-source → research. Offline Q&A → ask_grok (no live search).\n\
             depth=quick|standard|deep (cost/exploration). result=digest|evidence|both (fidelity).\n\
             Cost: ask_grok low; x_search mid–high (evidence higher); research high.\n\
             Long calls: timeout_secs then poll job_status (max 10 concurrent jobs; over the cap → retryable RATE_LIMITED). On REAUTH_REQUIRED tell user: grok-mcp auth login."
        ));
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grok_client::ClientConfig;
    use rmcp::ServerHandler;

    fn test_server() -> GrokMcpServer {
        let client = GrokClient::new(ClientConfig::default()).expect("client");
        GrokMcpServer::new(None, client)
    }

    #[test]
    fn get_info_v2_routing() {
        let info = test_server().get_info();
        let desc = info.server_info.description.as_deref().unwrap_or("");
        let instructions = info.instructions.as_deref().unwrap_or("");

        assert!(desc.contains("evidence") || desc.contains("x.com"), "desc={desc}");
        assert!(instructions.contains("result=evidence"), "instr={instructions}");
        assert!(
            instructions.contains("cannot open x.com") || instructions.contains("x.com"),
            "instr={instructions}"
        );
        assert!(!instructions.contains("verbosity"), "verbosity removed: {instructions}");
        assert!(
            instructions.contains("REAUTH_REQUIRED") && instructions.contains("auth login"),
            "instr={instructions}"
        );
    }

    #[test]
    fn x_search_description_mentions_evidence() {
        let router = tools::router();
        let tool = router.get("x_search").expect("x_search registered");
        let d = tool.description.as_deref().unwrap_or("");
        assert!(d.contains("evidence"), "desc={d}");
        assert!(
            d.contains("NOT a bit-perfect") || d.contains("best-effort"),
            "fidelity warning must be in description: {d}"
        );
        assert!(
            d.contains("x.com") || d.contains("X posts") || d.contains("X (Twitter"),
            "desc={d}"
        );
    }

    #[test]
    fn research_description_defers_x_only() {
        let router = tools::router();
        let tool = router.get("research").expect("research registered");
        let d = tool.description.as_deref().unwrap_or("");
        assert!(d.contains("x_search"), "desc={d}");
        assert!(!d.contains("verbosity"), "desc={d}");
    }
}
