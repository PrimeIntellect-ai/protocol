use alloy::primitives::U256;
use anyhow::{Context, Error, Result};
use directories::ProjectDirs;
use hex;
use log::debug;
use log::{error, info};
use serde::{Deserialize, Serialize};
use shared::utils::google_cloud::resolve_mapping_for_sha;
use shared::web3::contracts::implementations::prime_network_contract::PrimeNetworkContract;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use toml;

use crate::validators::Validator;
use shared::web3::contracts::implementations::work_validators::synthetic_data_validator::SyntheticDataWorkValidator;

fn get_default_state_dir() -> Option<String> {
    ProjectDirs::from("com", "prime", "validator")
        .map(|proj_dirs| proj_dirs.data_local_dir().to_string_lossy().into_owned())
}

fn state_filename(pool_id: &str) -> String {
    format!("work_state_{}.toml", pool_id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedWorkState {
    pool_id: U256,
    last_validation_timestamp: U256,
}

pub struct SyntheticDataValidator {
    pool_id: U256,
    validator: SyntheticDataWorkValidator,
    prime_network: PrimeNetworkContract,
    last_validation_timestamp: U256,
    state_dir: Option<PathBuf>,
    leviticus_url: String,
    leviticus_token: Option<String>,
    penalty: U256,
    s3_credentials: Option<String>,
    bucket_name: Option<String>,
}

impl Validator for SyntheticDataValidator {
    type Error = anyhow::Error;

    fn name(&self) -> &str {
        "Synthetic Data Validator"
    }
}

impl SyntheticDataValidator {
    pub fn new(
        state_dir: Option<String>,
        pool_id_str: String,
        validator: SyntheticDataWorkValidator,
        prime_network: PrimeNetworkContract,
        leviticus_url: String,
        leviticus_token: Option<String>,
        penalty: U256,
        s3_credentials: Option<String>,
        bucket_name: Option<String>,
    ) -> Self {
        let pool_id = pool_id_str.parse::<U256>().expect("Invalid pool ID");
        let default_state_dir = get_default_state_dir();
        debug!("Default state dir: {:?}", default_state_dir);
        let state_path = state_dir
            .map(PathBuf::from)
            .or_else(|| default_state_dir.map(PathBuf::from));
        debug!("State path: {:?}", state_path);
        let mut last_validation_timestamp: Option<_> = None;

        // Try to load state, log info if creating new file
        if let Some(path) = &state_path {
            let state_file = path.join(state_filename(&pool_id.to_string()));
            if !state_file.exists() {
                debug!(
                    "No state file found at {:?}, will create on first state change",
                    state_file
                );
            } else if let Ok(Some(loaded_state)) =
                SyntheticDataValidator::load_state(path, &pool_id.to_string())
            {
                debug!("Loaded previous state from {:?}", state_file);
                last_validation_timestamp = Some(loaded_state.last_validation_timestamp);
            } else {
                debug!("Failed to load state from {:?}", state_file);
            }
        }

        // if no last time, set it to 24 hours ago, as nothing before that can be invalidated
        if last_validation_timestamp.is_none() {
            last_validation_timestamp = Some(U256::from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("Failed to get current timestamp")
                    .as_secs()
                    .saturating_sub(24 * 60 * 60),
            ));
        }

        Self {
            pool_id,
            validator,
            prime_network,
            last_validation_timestamp: last_validation_timestamp.unwrap(),
            state_dir: state_path.clone(),
            leviticus_url,
            leviticus_token,
            penalty,
            s3_credentials,
            bucket_name,
        }
    }

    fn save_state(&self) -> Result<()> {
        // Get values without block_on
        let state = PersistedWorkState {
            pool_id: self.pool_id,
            last_validation_timestamp: self.last_validation_timestamp,
        };

        if let Some(ref state_dir) = self.state_dir {
            fs::create_dir_all(state_dir)?;
            let state_path = state_dir.join(state_filename(&self.pool_id.to_string()));
            let toml = toml::to_string_pretty(&state)?;
            fs::write(&state_path, toml)?;
            debug!("Saved state to {:?}", state_path);
        }
        Ok(())
    }

    fn load_state(state_dir: &Path, pool_id: &str) -> Result<Option<PersistedWorkState>> {
        let state_path = state_dir.join(state_filename(pool_id));
        if state_path.exists() {
            let contents = fs::read_to_string(state_path)?;
            let state: PersistedWorkState = toml::from_str(&contents)?;
            return Ok(Some(state));
        }
        Ok(None)
    }

    pub async fn invalidate_work(&self, work_key: &str) -> Result<()> {
        let data = hex::decode(work_key)
            .map_err(|e| Error::msg(format!("Failed to decode hex work key: {}", e)))?;
        println!("Invalidating work: {}", work_key);
        match self
            .prime_network
            .invalidate_work(self.pool_id, self.penalty, data)
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                error!("Failed to invalidate work {}: {}", work_key, e);
                Err(Error::msg(format!("Failed to invalidate work: {}", e)))
            }
        }
    }

    pub async fn validate_work(&mut self) -> Result<()> {
        info!("Validating work for pool ID: {:?}", self.pool_id);

        // Get all work keys for the pool
        let work_keys = self
            .validator
            .get_work_since(self.pool_id, self.last_validation_timestamp)
            .await
            .context("Failed to get work keys")?;

        info!("Found {} work keys to validate", work_keys.len());

        // Process each work key
        for work_key in work_keys {
            info!("Processing work key: {}", work_key);
            match self.validator.get_work_info(self.pool_id, &work_key).await {
                Ok(work_info) => {
                    info!(
                        "Got work info - Provider: {:?}, Node: {:?}, Timestamp: {}",
                        work_info.provider, work_info.node_id, work_info.timestamp
                    );

                    // This is a temporary solution to get the original file name
                    // it will be replaced by file data loading from dht
                    println!("Resolving original file name for work key: {}", work_key);

                    let original_file_name = resolve_mapping_for_sha(
                        self.bucket_name.clone().unwrap().as_str(),
                        &self.s3_credentials.clone().unwrap(),
                        &work_key,
                    )
                    .await
                    .unwrap();

                    if original_file_name.is_empty() {
                        error!(
                            "Failed to resolve original file name for work key: {}",
                            work_key
                        );
                        continue;
                    }
                    let cleaned_file_name = if original_file_name.starts_with('/') {
                        original_file_name[1..].to_string()
                    } else {
                        original_file_name.clone()
                    };
                    println!("Original file name: {}", cleaned_file_name);

                    // Start validation by calling validation endpoint with retries
                    let validate_url =
                        format!("{}/validate/{}", self.leviticus_url, cleaned_file_name);
                    println!("Validation URL: {}", validate_url);

                    let mut client = reqwest::Client::builder();

                    // Add auth token if provided
                    if let Some(token) = &self.leviticus_token {
                        client = client.default_headers({
                            let mut headers = reqwest::header::HeaderMap::new();
                            headers.insert(
                                reqwest::header::AUTHORIZATION,
                                reqwest::header::HeaderValue::from_str(&format!(
                                    "Bearer {}",
                                    token
                                ))
                                .expect("Invalid token"),
                            );
                            headers
                        });
                    }

                    let client = client.build().expect("Failed to build HTTP client");

                    let mut validate_attempts = 0;
                    const MAX_VALIDATE_ATTEMPTS: u32 = 3;

                    let validation_result = loop {
                        let body = serde_json::json!({
                            "file_sha": work_key
                        });

                        match client.post(&validate_url).json(&body).send().await {
                            Ok(_) => {
                                info!("Started validation for work key: {}", work_key);
                                break Ok(());
                            }
                            Err(e) => {
                                validate_attempts += 1;
                                error!(
                                    "Attempt {} failed to start validation for {}: {}",
                                    validate_attempts, work_key, e
                                );

                                if validate_attempts >= MAX_VALIDATE_ATTEMPTS {
                                    break Err(e);
                                }

                                // Exponential backoff
                                tokio::time::sleep(tokio::time::Duration::from_secs(
                                    2u64.pow(validate_attempts),
                                ))
                                .await;
                            }
                        }
                    };

                    match validation_result {
                        Ok(_) => {
                            // Poll status endpoint until we get a proper response
                            let status_url =
                                format!("{}/status/{}", self.leviticus_url, cleaned_file_name);
                            let mut status_attempts = 0;
                            const MAX_STATUS_ATTEMPTS: u32 = 5;

                            loop {
                                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                                match client.get(&status_url).send().await {
                                    Ok(response) => {
                                        match response.json::<serde_json::Value>().await {
                                            Ok(status_json) => {
                                                println!("Status JSON: {:?}", status_json);
                                                match status_json
                                                    .get("status")
                                                    .and_then(|s| s.as_str())
                                                {
                                                    Some(status) => {
                                                        info!(
                                                            "Validation status for {}: {}",
                                                            work_key, status
                                                        );

                                                        match status {
                                                            "accept" => {
                                                                info!(
                                                                    "Work {} was accepted",
                                                                    work_key
                                                                );
                                                                break;
                                                            }
                                                            "reject" => {
                                                                error!(
                                                                    "Work {} was rejected",
                                                                    work_key
                                                                );
                                                                if let Err(e) = self
                                                                    .invalidate_work(&work_key)
                                                                    .await
                                                                {
                                                                    error!("Failed to invalidate work {}: {}", work_key, e);
                                                                } else {
                                                                    info!("Successfully invalidated work {}", work_key);
                                                                }
                                                                break;
                                                            }
                                                            "crashed" => {
                                                                error!(
                                                                    "Validation crashed for {}",
                                                                    work_key
                                                                );
                                                                break;
                                                            }
                                                            "pending" => {
                                                                status_attempts += 1;
                                                                if status_attempts
                                                                    >= MAX_STATUS_ATTEMPTS
                                                                {
                                                                    error!("Max status attempts reached for {}", work_key);
                                                                    break;
                                                                }
                                                            }
                                                            _ => {
                                                                status_attempts += 1;
                                                                error!(
                                                                    "Unknown status {} for {}",
                                                                    status, work_key
                                                                );
                                                                if status_attempts
                                                                    >= MAX_STATUS_ATTEMPTS
                                                                {
                                                                    break;
                                                                }
                                                            }
                                                        }
                                                    }
                                                    None => {
                                                        status_attempts += 1;
                                                        error!(
                                                            "No status field in response for {}",
                                                            work_key
                                                        );
                                                        if status_attempts >= MAX_STATUS_ATTEMPTS {
                                                            error!("Max status attempts reached for {}", work_key);
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                status_attempts += 1;
                                                error!("Attempt {} failed to parse status JSON for {}: {}", status_attempts, work_key, e);

                                                if status_attempts >= MAX_STATUS_ATTEMPTS {
                                                    error!(
                                                        "Max status attempts reached for {}",
                                                        work_key
                                                    );
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        status_attempts += 1;
                                        error!(
                                            "Attempt {} failed to get status for {}: {}",
                                            status_attempts, work_key, e
                                        );

                                        if status_attempts >= MAX_STATUS_ATTEMPTS {
                                            error!("Max status attempts reached for {}", work_key);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            error!("Failed all validation attempts for {}", work_key);
                            continue;
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to get work info for key {}: {}", work_key, e);
                    continue;
                }
            }
        }

        // Update last validation timestamp to current time
        // TODO: We should only set this once we are sure that we have validated all keys
        self.last_validation_timestamp = U256::from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .context("Failed to get current timestamp")?
                .as_secs(),
        );

        self.save_state()?;

        Ok(())
    }
}
