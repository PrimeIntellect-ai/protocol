use crate::checks::issue::{IssueReport, IssueType};
use crate::console::Console;
use anyhow::Result;
use std::net::TcpListener;
use std::sync::Arc;
use tokio::sync::RwLock;

fn try_bind_port(port: u16) -> Result<()> {
    let addr = format!("0.0.0.0:{}", port);
    // If bind succeeds, port is available; if it fails, port is taken.
    let listener = TcpListener::bind(addr)?;
    drop(listener); // Release the port immediately.
    Ok(())
}

pub async fn check_port_available(issues: &Arc<RwLock<IssueReport>>, port: u16) -> Result<()> {
    let issue_tracker = issues.read().await;

    match try_bind_port(port) {
        Ok(_) => Console::success("Port is available"),
        Err(e) => {
            issue_tracker.add_issue(
                IssueType::PortUnavailable,
                format!("Port {} is not available: {}", port, e),
            );
        }
    }

    Ok(())
}
