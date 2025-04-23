use super::{docker::check_docker_installed, port::check_port_available};
use crate::checks::issue::IssueReport;
use crate::console::Console;
use shared::models::node::Node;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct SoftwareChecker {
    issues: Arc<RwLock<IssueReport>>,
}

impl SoftwareChecker {
    pub fn new(issues: Option<Arc<RwLock<IssueReport>>>) -> Self {
        Self {
            issues: issues.unwrap_or_else(|| Arc::new(RwLock::new(IssueReport::new()))),
        }
    }

    pub async fn check_software(
        &self,
        node_config: &Node,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Check Docker installation and connectivity
        Console::title("Docker:");
        check_docker_installed(&self.issues).await?;

        // Check port availability
        Console::title("Port:");
        check_port_available(&self.issues, node_config.port).await?;

        Ok(())
    }
}
