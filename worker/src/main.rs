mod api;
mod checks;
mod cli;
mod console;
mod docker;
mod metrics;
mod operations;
mod services;
mod state;
mod utils;
use clap::Parser;
use cli::{execute_command, Cli};
use std::panic;
use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use utils::logging::setup_logging;
pub type TaskHandles = Arc<Mutex<Vec<JoinHandle<()>>>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let task_handles: TaskHandles = Arc::new(Mutex::new(Vec::<JoinHandle<()>>::new()));

    let cli = Cli::parse();

    if let Err(e) = setup_logging(Some(&cli)) {
        eprintln!(
            "Warning: Failed to initialize logging: {}. Using default logging.",
            e
        );
    }

    // Set up panic hook to log panics
    panic::set_hook(Box::new(|panic_info| {
        let location = panic_info
            .location()
            .unwrap_or_else(|| panic::Location::caller());
        let message = match panic_info.payload().downcast_ref::<&str>() {
            Some(s) => *s,
            None => match panic_info.payload().downcast_ref::<String>() {
                Some(s) => s.as_str(),
                None => "Unknown panic payload",
            },
        };

        log::error!(
            "PANIC: '{}' at {}:{}",
            message,
            location.file(),
            location.line()
        );
    }));

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
        Ok(_) => log::info!("All tasks cleaned up successfully"),
        Err(_) => log::warn!("Timeout waiting for tasks to cleanup"),
    }
    Ok(())
}
