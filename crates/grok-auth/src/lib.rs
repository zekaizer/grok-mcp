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

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use tokio::sync::Mutex;

/// Skew window before `expires_at` when we proactively refresh (5 minutes).
pub const REFRESH_SKEW_SECS: u64 = 300;

/// Serialize refresh + recovery so concurrent tool calls cannot race-rotate RTs.
static REFRESH_GATE: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// True when xAI rejected the refresh token (revoked / invalid_grant).
#[must_use]
pub fn is_refresh_revoked(err: &AuthError) -> bool {
    match err {
        AuthError::RefreshRejected(msg) => {
            let m = msg.to_ascii_lowercase();
            m.contains("invalid_grant") || m.contains("revoked") || m.contains("invalid_token")
        }
        AuthError::NoRefreshToken => true,
        _ => false,
    }
}

/// Prefer CLI record when it is strictly fresher (later expires_at) or store is empty of RT.
#[must_use]
pub fn should_adopt_cli(cli: &AuthRecord, store: Option<&AuthRecord>) -> bool {
    if !cli.has_access_token() {
        return false;
    }
    let Some(store) = store else {
        return true;
    };
    if !store.has_access_token() {
        return true;
    }
    let cli_exp = cli.expires_at.as_deref().and_then(parse_rfc3339_unix);
    let store_exp = store.expires_at.as_deref().and_then(parse_rfc3339_unix);
    match (cli_exp, store_exp) {
        (Some(c), Some(s)) if c > s => true,
        (Some(_), None) => true,
        _ => {
            // Different refresh token with equal/unknown expiry: adopt if store RT missing.
            store.refresh_token.as_ref().is_none_or(|r| r.is_empty())
                && cli
                    .refresh_token
                    .as_ref()
                    .is_some_and(|r| !r.is_empty())
        }
    }
}

fn try_cli_record() -> Option<AuthRecord> {
    record_from_grok_cli_path(&grok_cli_auth_path()).ok()
}

/// Load store, or seed from Grok CLI when missing (zero-touch import).
fn load_or_seed_store(store_path: &Path) -> Result<AuthRecord, AuthError> {
    if let Some(rec) = load_store(store_path)?
        && rec.has_access_token()
    {
        return Ok(rec);
    }
    let cli = try_cli_record().ok_or(AuthError::NotAuthenticated)?;
    save_store(store_path, &cli)?;
    tracing::info!(
        store = %store_path.display(),
        expires_at = cli.expires_at.as_deref().unwrap_or("-"),
        "seeded grok-mcp auth store from Grok CLI"
    );
    Ok(cli)
}

/// If CLI is fresher, copy into the mcp store (does not mutate CLI file).
fn maybe_adopt_fresher_cli(store_path: &Path, store: &AuthRecord) -> Result<AuthRecord, AuthError> {
    let Some(cli) = try_cli_record() else {
        return Ok(store.clone());
    };
    if !should_adopt_cli(&cli, Some(store)) {
        return Ok(store.clone());
    }
    // Avoid no-op write when tokens identical.
    if cli.refresh_token == store.refresh_token && cli.access_token == store.access_token {
        return Ok(store.clone());
    }
    save_store(store_path, &cli)?;
    tracing::info!(
        store = %store_path.display(),
        expires_at = cli.expires_at.as_deref().unwrap_or("-"),
        "adopted fresher Grok CLI credentials into grok-mcp store"
    );
    Ok(cli)
}

async fn refresh_and_save(
    http: &reqwest::Client,
    store_path: &Path,
    record: &mut AuthRecord,
) -> Result<(), AuthError> {
    refresh_record(http, record).await?;
    save_store(store_path, record)?;
    Ok(())
}

/// After invalid_grant: re-read disk, adopt CLI if possible, retry refresh once.
async fn recover_after_revoked(
    http: &reqwest::Client,
    store_path: &Path,
    failed: &AuthRecord,
) -> Result<AuthRecord, AuthError> {
    // 1) Another process may have refreshed the store.
    if let Some(disk) = load_store(store_path)?
        && disk.has_access_token()
    {
        if !needs_refresh(disk.expires_at.as_deref(), REFRESH_SKEW_SECS) {
            tracing::info!("auth recovery: store already refreshed by peer");
            return Ok(disk);
        }
        if disk.refresh_token != failed.refresh_token {
            let mut disk = disk;
            match refresh_and_save(http, store_path, &mut disk).await {
                Ok(()) => {
                    tracing::info!("auth recovery: refreshed using updated store RT");
                    return Ok(disk);
                }
                Err(e) if is_refresh_revoked(&e) => {
                    tracing::warn!(error = %e, "auth recovery: store RT still revoked");
                }
                Err(e) => return Err(e),
            }
        }
    }

    // 2) Re-import from Grok CLI (Build keeps the live session).
    let Some(cli) = try_cli_record() else {
        return Err(AuthError::RefreshRejected(
            "refresh token revoked and Grok CLI auth not available".into(),
        ));
    };

    if cli.refresh_token == failed.refresh_token
        && needs_refresh(cli.expires_at.as_deref(), REFRESH_SKEW_SECS)
    {
        // CLI holds the same dead RT — human login required.
        return Err(AuthError::RefreshRejected(
            "refresh token revoked (Grok CLI has the same token); run: grok-mcp auth login"
                .into(),
        ));
    }

    save_store(store_path, &cli)?;
    tracing::info!(
        expires_at = cli.expires_at.as_deref().unwrap_or("-"),
        "auth recovery: re-imported from Grok CLI after invalid_grant"
    );

    if !needs_refresh(cli.expires_at.as_deref(), REFRESH_SKEW_SECS) {
        return Ok(cli);
    }

    let mut cli = cli;
    refresh_and_save(http, store_path, &mut cli).await?;
    Ok(cli)
}

