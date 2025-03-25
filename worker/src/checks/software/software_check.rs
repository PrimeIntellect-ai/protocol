use super::docker::check_docker_installed;
use crate::checks::issue::IssueReport;
use crate::console::Console;
use std::sync::{Arc, RwLock};

pub fn run_software_check(
    issues: Option<Arc<RwLock<IssueReport>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    Console::section("Software Checks");
    let issues = issues.unwrap_or_else(|| Arc::new(RwLock::new(IssueReport::new())));

    // Check Docker installation and connectivity
    Console::title("Docker:");
    check_docker_installed(&issues)?;

    Ok(())
}
