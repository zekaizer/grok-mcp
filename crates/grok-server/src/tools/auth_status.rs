//! `auth_status` — non-secret credential health (tool_spec).

use grok_auth::status_snapshot;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::schemars::JsonSchema;
use rmcp::{ErrorData, tool, tool_router};
use serde::{Deserialize, Serialize};

use crate::GrokMcpServer;
use crate::envelope::{ErrorCode, Fail};

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AuthStatusArgs {
    /// Include account hints (email / user id) when available.
    #[serde(default = "default_true")]
    pub include_account_hints: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AuthStatusOk {
    pub ok: bool,
    pub authenticated: bool,
    pub billing_path: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountHints>,
    pub api_key_opt_in: bool,
    pub api_key_present: bool,
    pub store_path: String,
    pub grok_cli_present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AccountHints {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

#[tool_router(router = auth_status_router, vis = "pub(crate)")]
impl GrokMcpServer {
    #[tool(
        description = "Report whether grok-mcp can call xAI (subscription OAuth or opt-in API key). Never returns secrets. Access tokens refresh automatically near expiry; call this on REAUTH_REQUIRED or after login/import. expires_at alone does not mean the session dies — refresh uses the stored refresh_token.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn auth_status(
        &self,
        Parameters(params): Parameters<AuthStatusArgs>,
    ) -> Result<Json<AuthStatusOk>, ErrorData> {
        match status_snapshot(self.auth_file.clone()) {
            Ok(snap) => {
                let account = if params.include_account_hints
                    && (snap.email.is_some() || snap.user_id.is_some())
                {
                    Some(AccountHints {
                        email: snap.email,
                        user_id: snap.user_id,
                    })
                } else {
                    None
                };
                Ok(Json(AuthStatusOk {
                    ok: true,
                    authenticated: snap.authenticated,
                    billing_path: billing_path_str(snap.billing_path).into(),
                    source: source_str(snap.source).into(),
                    expires_at: snap.expires_at,
                    account,
                    api_key_opt_in: snap.api_key_opt_in,
                    api_key_present: snap.api_key_present,
                    store_path: snap.store_path.display().to_string(),
                    grok_cli_present: snap.grok_cli_present,
                    last_error: None,
                }))
            }
            Err(e) => {
                Err(Fail::new(ErrorCode::UpstreamError, e.to_string(), false).into_error_data())
            }
        }
    }
}

fn billing_path_str(p: grok_auth::BillingPath) -> &'static str {
    match p {
        grok_auth::BillingPath::SubscriptionOauth => "subscription_oauth",
        grok_auth::BillingPath::ApiKey => "api_key",
        grok_auth::BillingPath::None => "none",
    }
}

fn source_str(s: grok_auth::AuthSource) -> &'static str {
    match s {
        grok_auth::AuthSource::GrokCli => "grok_cli",
        grok_auth::AuthSource::DeviceCode => "device_code",
        grok_auth::AuthSource::ApiKey => "api_key",
        grok_auth::AuthSource::None => "none",
    }
}
