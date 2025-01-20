// src/console/logger.rs
use colored::*;
use console::style;
use std::time::Duration;
pub struct Console;

impl Console {
    pub fn section(title: &str) {
        println!();
        println!(
            "{}",
            style(format!("╔═══════════════════════════════════╗"))
                .bold()
                .magenta()
        );
        println!("{}", style(format!("║  {}  ", title)).bold().cyan()); // Changed bright_cyan to cyan
        println!(
            "{}",
            style(format!("╚═══════════════════════════════════╝"))
                .bold()
                .magenta()
        );
    }

    pub fn title(text: &str) {
        println!();
        println!(
            "{}",
            style(format!("╔═══════════════════════════════════╗"))
                .bold()
                .magenta()
        );
        println!("║ {}", style(text).bold().green()); // Changed bright_green to green
        println!(
            "{}",
            style(format!("╚═══════════════════════════════════╝"))
                .bold()
                .magenta()
        );
    }

    pub fn info(label: &str, value: &str) {
        println!(
            "║ {} {}",
            style(format!("{}: ", label)).dim(),
            style(value).white() // Changed bright_white to white
        );
    }

    pub fn success(text: &str) {
        println!("║ {} {}", style("✓").green().bold(), style(text).green()); // Changed bright_green to green
    }

    pub fn warning(text: &str) {
        println!("║ {} {}", style("⚠").yellow().bold(), style(text).yellow()); // Changed bright_yellow to yellow
    }

    pub fn error(text: &str) {
        println!("║ {} {}", style("✗").red().bold(), style(text).red()); // Changed bright_red to red
    }

    pub fn progress(text: &str) {
        println!("║ {} {}", style("→").cyan().bold(), style(text).cyan()); // Changed bright_cyan to cyan
    }
    pub fn spinner(text: &str) -> indicatif::ProgressBar {
        let pb = indicatif::ProgressBar::new_spinner();
        pb.set_style(
            indicatif::ProgressStyle::default_spinner()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
                .template("{spinner:.magenta} {msg}")
                .unwrap(),
        );
        pb.set_message(text.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb.set_draw_target(indicatif::ProgressDrawTarget::hidden()); // Keep the spinner hidden to avoid console history
        pb
    }
}
