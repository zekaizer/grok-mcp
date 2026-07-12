//! `research` — multi-step research via Responses + optional web/X tools.

use grok_client::{
    CreateResponseRequest, ReasoningParam, debug_payload_budget, extract_output_text,
    parse_json_object, truncate_chars,
};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::schemars::JsonSchema;
use rmcp::{ErrorData, tool, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::GrokMcpServer;
use crate::envelope::{ErrorCode, Fail};
use crate::jobs::{JobKind, RunOutcome, next_poll_hint, run_with_timeout};
use crate::modes::{
    ResultMode, cost_hint_for, parse_depth_effort, parse_result_mode, result_char_budget,
};
use crate::upstream::client_error_to_fail;
use crate::usage_out::{UsageOut, usage_out_and_log};

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ResearchArgs {
    pub query: String,
    #[serde(default)]
    pub sources: Option<Vec<String>>,
    /// `digest` (default) | `evidence` | `both`
    #[serde(default)]
    pub result: Option<String>,
    /// `quick` | `standard` (default) | `deep`
    #[serde(default)]
    pub depth: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub timeout_secs: Option<u32>,
    #[serde(default)]
    pub debug: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ResearchOk {
    pub ok: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_points: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citations: Option<Vec<CitationItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_payload: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct CitationItem {
    pub title: String,
    pub url: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub quote_complete: bool,
}

#[tool_router(router = research_router, vis = "pub(crate)")]
impl GrokMcpServer {
    #[tool(
        description = "Multi-step live research (current web/news; optional X) via xAI Grok. Expensive (high SuperGrok quota). For X posts/tweets/x.com-only work use x_search instead. result=digest|evidence|both (evidence fills citation quotes; host fetch not assumed). depth=quick|standard|deep. No live sources needed → ask_grok. Optional timeout_secs → job_status.",
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
                result_mode: None,
                answer: None,
                key_points: None,
                citations: None,
                confidence: None,
                model: None,
                usage: None,
                cost_hint: None,
                truncated: None,
                debug_payload: None,
            },
        }))
    }
}

impl GrokMcpServer {
    async fn run_research(&self, query: String, params: ResearchArgs) -> Result<ResearchOk, Fail> {
        let token = self.access_token().await?;
        let mode = parse_result_mode(params.result.as_deref())?;
        let effort = parse_depth_effort(params.depth.as_deref())?;
        let max_out = params.max_output_tokens.unwrap_or(2048).clamp(64, 8192);
        let model = self.client.resolve_model(params.model.as_deref());
        let tools = native_tools(params.sources);
        let debug = params.debug.unwrap_or(false);

        let req = CreateResponseRequest {
            model: model.clone(),
            input: json!(query),
            instructions: Some(research_instructions(mode)),
            tools,
            max_output_tokens: Some(max_out),
            reasoning: Some(ReasoningParam {
                effort: effort.into(),
            }),
            stream: false,
        };

        let body = self
            .client
            .create_response(&token, &req)
            .await
            .map_err(|e| client_error_to_fail(&e))?;

        let text = extract_output_text(&body);
        let budget = result_char_budget(mode);

        let mut key_points: Vec<String> = Vec::new();
        let mut citations: Vec<CitationItem> = Vec::new();
        let mut confidence = "medium".to_string();
        let mut truncated = false;

        let answer = if let Some(obj) = parse_json_object(&text) {
            let (ans, t) = if let Some(a) = obj.get("answer").and_then(|v| v.as_str()) {
                truncate_chars(a, budget)
            } else {
                truncate_chars(&text, budget)
            };
            truncated |= t;
            if let Some(arr) = obj.get("key_points").and_then(|v| v.as_array()) {
                key_points = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .take(12)
                    .collect();
            }
            // Accept both "citations" (v2) and "sources" (legacy model habit).
            let cite_arr = obj
                .get("citations")
                .or_else(|| obj.get("sources"))
                .and_then(|v| v.as_array());
            if let Some(arr) = cite_arr {
                citations = arr
                    .iter()
                    .filter_map(|v| parse_citation(v, mode))
                    .take(12)
                    .collect();
            }
            if let Some(c) = obj.get("confidence").and_then(|v| v.as_str()) {
                confidence = c.to_string();
            }
            ans
        } else {
            let (a, t) = truncate_chars(&text, budget);
            truncated |= t;
            a
        };

        if mode.wants_evidence() {
            let has_quote = citations.iter().any(|c| {
                c.quote
                    .as_ref()
                    .map(|q| !q.trim().is_empty())
                    .unwrap_or(false)
            });
            if !has_quote {
                return Err(Fail::new(
                    ErrorCode::EvidenceUnavailable,
                    "no citation quotes available for result=evidence",
                    true,
                ));
            }
        }

        let debug_payload = if debug {
            let (d, t) = truncate_chars(&text, debug_payload_budget());
            truncated |= t;
            Some(d)
        } else {
            None
        };

        let model_out = body.model.clone().unwrap_or(model);
        let usage = usage_out_and_log("research", &model_out, &body);
        let cost = cost_hint_for("research", effort, Some(mode));

        Ok(ResearchOk {
            ok: true,
            status: "completed".into(),
            job_id: None,
            next: None,
            elapsed_secs: None,
            result_mode: Some(mode.as_str().into()),
            answer: if mode.wants_digest() || !answer.is_empty() {
                Some(answer)
            } else {
                None
            },
            key_points: Some(key_points),
            citations: Some(citations),
            confidence: Some(confidence),
            model: Some(model_out),
            usage: Some(usage),
            cost_hint: Some(cost.into()),
            truncated: Some(truncated),
            debug_payload,
        })
    }
}

fn research_instructions(mode: ResultMode) -> String {
    match mode {
        ResultMode::Digest => r#"You are a research agent. Use available tools when needed, then answer with ONLY a JSON object (no markdown fences):
{
  "answer": "dense paragraph answer",
  "key_points": ["bullet", "..."],
  "citations": [{"title":"...","url":"https://...","kind":"web|x"}],
  "confidence": "low|medium|high"
}
Rules: prefer real URLs; keep answer short; no raw page dumps; citations max 12."#
            .into(),
        ResultMode::Evidence | ResultMode::Both => r#"You are a research agent. Use available tools when needed, then answer with ONLY a JSON object (no markdown fences):
{
  "answer": "dense paragraph answer",
  "key_points": ["bullet", "..."],
  "citations": [{"title":"...","url":"https://...","kind":"web|x","quote":"verbatim excerpt or full short passage","quote_complete":true}],
  "confidence": "low|medium|high"
}
Rules: every citation that supports a claim MUST include quote with source wording (no paraphrase of the quote); set quote_complete=false if truncated; max 12 citations; host may not fetch URLs."#
            .into(),
    }
}

fn parse_citation(v: &Value, mode: ResultMode) -> Option<CitationItem> {
    let title = v.get("title")?.as_str()?.to_string();
    let url = v.get("url")?.as_str()?.to_string();
    let kind = v
        .get("kind")
        .and_then(|k| k.as_str())
        .unwrap_or("web")
        .to_string();
    let quote = v
        .get("quote")
        .and_then(|q| q.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty());
    let quote_complete = v
        .get("quote_complete")
        .and_then(|x| x.as_bool())
        .unwrap_or(mode.wants_evidence() && quote.is_some());
    Some(CitationItem {
        title,
        url,
        kind,
        quote,
        quote_complete,
    })
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
    if tools.is_empty() {
        None
    } else {
        Some(tools)
    }
}
