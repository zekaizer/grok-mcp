//! grok-mcp MCP server: tool wiring over grok-auth + grok-client.
//!
//! Phase-1 tools are registered; research / x_search / ask_grok return a
//! structured "not implemented" body until the Responses client lands.

pub mod config;
pub mod envelope;
pub mod http;
pub mod jobs;
pub mod logging;
pub mod oauth;
pub mod server;
pub mod tools;
pub mod upstream;

pub use config::{Cli, Command, Config, HttpConfig, LogFormat, Transport};
pub use server::GrokMcpServer;
