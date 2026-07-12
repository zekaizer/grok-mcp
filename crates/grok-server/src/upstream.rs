//! Map auth/client errors onto tool_spec error codes and load a bearer token.

use grok_auth::{AuthError, load_valid_record};
use grok_client::ClientError;

use crate::GrokMcpServer;
use crate::envelope::{ErrorCode, Fail};

impl GrokMcpServer {
    /// Load a valid access token (refreshing if needed).
    pub(crate) async fn access_token(&self) -> Result<String, Fail> {
        match load_valid_record(self.client.http(), self.auth_file.clone()).await {
            Ok(rec) => Ok(rec.access_token),
            Err(e) => Err(auth_error_to_fail(&e)),
        }
    }
}

pub(crate) fn auth_error_to_fail(e: &AuthError) -> Fail {
    match e {
        AuthError::NotAuthenticated
        | AuthError::NoRefreshToken
        | AuthError::GrokCliMissing { .. }
        | AuthError::GrokCliNoUsableEntry => {
            Fail::new(ErrorCode::ReauthRequired, e.to_string(), false)
        }
        AuthError::RefreshRejected(msg) if msg.contains("401") || msg.contains("invalid_grant") => {
            Fail::new(ErrorCode::ReauthRequired, e.to_string(), false)
        }
        AuthError::RefreshRejected(msg) if msg.contains("403") => {
            Fail::new(ErrorCode::EntitlementDenied, e.to_string(), false)
        }
        AuthError::RefreshHttp(_) | AuthError::RefreshRejected(_) => {
            Fail::new(ErrorCode::UpstreamError, e.to_string(), true)
        }
        other => Fail::new(ErrorCode::UpstreamError, other.to_string(), false),
    }
}

pub(crate) fn client_error_to_fail(e: &ClientError) -> Fail {
    match e {
        ClientError::Upstream { status: 401, .. } => {
            Fail::new(ErrorCode::ReauthRequired, e.to_string(), false)
        }
        ClientError::Upstream { status: 403, .. } => {
            Fail::new(ErrorCode::EntitlementDenied, e.to_string(), false)
        }
        ClientError::Upstream { status: 429, .. } => {
            Fail::new(ErrorCode::RateLimited, e.to_string(), true)
        }
        ClientError::Upstream { .. } | ClientError::Request(_) => {
            Fail::new(ErrorCode::UpstreamError, e.to_string(), true)
        }
        ClientError::Decode(_) | ClientError::HttpBuild(_) => {
            Fail::new(ErrorCode::UpstreamError, e.to_string(), false)
        }
        ClientError::NotImplemented(msg) => Fail::not_implemented(msg),
    }
}
