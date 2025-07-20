use anyhow::Result;
use clap::Parser;
use log::debug;
use log::LevelFilter;

use orchestrator::Cli;
use shared::utils::signal::trigger_cancellation_on_signal;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let log_level = match cli.log_level.as_str() {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => anyhow::bail!("invalid log level: {}", cli.log_level),
    };
    env_logger::Builder::new()
        .filter_level(log_level)
        .format_timestamp(None)
        .filter_module("tracing::span", log::LevelFilter::Warn)
        .init();

    debug!("Log level: {log_level}");

    let cancellation_token = CancellationToken::new();
    let _signal_handle = trigger_cancellation_on_signal(cancellation_token.clone())?;

    tokio::select! {
        cmd_result = cli.run(cancellation_token.clone()) => {
            if let Err(e) = cmd_result {
                log::error!("Command execution error: {e}");
                cancellation_token.cancel();
            }
        }
        _ = cancellation_token.cancelled() => {
            log::info!("Received cancellation request");
        }
    }

    Ok(())
}
