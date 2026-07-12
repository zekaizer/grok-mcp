//! xAI Responses API (`POST /v1/responses`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::GrokClient;
use crate::error::ClientError;

/// Request body for create response (subset we use).
#[derive(Debug, Clone, Serialize)]
pub struct CreateResponseRequest {
    pub model: String,
    pub input: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningParam>,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReasoningParam {
    pub effort: String,
}

/// Parsed subset of the Responses API body.
#[derive(Debug, Clone, Deserialize)]
pub struct ResponseBody {
    pub id: Option<String>,
    pub model: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub output: Vec<Value>,
    pub usage: Option<Usage>,
    /// Billable successful server-side tool counts (xAI agentic responses).
    #[serde(default)]
    pub server_side_tool_usage: Option<Value>,
    /// Attempted tool calls (when present on the response root).
    #[serde(default)]
    pub tool_calls: Option<Value>,
    pub error: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub input_tokens_details: Option<Value>,
    #[serde(default)]
    pub output_tokens_details: Option<Value>,
    #[serde(default)]
    pub num_server_side_tools_used: Option<u64>,
    #[serde(default)]
    pub num_sources_used: Option<u64>,
}

impl Usage {
    #[must_use]
    pub fn reasoning_tokens(&self) -> Option<u64> {
        self.output_tokens_details
            .as_ref()
            .and_then(|v| v.get("reasoning_tokens"))
            .and_then(|v| v.as_u64())
    }

    #[must_use]
    pub fn cached_tokens(&self) -> Option<u64> {
        self.input_tokens_details
            .as_ref()
            .and_then(|v| {
                v.get("cached_tokens")
                    .or_else(|| v.get("cached_prompt_text_tokens"))
            })
            .and_then(|v| v.as_u64())
    }
}

/// Host-facing + log-friendly usage metrics (no response body content).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageReport {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_server_side_tools_used: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_sources_used: Option<u64>,
    /// Successful tool categories from `server_side_tool_usage` (e.g. WEB_SEARCH: 2).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub server_side_tool_usage: BTreeMap<String, u64>,
    /// Counts of `output[].type` that look like tool calls (web_search_call, …).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub output_tool_call_counts: BTreeMap<String, u64>,
}

/// Build a usage report from a Responses body (metrics only).
#[must_use]
pub fn usage_report(body: &ResponseBody) -> UsageReport {
    let usage = body.usage.as_ref();
    let mut report = UsageReport {
        input_tokens: usage.and_then(|u| u.input_tokens).unwrap_or(0),
        output_tokens: usage.and_then(|u| u.output_tokens).unwrap_or(0),
        reasoning_tokens: usage.and_then(|u| u.reasoning_tokens()).unwrap_or(0),
        total_tokens: usage.and_then(|u| u.total_tokens),
        cached_tokens: usage.and_then(|u| u.cached_tokens()),
        num_server_side_tools_used: usage.and_then(|u| u.num_server_side_tools_used),
        num_sources_used: usage.and_then(|u| u.num_sources_used),
        server_side_tool_usage: BTreeMap::new(),
        output_tool_call_counts: count_output_tool_calls(&body.output),
    };

    if let Some(map) = body.server_side_tool_usage.as_ref() {
        report.server_side_tool_usage = value_to_count_map(map);
    }

    // If root field missing, some payloads put counts only under usage — already handled.
    // Fall back: if num_server_side_tools_used unset, sum output tool call counts.
    if report.num_server_side_tools_used.is_none() {
        let sum: u64 = report.output_tool_call_counts.values().sum();
        if sum > 0 {
            report.num_server_side_tools_used = Some(sum);
        }
    }

    report
}

/// Count Responses `output` items that are server-side tool call records.
#[must_use]
pub fn count_output_tool_calls(output: &[Value]) -> BTreeMap<String, u64> {
    let mut map = BTreeMap::new();
    for item in output {
        let Some(ty) = item.get("type").and_then(|v| v.as_str()) else {
            continue;
        };
        if is_tool_call_output_type(ty) {
            *map.entry(ty.to_string()).or_insert(0) += 1;
        }
    }
    map
}

#[must_use]
fn is_tool_call_output_type(ty: &str) -> bool {
    matches!(
        ty,
        "web_search_call"
            | "x_search_call"
            | "code_interpreter_call"
            | "file_search_call"
            | "mcp_call"
            | "image_generation_call"
            | "function_call"
    ) || ty.ends_with("_call") && ty != "function_call_output"
}

fn value_to_count_map(v: &Value) -> BTreeMap<String, u64> {
    let mut map = BTreeMap::new();
    let Some(obj) = v.as_object() else {
        return map;
    };
    for (k, val) in obj {
        let n = val
            .as_u64()
            .or_else(|| val.as_i64().map(|i| i.max(0) as u64))
            .or_else(|| val.as_f64().map(|f| f.max(0.0) as u64));
        if let Some(n) = n {
            map.insert(k.clone(), n);
        }
    }
    map
}

