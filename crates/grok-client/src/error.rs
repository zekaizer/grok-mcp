/// Errors from the xAI HTTP client layer.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("failed to build HTTP client: {0}")]
    HttpBuild(#[source] reqwest::Error),
    #[error("HTTP request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("upstream status {status}: {body}")]
    Upstream { status: u16, body: String },
    #[error("failed to decode upstream JSON: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}
