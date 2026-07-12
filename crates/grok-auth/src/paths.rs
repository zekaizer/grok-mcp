use std::path::PathBuf;

use crate::AuthError;

const QUALIFIER: &str = "";
const ORGANIZATION: &str = "";
const APPLICATION: &str = "grok-mcp";

/// Default path for the grok-mcp owned auth store (`~/.config/grok-mcp/auth.json`
/// on Linux via the XDG base-dir convention).
pub fn default_auth_store_path() -> Result<PathBuf, AuthError> {
    let dirs = directories::ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .ok_or(AuthError::NoConfigDir)?;
    Ok(dirs.config_dir().join("auth.json"))
}

/// Path to the official Grok CLI / Grok Build credential file.
#[must_use]
pub fn grok_cli_auth_path() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.home_dir().join(".grok").join("auth.json"))
        .unwrap_or_else(|| PathBuf::from(".grok/auth.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_store_ends_with_auth_json() {
        let p = default_auth_store_path().expect("config dir");
        assert_eq!(p.file_name().and_then(|s| s.to_str()), Some("auth.json"));
        assert!(
            p.to_string_lossy().contains("grok-mcp"),
            "path should include app name: {p:?}"
        );
    }

    #[test]
    fn grok_cli_path_ends_with_auth_json() {
        let p = grok_cli_auth_path();
        assert_eq!(p.file_name().and_then(|s| s.to_str()), Some("auth.json"));
    }
}
