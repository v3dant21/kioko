use super::{fault, Error, Result};
use crate::AppState;
use aws_sdk_s3::{error::ProvideErrorMetadata, primitives::ByteStream};
use axum::http::StatusCode;
use serde::{de::DeserializeOwned, Serialize};

pub async fn read(s: &AppState, key: &str) -> Result<(axum::body::Bytes, String)> {
    let obj = s
        .s3_client
        .get_object()
        .bucket(&s.bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| {
            if e.as_service_error().and_then(|e| e.code()) == Some("NoSuchKey") {
                Error(StatusCode::NOT_FOUND, "Item not found".into())
            } else {
                fault(e)
            }
        })?;
    let tag = obj.e_tag().unwrap_or_default().to_owned();
    Ok((obj.body.collect().await.map_err(fault)?.into_bytes(), tag))
}

pub async fn json<T: DeserializeOwned>(s: &AppState, key: &str) -> Result<(T, String)> {
    let (bytes, tag) = read(s, key).await?;
    Ok((serde_json::from_slice(&bytes).map_err(fault)?, tag))
}

// A missing ETag means create-only. Existing objects are updated by compare-and-swap.
pub async fn put(s: &AppState, key: &str, bytes: Vec<u8>, tag: Option<&str>) -> Result<bool> {
    let req = s
        .s3_client
        .put_object()
        .bucket(&s.bucket)
        .key(key)
        .body(ByteStream::from(bytes));
    let req = match tag {
        Some(tag) => req.if_match(tag),
        None => req.if_none_match("*"),
    };
    match req.send().await {
        Ok(_) => Ok(true),
        Err(e)
            if matches!(
                e.as_service_error().and_then(|e| e.code()),
                Some("PreconditionFailed")
            ) =>
        {
            Ok(false)
        }
        Err(e)
            if e.as_service_error().and_then(|e| e.code())
                == Some("ConditionalRequestConflict") =>
        {
            Err(super::conflict("Concurrent storage operation. Retry."))
        }
        Err(e) => Err(fault(e)),
    }
}

pub async fn put_json<T: Serialize>(
    s: &AppState,
    key: &str,
    value: &T,
    tag: Option<&str>,
) -> Result<bool> {
    put(s, key, serde_json::to_vec(value).map_err(fault)?, tag).await
}

pub async fn list(s: &AppState, prefix: &str) -> Result<Vec<String>> {
    let mut keys = Vec::new();
    let mut pages = s
        .s3_client
        .list_objects_v2()
        .bucket(&s.bucket)
        .prefix(prefix)
        .into_paginator()
        .send();
    while let Some(page) = pages.next().await {
        keys.extend(
            page.map_err(fault)?
                .contents()
                .iter()
                .filter_map(|x| x.key().map(str::to_owned)),
        );
    }
    Ok(keys)
}

// List versions even on unversioned buckets (their objects have the null version).
pub async fn purge(s: &AppState, prefix: &str) -> Result<()> {
    loop {
        let page = s
            .s3_client
            .list_object_versions()
            .bucket(&s.bucket)
            .prefix(prefix)
            .max_keys(500)
            .send()
            .await
            .map_err(fault)?;
        let objects = page
            .versions()
            .iter()
            .map(|x| (x.key(), x.version_id()))
            .chain(
                page.delete_markers()
                    .iter()
                    .map(|x| (x.key(), x.version_id())),
            );
        let mut count = 0;
        for (key, version) in objects {
            let key = key.ok_or_else(|| fault("Missing key in version listing"))?;
            s.s3_client
                .delete_object()
                .bucket(&s.bucket)
                .key(key)
                .set_version_id(version.map(str::to_owned))
                .send()
                .await
                .map_err(fault)?;
            count += 1;
        }
        if count == 0 {
            return Ok(());
        }
    }
}
