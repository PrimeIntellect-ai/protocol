use lazy_static::lazy_static;
use prometheus::{
    register_counter_vec, register_gauge_vec, register_histogram_vec, CounterVec, GaugeVec,
    HistogramVec, TextEncoder,
};

lazy_static! {
    pub static ref VALIDATION_LOOP_DURATION: HistogramVec = register_histogram_vec!(
        "validator_validation_loop_duration_seconds",
        "Duration of the validation loop",
        &["validator_id", "pool_id"],
        vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0]
    ).unwrap();

    // === SYNTHETIC DATA VALIDATION METRICS ===

    // Total work keys invalidated
    pub static ref WORK_KEYS_INVALIDATED: CounterVec = register_counter_vec!(
        "validator_work_keys_invalidated_total",
        "Total work keys invalidated",
        &["validator_id", "pool_id"]
    ).unwrap();

    pub static ref WORK_KEYS_SOFT_INVALIDATED: CounterVec = register_counter_vec!(
        "validator_work_keys_soft_invalidated_total",
        "Total work keys soft invalidated",
        &["validator_id", "pool_id", "group_key"]
    ).unwrap();

    // Errors in synthetic data validation

    /// Total work keys processed by the validator
    pub static ref WORK_KEYS_TO_PROCESS: GaugeVec = register_gauge_vec!(
        "validator_work_keys_to_process",
        "Total work keys to process in current validation loop (depends on settings)",
        &["validator_id", "pool_id"]
    ).unwrap();

    pub static ref ERRORS: CounterVec = register_counter_vec!(
        "validator_errors_total",
        "Total errors",
        &["validator_id", "pool_id", "error"]
    ).unwrap();

    pub static ref API_DURATION: HistogramVec = register_histogram_vec!(
        "validator_api_duration_seconds",
        "API request duration",
        &["validator_id", "pool_id", "endpoint"], // endpoint: validate, validategroup, status, statusgroup
        vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0]
    ).unwrap();

    pub static ref API_REQUESTS: CounterVec = register_counter_vec!(
        "validator_api_requests_total",
        "Total API requests",
        &["validator_id", "pool_id", "endpoint", "status"]
    ).unwrap();

    pub static ref GROUP_VALIDATIONS: CounterVec = register_counter_vec!(
        "validator_group_validations_total",
        "Total group validations by result",
        &["validator_id", "pool_id", "group_id", "toploc_config_name", "result"] // result: accept, reject, crashed, pending, unknown
    ).unwrap();

    pub static ref GROUP_WORK_UNITS_CHECK_TOTAL: CounterVec = register_counter_vec!(
        "validator_group_work_units_check_total",
        "Whether the work units match the group size",
        &["validator_id", "pool_id", "group_id", "toploc_config_name", "result"] // result: match, mismatch
    ).unwrap();
}

pub fn export_metrics() -> Result<String, prometheus::Error> {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    encoder.encode_to_string(&metric_families)
}

#[derive(Clone, Debug)]
pub struct MetricsContext {
    pub validator_id: String,
    pub pool_id: Option<String>,
}

impl MetricsContext {
    pub fn new(validator_id: String, pool_id: Option<String>) -> Self {
        Self {
            validator_id,
            pool_id,
        }
    }

    pub fn record_work_keys_to_process(&self, count: f64) {
        if let Some(pool_id) = &self.pool_id {
            WORK_KEYS_TO_PROCESS
                .with_label_values(&[&self.validator_id as &str, pool_id])
                .set(count);
        }
    }

    pub fn record_work_key_invalidation(&self) {
        if let Some(pool_id) = &self.pool_id {
            WORK_KEYS_INVALIDATED
                .with_label_values(&[&self.validator_id as &str, pool_id])
                .inc();
        }
    }

    pub fn record_group_soft_invalidation(&self, group_key: &str) {
        if let Some(pool_id) = &self.pool_id {
            WORK_KEYS_SOFT_INVALIDATED
                .with_label_values(&[&self.validator_id as &str, pool_id, group_key])
                .inc();
        }
    }

    pub fn record_work_key_error(&self, error: &str) {
        if let Some(pool_id) = &self.pool_id {
            ERRORS
                .with_label_values(&[&self.validator_id as &str, pool_id, error])
                .inc();
        }
    }

    pub fn record_group_validation_status(
        &self,
        group_id: &str,
        toploc_config_name: &str,
        result: &str,
    ) {
        if let Some(pool_id) = &self.pool_id {
            GROUP_VALIDATIONS
                .with_label_values(&[
                    &self.validator_id as &str,
                    pool_id,
                    group_id,
                    toploc_config_name,
                    result,
                ])
                .inc();
        }
    }

    pub fn record_group_work_units_check_result(
        &self,
        group_id: &str,
        toploc_config_name: &str,
        result: &str,
    ) {
        if let Some(pool_id) = &self.pool_id {
            GROUP_WORK_UNITS_CHECK_TOTAL
                .with_label_values(&[
                    &self.validator_id as &str,
                    pool_id,
                    group_id,
                    toploc_config_name,
                    result,
                ])
                .inc();
        }
    }

    pub fn record_validation_loop_duration(&self, duration_s: f64) {
        let pool_id = self.pool_id.as_deref().unwrap_or("hw_validator");
        VALIDATION_LOOP_DURATION
            .with_label_values(&[&self.validator_id as &str, pool_id])
            .observe(duration_s);
    }

    pub fn record_api_duration(&self, endpoint: &str, duration_s: f64) {
        let pool_id = self.pool_id.as_deref().unwrap_or("hw_validator");
        API_DURATION
            .with_label_values(&[&self.validator_id as &str, pool_id, endpoint])
            .observe(duration_s);
    }

    pub fn record_api_request(&self, endpoint: &str, status: &str) {
        let pool_id = self.pool_id.as_deref().unwrap_or("hw_validator");
        API_REQUESTS
            .with_label_values(&[&self.validator_id as &str, pool_id, endpoint, status])
            .inc();
    }
}
