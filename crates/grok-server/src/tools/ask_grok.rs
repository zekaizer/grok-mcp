//! `ask_grok` — single-shot sub-LLM offload (no live search).

use grok_client::{
    CreateResponseRequest, ReasoningParam, debug_payload_budget, extract_output_text,
    truncate_chars,
};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::schemars::JsonSchema;
use rmcp::{ErrorData, tool, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::GrokMcpServer;
use crate::envelope::{ErrorCode, Fail};
use crate::jobs::{JobKind, RunOutcome, next_poll_hint, run_with_timeout};
use crate::modes::{cost_hint_for, depth_char_budget, parse_depth_effort};
use crate::upstream::client_error_to_fail;
use crate::usage_out::{UsageOut, usage_out_and_log};

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AskGrokArgs {
    pub prompt: String,
    #[serde(default)]
    pub system: Option<String>,
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
pub struct AskGrokOk {
    pub ok: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
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

#[tool_router(router = ask_grok_router, vis = "pub(crate)")]
impl GrokMcpServer {
    #[tool(
        description = "Low-cost single-shot offload to Grok: Q&A, critique, analysis — no web/X search. Prefer over research when live sources are not needed. For X posts use x_search; for current news use research. depth=quick|standard|deep. Async by default: returns inline if done within ~25s, else status=running + job_id → poll job_status (timeout_secs 1-300 overrides the window). Up to 10 run concurrently plus 20 queued; RATE_LIMITED (retryable) only when the queue is full.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn ask_grok(
        &self,
        Parameters(params): Parameters<AskGrokArgs>,
    ) -> Result<Json<AskGrokOk>, ErrorData> {
        let prompt = params.prompt.trim().to_string();
        if prompt.is_empty() {
            return Err(
                Fail::new(ErrorCode::InvalidParams, "prompt must be non-empty", false)
                    .into_error_data(),
            );
        }
        if prompt.len() > 100_000 {
            return Err(Fail::new(
                ErrorCode::InvalidParams,
                "prompt exceeds 100000 characters",
                false,
            )
            .into_error_data());
        }
        if let Some(sys) = &params.system
            && sys.len() > 8000
        {
            return Err(Fail::new(
                ErrorCode::InvalidParams,
                "system exceeds 8000 characters",
                false,
            )
            .into_error_data());
        }

        let timeout_secs = params.timeout_secs;
        let server = self.clone();
        let outcome = run_with_timeout(&self.jobs, JobKind::AskGrok, timeout_secs, move || {
            let server = server;
            let params = params;
            let prompt = prompt;
            async move { server.run_ask_grok(prompt, params).await }
        })
        .await
        .map_err(Fail::into_error_data)?;

        Ok(Json(match outcome {
            RunOutcome::Completed(r) => r,
            RunOutcome::Running {
                job_id,
                elapsed_secs,
                status,
            } => AskGrokOk {
                ok: true,
                status: status.clone(),
                job_id: Some(job_id.clone()),
                next: Some(next_poll_hint(&job_id, &status)),
                elapsed_secs: Some(elapsed_secs),
                text: None,
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
    async fn run_ask_grok(&self, prompt: String, params: AskGrokArgs) -> Result<AskGrokOk, Fail> {
        let token = self.access_token().await?;
        let effort = parse_depth_effort(params.depth.as_deref())?;
        let max_out = params.max_output_tokens.unwrap_or(2048).clamp(64, 8192);
        let model = self.client.resolve_model(params.model.as_deref());
        let debug = params.debug.unwrap_or(false);

        let mut instructions = match effort {
            "high" => {
                "Answer thoroughly and clearly. Prefer structure (headings/bullets) when useful."
                    .to_string()
            }
            "low" => "Answer concisely. Prefer short paragraphs or bullets. No preamble.".into(),
            _ => "Answer clearly and relatively concisely. Structure when useful.".into(),
        };
        if let Some(sys) = params
            .system
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            instructions = format!("{sys}\n\n{instructions}");
        }

        let req = CreateResponseRequest {
            model: model.clone(),
            input: json!(prompt),
            instructions: Some(instructions),
            tools: None,
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

        let raw_text = extract_output_text(&body);
        let budget = depth_char_budget(effort);
        let (text, mut truncated) = truncate_chars(&raw_text, budget);
        let debug_payload = if debug {
            let (d, t) = truncate_chars(&raw_text, debug_payload_budget());
            truncated |= t;
            Some(d)
        } else {
            None
        };

        let model_out = body.model.clone().unwrap_or(model);
        let usage = usage_out_and_log("ask_grok", &model_out, &body);
        let cost = cost_hint_for("ask_grok", effort, None);

        Ok(AskGrokOk {
            ok: true,
            status: "completed".into(),
            job_id: None,
            next: None,
            elapsed_secs: None,
            text: Some(text),
            model: Some(model_out),
            usage: Some(usage),
            cost_hint: Some(cost.into()),
            truncated: Some(truncated),
            debug_payload,
        })
    }
}
