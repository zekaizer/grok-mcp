//! CLI and runtime configuration (CLI > env > default).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

/// Default Streamable HTTP bind (loopback).
pub const DEFAULT_HTTP_ADDR: &str = "127.0.0.1:8765";

/// grok-mcp command line.
#[derive(Debug, Parser)]
#[command(
    name = "grok-mcp",
    version,
    about = "MCP server: offload research and X search to xAI Grok (SuperGrok subscription)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Serve MCP over stdio (stdin/stdout). Logs go to stderr.
    #[arg(long, env = "GROK_MCP_STDIO")]
    pub stdio: bool,

    /// Serve MCP over Streamable HTTP.
    #[arg(long, conflicts_with = "stdio")]
    pub http: bool,

    /// HTTP bind address [default: 127.0.0.1:8766].
    #[arg(long, env = "GROK_MCP_HTTP_ADDR")]
    pub http_addr: Option<String>,

    /// Read HTTP bearer / OAuth gate token from this file (trimmed). Prefer over
    /// putting secrets on argv. Falls back to `GROK_MCP_HTTP_TOKEN`.
    #[arg(long, value_name = "PATH")]
    pub http_token_file: Option<PathBuf>,

    /// Comma-separated Host allowlist (`*` disables). Default: loopback hosts.
    #[arg(long, env = "GROK_MCP_HTTP_ALLOWED_HOSTS")]
    pub http_allowed_hosts: Option<String>,

    /// Comma-separated Origin allowlist (`*` disables). Default: loopback origins.
    #[arg(long, env = "GROK_MCP_HTTP_ALLOWED_ORIGINS")]
    pub http_allowed_origins: Option<String>,

    /// Server state directory (OAuth store). Default: `$XDG_STATE_HOME/grok-mcp`.
    #[arg(long, env = "GROK_MCP_STATE_DIR")]
    pub state_dir: Option<String>,

    /// Override auth store path (else `GROK_MCP_AUTH_FILE` or default).
    #[arg(long, env = "GROK_MCP_AUTH_FILE")]
    pub auth_file: Option<PathBuf>,

    /// Log format: `text` (default) or `json`.
    #[arg(long, env = "GROK_MCP_LOG_FORMAT", default_value = "text")]
    pub log_format: String,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage xAI credentials (import / device-code / status).
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuthAction {
    /// Print non-secret credential status.
    Status,
    /// OAuth device-code login (prints URL + code; polls until approved).
    Login,
    /// Import credentials from ~/.grok/auth.json into the grok-mcp store.
    Import,
    /// Delete the grok-mcp auth store only.
    Logout,
}

/// Selected MCP transport.
#[derive(Debug, Clone)]
pub enum Transport {
    Stdio,
    Http(HttpConfig),
}

/// Log encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Text,
    Json,
}

/// HTTP transport configuration.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub addr: SocketAddr,
    /// Static bearer + OAuth authorize gate. When set, OAuth AS is co-hosted.
    pub token: Option<String>,
    pub allowed_hosts: Option<Vec<String>>,
    pub allowed_origins: Option<Vec<String>>,
    pub state_dir: PathBuf,
}

impl HttpConfig {
    #[must_use]
    pub fn oauth_enabled(&self) -> bool {
        self.token.is_some()
    }
}

/// Resolved runtime config for serving MCP.
#[derive(Debug, Clone)]
pub struct Config {
    pub transport: Transport,
    pub auth_file: Option<PathBuf>,
    pub log_format: LogFormat,
}

impl Cli {
    pub fn log_format(&self) -> Result<LogFormat> {
        parse_log_format(&self.log_format)
    }
}

