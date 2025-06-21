use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use shared::models::node::NodeLocation;
use std::time::Duration;

#[derive(Debug, Deserialize, Serialize)]
struct IpApiResponse {
    ip: String,
    city: Option<String>,
    region: Option<String>,
    country: Option<String>,
    #[serde(default)]
    latitude: f64,
    #[serde(default)]
    longitude: f64,
}

pub struct LocationService {
    client: Client,
    base_url: String,
    enabled: bool,
    api_key: String,
}

impl LocationService {
    pub fn new(base_url: Option<String>, api_key: Option<String>) -> Self {
        let enabled = base_url.is_some();
        let base_url = base_url.unwrap_or_else(|| "https://ipapi.co".to_string());
        let api_key = api_key.unwrap_or_default();
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            base_url,
            enabled,
            api_key,
        }
    }

    pub async fn get_location(&self, ip_address: &str) -> Result<Option<NodeLocation>> {
        if !self.enabled {
            return Ok(None);
        }

        let url = format!(
            "{}/{}/json/?key={}",
            self.base_url, ip_address, self.api_key
        );

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to send request to location service")?;

        let api_response: IpApiResponse = response
            .json()
            .await
            .context("Failed to parse location service response")?;

        Ok(Some(NodeLocation {
            latitude: api_response.latitude,
            longitude: api_response.longitude,
            city: api_response.city,
            region: api_response.region,
            country: api_response.country,
        }))
    }
}

impl Default for LocationService {
    fn default() -> Self {
        Self::new(None, None)
    }
}
