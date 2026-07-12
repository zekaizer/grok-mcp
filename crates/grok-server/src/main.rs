//! grok-mcp — MCP server binary (stdio Phase A; Streamable HTTP Phase B).

use anyhow::Result;
use clap::Parser;
use grok_auth::{
    DeviceLoginOptions, REFRESH_SKEW_SECS, delete_store, device_login, import_from_grok_cli,
    load_valid_record, needs_refresh, refresh_record, resolve_store_path, save_store,
    status_snapshot,
};
use grok_client::{ClientConfig, GrokClient};
use grok_server::GrokMcpServer;
use grok_server::config::{AuthAction, Cli, Command, Transport, config_for_serve};
use grok_server::logging;
use rmcp::ServiceExt;
use rmcp::transport::stdio;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let log_format = cli.log_format()?;
    logging::init(log_format)?;

    match &cli.command {
        Some(Command::Auth { action }) => run_auth(action, cli.auth_file.clone()).await,
        None => run_serve(cli).await,
    }
}

async fn run_auth(action: &AuthAction, auth_file: Option<std::path::PathBuf>) -> Result<()> {
    match action {
        AuthAction::Status => {
            let snap = status_snapshot(auth_file)?;
            println!("authenticated:  {}", snap.authenticated);
            println!("billing_path:   {:?}", snap.billing_path);
            println!("source:         {:?}", snap.source);
            println!("store:          {}", snap.store_path.display());
            println!(
                "grok_cli:       {} (present={})",
                snap.grok_cli_path.display(),
                snap.grok_cli_present
            );
            println!("api_key_opt_in: {}", snap.api_key_opt_in);
            println!("api_key_present:{}", snap.api_key_present);
            if let Some(exp) = &snap.expires_at {
                println!("expires_at:     {exp}");
            }
            if let Some(email) = &snap.email {
                println!("email:          {email}");
            }
            Ok(())
        }
        AuthAction::Login => {
            let store_path = resolve_store_path(auth_file)?;
            let http = reqwest::Client::builder()
                .user_agent(concat!("grok-mcp/", env!("CARGO_PKG_VERSION")))
                .timeout(std::time::Duration::from_secs(60))
                .build()?;
            let options = DeviceLoginOptions::default();
            println!("Starting xAI device-code login…");
            let record = device_login(&http, &options, |prompt| {
                println!();
                if let Some(url) = &prompt.verification_uri_complete {
                    println!("Open this URL in a browser:");
                    println!("  {url}");
                } else {
                    println!("Open this URL in a browser:");
                    println!("  {}", prompt.verification_uri);
                    println!("Enter code: {}", prompt.user_code);
                }
                println!();
                println!("User code:  {}", prompt.user_code);
                println!(
                    "Expires in: {}s (polling every {}s)",
                    prompt.expires_in, prompt.interval
                );
                println!("Waiting for approval…");
            })
            .await?;
            save_store(&store_path, &record)?;
            println!();
            println!("login successful");
            println!("stored at     {}", store_path.display());
            if let Some(email) = &record.email {
                println!("email         {email}");
            }
            if let Some(exp) = &record.expires_at {
                println!("expires_at    {exp}");
            }
            println!("source        {:?}", record.source);
            Ok(())
        }
        AuthAction::Import => {
            let result = import_from_grok_cli(None, auth_file.clone())?;
            let mut record = result.record;
            let http = reqwest::Client::builder()
                .user_agent(concat!("grok-mcp/", env!("CARGO_PKG_VERSION")))
                .build()?;

            if needs_refresh(record.expires_at.as_deref(), REFRESH_SKEW_SECS) {
                println!("access token near expiry; refreshing…");
                refresh_record(&http, &mut record).await?;
                save_store(&result.store_path, &record)?;
            }

            println!("imported from {}", result.source_path.display());
            println!("stored at     {}", result.store_path.display());
            if let Some(email) = &record.email {
                println!("email         {email}");
            }
            if let Some(exp) = &record.expires_at {
                println!("expires_at    {exp}");
            }
            println!("source        {:?}", record.source);
            Ok(())
        }
        AuthAction::Logout => {
            let path = resolve_store_path(auth_file)?;
            delete_store(&path)?;
            tracing::info!(path = %path.display(), "auth store removed");
            println!("logged out (removed {})", path.display());
            Ok(())
        }
    }
}

async fn run_serve(cli: Cli) -> Result<()> {
    let config = config_for_serve(&cli)?;
    let client = GrokClient::new(ClientConfig::from_env())?;

    match load_valid_record(client.http(), config.auth_file.clone()).await {
        Ok(rec) => {
            tracing::info!(
                email = rec.email.as_deref().unwrap_or("-"),
                expires_at = rec.expires_at.as_deref().unwrap_or("-"),
                "xAI credentials ready"
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "no valid xAI credentials yet (auth import | auth login)");
        }
    }

    let server = GrokMcpServer::new(config.auth_file.clone(), client);

    match config.transport {
        Transport::Stdio => {
            tracing::info!("starting grok-mcp on stdio");
            let service = server.serve(stdio()).await?;
            service.waiting().await?;
        }
        Transport::Http(http_cfg) => {
            grok_server::http::serve(server, &http_cfg).await?;
        }
    }
    Ok(())
}
