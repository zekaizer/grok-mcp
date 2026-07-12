//! `research` — multi-step research via Responses + optional web/X tools.

use grok_client::{
    CreateResponseRequest, ReasoningParam, extract_output_text, parse_json_object, truncate_chars,
    verbosity_char_budget,
};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::schemars::JsonSchema;
use rmcp::{ErrorData, tool, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::GrokMcpServer;
use crate::envelope::{ErrorCode, Fail};
use crate::jobs::{JobKind, RunOutcome, next_poll_hint, run_with_timeout};
use crate::upstream::client_error_to_fail;
use crate::usage_out::{UsageOut, usage_out_and_log};

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ResearchArgs {
    pub query: String,
    #[serde(default)]
    pub sources: Option<Vec<String>>,
    /// `summary` (default) | `detailed` | `raw` — host-facing size, not reasoning depth.
    #[serde(default)]
    pub verbosity: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// `low` | `medium` (default) | `high`
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    /// If set (1–300), wait at most this many seconds then return `status=running` + `job_id` for `job_status` polling. Omit for fully synchronous wait.
    #[serde(default)]
    pub timeout_secs: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ResearchOk {
    pub ok: bool,
    /// `completed` or `running` (when `timeout_secs` elapsed before finish).
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_points: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<SourceItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SourceItem {
    pub title: String,
    pub url: String,
    pub kind: String,
}

const RESEARCH_INSTRUCTIONS: &str = r#"You are a research agent. Use available tools when needed, then answer with ONLY a JSON object (no markdown fences):
{
  "answer": "dense paragraph answer",
  "key_points": ["bullet", "..."],
  "sources": [{"title":"...", "url":"https://...", "kind":"web|x"}],
  "confidence": "low|medium|high"
}
Rules: prefer citations with real URLs; keep answer short; no raw page dumps; sources max 12."#;

#[tool_router(router = research_router, vis = "pub(crate)")]
impl GrokMcpServer {
    #[tool(
        description = "Multi-step live research (current web/news and optional X) via xAI Grok. Use for breaking news, fact-checking, or topics that need web sources. Expensive (high SuperGrok quota). For X posts / tweets / x.com-only investigation use x_search instead. For no live sources use ask_grok. Returns a dense digest (verbosity: summary|detailed|raw). Optional timeout_secs (1–300): still running after N seconds → status=running + job_id, then poll job_status. Omit for full sync wait.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = true
        )
    )]
    pub async fn research(
        &self,
        Parameters(params): Parameters<ResearchArgs>,
    ) -> Result<Json<ResearchOk>, ErrorData> {
        let query = params.query.trim().to_string();
        if query.is_empty() {
            return Err(
                Fail::new(ErrorCode::InvalidParams, "query must be non-empty", false)
                    .into_error_data(),
            );
        }
        if query.len() > 4000 {
            return Err(Fail::new(
                ErrorCode::InvalidParams,
                "query exceeds 4000 characters",
                false,
            )
            .into_error_data());
        }

        let timeout_secs = params.timeout_secs;
        let server = self.clone();
        let outcome = run_with_timeout(&self.jobs, JobKind::Research, timeout_secs, move || {
            let server = server;
            let params = params;
            let query = query;
            async move { server.run_research(query, params).await }
        })
        .await
        .map_err(Fail::into_error_data)?;

        Ok(Json(match outcome {
            RunOutcome::Completed(r) => r,
            RunOutcome::Running {
                job_id,
                elapsed_secs,
            } => ResearchOk {
                ok: true,
                status: "running".into(),
                job_id: Some(job_id.clone()),
                next: Some(next_poll_hint(&job_id)),
                elapsed_secs: Some(elapsed_secs),
                answer: None,
                key_points: None,
                sources: None,
                confidence: None,
                model: None,
                usage: None,
                truncated: None,
                raw: None,
            },
        }))
    }
}

