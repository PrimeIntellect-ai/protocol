use console::{style, Term};
use std::cmp;
use unicode_width::UnicodeWidthStr;

pub struct Console;

impl Console {
    /// Maximum content width for the box.
    const MAX_WIDTH: usize = 80;

    /// Calculates the content width for boxes.
    /// It uses the available terminal width (minus a margin) and caps it at MAX_WIDTH.
    fn get_content_width() -> usize {
        let term_width = Term::stdout().size().1 as usize;
        // Leave a margin of 10 columns.
        let available = if term_width > 10 {
            term_width - 10
        } else {
            term_width
        };
        cmp::min(available, Self::MAX_WIDTH)
    }

    /// Centers a given text within a given width based on its display width.
    fn center_text(text: &str, width: usize) -> String {
        let text_width = UnicodeWidthStr::width(text);
        if width > text_width {
            let total_padding = width - text_width;
            let left = total_padding / 2;
            let right = total_padding - left;
            format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
        } else {
            text.to_string()
        }
    }

    /// Prints a section header as an aligned box.
    pub fn section(title: &str) {
        let content_width = Self::get_content_width();
        let top_border = format!("╔{}╗", "═".repeat(content_width));
        let centered_title = Self::center_text(title, content_width);
        let middle_line = format!("║{}║", centered_title);
        let bottom_border = format!("╚{}╝", "═".repeat(content_width));

        log::info!();
        log::info!("{}", style(top_border).white().bold());
        log::info!("{}", style(middle_line).white().bold());
        log::info!("{}", style(bottom_border).white().bold());
    }

    /// Prints a sub-title.
    pub fn title(text: &str) {
        log::info!();
        log::info!("{}", style(text).white().bold());
    }

    /// Prints an informational message.
    pub fn info(label: &str, value: &str) {
        log::info!("{}: {}", style(label).dim().white(), style(value).white());
    }

    /// Prints a success message.
    pub fn success(text: &str) {
        log::info!("{} {}", style("✓").green().bold(), style(text).green());
    }

    /// Prints a warning message.
    pub fn warning(text: &str) {
        log::info!("{} {}", style("⚠").yellow().bold(), style(text).yellow());
    }

    /// Prints an error message.
    pub fn error(text: &str) {
        log::info!("{} {}", style("✗").red().bold(), style(text).red());
    }

    /// Prints a progress message.
    pub fn progress(text: &str) {
        log::info!("{} {}", style("→").cyan().bold(), style(text).cyan());
    }
}
