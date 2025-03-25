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

pub async fn generate_mapping_file(
    bucket: &str,
    credentials_base64: &str,
    sha256: &str,
    file_name: &str,
) -> Result<String> {
    let client = create_gcs_client(credentials_base64).await?;
    let mapping_path = format!("mapping/{}", sha256); // Use sha256 as filename
    let upload_type = UploadType::Simple(Media::new(mapping_path.clone()));

    let file_name = if file_name.starts_with('/') {
        &file_name[1..]
    } else {
        file_name
    };
    let content = file_name.to_string().into_bytes();

    let uploaded = client
        .upload_object(
            &UploadObjectRequest {
                bucket: bucket.to_string(),
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
    let mapping_path = format!("mapping/{}", sha256);

    // Download the mapping file content
    let content = client
        .download_object(
            &GetObjectRequest {
                bucket: bucket.to_string(),
                object: mapping_path.clone(),
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
    let object_path = if object_path.starts_with('/') {
        &object_path[1..]
    } else {
        object_path
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
        .signed_url(bucket, object_path, None, None, options)
        .await?;
    Ok(signed_url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    #[tokio::test]
    async fn test_generate_mapping_file() {
        let bucket_name = std::env::var("BUCKET_NAME").expect("BUCKET_NAME not set");
        let credentials_base64 = std::env::var("S3_CREDENTIALS").expect("S3_CREDENTIALS not set");

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

        let original_file_name =
            resolve_mapping_for_sha(&bucket_name, &credentials_base64, &random_sha256)
                .await
                .unwrap();

        println!("original_file_name: {}", original_file_name);
        assert_eq!(original_file_name, "run_1/file.parquet");
    }
}