/// Log usage metrics only (no prompt/response body).
pub fn log_usage(tool: &str, model: &str, report: &UsageReport) {
    tracing::info!(
        target: "grok_client::usage",
        tool,
        model,
        input_tokens = report.input_tokens,
        output_tokens = report.output_tokens,
        reasoning_tokens = report.reasoning_tokens,
        total_tokens = report.total_tokens,
        cached_tokens = report.cached_tokens,
        num_server_side_tools_used = report.num_server_side_tools_used,
        num_sources_used = report.num_sources_used,
        server_side_tool_usage = ?report.server_side_tool_usage,
        output_tool_call_counts = ?report.output_tool_call_counts,
        "xai responses usage"
    );
}

impl GrokClient {
    /// Call `POST {base}/responses` with a bearer access token.
    pub async fn create_response(
        &self,
        access_token: &str,
        request: &CreateResponseRequest,
    ) -> Result<ResponseBody, ClientError> {
        let url = format!("{}/responses", self.config.base_url.trim_end_matches('/'));
        let resp = self
            .http
            .post(&url)
            .bearer_auth(access_token)
            .json(request)
            .send()
            .await
            .map_err(ClientError::Request)?;

        let status = resp.status();
        let bytes = resp.bytes().await.map_err(ClientError::Request)?;

        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes).into_owned();
            let snippet: String = body.chars().take(500).collect();
            return Err(ClientError::Upstream {
                status: status.as_u16(),
                body: snippet,
            });
        }

        serde_json::from_slice(&bytes).map_err(ClientError::Decode)
    }
}

/// Concatenate all `output_text` fragments from a Responses `output` array.
#[must_use]
pub fn extract_output_text(body: &ResponseBody) -> String {
    let mut parts: Vec<String> = Vec::new();
    for item in &body.output {
        let ty = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if ty != "message" {
            continue;
        }
        let Some(content) = item.get("content").and_then(|v| v.as_array()) else {
            continue;
        };
        for block in content {
            let bty = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if (bty == "output_text" || bty == "text")
                && let Some(t) = block.get("text").and_then(|v| v.as_str())
            {
                parts.push(t.to_string());
            }
        }
    }
    parts.join("\n")
}

/// Truncate string to at most `max_chars` (char boundary).
#[must_use]
pub fn truncate_chars(s: &str, max_chars: usize) -> (String, bool) {
    if s.chars().count() <= max_chars {
        return (s.to_string(), false);
    }
    let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    (format!("{truncated}…"), true)
}

/// Verbosity → soft character budget for host-facing text (tool_spec).
#[must_use]
pub fn verbosity_char_budget(verbosity: &str) -> usize {
    match verbosity {
        "raw" => 32 * 1024,
        "detailed" => 16 * 1024,
        _ => 4 * 1024, // summary default
    }
}

/// Try to parse a JSON object from model text (strip fences if present).
#[must_use]
pub fn parse_json_object(text: &str) -> Option<Value> {
    let t = text.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .map(|s| s.strip_suffix("```").unwrap_or(s).trim())
        .unwrap_or(t);
    let start = t.find('{')?;
    let end = t.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&t[start..=end]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_text_from_message() {
        let body = ResponseBody {
            id: None,
            model: None,
            status: Some("completed".into()),
            output: vec![
                json!({"type": "reasoning", "summary": []}),
                json!({
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "hello"}]
                }),
            ],
            usage: None,
            server_side_tool_usage: None,
            tool_calls: None,
            error: None,
        };
        assert_eq!(extract_output_text(&body), "hello");
    }

    #[test]
    fn count_search_calls_in_output() {
        let out = vec![
            json!({"type": "web_search_call", "id": "1"}),
            json!({"type": "web_search_call", "id": "2"}),
            json!({"type": "x_search_call", "id": "3"}),
            json!({"type": "message", "content": []}),
        ];
        let m = count_output_tool_calls(&out);
        assert_eq!(m.get("web_search_call"), Some(&2));
        assert_eq!(m.get("x_search_call"), Some(&1));
    }

    #[test]
    fn usage_report_from_server_side_map() {
        let body = ResponseBody {
            id: None,
            model: Some("grok-4.5".into()),
            status: Some("completed".into()),
            output: vec![json!({"type": "web_search_call"})],
            usage: Some(Usage {
                input_tokens: Some(454_000),
                output_tokens: Some(3000),
                total_tokens: Some(457_000),
                input_tokens_details: Some(json!({"cached_tokens": 100_000})),
                output_tokens_details: Some(json!({"reasoning_tokens": 2000})),
                num_server_side_tools_used: Some(5),
                num_sources_used: Some(12),
            }),
            server_side_tool_usage: Some(json!({
                "SERVER_SIDE_TOOL_WEB_SEARCH": 3,
                "SERVER_SIDE_TOOL_X_SEARCH": 2
            })),
            tool_calls: None,
            error: None,
        };
        let r = usage_report(&body);
        assert_eq!(r.input_tokens, 454_000);
        assert_eq!(r.reasoning_tokens, 2000);
        assert_eq!(r.cached_tokens, Some(100_000));
        assert_eq!(
            r.server_side_tool_usage.get("SERVER_SIDE_TOOL_WEB_SEARCH"),
            Some(&3)
        );
        assert_eq!(r.output_tool_call_counts.get("web_search_call"), Some(&1));
    }

    #[test]
    fn parse_json_with_fence() {
        let v = parse_json_object("```json\n{\"a\":1}\n```").unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn truncate() {
        let (s, t) = truncate_chars("abcdef", 4);
        assert!(t);
        assert_eq!(s.chars().count(), 4);
    }
}
