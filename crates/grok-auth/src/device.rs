//! OAuth 2.0 device-code login against xAI (ADR-0003).
//!
//! Endpoints from `https://auth.x.ai/.well-known/openid-configuration`.

use std::time::Duration;

use serde::Deserialize;

use crate::DEFAULT_TOKEN_ENDPOINT;
use crate::error::AuthError;
use crate::form::form_body;
use crate::store::{AuthRecord, AuthSource};
use crate::timeutil::{expires_at_from_expires_in, now_rfc3339};

/// Grok CLI / Grok Build public OIDC client id (public client, no secret).
pub const DEFAULT_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";

/// Device authorization endpoint.
pub const DEFAULT_DEVICE_ENDPOINT: &str = "https://auth.x.ai/oauth2/device/code";

/// Userinfo endpoint (optional email/sub enrichment).
pub const DEFAULT_USERINFO_ENDPOINT: &str = "https://auth.x.ai/oauth2/userinfo";

/// Scopes needed for API access + refresh tokens.
pub const DEFAULT_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";

/// What the CLI should show the user.
#[derive(Debug, Clone)]
pub struct DevicePrompt {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
}

/// Successful device authorization response.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    #[serde(default)]
    pub interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TokenSuccess {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    token_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenError {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    sub: Option<String>,
}

/// Options for device-code login.
#[derive(Debug, Clone)]
pub struct DeviceLoginOptions {
    pub client_id: String,
    pub device_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    pub scope: String,
}

impl Default for DeviceLoginOptions {
    fn default() -> Self {
        Self {
            client_id: std::env::var("GROK_MCP_OIDC_CLIENT_ID")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_CLIENT_ID.to_string()),
            device_endpoint: DEFAULT_DEVICE_ENDPOINT.to_string(),
            token_endpoint: DEFAULT_TOKEN_ENDPOINT.to_string(),
            userinfo_endpoint: DEFAULT_USERINFO_ENDPOINT.to_string(),
            scope: DEFAULT_SCOPE.to_string(),
        }
    }
}

/// Run the full device-code flow; call `on_prompt` once with user instructions.
///
/// Blocks (async) until the user approves, the code expires, or an error occurs.
pub async fn device_login<F>(
    http: &reqwest::Client,
    options: &DeviceLoginOptions,
    on_prompt: F,
) -> Result<AuthRecord, AuthError>
where
    F: FnOnce(&DevicePrompt),
{
    let device = request_device_code(http, options).await?;
    let interval = device.interval.unwrap_or(5).max(1);
    let prompt = DevicePrompt {
        user_code: device.user_code.clone(),
        verification_uri: device.verification_uri.clone(),
        verification_uri_complete: device.verification_uri_complete.clone(),
        expires_in: device.expires_in,
        interval,
    };
    on_prompt(&prompt);

    let token = poll_for_token(
        http,
        options,
        &device.device_code,
        interval,
        device.expires_in,
    )
    .await?;

    let mut record = AuthRecord {
        version: AuthRecord::CURRENT_VERSION,
        source: AuthSource::DeviceCode,
        imported_from: None,
        access_token: token.access_token,
        refresh_token: token.refresh_token.filter(|s| !s.is_empty()),
        expires_at: token.expires_in.map(expires_at_from_expires_in),
        oidc_issuer: Some("https://auth.x.ai".into()),
        oidc_client_id: Some(options.client_id.clone()),
        token_endpoint: Some(options.token_endpoint.clone()),
        email: None,
        user_id: None,
        updated_at: Some(now_rfc3339()),
    };
    let _ = token.token_type;

    if let Ok((email, sub)) =
        fetch_userinfo(http, &options.userinfo_endpoint, &record.access_token).await
    {
        record.email = email;
        record.user_id = sub;
    }

    Ok(record)
}

/// Request a device code (public for live smoke tests).
pub async fn request_device_code(
    http: &reqwest::Client,
    options: &DeviceLoginOptions,
) -> Result<DeviceCodeResponse, AuthError> {
    let body = form_body(&[
        ("client_id", options.client_id.as_str()),
        ("scope", options.scope.as_str()),
    ]);
    let resp = http
        .post(&options.device_endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| AuthError::DeviceHttp(e.to_string()))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AuthError::DeviceHttp(e.to_string()))?;

    if !status.is_success() {
        return Err(AuthError::DeviceRejected(format!(
            "HTTP {status}: {}",
            snippet(&text)
        )));
    }

    serde_json::from_str(&text).map_err(|e| AuthError::DeviceDecode(e.to_string()))
}

