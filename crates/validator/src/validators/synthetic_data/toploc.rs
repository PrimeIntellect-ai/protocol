use super::ValidationResult;
use anyhow::Error;
use log::debug;
use log::{error, info};
use redis::Commands;
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

    pub fn matches_file_name(&self, file_name: &str) -> bool {
        match &self.config.file_prefix_filter {
            Some(prefix) => file_name.starts_with(prefix),
            None => true,
        }
    }

    pub async fn trigger_remote_toploc_validation(
        &self,
        file_sha: &str,
        key_address: &str,
        file_name: &str,
    ) -> Result<(), Error> {
        let validate_url = format!("{}/validate/{}", self.config.server_url, file_name);
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
            Ok(_) => {
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

    pub async fn get_validation_status(&self, file_name: &str) -> Result<ValidationResult, Error> {
        let url = format!("{}/status/{}", self.config.server_url, file_name);

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
