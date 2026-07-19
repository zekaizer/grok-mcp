//! `job_status` — poll async tool jobs started with timeout_secs.

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::schemars::JsonSchema;
use rmcp::{ErrorData, tool, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::GrokMcpServer;
use crate::envelope::{ErrorCode, Fail};
use crate::jobs::next_poll_hint;

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct JobStatusArgs {
    pub job_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct JobStatusOk {
    pub ok: bool,
    /// `running` | `completed` | `failed` | `not_found`
    pub status: String,
    pub job_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_secs: Option<u64>,
    /// Present when status=running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    /// Full tool result when status=completed (research / ask_grok / x_search shape).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[tool_router(router = job_status_router, vis = "pub(crate)")]
impl GrokMcpServer {
    #[tool(
        description = "Poll a background job started when research / ask_grok / x_search returned status=running (timeout_secs). Pass job_id until status is completed (result filled) or failed. Jobs are in-memory and lost on server restart; finished jobs expire after ~30 minutes. At most 10 jobs run concurrently; starting more returns retryable RATE_LIMITED.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn job_status(
        &self,
        Parameters(params): Parameters<JobStatusArgs>,
    ) -> Result<Json<JobStatusOk>, ErrorData> {
        let job_id = params.job_id.trim().to_string();
        if job_id.is_empty() {
            return Err(
                Fail::new(ErrorCode::InvalidParams, "job_id must be non-empty", false)
                    .into_error_data(),
            );
        }

        match self.jobs.get(&job_id) {
            None => Ok(Json(JobStatusOk {
                ok: true,
                status: "not_found".into(),
                job_id,
                kind: None,
                elapsed_secs: None,
                next: None,
                result: None,
                error_code: Some("NOT_FOUND".into()),
                error_message: Some(
                    "unknown or expired job_id (restart clears jobs; TTL ~30m after finish)".into(),
                ),
            })),
            Some(snap) => {
                let next = if snap.status == "running" {
                    Some(next_poll_hint(&snap.job_id))
                } else {
                    None
                };
                Ok(Json(JobStatusOk {
                    ok: true,
                    status: snap.status,
                    job_id: snap.job_id,
                    kind: Some(snap.kind.as_str().into()),
                    elapsed_secs: Some(snap.elapsed_secs),
                    next,
                    result: snap.result,
                    error_code: snap.error_code,
                    error_message: snap.error_message,
                }))
            }
        }
    }
}
