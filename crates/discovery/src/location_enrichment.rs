use crate::location_service::LocationService;
use crate::store::node_store::NodeStore;
use anyhow::Result;
use log::{error, info, warn};
use redis::AsyncCommands;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

const LOCATION_RETRY_KEY: &str = "location:retries:";
const MAX_RETRIES: u32 = 3;
const BATCH_SIZE: usize = 10;
const RATE_LIMIT_DELAY_MS: u64 = 100;

pub struct LocationEnrichmentService {
    node_store: Arc<NodeStore>,
    location_service: Arc<LocationService>,
    redis_client: redis::Client,
}

impl LocationEnrichmentService {
    pub fn new(
        node_store: Arc<NodeStore>,
        location_service: Arc<LocationService>,
        redis_url: &str,
    ) -> Result<Self> {
        let redis_client = redis::Client::open(redis_url)?;
        Ok(Self {
            node_store,
            location_service,
            redis_client,
        })
    }

    pub async fn run(&self, interval_seconds: u64) -> Result<()> {
        let mut interval = interval(Duration::from_secs(interval_seconds));

        loop {
            interval.tick().await;

            if let Err(e) = self.enrich_nodes_without_location().await {
                error!("Location enrichment cycle failed: {e}");
            }
        }
    }

    async fn enrich_nodes_without_location(&self) -> Result<()> {
        let nodes = self.node_store.get_nodes().await?;
        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;

        let nodes_without_location: Vec<_> = nodes
            .into_iter()
            .filter(|node| node.location.is_none())
            .collect();

        if nodes_without_location.is_empty() {
            return Ok(());
        }

        info!(
            "Found {} nodes without location data",
            nodes_without_location.len()
        );

        // Process in batches to respect rate limits
        for chunk in nodes_without_location.chunks(BATCH_SIZE) {
            for node in chunk {
                let retry_key = format!("{}{}", LOCATION_RETRY_KEY, node.id);
                let retries: u32 = conn.get(&retry_key).await.unwrap_or(0);

                if retries >= MAX_RETRIES {
                    continue; // Skip nodes that have exceeded retry limit
                }

                match self.location_service.get_location(&node.ip_address).await {
                    Ok(Some(location)) => {
                        info!(
                            "Successfully fetched location for node {}: {:?}",
                            node.id, location
                        );

                        let mut updated_node = node.clone();
                        updated_node.location = Some(location);

                        if let Err(e) = self.node_store.update_node(updated_node).await {
                            error!("Failed to update node {} with location: {}", node.id, e);
                        } else {
                            let _: () = conn.del(&retry_key).await?;
                        }
                    }
                    Ok(None) => {
                        // Location service is disabled
                        break;
                    }
                    Err(e) => {
                        warn!(
                            "Failed to fetch location for node {} (attempt {}/{}): {}",
                            node.id,
                            retries + 1,
                            MAX_RETRIES,
                            e
                        );

                        // Increment retry counter
                        let _: () = conn.set_ex(&retry_key, retries + 1, 86400).await?;
                        // Expire after 24h
                    }
                }

                // Rate limiting - wait between requests
                tokio::time::sleep(Duration::from_millis(RATE_LIMIT_DELAY_MS)).await;
            }

            // Longer wait between batches
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        Ok(())
    }
}
