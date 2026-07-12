//! Import credentials from Grok CLI into the grok-mcp store (ADR-0003).

use std::path::{Path, PathBuf};

use crate::error::AuthError;
use crate::grok_cli::record_from_grok_cli_path;
use crate::paths::grok_cli_auth_path;
use crate::store::{AuthRecord, resolve_store_path, save_store};

/// Result of a successful import.
#[derive(Debug, Clone)]
pub struct ImportResult {
    pub record: AuthRecord,
    pub store_path: PathBuf,
    pub source_path: PathBuf,
}

/// Import from `source` (or default `~/.grok/auth.json`) into the grok-mcp store.
///
/// Never mutates the CLI file. Overwrites the grok-mcp store.
pub fn import_from_grok_cli(
    source: Option<&Path>,
    store: Option<PathBuf>,
) -> Result<ImportResult, AuthError> {
    let source_path = source
        .map(Path::to_path_buf)
        .unwrap_or_else(grok_cli_auth_path);
    let store_path = resolve_store_path(store)?;
    let record = record_from_grok_cli_path(&source_path)?;
    save_store(&store_path, &record)?;
    tracing::info!(
        store = %store_path.display(),
        source = %source_path.display(),
        email = record.email.as_deref().unwrap_or("-"),
        "imported credentials from Grok CLI"
    );
    Ok(ImportResult {
        record,
        store_path,
        source_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use tempfile::TempDir;

    #[test]
    fn import_writes_store() {
        let dir = TempDir::new().unwrap();
        let cli = dir.path().join("cli-auth.json");
        let store = dir.path().join("mcp-auth.json");
        fs::write(
            &cli,
            r#"{
              "https://auth.x.ai::cid": {
                "key": "access",
                "refresh_token": "refresh",
                "expires_at": "2099-01-01T00:00:00Z",
                "email": "a@b.c",
                "oidc_client_id": "cid"
              }
            }"#,
        )
        .unwrap();

        let result = import_from_grok_cli(Some(&cli), Some(store.clone())).unwrap();
        assert_eq!(result.record.access_token, "access");
        assert!(store.is_file());
        let loaded = crate::store::load_store(&store).unwrap().unwrap();
        assert_eq!(loaded.access_token, "access");
        assert_eq!(loaded.source, crate::store::AuthSource::GrokCli);
    }
}
