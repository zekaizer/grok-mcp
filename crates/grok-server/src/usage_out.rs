//! Host-facing usage block shared by generative tools.

use std::collections::BTreeMap;

use grok_client::{ResponseBody, UsageReport, log_usage, usage_report};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct UsageOut {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
    /// Successful server-side tool invocations (xAI), when provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_server_side_tools_used: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_sources_used: Option<u64>,
    /// e.g. `SERVER_SIDE_TOOL_WEB_SEARCH` → count (billable successes when present).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub server_side_tool_usage: BTreeMap<String, u64>,
    /// Counts of Responses `output[].type` tool-call items (web_search_call, …).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub output_tool_call_counts: BTreeMap<String, u64>,
}

impl From<UsageReport> for UsageOut {
    fn from(r: UsageReport) -> Self {
        Self {
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            reasoning_tokens: r.reasoning_tokens,
            total_tokens: r.total_tokens,
            cached_tokens: r.cached_tokens,
            num_server_side_tools_used: r.num_server_side_tools_used,
            num_sources_used: r.num_sources_used,
            server_side_tool_usage: r.server_side_tool_usage,
            output_tool_call_counts: r.output_tool_call_counts,
        }
    }
}

/// Build host usage + emit structured log (metrics only).
pub fn usage_out_and_log(tool: &str, model: &str, body: &ResponseBody) -> UsageOut {
    let report = usage_report(body);
    log_usage(tool, model, &report);
    report.into()
}