/// Build serve config from CLI.
pub fn config_for_serve(cli: &Cli) -> Result<Config> {
    let log_format = cli.log_format()?;
    let kind = if cli.http {
        TransportKind::Http
    } else if cli.stdio || std::env::var("GROK_MCP_TRANSPORT").as_deref() == Ok("stdio") {
        TransportKind::Stdio
    } else if std::env::var("GROK_MCP_TRANSPORT").as_deref() == Ok("http") {
        TransportKind::Http
    } else {
        TransportKind::Stdio
    };

    let transport = match kind {
        TransportKind::Stdio => Transport::Stdio,
        TransportKind::Http => {
            let token = resolve_token(
                cli.http_token_file.as_deref(),
                std::env::var("GROK_MCP_HTTP_TOKEN").ok(),
            )?;
            Transport::Http(HttpConfig::resolve(
                cli.http_addr.clone(),
                token,
                cli.http_allowed_hosts.clone(),
                cli.http_allowed_origins.clone(),
                resolve_state_dir(
                    cli.state_dir.clone(),
                    std::env::var("XDG_STATE_HOME").ok(),
                    std::env::var("HOME").ok(),
                ),
            )?)
        }
    };

    Ok(Config {
        transport,
        auth_file: cli.auth_file.clone(),
        log_format,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransportKind {
    Stdio,
    Http,
}

impl HttpConfig {
    fn resolve(
        addr: Option<String>,
        token: Option<String>,
        allowed_hosts: Option<String>,
        allowed_origins: Option<String>,
        state_dir: PathBuf,
    ) -> Result<Self> {
        let addr_str = addr
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_HTTP_ADDR.to_string());
        let addr: SocketAddr = addr_str.trim().parse().with_context(|| {
            format!("GROK_MCP_HTTP_ADDR is not a valid socket address: {addr_str:?}")
        })?;
        let token = token
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        let allowed_hosts = parse_allowlist(allowed_hosts, "GROK_MCP_HTTP_ALLOWED_HOSTS")?;
        let allowed_origins = parse_allowlist(allowed_origins, "GROK_MCP_HTTP_ALLOWED_ORIGINS")?;
        Ok(Self {
            addr,
            token,
            allowed_hosts,
            allowed_origins,
            state_dir,
        })
    }
}

fn resolve_token(token_file: Option<&Path>, env_token: Option<String>) -> Result<Option<String>> {
    if let Some(path) = token_file {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read HTTP token file {}", path.display()))?;
        return Ok(Some(raw));
    }
    Ok(env_token)
}

fn resolve_state_dir(
    explicit: Option<String>,
    xdg_state_home: Option<String>,
    home: Option<String>,
) -> PathBuf {
    let non_empty = |s: String| (!s.trim().is_empty()).then_some(s);
    if let Some(dir) = explicit.and_then(non_empty) {
        return PathBuf::from(dir.trim());
    }
    if let Some(xdg) = xdg_state_home.and_then(non_empty) {
        return PathBuf::from(xdg.trim()).join("grok-mcp");
    }
    if let Some(home) = home.and_then(non_empty) {
        return PathBuf::from(home.trim()).join(".local/state/grok-mcp");
    }
    PathBuf::from(".grok-mcp-state")
}

fn parse_allowlist(raw: Option<String>, var: &str) -> Result<Option<Vec<String>>> {
    match raw.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some("*") => Ok(Some(Vec::new())),
        Some(list) => {
            let items: Vec<String> = list
                .split(',')
                .map(|h| h.trim().to_string())
                .filter(|h| !h.is_empty())
                .collect();
            if items.is_empty() {
                bail!("{var} lists no entries; use \"*\" to disable the guard or unset it");
            }
            Ok(Some(items))
        }
    }
}

fn parse_log_format(s: &str) -> Result<LogFormat> {
    match s.to_ascii_lowercase().as_str() {
        "text" => Ok(LogFormat::Text),
        "json" => Ok(LogFormat::Json),
        other => bail!("invalid log format {other:?}; expected text or json"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_log_formats() {
        assert_eq!(parse_log_format("text").unwrap(), LogFormat::Text);
        assert_eq!(parse_log_format("JSON").unwrap(), LogFormat::Json);
        assert!(parse_log_format("xml").is_err());
    }

    #[test]
    fn http_defaults() {
        let cfg = HttpConfig::resolve(None, None, None, None, PathBuf::from("/tmp/x")).unwrap();
        assert_eq!(cfg.addr, DEFAULT_HTTP_ADDR.parse().unwrap());
        assert!(cfg.token.is_none());
        assert!(!cfg.oauth_enabled());
    }

    #[test]
    fn http_token_enables_oauth() {
        let cfg = HttpConfig::resolve(
            Some("0.0.0.0:9000".into()),
            Some("s3cret".into()),
            Some("*".into()),
            Some("*".into()),
            PathBuf::from("/tmp/x"),
        )
        .unwrap();
        assert_eq!(cfg.addr, "0.0.0.0:9000".parse().unwrap());
        assert!(cfg.oauth_enabled());
        assert_eq!(cfg.allowed_hosts, Some(vec![]));
    }

    #[test]
    fn allowlist_star_and_malformed() {
        assert_eq!(
            parse_allowlist(Some("*".into()), "V").unwrap(),
            Some(vec![])
        );
        assert!(parse_allowlist(Some(",".into()), "V").is_err());
    }
}
