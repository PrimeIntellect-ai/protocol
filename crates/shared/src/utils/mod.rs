use std::collections::HashMap;

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;
pub mod google_cloud;
use anyhow::Result;

#[async_trait]
pub trait StorageProvider: Send + Sync {
    /// Check if a file exists in storage
    async fn file_exists(&self, object_path: &str) -> Result<bool>;

    /// Generate a mapping file that maps SHA256 hash to original filename
    async fn generate_mapping_file(&self, sha256: &str, file_name: &str) -> Result<String>;

    /// Resolve mapping for a given SHA256 hash to get the original filename
    async fn resolve_mapping_for_sha(&self, sha256: &str) -> Result<String>;

    /// Generate a signed URL for uploading (optional for mock)
    async fn generate_upload_signed_url(
        &self,
        object_path: &str,
        content_type: Option<String>,
        expiration: std::time::Duration,
        max_bytes: Option<u64>,
    ) -> Result<String>;
}

pub struct MockStorageProvider {
    mapping_files: Arc<Mutex<HashMap<String, String>>>, // Maps SHA256 to filename
    files: Arc<Mutex<HashMap<String, String>>>,         // Maps filepath to content
}

impl Default for MockStorageProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MockStorageProvider {
    pub fn new() -> Self {
        Self {
            mapping_files: Arc::new(Mutex::new(HashMap::new())),
            files: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_data(
        mapping_files: HashMap<String, String>,
        files: HashMap<String, String>,
    ) -> Self {
        Self {
            mapping_files: Arc::new(Mutex::new(mapping_files)),
            files: Arc::new(Mutex::new(files)),
        }
    }

    pub async fn add_mapping_file(&self, sha256: &str, file_name: &str) {
        let mut mappings = self.mapping_files.lock().await;
        mappings.insert(sha256.to_string(), file_name.to_string());
    }

    pub async fn add_file(&self, path: &str, content: &str) {
        let mut files = self.files.lock().await;
        files.insert(path.to_string(), content.to_string());
    }
}

#[async_trait]
impl StorageProvider for MockStorageProvider {
    async fn file_exists(&self, object_path: &str) -> Result<bool> {
        let files = self.files.lock().await;
        Ok(files.contains_key(object_path))
    }

    async fn generate_mapping_file(&self, sha256: &str, file_name: &str) -> Result<String> {
        // Store the mapping of SHA256 to filename
        let mapping_path = format!("mapping/{}", sha256);
        self.add_mapping_file(sha256, file_name).await;

        // Also store the mapping file content in our mock storage
        let mapping_content = format!("{}:{}", sha256, file_name);
        self.add_file(&mapping_path, &mapping_content).await;

        Ok(mapping_path)
    }

    async fn resolve_mapping_for_sha(&self, sha256: &str) -> Result<String> {
        // Retrieve the original filename from the mapping
        let mappings = self.mapping_files.lock().await;
        mappings
            .get(sha256)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("No mapping found for SHA256: {}", sha256))
    }

    async fn generate_upload_signed_url(
        &self,
        object_path: &str,
        _content_type: Option<String>,
        _expiration: std::time::Duration,
        _max_bytes: Option<u64>,
    ) -> Result<String> {
        // For a mock, we can return a fake signed URL
        Ok(format!(
            "https://mock-storage.example.com/upload/{}",
            object_path
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_storage_provider() {
        let provider = MockStorageProvider::new();
        provider.add_mapping_file("sha256", "file.txt").await;
        provider.add_file("file.txt", "content").await;
        let map_file_link = provider.resolve_mapping_for_sha("sha256").await.unwrap();
        println!("map_file_link: {}", map_file_link);
        assert_eq!(map_file_link, "file.txt");

        assert_eq!(
            provider.resolve_mapping_for_sha("sha256").await.unwrap(),
            "file.txt"
        );
    }
}
