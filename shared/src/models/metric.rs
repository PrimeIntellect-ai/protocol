use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct MetricKey {
    pub task_id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    pub value: f64,
}

impl Metric {
    pub fn new(value: f64) -> Result<Self> {
        let metric = Self { value };
        metric.validate()?;
        Ok(metric)
    }

    pub fn validate(&self) -> Result<()> {
        if !self.value.is_finite() {
            bail!("Value must be a finite number");
        }
        Ok(())
    }
}

pub type MetricsMap = HashMap<MetricKey, Metric>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_metric() -> Result<()> {
        let valid_values = vec![1.0, 100.0, -5.0, 0.0];

        for value in valid_values {
            let metric = Metric::new(value)?;
            assert!(metric.validate().is_ok());
        }
        Ok(())
    }

    #[test]
    fn test_invalid_metrics() {
        let invalid_values = vec![(f64::INFINITY, "infinite value"), (f64::NAN, "NaN value")];

        for (value, case) in invalid_values {
            let metric = Metric::new(value);
            assert!(metric.is_err(), "Should fail for {}", case);
        }
    }

    #[test]
    fn test_metric_key() {
        let key1 = MetricKey {
            task_id: "task1".to_string(),
            label: "cpu".to_string(),
        };
        let key2 = MetricKey {
            task_id: "task1".to_string(),
            label: "cpu".to_string(),
        };
        let key3 = MetricKey {
            task_id: "task2".to_string(),
            label: "cpu".to_string(),
        };

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }
}
