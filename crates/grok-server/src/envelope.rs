//! Shared success / error JSON shapes from docs/tool_spec.md.

use rmcp::ErrorData;
use rmcp::schemars::JsonSchema;
use serde::Serialize;

/// Machine-readable error codes (tool_spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[schemars(crate = "rmcp::schemars")]
pub enum ErrorCode {
    ReauthRequired,
    EntitlementDenied,
    RateLimited,
    UpstreamError,
    InvalidParams,
    EvidenceUnavailable,
    OutputTruncated,
    ApiKeyDisabled,
    Timeout,
    NotImplemented,
}

impl ErrorCode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReauthRequired => "REAUTH_REQUIRED",
            Self::EntitlementDenied => "ENTITLEMENT_DENIED",
            Self::RateLimited => "RATE_LIMITED",
            Self::UpstreamError => "UPSTREAM_ERROR",
            Self::InvalidParams => "INVALID_PARAMS",
            Self::EvidenceUnavailable => "EVIDENCE_UNAVAILABLE",
            Self::OutputTruncated => "OUTPUT_TRUNCATED",
            Self::ApiKeyDisabled => "API_KEY_DISABLED",
            Self::Timeout => "TIMEOUT",
            Self::NotImplemented => "NOT_IMPLEMENTED",
        }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Tool-level failure wrapper (`ok: false`) — also mapped to MCP ErrorData.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Fail {
    pub ok: bool,
    pub error: ErrorBody,
}

impl Fail {
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            ok: false,
            error: ErrorBody {
                code,
                message: message.into(),
                retryable,
                details: None,
            },
        }
    }

    #[must_use]
    pub fn not_implemented(what: &str) -> Self {
        Self::new(
            ErrorCode::NotImplemented,
            format!("{what} is not implemented yet"),
            false,
        )
    }

    /// Map to rmcp ErrorData (invalid_params vs internal by code).
    #[must_use]
    pub fn into_error_data(self) -> ErrorData {
        let msg = format!("{}: {}", self.error.code.as_str(), self.error.message);
        let data = serde_json::to_value(&self).ok();
        match self.error.code {
            ErrorCode::InvalidParams => ErrorData::invalid_params(msg, data),
            ErrorCode::ReauthRequired
            | ErrorCode::EntitlementDenied
            | ErrorCode::RateLimited
            | ErrorCode::UpstreamError
            | ErrorCode::Timeout
            | ErrorCode::ApiKeyDisabled
            | ErrorCode::OutputTruncated
            | ErrorCode::EvidenceUnavailable
            | ErrorCode::NotImplemented => ErrorData::internal_error(msg, data),
        }
    }
}
