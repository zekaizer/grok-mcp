//! OAuth refresh_token grant against xAI (`https://auth.x.ai/oauth2/token`).

use serde::Deserialize;

use crate::error::AuthError;
use crate::form::form_body;
use crate::store::AuthRecord;
use crate::timeutil::{expires_at_from_expires_in, now_rfc3339};

/// Default token endpoint from OIDC discovery (`auth.x.ai`).
pub const DEFAULT_TOKEN_ENDPOINT: &str = "https://auth.x.ai/oauth2/token";

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    token_type: Option<String>,
}

/// Refresh `record` in place using `http`. Updates access/refresh/expiry fields.
pub async fn refresh_record(
    http: &reqwest::Client,
    record: &mut AuthRecord,
) -> Result<(), AuthError> {
    let refresh = record
        .refresh_token
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or(AuthError::NoRefreshToken)?;
    let client_id = record
        .oidc_client_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or(AuthError::NoClientId)?;
    let endpoint = record
        .token_endpoint
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_TOKEN_ENDPOINT);

    let body = form_body(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", client_id),
    ]);
    let resp = http
        .post(endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| AuthError::RefreshHttp(e.to_string()))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| AuthError::RefreshHttp(e.to_string()))?;

    if !status.is_success() {
        let snippet: String = body.chars().take(300).collect();
        return Err(AuthError::RefreshRejected(format!(
            "HTTP {status}: {snippet}"
        )));
    }

    let token: TokenResponse =
        serde_json::from_str(&body).map_err(|e| AuthError::RefreshDecode(e.to_string()))?;

    if token.access_token.is_empty() {
        return Err(AuthError::RefreshDecode(
            "empty access_token in response".into(),
        ));
    }

    record.access_token = token.access_token;
    if let Some(rt) = token.refresh_token.filter(|s| !s.is_empty()) {
        record.refresh_token = Some(rt);
    }
    if let Some(exp_in) = token.expires_in {
        record.expires_at = Some(expires_at_from_expires_in(exp_in));
    }
    record.updated_at = Some(now_rfc3339());
    let _ = token.token_type;

    tracing::info!(
        expires_at = record.expires_at.as_deref().unwrap_or("-"),
        "refreshed access token"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_token_response() {
        let raw = r#"{
          "access_token": "new-access",
          "refresh_token": "new-refresh",
          "expires_in": 3600,
          "token_type": "Bearer"
        }"#;
        let t: TokenResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(t.access_token, "new-access");
        assert_eq!(t.expires_in, Some(3600));
    }
}
