mod api;
mod checks;
mod cli;
mod console;
mod docker;
mod operations;
mod services;
use clap::Parser;
use cli::{execute_command, Cli};
use log::{debug, LevelFilter};
use std::process;

fn cleanup_and_exit() {
    log::info!("Starting cleanup");
    // Force sync any remaining logs
    log::logger().flush();
    std::thread::sleep(std::time::Duration::from_secs(2));
    process::exit(0);
}

fn main() {
    std::panic::set_hook(Box::new(|panic_info| {
        log::error!("Process panicked: {:?}", panic_info);
        cleanup_and_exit();
    }));

    ctrlc::set_handler(move || {
        log::info!("Received interrupt signal");
        cleanup_and_exit();
    })
    .expect("Error setting Ctrl-C handler");

    // Initialize logging with debug level and console output
    env_logger::Builder::new()
        .filter_level(LevelFilter::Info)
        .format_timestamp(None)
        .init();

    debug!("Parsing CLI arguments");
    let cli = Cli::parse();

    debug!("Executing command");
    execute_command(&cli.command);
}
