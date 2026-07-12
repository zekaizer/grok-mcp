//! On-disk auth record and non-secret status snapshots (ADR-0003).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AuthError;
use crate::paths::{default_auth_store_path, grok_cli_auth_path};

/// Where a credential was loaded from (public, non-secret).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthSource {
    GrokCli,
    DeviceCode,
    ApiKey,
    None,
}

/// Which billing path would be used for the next request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingPath {
    SubscriptionOauth,
    ApiKey,
    None,
}

/// Versioned grok-mcp auth store (secrets stay on disk; never log tokens).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRecord {
    pub version: u32,
    pub source: AuthSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<String>,
    /// Access token — never expose via MCP tools.
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl AuthRecord {
    pub const CURRENT_VERSION: u32 = 1;

    #[must_use]
    pub fn has_access_token(&self) -> bool {
        !self.access_token.is_empty()
    }
}

/// Non-secret view for `auth_status` / CLI (tool_spec).
#[derive(Debug, Clone, Serialize)]
pub struct CredentialSnapshot {
    pub authenticated: bool,
    pub billing_path: BillingPath,
    pub source: AuthSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    pub store_path: PathBuf,
    pub grok_cli_path: PathBuf,
    pub grok_cli_present: bool,
    pub api_key_opt_in: bool,
    pub api_key_present: bool,
}

/// Load a grok-mcp store file if it exists.
pub fn load_store(path: &Path) -> Result<Option<AuthRecord>, AuthError> {
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|source| AuthError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let record: AuthRecord = serde_json::from_slice(&bytes).map_err(|source| AuthError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    if record.version != AuthRecord::CURRENT_VERSION {
        return Err(AuthError::UnsupportedVersion {
            path: path.to_path_buf(),
            version: record.version,
        });
    }
    Ok(Some(record))
}

/// Atomically write the store (temp file + rename). Parent dirs are created.
pub fn save_store(path: &Path, record: &AuthRecord) -> Result<(), AuthError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| AuthError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let json = serde_json::to_vec_pretty(record).map_err(|source| AuthError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|source| AuthError::Io {
        path: tmp.clone(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, path).map_err(|source| AuthError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Delete the grok-mcp store if present. Missing file is success.
pub fn delete_store(path: &Path) -> Result<(), AuthError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(AuthError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Resolve the effective store path: `GROK_MCP_AUTH_FILE` or default.
pub fn resolve_store_path(explicit: Option<PathBuf>) -> Result<PathBuf, AuthError> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    if let Ok(p) = std::env::var("GROK_MCP_AUTH_FILE")
        && !p.is_empty()
    {
        return Ok(PathBuf::from(p));
    }
    default_auth_store_path()
}

/// Whether API-key billing is allowed (`GROK_MCP_ALLOW_API_KEY`).
#[must_use]
pub fn api_key_opt_in() -> bool {
    match std::env::var("GROK_MCP_ALLOW_API_KEY") {
        Ok(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => false,
    }
}

#[must_use]
pub fn api_key_present() -> bool {
    std::env::var("XAI_API_KEY")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

/// Load credentials per ADR-0003 order (store only; no import side effects yet).
///
/// Returns `Ok(None)` when nothing usable is on disk and API key path is off
/// or empty. Does not read tokens into logs.
pub fn resolve(explicit_store: Option<PathBuf>) -> Result<Option<AuthRecord>, AuthError> {
    let store_path = resolve_store_path(explicit_store)?;
    if let Some(record) = load_store(&store_path)?
        && record.has_access_token()
    {
        return Ok(Some(record));
    }
    // Import-from-CLI is a CLI action (`auth import`); runtime resolve does not
    // copy yet so we never surprise-write. Callers can still detect CLI presence
    // via `status_snapshot`.
    Ok(None)
}

/// Build a non-secret status snapshot for tools and CLI.
pub fn status_snapshot(explicit_store: Option<PathBuf>) -> Result<CredentialSnapshot, AuthError> {
    let store_path = resolve_store_path(explicit_store)?;
    let grok_cli_path = grok_cli_auth_path();
    let grok_cli_present = grok_cli_path.is_file();
    let opt_in = api_key_opt_in();
    let key_present = api_key_present();

    let record = load_store(&store_path)?;
    let oauth_ok = record.as_ref().is_some_and(AuthRecord::has_access_token);

    let (billing_path, source, authenticated, expires_at, email, user_id) = if oauth_ok {
        let r = record.as_ref().expect("checked");
        (
            BillingPath::SubscriptionOauth,
            r.source,
            true,
            r.expires_at.clone(),
            r.email.clone(),
            r.user_id.clone(),
        )
    } else if opt_in && key_present {
        (
            BillingPath::ApiKey,
            AuthSource::ApiKey,
            true,
            None,
            None,
            None,
        )
    } else {
        (BillingPath::None, AuthSource::None, false, None, None, None)
    };

    Ok(CredentialSnapshot {
        authenticated,
        billing_path,
        source,
        expires_at,
        email,
        user_id,
        store_path,
        grok_cli_path,
        grok_cli_present,
        api_key_opt_in: opt_in,
        api_key_present: key_present,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_record() -> AuthRecord {
        AuthRecord {
            version: AuthRecord::CURRENT_VERSION,
            source: AuthSource::DeviceCode,
            imported_from: None,
            access_token: "test-access".into(),
            refresh_token: Some("test-refresh".into()),
            expires_at: Some("2099-01-01T00:00:00Z".into()),
            oidc_issuer: Some("https://auth.x.ai".into()),
            oidc_client_id: Some("client".into()),
            token_endpoint: None,
            email: Some("user@example.com".into()),
            user_id: Some("u1".into()),
            updated_at: None,
        }
    }

    #[test]
    fn round_trip_store() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("auth.json");
        save_store(&path, &sample_record()).unwrap();
        let loaded = load_store(&path).unwrap().expect("present");
        assert_eq!(loaded.access_token, "test-access");
        assert_eq!(loaded.source, AuthSource::DeviceCode);
    }

    #[test]
    fn delete_missing_is_ok() {
        let dir = TempDir::new().unwrap();
        delete_store(&dir.path().join("nope.json")).unwrap();
    }

    #[test]
    fn status_unauthenticated_without_store() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("auth.json");
        let snap = status_snapshot(Some(path)).unwrap();
        assert!(!snap.authenticated);
        assert_eq!(snap.billing_path, BillingPath::None);
    }
}
