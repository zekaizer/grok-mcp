//! `x_search` — X search via native x_search tool + dense digest.

use grok_client::{
    CreateResponseRequest, ReasoningParam, extract_output_text, parse_json_object, truncate_chars,
    verbosity_char_budget,
};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::schemars::JsonSchema;
use rmcp::{ErrorData, tool, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::GrokMcpServer;
use crate::envelope::{ErrorCode, Fail};
use crate::jobs::{JobKind, RunOutcome, next_poll_hint, run_with_timeout};
use crate::upstream::client_error_to_fail;
use crate::usage_out::{UsageOut, usage_out_and_log};

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct XSearchArgs {
    pub query: String,
    /// `summary` (default) | `detailed` | `raw`
    #[serde(default)]
    pub verbosity: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_results: Option<u32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    /// If set (1–300), wait at most N seconds then return status=running + job_id for job_status. Omit for full sync wait.
    #[serde(default)]
    pub timeout_secs: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct XSearchOk {
    pub ok: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posts: Option<Vec<PostItem>>,
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
pub struct PostItem {
    pub author: String,
    pub text: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engagement_hint: Option<String>,
}

#[tool_router(router = x_search_router, vis = "pub(crate)")]
impl GrokMcpServer {
    #[tool(
        description = "X (Twitter) only via xAI native x_search — lighter than research with sources=[x]. Returns summary + short posts. Prefer research when web+X multi-step is needed. Optional timeout_secs (1–300) returns status=running + job_id; poll job_status. Omit for full sync wait.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = true
        )
    )]
    pub async fn x_search(
        &self,
        Parameters(params): Parameters<XSearchArgs>,
    ) -> Result<Json<XSearchOk>, ErrorData> {
        let query = params.query.trim().to_string();
        if query.is_empty() {
            return Err(
                Fail::new(ErrorCode::InvalidParams, "query must be non-empty", false)
                    .into_error_data(),
            );
        }
        if query.len() > 1000 {
            return Err(Fail::new(
                ErrorCode::InvalidParams,
                "query exceeds 1000 characters",
                false,
            )
            .into_error_data());
        }

        let timeout_secs = params.timeout_secs;
        let server = self.clone();
        let outcome = run_with_timeout(&self.jobs, JobKind::XSearch, timeout_secs, move || {
            let server = server;
            let params = params;
            let query = query;
            async move { server.run_x_search(query, params).await }
        })
        .await
        .map_err(Fail::into_error_data)?;

        Ok(Json(match outcome {
            RunOutcome::Completed(r) => r,
            RunOutcome::Running {
                job_id,
                elapsed_secs,
            } => XSearchOk {
                ok: true,
                status: "running".into(),
                job_id: Some(job_id.clone()),
                next: Some(next_poll_hint(&job_id)),
                elapsed_secs: Some(elapsed_secs),
                summary: None,
                posts: None,
                model: None,
                usage: None,
                truncated: None,
                raw: None,
            },
        }))
    }
}

impl GrokMcpServer {
    async fn run_x_search(&self, query: String, params: XSearchArgs) -> Result<XSearchOk, Fail> {
        let token = self.access_token().await?;

        let verbosity = params
            .verbosity
            .as_deref()
            .unwrap_or("summary")
            .to_ascii_lowercase();
        let max_results = params.max_results.unwrap_or(8).clamp(1, 20);
        let max_out = params.max_output_tokens.unwrap_or(1024).clamp(64, 4096);
        let model = self.client.resolve_model(params.model.as_deref());

        let instructions = format!(
            "Search X for the user query. Return ONLY JSON (no fences):\n\
             {{\"summary\":\"...\",\"posts\":[{{\"author\":\"@handle\",\"text\":\"short excerpt\",\"url\":\"https://x.com/...\",\"engagement_hint\":\"optional\"}}]}}\n\
             Include at most {max_results} posts. Prefer high-signal posts. Keep excerpts short."
        );

        let req = CreateResponseRequest {
            model: model.clone(),
            input: json!(query),
            instructions: Some(instructions),
            tools: Some(vec![json!({"type": "x_search"})]),
            max_output_tokens: Some(max_out),
            reasoning: Some(ReasoningParam {
                effort: "low".into(),
            }),
            stream: false,
        };

        let body = self
            .client
            .create_response(&token, &req)
            .await
            .map_err(|e| client_error_to_fail(&e))?;

        let text = extract_output_text(&body);
        let budget = verbosity_char_budget(&verbosity);

        let mut posts: Vec<PostItem> = Vec::new();
        let mut raw_out: Option<String> = None;

        let (summary, mut summary_trunc) = if let Some(obj) = parse_json_object(&text) {
            let pair = if let Some(s) = obj.get("summary").and_then(|v| v.as_str()) {
                truncate_chars(s, budget)
            } else {
                truncate_chars(&text, budget)
            };
            if let Some(arr) = obj.get("posts").and_then(|v| v.as_array()) {
                posts = arr
                    .iter()
                    .filter_map(|v| {
                        Some(PostItem {
                            author: v
                                .get("author")
                                .and_then(|a| a.as_str())
                                .unwrap_or("")
                                .to_string(),
                            text: v.get("text")?.as_str()?.to_string(),
                            url: v
                                .get("url")
                                .and_then(|u| u.as_str())
                                .unwrap_or("")
                                .to_string(),
                            engagement_hint: v
                                .get("engagement_hint")
                                .and_then(|e| e.as_str())
                                .map(str::to_string),
                        })
                    })
                    .take(max_results as usize)
                    .collect();
            }
            pair
        } else {
            truncate_chars(&text, budget)
        };

        if verbosity == "raw" {
            let (r, rt) = truncate_chars(&text, budget);
            raw_out = Some(r);
            summary_trunc = summary_trunc || rt;
        }

        let model_out = body.model.clone().unwrap_or(model);
        let usage = usage_out_and_log("x_search", &model_out, &body);
        Ok(XSearchOk {
            ok: true,
            status: "completed".into(),
            job_id: None,
            next: None,
            elapsed_secs: None,
            summary: Some(summary),
            posts: Some(posts),
            model: Some(model_out),
            usage: Some(usage),
            truncated: Some(summary_trunc),
            raw: raw_out,
        })
    }
}
