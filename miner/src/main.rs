mod api;
mod cli;
mod system;
use clap::Parser;
use cli::{execute_command, Cli};

fn main() {
    let cli = Cli::parse();
    execute_command(&cli.command);
}
