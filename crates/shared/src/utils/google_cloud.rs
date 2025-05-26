use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use google_cloud_storage::client::google_cloud_auth::credentials::CredentialsFile;
use google_cloud_storage::client::{Client, ClientConfig};
use google_cloud_storage::http::objects::download::Range;
use google_cloud_storage::http::objects::get::GetObjectRequest;
use google_cloud_storage::http::objects::upload::{Media, UploadObjectRequest, UploadType};
use google_cloud_storage::sign::{SignedURLMethod, SignedURLOptions};
use log::debug;
use std::time::Duration;

/// Creates a GCS client from base64-encoded credentials
async fn create_gcs_client(credentials_base64: &str) -> Result<Client> {
    // Decode base64 to JSON string
    let credentials_json = general_purpose::STANDARD.decode(credentials_base64)?;
    let credentials_str = String::from_utf8(credentials_json)?;

    // Create client config directly from the JSON string
    let credentials = CredentialsFile::new_from_str(&credentials_str)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to parse credentials: {}", e))?;

    let config = ClientConfig::default()
        .with_credentials(credentials)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to configure client: {}", e))?;

    Ok(Client::new(config))
}

fn get_bucket_name(bucket: &str) -> (String, String) {
    if let Some(idx) = bucket.find('/') {
        let (bucket_part, subpath_part) = bucket.split_at(idx);
        (
            bucket_part.to_string(),
            subpath_part.trim_start_matches('/').to_string(),
        )
    } else {
        (bucket.to_string(), "".to_string())
    }
}

/// Generates a mapping file in GCS that maps a file's SHA256 hash to its original filename.
/// This is necessary because on-chain storage only stores the file's SHA256 hash,
/// so we need this mapping to retrieve the original filename when needed.
pub async fn generate_mapping_file(
    bucket: &str,
    credentials_base64: &str,
    sha256: &str,
    file_name: &str,
) -> Result<String> {
    let client = create_gcs_client(credentials_base64).await?;
    let mapping_path = format!("mapping/{}", sha256);

    let file_name = file_name.strip_prefix('/').unwrap_or(file_name);
    let content = file_name.to_string().into_bytes();

    let (bucket_name, subpath) = get_bucket_name(bucket);
    let object_path = if !subpath.is_empty() {
        format!("{}/{}", subpath, mapping_path)
    } else {
        mapping_path.clone()
    };

    let upload_type = UploadType::Simple(Media::new(object_path.clone()));

    let uploaded = client
        .upload_object(
            &UploadObjectRequest {
                bucket: bucket_name,
                ..Default::default()
            },
            content,
            &upload_type,
        )
        .await;

    debug!("Uploaded mapping file: {:?}", uploaded);

    Ok(mapping_path) // Return the mapping_path instead
}

pub async fn resolve_mapping_for_sha(
    bucket: &str,
    credentials_base64: &str,
    sha256: &str,
) -> Result<String> {
    let client = create_gcs_client(credentials_base64).await?;

    let (bucket_name, subpath) = get_bucket_name(bucket);
    let mapping_path = format!("mapping/{}", sha256);

    let object_path = if !subpath.is_empty() {
        format!("{}/{}", subpath, mapping_path)
    } else {
        mapping_path.clone()
    };

    // Download the mapping file content
    let content = client
        .download_object(
            &GetObjectRequest {
                bucket: bucket_name,
                object: object_path.clone(),
                ..Default::default()
            },
            &Range::default(),
        )
        .await?;

    // Convert bytes to string
    let file_name = String::from_utf8(content)?;

    Ok(file_name)
}

pub async fn generate_upload_signed_url(
    bucket: &str,
    object_path: &str,
    credentials_base64: &str,
    content_type: Option<String>,
    expiration: Duration,
    max_bytes: Option<u64>,
) -> Result<String> {
    // Ensure object_path does not start with a /
    let object_path = object_path.strip_prefix('/').unwrap_or(object_path); // Adjusted to use strip_prefix

    let (bucket_name, subpath) = get_bucket_name(bucket);
    let object_path = if !subpath.is_empty() {
        format!("{}/{}", subpath, object_path)
    } else {
        object_path.to_string()
    };

    let client = create_gcs_client(credentials_base64).await?;

    // Set options for the signed URL
    let mut options = SignedURLOptions {
        method: SignedURLMethod::PUT,
        expires: expiration,
        content_type,
        ..Default::default()
    };

    // Set max bytes if specified
    if let Some(bytes) = max_bytes {
        options.headers = vec![format!("content-length:{}", bytes)];
    }

    // Generate the signed URL
    let signed_url = client
        .signed_url(
            bucket_name.as_str(),
            object_path.as_str(),
            None,
            None,
            options,
        )
        .await?;
    Ok(signed_url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;
    #[tokio::test]
    async fn test_generate_mapping_file() {
        // Check if required environment variables are set
        let bucket_name = match std::env::var("BUCKET_NAME") {
            Ok(name) => name,
            Err(_) => {
                println!("Skipping test: BUCKET_NAME not set");
                return;
            }
        };

        let credentials_base64 = match std::env::var("S3_CREDENTIALS") {
            Ok(credentials) => credentials,
            Err(_) => {
                println!("Skipping test: S3_CREDENTIALS not set");
                return;
            }
        };

        let random_sha256: String = rand::rng().random_range(0..=u64::MAX).to_string();

        let mapping_content: String = generate_mapping_file(
            &bucket_name,
            &credentials_base64,
            &random_sha256,
            "run_1/file.parquet",
        )
        .await
        .unwrap();
        println!("mapping_content: {}", mapping_content);

        println!("bucket_name: {}", bucket_name);

        let original_file_name =
            resolve_mapping_for_sha(&bucket_name, &credentials_base64, &random_sha256)
                .await
                .unwrap();

        println!("original_file_name: {}", original_file_name);
        assert_eq!(original_file_name, "run_1/file.parquet");
    }
}
