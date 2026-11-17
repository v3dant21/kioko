use aws_sdk_s3::{Client, primitives::ByteStream};
use crate::error::AppError;

pub fn get_public_url(bucket: &str, key: &str, region: &str) -> String {
    match std::env::var("AWS_ENDPOINT_URL") {
        Ok(endpoint) if endpoint.contains("localhost") || endpoint.contains("localstack") => {
            format!("http://{bucket}.s3.localhost.localstack.cloud:4566/{key}")
        }
        _ => format!("https://{bucket}.s3.{region}.amazonaws.com/{key}"),
    }
}

// Upload bytes to S3 and return the public URL.
pub async fn upload_to_s3(
    client: &Client,
    bucket: &str,
    region: &str,
    key: &str,
    data: Vec<u8>,
    content_type: &str,
) -> Result<String, AppError> {
    client
        .put_object()
        .bucket(bucket)
        .key(key)
        .body(ByteStream::from(data))
        .content_type(content_type)
        .send()
        .await
        .map_err(|e| AppError::S3(format!("S3 upload error for key={key}: {e:?}")))?;

    Ok(get_public_url(bucket, key, region))
}

// Download an object from S3 and return its bytes.
pub async fn download_from_s3(
    client: &Client,
    bucket: &str,
    key: &str,
) -> Result<Vec<u8>, AppError> {
    let resp = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| AppError::S3(format!("S3 download error: {e}")))?;

    let bytes = resp
        .body
        .collect()
        .await
        .map_err(|e| AppError::S3(format!("S3 body read error: {e}")))?
        .into_bytes()
        .to_vec();

    Ok(bytes)
}

pub fn validate_gallery_id(id: &str) -> Result<(), AppError> {
    if id.is_empty() || id == "." || id == ".." || id.contains(['/', '\\']) {
        return Err(AppError::BadRequest("Invalid gallery ID".into()));
    }
    Ok(())
}

/// List all object keys under `galleries/<gallery_id>/`.
pub async fn list_gallery_images(
    client: &Client,
    bucket: &str,
    gallery_id: &str,
) -> Result<Vec<String>, AppError> {
    validate_gallery_id(gallery_id)?;
    let prefix = format!("galleries/{gallery_id}/");
    let mut pages = client.list_objects_v2().bucket(bucket).prefix(prefix).into_paginator().send();
    let mut keys = Vec::new();
    while let Some(page) = pages.next().await {
        let page = page.map_err(|e| AppError::S3(format!("S3 list error: {e}")))?;
        keys.extend(page.contents().iter().filter_map(|obj| obj.key().map(str::to_owned)));
    }
    Ok(keys)
}
