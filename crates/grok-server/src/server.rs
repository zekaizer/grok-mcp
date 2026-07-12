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
            "grok-mcp v{version} — SuperGrok / xAI Responses MCP: live X (x.com) search, \
             current web/news research, and cheap offline ask_grok \
             (tools: research, x_search, ask_grok, job_status, auth_status)"
        ));
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(format!(
            "grok-mcp v{version} offloads to xAI Grok (SuperGrok). \
             Routing: user asks about X posts, tweets, X accounts, or x.com discourse → call x_search (do not rely on host built-in search alone). \
             Current web/news or multi-source fact-check → research. Offline Q&A/critique → ask_grok (no live search). \
             Cost: ask_grok low; x_search mid; research expensive. \
             verbosity is summary|detailed|raw (not low/medium/high — that is reasoning_effort). \
             For long calls set timeout_secs (e.g. 60–120); if status=running, poll job_status with job_id. \
             On REAUTH_REQUIRED, tell the user to run: grok-mcp auth login."
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
    fn get_info_emphasizes_live_x_and_news() {
        let info = test_server().get_info();
        let desc = info.server_info.description.as_deref().unwrap_or("");
        let instructions = info.instructions.as_deref().unwrap_or("");

        assert!(desc.contains("live X") || desc.contains("x.com"), "desc={desc}");
        assert!(
            desc.contains("news") || instructions.contains("web/news"),
            "desc={desc} instr={instructions}"
        );
        assert!(instructions.contains("x_search"), "instr={instructions}");
        assert!(
            instructions.contains("X posts") || instructions.contains("tweets"),
            "routing must mention X posts/tweets: {instructions}"
        );
        assert!(
            instructions.contains("built-in") || instructions.contains("do not rely"),
            "must discourage skipping MCP for host search: {instructions}"
        );
        assert!(
            instructions.contains("no live search") || instructions.contains("ask_grok"),
            "instr={instructions}"
        );
        assert!(
            instructions.contains("REAUTH_REQUIRED") && instructions.contains("auth login"),
            "instr={instructions}"
        );
        assert!(
            !instructions.contains("only on REAUTH"),
            "drop over-strict 'only on' wording: {instructions}"
        );
    }

    #[test]
    fn x_search_description_targets_post_investigation() {
        let router = tools::router();
        let tool = router.get("x_search").expect("x_search registered");
        let d = tool.description.as_deref().unwrap_or("");
        assert!(d.contains("X (Twitter") || d.contains("x.com"), "desc={d}");
        assert!(
            d.contains("posts") || d.contains("tweets"),
            "must mention posts/tweets: {d}"
        );
        assert!(
            d.contains("ALWAYS") || d.contains("do not skip"),
            "must push host to call this tool: {d}"
        );
        assert!(
            d.to_lowercase().contains("built-in") || d.contains("host"),
            "must mention not using host search alone: {d}"
        );
    }

    #[test]
    fn research_description_defers_x_only_to_x_search() {
        let router = tools::router();
        let tool = router.get("research").expect("research registered");
        let d = tool.description.as_deref().unwrap_or("");
        assert!(
            d.contains("x_search"),
            "must point X-only work to x_search: {d}"
        );
    }
}
