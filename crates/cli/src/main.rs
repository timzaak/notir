use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

mod args;
mod js_runtime;
mod output;
mod script_api;
mod ws_client;

use args::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let default_filter = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter)),
        )
        .init();

    let js_engine = js_runtime::JsEngine::new(cli.script.as_deref())?;
    let mut writer =
        output::OutputWriter::new(cli.output, cli.file_mode, &cli.output_dir, &cli.id)?;

    let mut attempts = 0u32;

    loop {
        match ws_client::run_client(&cli.server, &cli.id, cli.mode, &js_engine, &mut writer).await {
            Ok(()) => {
                tracing::info!("Disconnected");
                if !cli.reconnect {
                    break;
                }
            }
            Err(e) => {
                tracing::error!("Connection error: {}", e);
                if !cli.reconnect {
                    return Err(e);
                }
            }
        }

        // Reconnect logic
        if cli.reconnect {
            attempts += 1;
            if cli.max_reconnect > 0 && attempts > cli.max_reconnect {
                tracing::error!("Max reconnect attempts ({}) reached", cli.max_reconnect);
                break;
            }
            tracing::info!(
                "Reconnecting in {} seconds (attempt {}/{})",
                cli.reconnect_interval,
                attempts,
                if cli.max_reconnect > 0 {
                    cli.max_reconnect.to_string()
                } else {
                    "unlimited".to_string()
                }
            );
            tokio::time::sleep(tokio::time::Duration::from_secs(cli.reconnect_interval)).await;
        } else {
            break;
        }
    }

    Ok(())
}
