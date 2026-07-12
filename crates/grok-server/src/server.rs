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
            "grok-mcp v{version} — SuperGrok / xAI Responses MCP bridge \
             (research, x_search, ask_grok, job_status, auth_status)"
        ));
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(format!(
            "grok-mcp v{version} offloads work to xAI Grok (SuperGrok subscription). \
             Tool choice: ask_grok = no search, low quota; x_search = X only; research = web/X multi-step (expensive). \
             verbosity is summary|detailed|raw (not low/medium/high — that is reasoning_effort). \
             For long calls set timeout_secs (e.g. 60–120); if status=running, poll job_status with job_id. \
             Access tokens refresh automatically; only on REAUTH_REQUIRED run: grok-mcp auth login (or auth import)."
        ));
        info
    }
}
