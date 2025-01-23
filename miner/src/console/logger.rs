use console::style;
use std::time::Duration;
pub struct Console;

impl Console {
    pub fn section(title: &str) {
        println!();
        println!(
            "{}",
            style("╔═══════════════════════════════════╗".to_string())
                .bold()
                .magenta()
        );
        println!("{}", style(format!("║  {}  ", title)).bold().magenta());
        println!(
            "{}",
            style("╚═══════════════════════════════════╝".to_string())
                .bold()
                .magenta()
        );
    }

    pub fn title(text: &str) {
        println!();
        println!(
            "{}",
            style("╔═══════════════════════════════════╗".to_string())
                .bold()
                .magenta()
        );
        println!("║ {}", style(text).bold().magenta());
        println!(
            "{}",
            style("╚═══════════════════════════════════╝".to_string())
                .bold()
                .magenta()
        );
    }

    pub fn info(label: &str, value: &str) {
        println!(
            "║ {} {}",
            style(format!("{}: ", label)).dim().magenta(),
            style(value).white()
        );
    }

    pub fn success(text: &str) {
        println!("║ {} {}", style("✓").green().bold(), style(text).green());
    }

    pub fn warning(text: &str) {
        println!("║ {} {}", style("⚠").yellow().bold(), style(text).yellow());
    }

    pub fn error(text: &str) {
        println!("║ {} {}", style("✗").red().bold(), style(text).red());
    }

    pub fn progress(text: &str) {
        println!("║ {} {}", style("→").cyan().bold(), style(text).cyan());
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
        pb.set_draw_target(indicatif::ProgressDrawTarget::hidden());
        pb
    }
}