async fn poll_for_token(
    http: &reqwest::Client,
    options: &DeviceLoginOptions,
    device_code: &str,
    mut interval_secs: u64,
    expires_in: u64,
) -> Result<TokenSuccess, AuthError> {
    let deadline = std::time::Instant::now() + Duration::from_secs(expires_in.max(1));
    let grant = "urn:ietf:params:oauth:grant-type:device_code";

    loop {
        if std::time::Instant::now() >= deadline {
            return Err(AuthError::DeviceExpired);
        }

        tokio::time::sleep(Duration::from_secs(interval_secs)).await;

        let body = form_body(&[
            ("grant_type", grant),
            ("device_code", device_code),
            ("client_id", options.client_id.as_str()),
        ]);
        let resp = http
            .post(&options.token_endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| AuthError::DeviceHttp(e.to_string()))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AuthError::DeviceHttp(e.to_string()))?;

        if status.is_success() {
            let tok: TokenSuccess =
                serde_json::from_str(&text).map_err(|e| AuthError::DeviceDecode(e.to_string()))?;
            if tok.access_token.is_empty() {
                return Err(AuthError::DeviceDecode(
                    "empty access_token in response".into(),
                ));
            }
            return Ok(tok);
        }

        // OAuth device polling errors are often HTTP 400 with JSON error codes.
        if let Ok(err) = serde_json::from_str::<TokenError>(&text) {
            match err.error.as_str() {
                "authorization_pending" => {
                    tracing::debug!("device login pending approval");
                    continue;
                }
                "slow_down" => {
                    interval_secs = interval_secs.saturating_add(5);
                    tracing::debug!(interval_secs, "device login slow_down");
                    continue;
                }
                "expired_token" | "expired_token_code" => {
                    return Err(AuthError::DeviceExpired);
                }
                "access_denied" => {
                    return Err(AuthError::DeviceDenied(
                        err.error_description.unwrap_or(err.error),
                    ));
                }
                other => {
                    return Err(AuthError::DeviceRejected(format!(
                        "{other}: {}",
                        err.error_description.unwrap_or_default()
                    )));
                }
            }
        }

        return Err(AuthError::DeviceRejected(format!(
            "HTTP {status}: {}",
            snippet(&text)
        )));
    }
}

async fn fetch_userinfo(
    http: &reqwest::Client,
    endpoint: &str,
    access_token: &str,
) -> Result<(Option<String>, Option<String>), AuthError> {
    let resp = http
        .get(endpoint)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| AuthError::DeviceHttp(e.to_string()))?;
    if !resp.status().is_success() {
        return Ok((None, None));
    }
    let info: UserInfo = resp
        .json()
        .await
        .map_err(|e| AuthError::DeviceDecode(e.to_string()))?;
    Ok((info.email, info.sub))
}

fn snippet(s: &str) -> String {
    s.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_device_code_response() {
        let raw = r#"{
          "device_code": "dc",
          "user_code": "ABCD-EFGH",
          "verification_uri": "https://accounts.x.ai/oauth2/device",
          "verification_uri_complete": "https://accounts.x.ai/oauth2/device?user_code=ABCD-EFGH",
          "expires_in": 1800,
          "interval": 5
        }"#;
        let d: DeviceCodeResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(d.user_code, "ABCD-EFGH");
        assert_eq!(d.interval, Some(5));
    }

    #[test]
    fn parse_token_error_pending() {
        let raw = r#"{"error":"authorization_pending","error_description":"still waiting"}"#;
        let e: TokenError = serde_json::from_str(raw).unwrap();
        assert_eq!(e.error, "authorization_pending");
    }

    #[test]
    fn default_options_use_known_client() {
        assert!(!DEFAULT_CLIENT_ID.is_empty());
        assert!(DEFAULT_SCOPE.contains("offline_access"));
    }

    #[tokio::test]
    #[ignore = "live xAI network"]
    async fn live_request_device_code() {
        let http = reqwest::Client::new();
        let d = request_device_code(&http, &DeviceLoginOptions::default())
            .await
            .expect("device code");
        assert!(!d.user_code.is_empty());
        assert!(!d.device_code.is_empty());
        assert!(d.verification_uri.contains("http"));
    }
}
