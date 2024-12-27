mod api;
mod cli;
mod checks;
use clap::Parser;
use cli::{execute_command, Cli};

fn main() {
    let cli = Cli::parse();
    execute_command(&cli.command);
}
