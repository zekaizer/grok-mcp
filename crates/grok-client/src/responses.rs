//! xAI Responses API (`POST /v1/responses`).

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
    pub output_tokens_details: Option<Value>,
}

impl Usage {
    #[must_use]
    pub fn reasoning_tokens(&self) -> Option<u64> {
        self.output_tokens_details
            .as_ref()
            .and_then(|v| v.get("reasoning_tokens"))
            .and_then(|v| v.as_u64())
    }
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
    // Find first { ... last }
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
            error: None,
        };
        assert_eq!(extract_output_text(&body), "hello");
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
