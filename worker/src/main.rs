mod api;
mod checks;
mod cli;
mod console;
mod docker;
mod metrics;
mod operations;
mod services;
use clap::Parser;
use cli::{execute_command, Cli};
use log::{debug, LevelFilter};
use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
pub type TaskHandles = Arc<Mutex<Vec<JoinHandle<()>>>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let task_handles: TaskHandles = Arc::new(Mutex::new(Vec::<JoinHandle<()>>::new()));
    env_logger::Builder::new()
        .filter_level(LevelFilter::Info)
        .format_timestamp(None)
        .init();

    debug!("Parsing CLI arguments");
    let cli = Cli::parse();
    debug!("Executing command");

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sighup = signal(SignalKind::hangup())?;
    let mut sigquit = signal(SignalKind::quit())?;

    let cancellation_token = CancellationToken::new();
    let signal_token = cancellation_token.clone();
    let command_token = cancellation_token.clone();
    let signal_handle = tokio::spawn(async move {
        tokio::select! {
            _ = sigterm.recv() => {
                log::info!("Received termination signal");
            }
            _ = sigint.recv() => {
                log::info!("Received interrupt signal");
            }
            _ = sighup.recv() => {
                log::info!("Received hangup signal");
            }
            _ = sigquit.recv() => {
                log::info!("Received quit signal");
            }
        }
        signal_token.cancel();
    });
    task_handles.lock().await.push(signal_handle);

    let task_handles_clone = task_handles.clone();

    tokio::select! {
        cmd_result = execute_command(&cli.command, command_token, task_handles_clone) => {
            if let Err(e) = cmd_result {
                log::error!("Command execution error: {}", e);
            }
        }
        _ = cancellation_token.cancelled() => {
            log::info!("Received cancellation request");
        }
    }

    let mut handles = task_handles.lock().await;

    for handle in handles.iter() {
        handle.abort();
    }

    // Wait for all tasks to finish/abort with timeout
    let cleanup = tokio::time::timeout(
        tokio::time::Duration::from_secs(5),
        futures::future::join_all(handles.drain(..)),
    )
    .await;

    match cleanup {
        Ok(_) => debug!("All tasks cleaned up successfully"),
        Err(_) => log::warn!("Timeout waiting for tasks to cleanup"),
    }

    log::info!("Shutdown complete");
    Ok(())
}
