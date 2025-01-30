use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Metric {
    pub label: String,
    pub value: f64,
    pub taskid: String,
}

impl Metric {
    pub fn new(label: String, value: f64, taskid: String) -> Result<Self> {
        let metric = Self {
            label,
            value,
            taskid,
        };
        metric.validate()?;
        Ok(metric)
    }

    pub fn validate(&self) -> Result<()> {
        // Validate label
        if self.label.is_empty() {
            bail!("Label cannot be empty");
        }
        if self.label.len() > 64 {
            bail!("Label cannot be longer than 64 characters");
        }

        // Validate value
        if !self.value.is_finite() {
            bail!("Value must be a finite number");
        }

        // Validate taskid
        if self.taskid.is_empty() {
            bail!("Task ID cannot be empty");
        }

        Ok(())
    }
}

/// A nested HashMap structure for storing metrics organized by task and label.
/// - Outer HashMap: task ID as the key
/// - Inner HashMap: metric label as the key, Metric struct as the value
///
/// Example structure:
/// {
///     "task_123": {
///         "cpu_usage": Metric { label: "cpu_usage", value: 75.5, taskid: "task_123" },
///         "memory_usage": Metric { label: "memory_usage", value: 1024.0, taskid: "task_123" }
///     },
///     "task_456": {
///         "network_in": Metric { label: "network_in", value: 500.0, taskid: "task_456" }
///     }
/// }
pub type MetricsMap = HashMap<String, HashMap<String, Metric>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_metric() -> Result<()> {
        let valid_cases = vec![
            ("simple", 1.0, "task_1"),
            ("data_processed", 100.0, "task_2"),
            ("my-metric-123", -5.0, "task_3"),
            ("aVeryLongButStillValidMetricName", 0.0, "task_4"),
        ];

        for (label, value, taskid) in valid_cases {
            let metric = Metric::new(label.to_string(), value, taskid.to_string())?;
            assert!(metric.validate().is_ok());
        }
        Ok(())
    }

    #[test]
    fn test_invalid_metrics() {
        let invalid_cases = vec![
            ("", 1.0, "", "empty label"),
            ("valid_name", f64::INFINITY, "task_5", "infinite value"),
            ("valid_name", f64::NAN, "task_6", "NaN value"),
            ("valid_name", 1.0, "", "empty taskid"),
        ];

        for (label, value, taskid, case) in invalid_cases {
            let metric = Metric::new(label.to_string(), value, taskid.to_string());
            println!("metric {}", metric.is_ok());
            assert!(metric.is_err(), "Should fail for {}", case);
        }
    }

    #[test]
    fn test_json_serialization() -> Result<()> {
        let metric = Metric::new("test_metric".to_string(), 42.0, "task_7".to_string())?;
        let json = serde_json::to_string(&metric)?;
        let deserialized: Metric = serde_json::from_str(&json)?;
        assert_eq!(metric.label, deserialized.label);
        assert_eq!(metric.value, deserialized.value);
        assert_eq!(metric.taskid, deserialized.taskid);
        Ok(())
    }
}
