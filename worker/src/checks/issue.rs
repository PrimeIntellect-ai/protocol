use crate::console::Console;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueType {
    NoGpu,                        // GPU required for compute
    DockerNotInstalled,           // Docker required for containers
    ContainerToolkitNotInstalled, // Container toolkit required for GPU
    InsufficientStorage,          // Minimum storage needed
    InsufficientMemory,           // Minimum RAM needed
    InsufficientCpu,              // Minimum CPU cores needed
    UnsupportedArchitecture,      // Must be x86_64/amd64
    NetworkConnectivityIssue,     // Network performance issues
}

impl IssueType {
    pub const fn severity(&self) -> Severity {
        match self {
            Self::NetworkConnectivityIssue => Severity::Warning,
            Self::InsufficientCpu => Severity::Warning,
            Self::InsufficientMemory => Severity::Warning,
            Self::InsufficientStorage => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Issue {
    issue_type: IssueType,
    message: String,
}

impl Issue {
    pub fn new(issue_type: IssueType, message: impl Into<String>) -> Self {
        Self {
            issue_type,
            message: message.into(),
        }
    }

    pub const fn severity(&self) -> Severity {
        self.issue_type.severity()
    }

    pub fn print(&self) {
        match self.severity() {
            Severity::Error => Console::error(&format!("{}", self)),
            Severity::Warning => Console::warning(&format!("{}", self)),
        }
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.issue_type, self.message)
    }
}

#[derive(Debug, Default)]
pub struct IssueReport {
    issues: Vec<Issue>,
}

impl IssueReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_issue(&mut self, issue_type: IssueType, message: impl Into<String>) {
        self.issues.push(Issue::new(issue_type, message));
    }

    pub fn print_issues(&self) {
        if self.issues.is_empty() {
            Console::success("No issues found");
            return;
        }

        Console::section("System Check Issues");
        for issue in &self.issues {
            issue.print();
        }
    }

    pub fn has_critical_issues(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| matches!(issue.severity(), Severity::Error))
    }
}
