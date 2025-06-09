use crate::metrics::MetricsContext;

use super::ValidationResult;
use anyhow::Error;
use log::debug;
use log::{error, info};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToplocConfig {
    pub server_url: String,
    pub auth_token: Option<String>,
    pub file_prefix_filter: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Toploc {
    config: ToplocConfig,
    client: reqwest::Client,
    metrics: Option<MetricsContext>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupValidationResult {
    pub status: ValidationResult,
    pub input_flops: f64,
    pub output_flops: f64,
    // This tells us which node(s) in a group actually failed the toploc validation
    pub failing_indices: Vec<i64>,
}

impl Toploc {
    pub fn new(config: ToplocConfig, metrics: Option<MetricsContext>) -> Self {
        let client = reqwest::Client::builder()
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                if let Some(token) = &config.auth_token {
                    headers.insert(
                        reqwest::header::AUTHORIZATION,
                        reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                            .expect("Invalid token"),
                    );
                }
                headers
            })
            .build()
            .expect("Failed to build HTTP client");

        Self {
            config,
            client,
            metrics,
        }
    }

    pub fn name(&self) -> String {
        let prefix = self
            .config
            .file_prefix_filter
            .clone()
            .unwrap_or_else(|| "n/a".to_string());
        prefix.to_string() // e.g. Qwen/Qwen3-14B
    }

    fn normalize_path(&self, path: &str) -> String {
        // Remove leading slashes and normalize any double slashes
        path.trim_start_matches('/').replace("//", "/")
    }

    fn remove_prefix_if_present(&self, file_name: &str) -> String {
        let normalized_name = self.normalize_path(file_name);
        match &self.config.file_prefix_filter {
            Some(prefix) if normalized_name.starts_with(prefix) => {
                self.normalize_path(&normalized_name[prefix.len()..])
            }
            _ => normalized_name,
        }
    }

    pub fn matches_file_name(&self, file_name: &str) -> bool {
        let normalized_name = self.normalize_path(file_name);
        match &self.config.file_prefix_filter {
            Some(prefix) => {
                normalized_name == *prefix || {
                    normalized_name.starts_with(prefix)
                        && normalized_name[prefix.len()..].starts_with('/')
                }
            }
            None => true,
        }
    }

    pub async fn trigger_single_file_validation(
        &self,
        file_sha: &str,
        key_address: &str,
        file_name: &str,
    ) -> Result<(), Error> {
        let processed_file_name = self.remove_prefix_if_present(file_name);
        let validate_url = format!(
            "{}/validate/{}",
            self.config.server_url, processed_file_name
        );
        debug!(
            "Triggering remote toploc validation for {} {}",
            file_name, validate_url
        );

        let body = serde_json::json!({
            "file_sha": file_sha,
            "address": key_address
        });

        let start_time = std::time::Instant::now();
        match &self.client.post(&validate_url).json(&body).send().await {
            Ok(response) => {
                let status = response.status();
                if !status.is_success() {
                    error!("Server returned error status {} for {}", status, file_name);
                    if let Some(metrics) = &self.metrics {
                        metrics.record_api_request(
                            "toploc_single_file_validation",
                            &status.to_string(),
                        );
                    }
                    return Err(Error::msg(format!(
                        "Server returned error status: {}",
                        status
                    )));
                }
                let trigger_duration = start_time.elapsed();
                if let Some(metrics) = &self.metrics {
                    metrics.record_api_duration(
                        "toploc_single_file_validation",
                        trigger_duration.as_secs_f64(),
                    );
                    metrics
                        .record_api_request("toploc_single_file_validation", &status.to_string());
                }
                info!(
                    "Remote toploc validation triggered for {} in {:?}",
                    file_name, trigger_duration
                );

                Ok(())
            }
            Err(e) => {
                error!(
                    "Failed to trigger remote toploc validation for {}: {}",
                    file_name, e
                );
                if let Some(metrics) = &self.metrics {
                    metrics.record_api_request("toploc_single_file_validation", "0");
                }
                Err(Error::msg(format!("Failed to trigger validation: {}", e)))
            }
        }
    }

    pub async fn trigger_group_file_validation(
        &self,
        file_name: &str,
        file_shas: Vec<String>,
        group_id: &str,
        file_number: u32,
        group_size: u32,
    ) -> Result<(), Error> {
        let processed_file_name = self.remove_prefix_if_present(file_name);
        let validate_url = format!(
            "{}/validategroup/{}",
            self.config.server_url, processed_file_name
        );

        info!(
            "Triggering remote toploc group validation for {} {}",
            file_name, validate_url
        );

        let body = serde_json::json!({
            "file_shas": file_shas,
            "group_id": group_id,
            "file_number": file_number,
            "group_size": group_size
        });

        let start_time = std::time::Instant::now();
        match &self.client.post(&validate_url).json(&body).send().await {
            Ok(response) => {
                let status = response.status();
                if !status.is_success() {
                    error!("Server returned error status {} for {}", status, file_name);
                    if let Some(metrics) = &self.metrics {
                        metrics.record_api_request(
                            "toploc_group_file_validation",
                            &status.to_string(),
                        );
                    }
                    return Err(Error::msg(format!(
                        "Server returned error status: {}",
                        status
                    )));
                }
                let trigger_duration = start_time.elapsed();
                if let Some(metrics) = &self.metrics {
                    metrics.record_api_duration(
                        "toploc_group_file_validation",
                        trigger_duration.as_secs_f64(),
                    );
                    metrics.record_api_request("toploc_group_file_validation", &status.to_string());
                }
                info!(
                    "Remote toploc group validation triggered for {} in {:?}",
                    file_name, trigger_duration
                );

                Ok(())
            }
            Err(e) => {
                error!(
                    "Failed to trigger remote toploc group validation for {}: {}",
                    file_name, e
                );
                if let Some(metrics) = &self.metrics {
                    metrics.record_api_request("toploc_group_file_validation", "0");
                }
                Err(Error::msg(format!(
                    "Failed to trigger group validation: {}",
                    e
                )))
            }
        }
    }

    pub async fn get_group_file_validation_status(
        &self,
        file_name: &str,
    ) -> Result<GroupValidationResult, Error> {
        let processed_file_name = self.remove_prefix_if_present(file_name);
        debug!("Processed file name: {}", processed_file_name);
        let url = format!(
            "{}/statusgroup/{}",
            self.config.server_url, processed_file_name
        );
        debug!("Processing URL: {}", url);

        let start_time = std::time::Instant::now();
        match self.client.get(&url).send().await {
            Ok(response) => {
                let status = response.status();
                if status != reqwest::StatusCode::OK {
                    error!("Unexpected status code {} for {}", status, file_name);
                    if let Some(metrics) = &self.metrics {
                        metrics.record_api_request("toploc_get_group_status", &status.to_string());
                    }
                    return Err(Error::msg(format!("Unexpected status code: {}", status)));
                }
                let status_json: serde_json::Value = response.json().await.map_err(|e| {
                    error!("Failed to parse JSON response for {}: {}", file_name, e);
                    Error::msg(format!("Failed to parse JSON response: {}", e))
                })?;

                let duration = start_time.elapsed();
                if let Some(metrics) = &self.metrics {
                    metrics.record_api_duration("toploc_get_group_status", duration.as_secs_f64());
                    metrics.record_api_request("toploc_get_group_status", &status.to_string());
                }

                if status_json.get("status").is_none() {
                    error!("No status found for {}", file_name);
                    Err(Error::msg("No status found"))
                } else {
                    match status_json.get("status").and_then(|s| s.as_str()) {
                        Some(status) => {
                            debug!("Validation status for {}: {}", file_name, status);

                            let validation_result = match status {
                                "accept" => ValidationResult::Accept,
                                "reject" => ValidationResult::Reject,
                                "crashed" => ValidationResult::Crashed,
                                "pending" => ValidationResult::Pending,
                                _ => ValidationResult::Unknown,
                            };

                            let input_flops = status_json
                                .get("input_flops")
                                .and_then(|f| f.as_f64())
                                .unwrap_or(0.0);
                            let output_flops = status_json
                                .get("output_flops")
                                .and_then(|f| f.as_f64())
                                .unwrap_or(0.0);

                            let failing_indices = status_json
                                .get("failing_indices")
                                .and_then(|f| f.as_array())
                                .map(|arr| {
                                    arr.iter().filter_map(|v| v.as_i64()).collect::<Vec<i64>>()
                                })
                                .unwrap_or_default();

                            Ok(GroupValidationResult {
                                status: validation_result,
                                input_flops,
                                output_flops,
                                failing_indices,
                            })
                        }
                        None => {
                            error!("No status found for {}", file_name);
                            Err(Error::msg("No status found"))
                        }
                    }
                }
            }
            Err(e) => {
                error!(
                    "Failed to poll remote toploc group validation for {}: {}",
                    file_name, e
                );
                Err(Error::msg(format!(
                    "Failed to poll remote toploc group validation: {}",
                    e
                )))
            }
        }
    }

    pub async fn get_single_file_validation_status(
        &self,
        file_name: &str,
    ) -> Result<ValidationResult, Error> {
        let processed_file_name = self.remove_prefix_if_present(file_name);
        let url = format!("{}/status/{}", self.config.server_url, processed_file_name);

        match self.client.get(&url).send().await {
            Ok(response) => {
                if response.status() != reqwest::StatusCode::OK {
                    error!(
                        "Unexpected status code {} for {}",
                        response.status(),
                        file_name
                    );
                    return Err(Error::msg(format!(
                        "Unexpected status code: {}",
                        response.status()
                    )));
                }
                let status_json: serde_json::Value = response.json().await.map_err(|e| {
                    error!("Failed to parse JSON response for {}: {}", file_name, e);
                    Error::msg(format!("Failed to parse JSON response: {}", e))
                })?;

                if status_json.get("status").is_none() {
                    error!("No status found for {}", file_name);
                    Err(Error::msg("No status found"))
                } else {
                    match status_json.get("status").and_then(|s| s.as_str()) {
                        Some(status) => {
                            debug!("Validation status for {}: {}", file_name, status);

                            let validation_result = match status {
                                "accept" => ValidationResult::Accept,
                                "reject" => ValidationResult::Reject,
                                "crashed" => ValidationResult::Crashed,
                                "pending" => ValidationResult::Pending,
                                _ => ValidationResult::Unknown,
                            };
                            Ok(validation_result)
                        }
                        None => {
                            error!("No status found for {}", file_name);
                            Err(Error::msg("No status found"))
                        }
                    }
                }
            }
            Err(e) => {
                error!(
                    "Failed to poll remote toploc validation for {}: {}",
                    file_name, e
                );
                Err(Error::msg(format!(
                    "Failed to poll remote toploc validation: {}",
                    e
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn test_single_file_validation_success() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        let _trigger_mock = server
            .mock("POST", "/validate/test-file.parquet")
            .with_status(200)
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "file_sha": "abc123",
                "address": "0x456"
            })))
            .create();

        let config = ToplocConfig {
            server_url: server.url(),
            auth_token: None,
            file_prefix_filter: None,
        };
        let toploc = Toploc::new(config, None);

        let result = toploc
            .trigger_single_file_validation("abc123", "0x456", "test-file.parquet")
            .await;

        assert!(result.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn test_group_file_validation_success() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        let _group_mock = server
            .mock("POST", "/validategroup/test-group.parquet")
            .with_status(200)
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "file_shas": ["sha1", "sha2"],
                "group_id": "group123",
                "file_number": 1,
                "group_size": 2
            })))
            .create();

        let config = ToplocConfig {
            server_url: server.url(),
            auth_token: None,
            file_prefix_filter: None,
        };
        let toploc = Toploc::new(config, None);

        let result = toploc
            .trigger_group_file_validation(
                "test-group.parquet",
                vec!["sha1".to_string(), "sha2".to_string()],
                "group123",
                1,
                2,
            )
            .await;

        assert!(result.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn test_group_file_validation_error() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        let _group_mock = server
            .mock("POST", "/validategroup/test-group.parquet")
            .with_status(400)
            .create();

        let config = ToplocConfig {
            server_url: server.url(),
            auth_token: None,
            file_prefix_filter: None,
        };
        let toploc = Toploc::new(config, None);

        let result = toploc
            .trigger_group_file_validation(
                "test-group.parquet",
                vec!["sha1".to_string(), "sha2".to_string()],
                "group123",
                1,
                2,
            )
            .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Server returned error status: 400"));
        Ok(())
    }

    #[tokio::test]
    async fn test_single_file_status_accept() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        let _status_mock = server
            .mock("GET", "/status/test-file.parquet")
            .with_status(200)
            .with_body(r#"{"status": "accept"}"#)
            .create();

        let config = ToplocConfig {
            server_url: server.url(),
            auth_token: None,
            file_prefix_filter: None,
        };
        let toploc = Toploc::new(config, None);

        let result = toploc
            .get_single_file_validation_status("test-file.parquet")
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ValidationResult::Accept);
        Ok(())
    }

    #[tokio::test]
    async fn test_single_file_status_reject() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        let _status_mock = server
            .mock("GET", "/status/test-file.parquet")
            .with_status(200)
            .with_body(r#"{"status": "reject"}"#)
            .create();

        let config = ToplocConfig {
            server_url: server.url(),
            auth_token: None,
            file_prefix_filter: None,
        };
        let toploc = Toploc::new(config, None);

        let result = toploc
            .get_single_file_validation_status("test-file.parquet")
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ValidationResult::Reject);
        Ok(())
    }

    #[tokio::test]
    async fn test_group_file_status_success_with_flops() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        let _status_mock = server
            .mock("GET", "/statusgroup/test-group.parquet")
            .with_status(200)
            .with_body(r#"{"status": "accept", "input_flops": 12345.67, "output_flops": 12345.67, "failing_indices": []}"#)
            .create();

        let config = ToplocConfig {
            server_url: server.url(),
            auth_token: None,
            file_prefix_filter: None,
        };
        let toploc = Toploc::new(config, None);

        let result = toploc
            .get_group_file_validation_status("test-group.parquet")
            .await;

        assert!(result.is_ok());
        let group_result = result.unwrap();
        assert_eq!(group_result.status, ValidationResult::Accept);
        assert_eq!(group_result.input_flops, 12345.67);
        assert_eq!(group_result.output_flops, 12345.67);
        assert!(group_result.failing_indices.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_group_file_status_reject_with_failing_indices() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        let _status_mock = server
            .mock("GET", "/statusgroup/test-group.parquet")
            .with_status(200)
            .with_body(r#"{"status": "reject", "input_flops": 0.0, "output_flops": 0.0, "failing_indices": [1, 3, 5]}"#)
            .create();

        let config = ToplocConfig {
            server_url: server.url(),
            auth_token: None,
            file_prefix_filter: None,
        };
        let toploc = Toploc::new(config, None);

        let result = toploc
            .get_group_file_validation_status("test-group.parquet")
            .await;

        assert!(result.is_ok());
        let group_result = result.unwrap();
        assert_eq!(group_result.status, ValidationResult::Reject);
        assert_eq!(group_result.input_flops, 0.0);
        assert_eq!(group_result.output_flops, 0.0);
        assert_eq!(group_result.failing_indices, vec![1, 3, 5]);
        Ok(())
    }
    #[tokio::test]
    async fn test_file_prefix_filter_matching() {
        let configs = vec![
            ToplocConfig {
                server_url: "http://test".to_string(),
                auth_token: None,
                file_prefix_filter: Some("Qwen/Qwen3-235B-A22B".to_string()),
            },
            ToplocConfig {
                server_url: "http://test".to_string(),
                auth_token: None,
                file_prefix_filter: Some("Qwen/Qwen3-32B".to_string()),
            },
            ToplocConfig {
                server_url: "http://test".to_string(),
                auth_token: None,
                file_prefix_filter: Some("Qwen/Qwen3-30B-A3B".to_string()),
            },
            ToplocConfig {
                server_url: "http://test".to_string(),
                auth_token: None,
                file_prefix_filter: Some("Qwen/Qwen3-14B".to_string()),
            },
            ToplocConfig {
                server_url: "http://test".to_string(),
                auth_token: None,
                file_prefix_filter: Some("deepseek-ai/DeepSeek-R1-0528".to_string()),
            },
            ToplocConfig {
                server_url: "http://test".to_string(),
                auth_token: None,
                file_prefix_filter: Some("deepseek-ai/DeepSeek-R1-0528-Qwen3-8B".to_string()),
            },
        ];

        let test_cases = vec![
            // Test Qwen 235B model
            ("Qwen/Qwen3-235B-A22B/data.parquet", Some(0)),
            ("Qwen/Qwen3-235B-A22B", Some(0)),
            ("Qwen/Qwen3-235B-A22B-extra/data.parquet", None),
            ("qwen/qwen3-235b-a22b/data.parquet", None), // Case sensitive
            // Test Qwen 32B model
            ("Qwen/Qwen3-32B/data.parquet", Some(1)),
            ("Qwen/Qwen3-32B", Some(1)),
            ("Qwen/Qwen3-32B-extra/data.parquet", None),
            // Test Qwen 30B model
            ("Qwen/Qwen3-30B-A3B/data.parquet", Some(2)),
            ("Qwen/Qwen3-30B-A3B", Some(2)),
            ("Qwen/Qwen3-30B-A3B-extra/data.parquet", None),
            // Test Qwen 14B model
            ("Qwen/Qwen3-14B/data.parquet", Some(3)),
            ("Qwen/Qwen3-14B", Some(3)),
            ("Qwen/Qwen3-14B-extra/data.parquet", None),
            // Test DeepSeek base model
            ("deepseek-ai/DeepSeek-R1-0528/data.parquet", Some(4)),
            ("deepseek-ai/DeepSeek-R1-0528", Some(4)),
            (
                "deepseek-ai/DeepSeek-R1-0528-Qwen3-8B/data.parquet",
                Some(5),
            ),
            ("deepseek-ai/deepseek-r1-0528/data.parquet", None), // Case sensitive
        ];

        for (test_file, expected_match) in test_cases {
            let mut matched = false;
            let mut matched_idx = None;

            for (idx, config) in configs.iter().enumerate() {
                let toploc = Toploc::new(config.clone(), None);
                if toploc.matches_file_name(test_file) {
                    matched = true;
                    matched_idx = Some(idx);
                    break;
                }
            }

            match expected_match {
                Some(expected_idx) => {
                    assert!(
                        matched,
                        "Expected file {} to match config {}",
                        test_file, expected_idx
                    );
                    assert_eq!(
                        matched_idx,
                        Some(expected_idx),
                        "File {} matched config {} but expected {}",
                        test_file,
                        matched_idx.unwrap(),
                        expected_idx
                    );
                }
                None => assert!(!matched, "File {} should not match any config", test_file),
            }
        }
    }

    #[tokio::test]
    async fn test_nested_filter() {
        let config = ToplocConfig {
            server_url: "http://test".to_string(),
            auth_token: None,
            file_prefix_filter: Some("Qwen/Qwen0.6".to_string()),
        };
        let toploc = Toploc::new(config, None);

        assert!(toploc.matches_file_name("/Qwen/Qwen0.6/-model-data.parquet"));
        assert!(toploc.matches_file_name("Qwen/Qwen0.6"));
        assert!(!toploc.matches_file_name("Qwen/Qwen0.7-model-data.parquet"));
        assert!(!toploc.matches_file_name("qwen3-lowercase.parquet")); // Case sensitive
    }

    #[tokio::test]
    async fn test_file_prefix_filter_none() {
        let config = ToplocConfig {
            server_url: "http://test".to_string(),
            auth_token: None,
            file_prefix_filter: None,
        };
        let toploc = Toploc::new(config, None);

        // Should match everything when no filter is set
        assert!(toploc.matches_file_name("Qwen3-model-data.parquet"));
        assert!(toploc.matches_file_name("GPT4-model-data.parquet"));
        assert!(toploc.matches_file_name("any-file-name.txt"));
        assert!(toploc.matches_file_name(""));
    }

    #[tokio::test]
    async fn test_auth_token_header() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        let _mock = server
            .mock("POST", "/validate/test.parquet")
            .with_status(200)
            .match_header("Authorization", "Bearer secret-token")
            .create();

        let config = ToplocConfig {
            server_url: server.url(),
            auth_token: Some("secret-token".to_string()),
            file_prefix_filter: None,
        };
        let toploc = Toploc::new(config, None);

        let result = toploc
            .trigger_single_file_validation("abc123", "0x456", "test.parquet")
            .await;

        assert!(result.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn test_network_timeout_error() -> Result<(), Error> {
        // Test with an invalid/unreachable URL to simulate network errors
        let config = ToplocConfig {
            server_url: "http://localhost:99999".to_string(), // Invalid port
            auth_token: None,
            file_prefix_filter: None,
        };
        let toploc = Toploc::new(config, None);

        let result = toploc
            .trigger_single_file_validation("abc123", "0x456", "test.parquet")
            .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to trigger validation"));
        Ok(())
    }
    #[tokio::test]
    async fn test_group_validation_with_auth_token() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        let _group_mock = server
            .mock("POST", "/validategroup/test-group.parquet")
            .with_status(200)
            .match_header("Authorization", "Bearer group-token")
            .create();

        let config = ToplocConfig {
            server_url: server.url(),
            auth_token: Some("group-token".to_string()),
            file_prefix_filter: None,
        };
        let toploc = Toploc::new(config, None);

        let result = toploc
            .trigger_group_file_validation(
                "test-group.parquet",
                vec!["sha1".to_string(), "sha2".to_string()],
                "group123",
                1,
                2,
            )
            .await;

        assert!(result.is_ok());
        Ok(())
    }
}
