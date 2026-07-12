//! Logging always targets stderr (stdout is the MCP JSON-RPC channel under stdio).

use anyhow::Result;
use tracing_subscriber::EnvFilter;

use crate::config::LogFormat;

/// Install the global tracing subscriber. Safe to call once at process start.
pub fn init(format: LogFormat) -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    match format {
        LogFormat::Text => {
            tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .with_env_filter(filter)
                .with_target(true)
                .init();
        }
        LogFormat::Json => {
            tracing_subscriber::fmt()
                .json()
                .flatten_event(true)
                .with_current_span(false)
                .with_span_list(false)
                .with_writer(std::io::stderr)
                .with_env_filter(filter)
                .init();
        }
    }
    Ok(())
}
