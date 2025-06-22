use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ToSchema)]
pub struct MetricEntry {
    pub key: MetricKey,
    pub value: f64,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct MetricKey {
    pub task_id: String,
    pub label: String,
}

impl MetricEntry {
    pub fn new(key: MetricKey, value: f64) -> Result<Self> {
        let entry = Self { key, value };
        entry.validate()?;
        Ok(entry)
    }

    pub fn validate(&self) -> Result<()> {
        if !self.value.is_finite() {
            bail!("Value must be a finite number");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_metric() -> Result<()> {
        let key = MetricKey {
            task_id: "task1".to_string(),
            label: "cpu".to_string(),
        };

        let valid_values = vec![1.0, 100.0, -5.0, 0.0];
        for value in valid_values {
            let entry = MetricEntry::new(key.clone(), value)?;
            assert!(entry.validate().is_ok());
        }
        Ok(())
    }

    #[test]
    fn test_invalid_metrics() {
        let key = MetricKey {
            task_id: "task1".to_string(),
            label: "cpu".to_string(),
        };

        let invalid_values = vec![(f64::INFINITY, "infinite value"), (f64::NAN, "NaN value")];
        for (value, case) in invalid_values {
            let entry = MetricEntry::new(key.clone(), value);
            assert!(entry.is_err(), "Should fail for {}", case);
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
