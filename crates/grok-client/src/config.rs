//! Client-side defaults (env override applied by the server layer).

/// Default xAI API base (tool_spec / ADR-0003).
pub const DEFAULT_BASE_URL: &str = "https://api.x.ai/v1";

/// Default model id until `GROK_MCP_DEFAULT_MODEL` or per-call override.
/// Update when the allowlist lands; keep a stable documented default.
pub const DEFAULT_MODEL: &str = "grok-4.5";

/// Configuration for [`crate::GrokClient`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    pub base_url: String,
    pub default_model: String,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            default_model: DEFAULT_MODEL.to_string(),
        }
    }
}

impl ClientConfig {
    /// Apply env overrides (`GROK_MCP_BASE_URL`, `GROK_MCP_DEFAULT_MODEL`).
    #[must_use]
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("GROK_MCP_BASE_URL")
            && !v.trim().is_empty()
        {
            cfg.base_url = v.trim().trim_end_matches('/').to_string();
        }
        if let Ok(v) = std::env::var("GROK_MCP_DEFAULT_MODEL")
            && !v.trim().is_empty()
        {
            cfg.default_model = v.trim().to_string();
        }
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_urls() {
        let c = ClientConfig::default();
        assert!(c.base_url.starts_with("https://"));
        assert!(!c.default_model.is_empty());
    }
}
