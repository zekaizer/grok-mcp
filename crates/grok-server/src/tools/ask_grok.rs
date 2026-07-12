//! `ask_grok` — single-shot sub-LLM offload.

use grok_client::{
    CreateResponseRequest, ReasoningParam, extract_output_text, truncate_chars,
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

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AskGrokArgs {
    pub prompt: String,
    #[serde(default)]
    pub system: Option<String>,
    /// `summary` (default) | `detailed` | `raw`
    #[serde(default)]
    pub verbosity: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// `low` | `medium` (default) | `high`
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    /// If set (1–300), wait at most N seconds then return status=running + job_id for job_status. Omit for full sync wait.
    #[serde(default)]
    pub timeout_secs: Option<u32>,
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
    pub truncated: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct UsageOut {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
}

#[tool_router(router = ask_grok_router, vis = "pub(crate)")]
impl GrokMcpServer {
    #[tool(
        description = "Low-cost single-shot offload to Grok: Q&A, critique, analysis — no web/X search. Prefer this over research when live sources are not needed. verbosity: summary|detailed|raw. Optional timeout_secs (1–300) returns status=running + job_id to poll via job_status; omit for full synchronous wait.",
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
            } => AskGrokOk {
                ok: true,
                status: "running".into(),
                job_id: Some(job_id.clone()),
                next: Some(next_poll_hint(&job_id)),
                elapsed_secs: Some(elapsed_secs),
                text: None,
                model: None,
                usage: None,
                truncated: None,
            },
        }))
    }
}

impl GrokMcpServer {
    async fn run_ask_grok(&self, prompt: String, params: AskGrokArgs) -> Result<AskGrokOk, Fail> {
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

        let mut instructions = match verbosity.as_str() {
            "detailed" | "raw" => {
                "Answer thoroughly and clearly. Prefer structure (headings/bullets) when useful."
                    .to_string()
            }
            _ => "Answer concisely. Prefer short paragraphs or bullets. No preamble.".to_string(),
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
        let (text, truncated) = truncate_chars(&text, budget);
        let usage = body.usage.as_ref();

        Ok(AskGrokOk {
            ok: true,
            status: "completed".into(),
            job_id: None,
            next: None,
            elapsed_secs: None,
            text: Some(text),
            model: Some(body.model.unwrap_or(model)),
            usage: Some(UsageOut {
                input_tokens: usage.and_then(|u| u.input_tokens).unwrap_or(0),
                output_tokens: usage.and_then(|u| u.output_tokens).unwrap_or(0),
                reasoning_tokens: usage.and_then(|u| u.reasoning_tokens()).unwrap_or(0),
            }),
            truncated: Some(truncated),
        })
    }
}
