//! Parse official Grok CLI / Grok Build `~/.grok/auth.json` (ADR-0003).

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::error::AuthError;
use crate::store::{AuthRecord, AuthSource};

/// One account entry in the Grok CLI auth map.
#[derive(Debug, Clone, Deserialize)]
pub struct GrokCliEntry {
    /// Access token (CLI field name is `key`).
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub oidc_issuer: Option<String>,
    #[serde(default)]
    pub oidc_client_id: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub auth_mode: Option<String>,
}

/// Top-level CLI file: map of `"issuer::client_id"` → entry.
pub type GrokCliFile = HashMap<String, GrokCliEntry>;

/// Load and parse a Grok CLI auth.json from disk.
pub fn load_grok_cli_file(path: &Path) -> Result<GrokCliFile, AuthError> {
    if !path.is_file() {
        return Err(AuthError::GrokCliMissing {
            path: path.to_path_buf(),
        });
    }
    let bytes = fs::read(path).map_err(|source| AuthError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_grok_cli_bytes(&bytes, path)
}

/// Parse CLI auth JSON bytes.
pub fn parse_grok_cli_bytes(bytes: &[u8], path: &Path) -> Result<GrokCliFile, AuthError> {
    serde_json::from_slice(bytes).map_err(|source| AuthError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Pick the best entry: prefer one with both access + refresh; else first with access.
pub fn select_entry(file: &GrokCliFile) -> Result<(&str, &GrokCliEntry), AuthError> {
    let mut with_refresh: Option<(&str, &GrokCliEntry)> = None;
    let mut with_access: Option<(&str, &GrokCliEntry)> = None;
    for (map_key, entry) in file {
        if entry.key.is_empty() {
            continue;
        }
        if with_access.is_none() {
            with_access = Some((map_key.as_str(), entry));
        }
        if entry.refresh_token.as_ref().is_some_and(|r| !r.is_empty()) {
            with_refresh = Some((map_key.as_str(), entry));
            break;
        }
    }
    with_refresh
        .or(with_access)
        .ok_or(AuthError::GrokCliNoUsableEntry)
}

fn client_id_from_map_key(map_key: &str) -> Option<String> {
    map_key
        .rsplit_once("::")
        .map(|(_, cid)| cid.to_string())
        .filter(|cid| !cid.is_empty())
}

/// Convert a CLI map entry into a grok-mcp [`AuthRecord`].
pub fn entry_to_record(map_key: &str, entry: &GrokCliEntry, imported_from: &Path) -> AuthRecord {
    let oidc_client_id = entry
        .oidc_client_id
        .clone()
        .or_else(|| client_id_from_map_key(map_key));

    AuthRecord {
        version: AuthRecord::CURRENT_VERSION,
        source: AuthSource::GrokCli,
        imported_from: Some(imported_from.display().to_string()),
        access_token: entry.key.clone(),
        refresh_token: entry.refresh_token.clone(),
        expires_at: entry.expires_at.clone(),
        oidc_issuer: entry
            .oidc_issuer
            .clone()
            .or_else(|| Some("https://auth.x.ai".into())),
        oidc_client_id,
        token_endpoint: Some(crate::DEFAULT_TOKEN_ENDPOINT.to_string()),
        email: entry.email.clone(),
        user_id: entry.user_id.clone(),
        updated_at: Some(crate::now_rfc3339()),
    }
}

/// Parse CLI file and build an AuthRecord (does not write).
pub fn record_from_grok_cli_path(path: &Path) -> Result<AuthRecord, AuthError> {
    let file = load_grok_cli_file(path)?;
    let (map_key, entry) = select_entry(&file)?;
    Ok(entry_to_record(map_key, entry, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const FIXTURE: &str = r#"{
  "https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828": {
    "key": "access-token-value",
    "auth_mode": "oidc",
    "refresh_token": "refresh-token-value",
    "expires_at": "2099-01-01T00:00:00Z",
    "oidc_issuer": "https://auth.x.ai",
    "oidc_client_id": "b1a00492-073a-47ea-816f-4c329264a828",
    "email": "user@example.com",
    "user_id": "u-1"
  }
}"#;

    #[test]
    fn parse_fixture_selects_entry() {
        let path = PathBuf::from("/tmp/fake-grok-auth.json");
        let file = parse_grok_cli_bytes(FIXTURE.as_bytes(), &path).unwrap();
        let (k, e) = select_entry(&file).unwrap();
        assert!(k.contains("auth.x.ai"));
        assert_eq!(e.key, "access-token-value");
        assert_eq!(e.refresh_token.as_deref(), Some("refresh-token-value"));
    }

    #[test]
    fn entry_to_record_maps_fields() {
        let path = PathBuf::from("/home/u/.grok/auth.json");
        let file = parse_grok_cli_bytes(FIXTURE.as_bytes(), &path).unwrap();
        let (k, e) = select_entry(&file).unwrap();
        let rec = entry_to_record(k, e, &path);
        assert_eq!(rec.access_token, "access-token-value");
        assert_eq!(rec.source, AuthSource::GrokCli);
        assert_eq!(rec.email.as_deref(), Some("user@example.com"));
        assert_eq!(
            rec.oidc_client_id.as_deref(),
            Some("b1a00492-073a-47ea-816f-4c329264a828")
        );
        assert!(rec.imported_from.as_ref().unwrap().contains("auth.json"));
    }

    #[test]
    fn client_id_from_map_key_when_field_missing() {
        let raw = r#"{
          "https://auth.x.ai::cid-from-key": {
            "key": "tok",
            "refresh_token": "r"
          }
        }"#;
        let path = PathBuf::from("/tmp/x.json");
        let file = parse_grok_cli_bytes(raw.as_bytes(), &path).unwrap();
        let (k, e) = select_entry(&file).unwrap();
        let rec = entry_to_record(k, e, &path);
        assert_eq!(rec.oidc_client_id.as_deref(), Some("cid-from-key"));
    }

    #[test]
    fn empty_file_errors() {
        let path = PathBuf::from("/tmp/empty.json");
        let file = parse_grok_cli_bytes(b"{}", &path).unwrap();
        assert!(matches!(
            select_entry(&file),
            Err(AuthError::GrokCliNoUsableEntry)
        ));
    }
}