impl GrokMcpServer {
    async fn run_research(&self, query: String, params: ResearchArgs) -> Result<ResearchOk, Fail> {
        let token = self.access_token().await?;

        let verbosity = params
            .verbosity
            .as_deref()
            .unwrap_or("summary")
            .to_ascii_lowercase();
        let effort = params
            .reasoning_effort
            .as_deref()
            .unwrap_or("medium")
            .to_ascii_lowercase();
        let max_out = params.max_output_tokens.unwrap_or(2048).clamp(64, 8192);
        let model = self.client.resolve_model(params.model.as_deref());
        let tools = native_tools(params.sources);

        let req = CreateResponseRequest {
            model: model.clone(),
            input: json!(query),
            instructions: Some(RESEARCH_INSTRUCTIONS.into()),
            tools,
            max_output_tokens: Some(max_out),
            reasoning: Some(ReasoningParam { effort }),
            stream: false,
        };

        let body = self
            .client
            .create_response(&token, &req)
            .await
            .map_err(|e| client_error_to_fail(&e))?;

        let text = extract_output_text(&body);
        let budget = verbosity_char_budget(&verbosity);

        let mut key_points: Vec<String> = Vec::new();
        let mut sources: Vec<SourceItem> = Vec::new();
        let mut confidence = "medium".to_string();
        let mut raw_out: Option<String> = None;

        let (answer, mut answer_trunc) = if let Some(obj) = parse_json_object(&text) {
            let pair = if let Some(a) = obj.get("answer").and_then(|v| v.as_str()) {
                truncate_chars(a, budget)
            } else {
                truncate_chars(&text, budget)
            };
            if let Some(arr) = obj.get("key_points").and_then(|v| v.as_array()) {
                key_points = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .take(12)
                    .collect();
            }
            if let Some(arr) = obj.get("sources").and_then(|v| v.as_array()) {
                sources = arr
                    .iter()
                    .filter_map(|v| {
                        Some(SourceItem {
                            title: v.get("title")?.as_str()?.to_string(),
                            url: v.get("url")?.as_str()?.to_string(),
                            kind: v
                                .get("kind")
                                .and_then(|k| k.as_str())
                                .unwrap_or("web")
                                .to_string(),
                        })
                    })
                    .take(12)
                    .collect();
            }
            if let Some(c) = obj.get("confidence").and_then(|v| v.as_str()) {
                confidence = c.to_string();
            }
            pair
        } else {
            truncate_chars(&text, budget)
        };

        if verbosity == "raw" {
            let (r, rt) = truncate_chars(&text, budget);
            raw_out = Some(r);
            answer_trunc = answer_trunc || rt;
        }

        let model_out = body.model.clone().unwrap_or(model);
        let usage = usage_out_and_log("research", &model_out, &body);
        Ok(ResearchOk {
            ok: true,
            status: "completed".into(),
            job_id: None,
            next: None,
            elapsed_secs: None,
            answer: Some(answer),
            key_points: Some(key_points),
            sources: Some(sources),
            confidence: Some(confidence),
            model: Some(model_out),
            usage: Some(usage),
            truncated: Some(answer_trunc),
            raw: raw_out,
        })
    }
}

fn native_tools(sources: Option<Vec<String>>) -> Option<Vec<Value>> {
    let list = sources.unwrap_or_else(|| vec!["web".into()]);
    if list.is_empty() {
        return None;
    }
    let mut tools = Vec::new();
    let mut seen_web = false;
    let mut seen_x = false;
    for s in list {
        match s.to_ascii_lowercase().as_str() {
            "web" if !seen_web => {
                tools.push(json!({"type": "web_search"}));
                seen_web = true;
            }
            "x" if !seen_x => {
                tools.push(json!({"type": "x_search"}));
                seen_x = true;
            }
            _ => {}
        }
    }
    if tools.is_empty() { None } else { Some(tools) }
}
