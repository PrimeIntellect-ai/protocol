mod api;
mod checks;
mod cli;
mod console;
mod docker;
mod operations;
mod services;
mod web3;
use clap::Parser;
use cli::{execute_command, Cli};
use log::{debug, LevelFilter};

fn main() {
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
