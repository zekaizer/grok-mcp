use std::path::PathBuf;

/// Errors from credential store I/O, import, and token refresh.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("auth store I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("auth store parse error at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("auth store has unsupported version {version} at {path}")]
    UnsupportedVersion { path: PathBuf, version: u32 },
    #[error("could not determine config directory for grok-mcp auth store")]
    NoConfigDir,
    #[error("Grok CLI auth file not found at {path}")]
    GrokCliMissing { path: PathBuf },
    #[error("Grok CLI auth file has no usable access token")]
    GrokCliNoUsableEntry,
    #[error("no credentials available (run grok-mcp auth import or auth login)")]
    NotAuthenticated,
    #[error("refresh token missing; re-authenticate")]
    NoRefreshToken,
    #[error("oidc client_id missing; cannot refresh")]
    NoClientId,
    #[error("token refresh HTTP error: {0}")]
    RefreshHttp(String),
    #[error("token refresh rejected: {0}")]
    RefreshRejected(String),
    #[error("token refresh response decode failed: {0}")]
    RefreshDecode(String),
    #[error("device-code HTTP error: {0}")]
    DeviceHttp(String),
    #[error("device-code rejected: {0}")]
    DeviceRejected(String),
    #[error("device-code response decode failed: {0}")]
    DeviceDecode(String),
    #[error("device-code expired; run auth login again")]
    DeviceExpired,
    #[error("device-code access denied: {0}")]
    DeviceDenied(String),
}
