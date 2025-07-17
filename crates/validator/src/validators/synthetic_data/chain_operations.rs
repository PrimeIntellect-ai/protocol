use super::*;

impl SyntheticDataValidator<WalletProvider> {
    #[cfg(test)]
    pub fn soft_invalidate_work(&self, work_key: &str) -> Result<(), Error> {
        info!("Soft invalidating work: {work_key}");

        if self.disable_chain_invalidation {
            info!("Chain invalidation is disabled, skipping work soft invalidation");
            return Ok(());
        }

        info!("Test mode: skipping actual work soft invalidation");
        let _ = &self.prime_network;
        Ok(())
    }

    #[cfg(not(test))]
    pub async fn soft_invalidate_work(&self, work_key: &str) -> Result<(), Error> {
        info!("Soft invalidating work: {work_key}");

        if self.disable_chain_invalidation {
            info!("Chain invalidation is disabled, skipping work soft invalidation");
            return Ok(());
        }

        let work_info = self
            .get_work_info_from_redis(work_key)
            .await?
            .ok_or_else(|| Error::msg("Work info not found for soft invalidation"))?;
        let work_key_bytes = hex::decode(work_key)
            .map_err(|e| Error::msg(format!("Failed to decode hex work key: {e}")))?;

        // Create 64-byte payload: work_key (32 bytes) + work_units (32 bytes)
        let mut data = Vec::with_capacity(64);
        data.extend_from_slice(&work_key_bytes);

        // Convert work_units to 32-byte representation
        let work_units_bytes = work_info.work_units.to_be_bytes::<32>();
        data.extend_from_slice(&work_units_bytes);

        match self
            .prime_network
            .soft_invalidate_work(self.pool_id, data)
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                error!("Failed to soft invalidate work {work_key}: {e}");
                Err(Error::msg(format!("Failed to soft invalidate work: {e}")))
            }
        }
    }

    #[cfg(test)]
    pub fn invalidate_work(&self, work_key: &str) -> Result<(), Error> {
        info!("Invalidating work: {work_key}");

        if let Some(metrics) = &self.metrics {
            metrics.record_work_key_invalidation();
        }

        if self.disable_chain_invalidation {
            info!("Chain invalidation is disabled, skipping work invalidation");
            return Ok(());
        }

        info!("Test mode: skipping actual work invalidation");
        let _ = &self.prime_network;
        let _ = &self.penalty;
        Ok(())
    }

    #[cfg(not(test))]
    pub async fn invalidate_work(&self, work_key: &str) -> Result<(), Error> {
        info!("Invalidating work: {work_key}");

        if let Some(metrics) = &self.metrics {
            metrics.record_work_key_invalidation();
        }

        if self.disable_chain_invalidation {
            info!("Chain invalidation is disabled, skipping work invalidation");
            return Ok(());
        }

        let data = hex::decode(work_key)
            .map_err(|e| Error::msg(format!("Failed to decode hex work key: {e}")))?;
        match self
            .prime_network
            .invalidate_work(self.pool_id, self.penalty, data)
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                error!("Failed to invalidate work {work_key}: {e}");
                Err(Error::msg(format!("Failed to invalidate work: {e}")))
            }
        }
    }

    pub async fn invalidate_according_to_invalidation_type(
        &self,
        work_key: &str,
        invalidation_type: InvalidationType,
    ) -> Result<(), Error> {
        match invalidation_type {
            #[cfg(test)]
            InvalidationType::Soft => self.soft_invalidate_work(work_key),
            #[cfg(not(test))]
            InvalidationType::Soft => self.soft_invalidate_work(work_key).await,
            #[cfg(test)]
            InvalidationType::Hard => self.invalidate_work(work_key),
            #[cfg(not(test))]
            InvalidationType::Hard => self.invalidate_work(work_key).await,
        }
    }
}
