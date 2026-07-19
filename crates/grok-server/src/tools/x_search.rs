//! `x_search` — X search via native x_search; digest and/or evidence posts.

use grok_client::{
    CreateResponseRequest, ReasoningParam, debug_payload_budget, extract_output_text,
    parse_json_object, truncate_chars,
};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::schemars::JsonSchema;
use rmcp::{ErrorData, tool, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::GrokMcpServer;
use crate::envelope::{ErrorCode, Fail};
use crate::jobs::{JobKind, RunOutcome, next_poll_hint, run_with_timeout};
use crate::modes::{
    ResultMode, cost_hint_for, evidence_status_for_posts, parse_depth_effort, parse_result_mode,
    post_text_cap, result_char_budget,
};
use crate::upstream::client_error_to_fail;
use crate::usage_out::{UsageOut, usage_out_and_log};

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct XSearchArgs {
    pub query: String,
    /// `digest` (default) | `evidence` | `both`
    #[serde(default)]
    pub result: Option<String>,
    /// `quick` | `standard` (default) | `deep`
    #[serde(default)]
    pub depth: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_items: Option<u32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    /// Offload window override (1–300s). Omitted uses the default (~25s). Within the window
    /// the result returns inline; past it the tool returns status=running + job_id for job_status.
    /// Up to 10 jobs run concurrently plus 20 queued; a full queue returns retryable RATE_LIMITED.
    #[serde(default)]
    pub timeout_secs: Option<u32>,
    #[serde(default)]
    pub debug: Option<bool>,
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
    pub result_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<DigestBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posts: Option<Vec<PostItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fidelity: Option<FidelityBlock>,
    /// When evidence was requested: `empty` | `partial` | `complete` (always success path).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_status: Option<String>,
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
pub struct DigestBlock {
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_points: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct FidelityBlock {
    pub mode: String,
    pub guarantee: String,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct PostItem {
    pub author: String,
    pub text: String,
    pub url: String,
    #[serde(default)]
    pub text_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engagement_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[tool_router(router = x_search_router, vis = "pub(crate)")]
impl GrokMcpServer {
    #[tool(
        description = "Search X (Twitter / x.com) posts. NOT a bit-perfect X API export: post text is best-effort via Grok (may paraphrase); do not use for legal/audit verbatim. ALWAYS use for X posts/tweets/discourse — hosts usually cannot fetch x.com. result=digest (default)=summary+excerpts; result=evidence=best-effort full post text (empty matches return ok with evidence_status=empty, not an error); result=both=digest+posts. depth=quick|standard|deep. Prefer over research for X-only. Async by default: returns inline if done within ~25s, else status=running + job_id → poll job_status (timeout_secs 1-300 overrides the window). Up to 10 run concurrently plus 20 queued; RATE_LIMITED (retryable) only when the queue is full.",
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
                result_mode: None,
                digest: None,
                posts: None,
                fidelity: None,
                evidence_status: None,
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
    async fn run_x_search(&self, query: String, params: XSearchArgs) -> Result<XSearchOk, Fail> {
        let token = self.access_token().await?;
        let mode = parse_result_mode(params.result.as_deref())?;
        let effort = parse_depth_effort(params.depth.as_deref())?;
        let max_items = params.max_items.unwrap_or(8).clamp(1, 20);
        let default_out = if mode.wants_evidence() { 4096 } else { 1024 };
        let max_out = params
            .max_output_tokens
            .unwrap_or(default_out)
            .clamp(64, 8192);
        let model = self.client.resolve_model(params.model.as_deref());
        let debug = params.debug.unwrap_or(false);

        let instructions = x_search_instructions(mode, max_items);

        let req = CreateResponseRequest {
            model: model.clone(),
            input: json!(query),
            instructions: Some(instructions),
            tools: Some(vec![json!({"type": "x_search"})]),
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
        let post_cap = post_text_cap(mode);

        let mut posts: Vec<PostItem> = Vec::new();
        let mut summary = String::new();
        let mut key_points: Vec<String> = Vec::new();
        let mut confidence = "medium".to_string();
        let mut truncated = false;

        if let Some(obj) = parse_json_object(&text) {
            if let Some(s) = obj.get("summary").and_then(|v| v.as_str()) {
                let (s, t) = truncate_chars(s, budget);
                summary = s;
                truncated |= t;
            }
            if let Some(arr) = obj.get("key_points").and_then(|v| v.as_array()) {
                key_points = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .take(12)
                    .collect();
            }
            if let Some(c) = obj.get("confidence").and_then(|v| v.as_str()) {
                confidence = c.to_string();
            }
            if let Some(arr) = obj.get("posts").and_then(|v| v.as_array()) {
                posts = arr
                    .iter()
                    .filter_map(|v| parse_post(v, post_cap, mode))
                    .take(max_items as usize)
                    .collect();
            }
        } else {
            let (s, t) = truncate_chars(&text, budget);
            summary = s;
            truncated |= t;
        }

        // Enforce aggregate budget on post texts for evidence (never fail the call).
        if mode.wants_evidence() {
            let mut used = 0usize;
            for p in &mut posts {
                let n = p.text.chars().count();
                if used + n > budget {
                    let remain = budget.saturating_sub(used);
                    if remain < 32 {
                        p.text.clear();
                        p.text_complete = false;
                        truncated = true;
                    } else {
                        let (t, tr) = truncate_chars(&p.text, remain);
                        p.text = t;
                        p.text_complete = false;
                        truncated |= tr;
                    }
                }
                used += p.text.chars().count();
            }
            posts.retain(|p| !p.text.is_empty());
        }

        let evidence_status = if mode.wants_evidence() {
            let pairs: Vec<(bool, bool)> = posts
                .iter()
                .map(|p| (!p.text.trim().is_empty(), p.text_complete))
                .collect();
            Some(evidence_status_for_posts(&pairs).to_string())
        } else {
            None
        };

        if summary.is_empty() {
            if posts.is_empty() {
                summary = "No matching posts found.".into();
                if confidence == "medium" {
                    confidence = "high".into();
                }
            } else if mode.wants_digest() || mode.wants_evidence() {
                summary = format!("{} posts matched.", posts.len());
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
        let usage = usage_out_and_log("x_search", &model_out, &body);
        let cost = cost_hint_for("x_search", effort, Some(mode));

        // Always return a digest when empty so "no posts" is visible even for result=evidence.
        let want_digest = mode.wants_digest() || posts.is_empty();
        let digest = if want_digest {
            Some(DigestBlock {
                summary,
                key_points: if key_points.is_empty() {
                    None
                } else {
                    Some(key_points)
                },
                confidence: Some(confidence),
            })
        } else {
            None
        };

        let fidelity = if mode.wants_evidence() {
            Some(FidelityBlock {
                mode: mode.as_str().into(),
                guarantee: "best_effort_from_xai_tools".into(),
                notes: "Not a bit-perfect X API export; host x.com fetch not required.".into(),
            })
        } else {
            None
        };

        Ok(XSearchOk {
            ok: true,
            status: "completed".into(),
            job_id: None,
            next: None,
            elapsed_secs: None,
            result_mode: Some(mode.as_str().into()),
            digest,
            posts: Some(posts),
            fidelity,
            evidence_status,
            model: Some(model_out),
            usage: Some(usage),
            cost_hint: Some(cost.into()),
            truncated: Some(truncated),
            debug_payload,
        })
    }
}

fn x_search_instructions(mode: ResultMode, max_items: u32) -> String {
    match mode {
        ResultMode::Digest => format!(
            "Search X for the user query. Return ONLY JSON (no fences):\n\
             {{\"summary\":\"…\",\"key_points\":[\"…\"],\"confidence\":\"low|medium|high\",\
             \"posts\":[{{\"author\":\"@handle\",\"text\":\"short excerpt\",\"url\":\"https://x.com/…\",\
             \"text_complete\":false,\"engagement_hint\":\"optional\",\"created_at\":null}}]}}\n\
             Include at most {max_items} posts. Prefer high-signal posts. Keep post text short (excerpts)."
        ),
        ResultMode::Evidence => format!(
            "Search X for the user query. The host CANNOT open x.com URLs — you MUST return full post bodies.\n\
             Return ONLY JSON (no fences):\n\
             {{\"summary\":\"one-line context\",\"confidence\":\"low|medium|high\",\
             \"posts\":[{{\"author\":\"@handle\",\"text\":\"FULL post text verbatim — no ellipsis, no paraphrase\",\
             \"url\":\"https://x.com/…\",\"text_complete\":true,\"engagement_hint\":\"optional\",\"created_at\":null}}]}}\n\
             Rules: at most {max_items} posts; text MUST be the complete post body as returned by X tools; \
             NEVER replace text with summaries or '…'; if a post is truncated upstream set text_complete=false \
             and still return all available characters; do not invent posts."
        ),
        ResultMode::Both => format!(
            "Search X for the user query. Host cannot open x.com — include full post bodies for quotes.\n\
             Return ONLY JSON (no fences):\n\
             {{\"summary\":\"dense overview\",\"key_points\":[\"…\"],\"confidence\":\"low|medium|high\",\
             \"posts\":[{{\"author\":\"@handle\",\"text\":\"FULL post text verbatim\",\
             \"url\":\"https://x.com/…\",\"text_complete\":true,\"engagement_hint\":\"optional\",\"created_at\":null}}]}}\n\
             At most {max_items} posts. summary/key_points for scout; posts[].text must be full bodies \
             (no intentional ellipsis or paraphrase). text_complete=false only if upstream truncated."
        ),
    }
}

fn parse_post(v: &serde_json::Value, post_cap: usize, mode: ResultMode) -> Option<PostItem> {
    let text_raw = v.get("text")?.as_str()?.to_string();
    if text_raw.trim().is_empty() {
        return None;
    }
    let claimed_complete = v
        .get("text_complete")
        .and_then(|x| x.as_bool())
        .unwrap_or(mode.wants_evidence());
    let (text, was_trunc) = truncate_chars(&text_raw, post_cap);
    let text_complete = claimed_complete && !was_trunc && !text_raw.trim_end().ends_with('…');
    Some(PostItem {
        author: v
            .get("author")
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .to_string(),
        text,
        url: v
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string(),
        text_complete,
        engagement_hint: v
            .get("engagement_hint")
            .and_then(|e| e.as_str())
            .map(str::to_string),
        created_at: v
            .get("created_at")
            .and_then(|c| c.as_str())
            .map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_instructions_forbid_short_excerpts() {
        let s = x_search_instructions(ResultMode::Evidence, 5);
        assert!(s.contains("FULL"));
        assert!(!s.contains("Keep post text short"));
        assert!(s.contains("CANNOT open x.com") || s.contains("cannot open x.com"));
    }

    #[test]
    fn digest_instructions_allow_short() {
        let s = x_search_instructions(ResultMode::Digest, 5);
        assert!(s.contains("short") || s.contains("excerpts"));
    }

    #[test]
    fn tool_description_leads_with_fidelity_warning() {
        let router = crate::tools::router();
        let d = router
            .get("x_search")
            .expect("x_search")
            .description
            .as_deref()
            .unwrap_or("");
        assert!(
            d.starts_with("Search X") || d.contains("NOT a bit-perfect"),
            "desc={d}"
        );
        assert!(
            d.contains("NOT a bit-perfect") || d.contains("best-effort"),
            "desc={d}"
        );
        assert!(
            d.contains("evidence_status=empty") || d.contains("empty"),
            "desc={d}"
        );
    }
}
