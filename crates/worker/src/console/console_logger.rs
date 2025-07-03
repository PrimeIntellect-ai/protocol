use console::{style, Term};
use std::cmp;
use unicode_width::UnicodeWidthStr;

pub(crate) struct Console;

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
    pub(crate) fn section(title: &str) {
        let content_width = Self::get_content_width();
        let top_border = format!("╔{}╗", "═".repeat(content_width));
        let centered_title = Self::center_text(title, content_width);
        let middle_line = format!("║{centered_title}║");
        let bottom_border = format!("╚{}╝", "═".repeat(content_width));

        println!();
        println!("{}", style(top_border).white().bold());
        println!("{}", style(middle_line).white().bold());
        println!("{}", style(bottom_border).white().bold());
    }

    /// Prints a sub-title.
    pub(crate) fn title(text: &str) {
        println!();
        println!("{}", style(text).white().bold());
    }

    /// Prints an informational message.
    pub(crate) fn info(label: &str, value: &str) {
        println!("{}: {}", style(label).dim().white(), style(value).white());
    }

    /// Prints a success message.
    pub(crate) fn success(text: &str) {
        println!("{} {}", style("✓").green().bold(), style(text).green());
    }

    /// Prints a warning message.
    pub(crate) fn warning(text: &str) {
        println!("{} {}", style("⚠").yellow().bold(), style(text).yellow());
    }

    /// Prints a user error message.
    /// This is a special case where the error is user-facing (e.g., missing GPU, configuration issues)
    /// rather than a system error. These errors are not logged to central logging systems
    /// and are only displayed to the user to help them resolve the issue.
    /// For actual system errors that should be tracked, use proper error logging instead.
    pub(crate) fn user_error(text: &str) {
        println!("{} {}", style("✗").red().bold(), style(text).red());
    }

    /// Prints a progress message.
    pub(crate) fn progress(text: &str) {
        println!("{} {}", style("→").cyan().bold(), style(text).cyan());
    }
}
