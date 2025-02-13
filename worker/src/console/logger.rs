use console::style;
use std::time::Duration;

pub struct Console;

impl Console {
    const fn get_width() -> usize {
        40 // Base width for horizontal lines
    }

    fn horizontal_border() -> String {
        "═".repeat(Self::get_width())
    }

    pub fn section(title: &str) {
        println!();
        let width = Self::get_width();
        let formatted_title = format!("{:^width$}", title, width = width);
        let border = Self::horizontal_border();

        println!("{}", style(format!("╔{}╗", border)).magenta().bold());
        println!("{}", style(formatted_title).magenta().bold());
        println!("{}", style(format!("╚{}╝", border)).magenta().bold());
    }

    pub fn title(text: &str) {
        println!();
        let width = Self::get_width();
        let formatted_text = format!("{:^width$}", text, width = width);
        let border = Self::horizontal_border();

        println!("{}", style(format!("╔{}╗", border)).magenta().bold());
        println!("{}", style(formatted_text).magenta().bold());
        println!("{}", style(format!("╚{}╝", border)).magenta().bold());
    }

    pub fn info(label: &str, value: &str) {
        println!("{}: {}", style(label).dim().magenta(), style(value).white());
    }

    pub fn success(text: &str) {
        println!("{} {}", style("✓").green().bold(), style(text).green());
    }

    pub fn warning(text: &str) {
        println!("{} {}", style("⚠").yellow().bold(), style(text).yellow());
    }

    pub fn error(text: &str) {
        println!("{} {}", style("✗").red().bold(), style(text).red());
    }

    pub fn progress(text: &str) {
        println!("{} {}", style("→").cyan().bold(), style(text).cyan());
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
        pb
    }
}