/// Load store credentials, refresh if near expiry, persist, and return a usable record.
///
/// Zero-touch behavior:
/// - Seed from `~/.grok/auth.json` when the mcp store is missing.
/// - Adopt a strictly fresher CLI session before refresh.
/// - Single-flight refresh (no concurrent RT rotation).
/// - On `invalid_grant`, re-read store / re-import CLI and retry once.
pub async fn load_valid_record(
    http: &reqwest::Client,
    explicit_store: Option<PathBuf>,
) -> Result<AuthRecord, AuthError> {
    let store_path = resolve_store_path(explicit_store)?;
    let mut record = load_or_seed_store(&store_path)?;
    record = maybe_adopt_fresher_cli(&store_path, &record)?;

    if !needs_refresh(record.expires_at.as_deref(), REFRESH_SKEW_SECS) {
        return Ok(record);
    }

    let _gate = REFRESH_GATE.lock().await;

    // Re-load under lock: another task may have finished refresh.
    record = load_or_seed_store(&store_path)?;
    record = maybe_adopt_fresher_cli(&store_path, &record)?;
    if !needs_refresh(record.expires_at.as_deref(), REFRESH_SKEW_SECS) {
        return Ok(record);
    }

    match refresh_and_save(http, &store_path, &mut record).await {
        Ok(()) => Ok(record),
        Err(e) if is_refresh_revoked(&e) => {
            tracing::warn!(error = %e, "access token refresh rejected; attempting zero-touch recovery");
            recover_after_revoked(http, &store_path, &record).await
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::AuthSource;

    fn rec(access: &str, refresh: Option<&str>, expires: Option<&str>) -> AuthRecord {
        AuthRecord {
            version: AuthRecord::CURRENT_VERSION,
            source: AuthSource::GrokCli,
            imported_from: None,
            access_token: access.into(),
            refresh_token: refresh.map(str::to_string),
            expires_at: expires.map(str::to_string),
            oidc_issuer: None,
            oidc_client_id: Some("cid".into()),
            token_endpoint: None,
            email: None,
            user_id: None,
            updated_at: None,
        }
    }

    #[test]
    fn revoked_detection() {
        assert!(is_refresh_revoked(&AuthError::RefreshRejected(
            r#"HTTP 400 Bad Request: {"error":"invalid_grant","error_description":"Refresh token has been revoked"}"#.into()
        )));
        assert!(is_refresh_revoked(&AuthError::NoRefreshToken));
        assert!(!is_refresh_revoked(&AuthError::RefreshHttp("timeout".into())));
    }

    #[test]
    fn adopt_when_cli_expires_later() {
        let store = rec("a", Some("rt-old"), Some("2026-07-12T16:53:44Z"));
        let cli = rec("b", Some("rt-new"), Some("2026-07-12T22:50:32Z"));
        assert!(should_adopt_cli(&cli, Some(&store)));
        assert!(!should_adopt_cli(&store, Some(&cli)));
    }

    #[test]
    fn adopt_when_store_missing() {
        let cli = rec("a", Some("rt"), Some("2099-01-01T00:00:00Z"));
        assert!(should_adopt_cli(&cli, None));
    }

    #[test]
    fn seed_and_adopt_roundtrip_on_disk() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = dir.path().join("mcp.json");
        let cli_path = dir.path().join("cli.json");
        std::fs::write(
            &cli_path,
            r#"{
              "https://auth.x.ai::cid": {
                "key": "cli-access",
                "refresh_token": "cli-refresh",
                "expires_at": "2099-06-01T00:00:00Z",
                "email": "a@b.c",
                "oidc_client_id": "cid"
              }
            }"#,
        )
        .unwrap();

        // Simulate load_or_seed with explicit CLI path via record_from + save
        let seeded = record_from_grok_cli_path(&cli_path).unwrap();
        save_store(&store, &seeded).unwrap();
        let loaded = load_store(&store).unwrap().unwrap();
        assert_eq!(loaded.access_token, "cli-access");

        // Older store, newer CLI → adopt
        let mut old = loaded.clone();
        old.expires_at = Some("2020-01-01T00:00:00Z".into());
        old.refresh_token = Some("stale-rt".into());
        save_store(&store, &old).unwrap();

        let cli = record_from_grok_cli_path(&cli_path).unwrap();
        assert!(should_adopt_cli(&cli, Some(&old)));
        save_store(&store, &cli).unwrap();
        let after = load_store(&store).unwrap().unwrap();
        assert_eq!(after.refresh_token.as_deref(), Some("cli-refresh"));
    }
}
