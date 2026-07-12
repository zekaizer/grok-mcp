//! Thin xAI Responses client (ADR-0002).

mod config;
mod error;
mod responses;

pub use config::{ClientConfig, DEFAULT_BASE_URL, DEFAULT_MODEL};
pub use error::ClientError;
pub use responses::{
    CreateResponseRequest, ReasoningParam, ResponseBody, Usage, UsageReport,
    count_output_tool_calls, extract_output_text, log_usage, parse_json_object, truncate_chars,
    debug_payload_budget, usage_report, verbosity_char_budget,
};

use reqwest::Client;

/// HTTP client bound to xAI base URL / default model.
#[derive(Debug, Clone)]
pub struct GrokClient {
    pub(crate) config: ClientConfig,
    pub(crate) http: Client,
}

impl GrokClient {
    /// Build a client with the given config.
    pub fn new(config: ClientConfig) -> Result<Self, ClientError> {
        let http = Client::builder()
            .user_agent(concat!("grok-mcp/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(ClientError::HttpBuild)?;
        Ok(Self { config, http })
    }

    #[must_use]
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }

    #[must_use]
    pub fn http(&self) -> &Client {
        &self.http
    }

    /// Resolve model id: per-call override or default.
    #[must_use]
    pub fn resolve_model(&self, override_model: Option<&str>) -> String {
        override_model
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(self.config.default_model.as_str())
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_builds_with_defaults() {
        let c = GrokClient::new(ClientConfig::default()).expect("build");
        assert_eq!(c.config().base_url, DEFAULT_BASE_URL);
        assert_eq!(c.config().default_model, DEFAULT_MODEL);
    }
}
