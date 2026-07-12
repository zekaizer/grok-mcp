//! xAI credential resolution and storage (ADR-0003).

mod device;
mod error;
mod form;
mod grok_cli;
mod import;
mod paths;
mod refresh;
mod store;
mod timeutil;

pub use device::{
    DEFAULT_CLIENT_ID, DEFAULT_DEVICE_ENDPOINT, DEFAULT_SCOPE, DEFAULT_USERINFO_ENDPOINT,
    DeviceCodeResponse, DeviceLoginOptions, DevicePrompt, device_login, request_device_code,
};
pub use error::AuthError;
pub use grok_cli::{
    GrokCliEntry, entry_to_record, load_grok_cli_file, parse_grok_cli_bytes,
    record_from_grok_cli_path, select_entry,
};
pub use import::{ImportResult, import_from_grok_cli};
pub use paths::{default_auth_store_path, grok_cli_auth_path};
pub use refresh::{DEFAULT_TOKEN_ENDPOINT, refresh_record};
pub use store::{
    AuthRecord, AuthSource, BillingPath, CredentialSnapshot, api_key_opt_in, api_key_present,
    delete_store, load_store, resolve, resolve_store_path, save_store, status_snapshot,
};
pub use timeutil::{expires_at_from_expires_in, needs_refresh, now_rfc3339, parse_rfc3339_unix};

/// Skew window before `expires_at` when we proactively refresh (5 minutes).
pub const REFRESH_SKEW_SECS: u64 = 300;

/// Load store credentials, refresh if near expiry, persist, and return a usable record.
pub async fn load_valid_record(
    http: &reqwest::Client,
    explicit_store: Option<std::path::PathBuf>,
) -> Result<AuthRecord, AuthError> {
    let store_path = resolve_store_path(explicit_store)?;
    let mut record = load_store(&store_path)?.ok_or(AuthError::NotAuthenticated)?;
    if !record.has_access_token() {
        return Err(AuthError::NotAuthenticated);
    }

    if needs_refresh(record.expires_at.as_deref(), REFRESH_SKEW_SECS) {
        refresh_record(http, &mut record).await?;
        save_store(&store_path, &record)?;
    }
    Ok(record)
}
