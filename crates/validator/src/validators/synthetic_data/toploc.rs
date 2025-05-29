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
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupValidationResult {
    pub status: ValidationResult,
    pub flops: f64,
    // This tells us which node(s) in a group actually failed the toploc validation
    pub failing_indices: Vec<i64>,
}

impl Toploc {
    pub fn new(config: ToplocConfig) -> Self {
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

        Self { config, client }
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
            Some(prefix) => normalized_name.starts_with(prefix),
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
        info!(
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
                if !response.status().is_success() {
                    error!(
                        "Server returned error status {} for {}",
                        response.status(),
                        file_name
                    );
                    return Err(Error::msg(format!(
                        "Server returned error status: {}",
                        response.status()
                    )));
                }
                let trigger_duration = start_time.elapsed();
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
        println!(
            "Triggering remote toploc group validation for {} {}",
            file_name, validate_url
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
                if !response.status().is_success() {
                    error!(
                        "Server returned error status {} for {}",
                        response.status(),
                        file_name
                    );
                    return Err(Error::msg(format!(
                        "Server returned error status: {}",
                        response.status()
                    )));
                }
                let trigger_duration = start_time.elapsed();
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
        println!("Getting group validation status for {}", file_name);
        let url = format!(
            "{}/statusgroup/{}",
            self.config.server_url, processed_file_name
        );
        debug!("URL: {}", url);
        println!("URL: {}", url);

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

                            let flops = status_json
                                .get("flops")
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
                                flops,
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
        let toploc = Toploc::new(config);

        let result = toploc
            .trigger_single_file_validation("abc123", "0x456", "test-file.parquet")
            .await;
        println!("Result for single: {:?}", result);

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
        let toploc = Toploc::new(config);

        let result = toploc
            .trigger_group_file_validation(
                "test-group.parquet",
                vec!["sha1".to_string(), "sha2".to_string()],
                "group123",
                1,
                2,
            )
            .await;
        println!("Resul for validation success: {:?}", result);

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
        let toploc = Toploc::new(config);

        let result = toploc
            .trigger_group_file_validation(
                "test-group.parquet",
                vec!["sha1".to_string(), "sha2".to_string()],
                "group123",
                1,
                2,
            )
            .await;

        println!("Result: {:?}", result);

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
        let toploc = Toploc::new(config);

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
        let toploc = Toploc::new(config);

        let result = toploc
            .get_single_file_validation_status("test-file.parquet")
            .await;
        println!("Result for single: {:?}", result);

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
            .with_body(r#"{"status": "accept", "flops": 12345.67, "failing_indices": []}"#)
            .create();

        let config = ToplocConfig {
            server_url: server.url(),
            auth_token: None,
            file_prefix_filter: None,
        };
        let toploc = Toploc::new(config);

        let result = toploc
            .get_group_file_validation_status("test-group.parquet")
            .await;

        assert!(result.is_ok());
        let group_result = result.unwrap();
        assert_eq!(group_result.status, ValidationResult::Accept);
        assert_eq!(group_result.flops, 12345.67);
        assert!(group_result.failing_indices.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_group_file_status_reject_with_failing_indices() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        let _status_mock = server
            .mock("GET", "/statusgroup/test-group.parquet")
            .with_status(200)
            .with_body(r#"{"status": "reject", "flops": 0.0, "failing_indices": [1, 3, 5]}"#)
            .create();

        let config = ToplocConfig {
            server_url: server.url(),
            auth_token: None,
            file_prefix_filter: None,
        };
        let toploc = Toploc::new(config);

        let result = toploc
            .get_group_file_validation_status("test-group.parquet")
            .await;

        assert!(result.is_ok());
        let group_result = result.unwrap();
        assert_eq!(group_result.status, ValidationResult::Reject);
        assert_eq!(group_result.flops, 0.0);
        assert_eq!(group_result.failing_indices, vec![1, 3, 5]);
        Ok(())
    }

    #[tokio::test]
    async fn test_file_prefix_filter_matching() {
        let config = ToplocConfig {
            server_url: "http://test".to_string(),
            auth_token: None,
            file_prefix_filter: Some("Qwen3".to_string()),
        };
        let toploc = Toploc::new(config);

        assert!(toploc.matches_file_name("Qwen3-model-data.parquet"));
        assert!(toploc.matches_file_name("Qwen3"));
        assert!(!toploc.matches_file_name("GPT4-model-data.parquet"));
        assert!(!toploc.matches_file_name("qwen3-lowercase.parquet")); // Case sensitive
    }

    #[tokio::test]
    async fn test_file_prefix_filter_none() {
        let config = ToplocConfig {
            server_url: "http://test".to_string(),
            auth_token: None,
            file_prefix_filter: None,
        };
        let toploc = Toploc::new(config);

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
        let toploc = Toploc::new(config);

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
        let toploc = Toploc::new(config);

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
        let toploc = Toploc::new(config);

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
