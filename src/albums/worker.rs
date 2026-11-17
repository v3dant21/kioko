use super::*;
use aws_sdk_s3::error::ProvideErrorMetadata;
pub fn start(s: AppState) {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(60));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            if let Err(e) = sweep(&s).await {
                eprintln!("Album cleanup: {}", e.1)
            }
        }
    });
}
async fn sweep(s: &AppState) -> Result<()> {
    let mut pages = s
        .s3_client
        .list_objects_v2()
        .bucket(&s.bucket)
        .prefix("v2/due/")
        .into_paginator()
        .send();
    while let Some(page) = pages.next().await {
        let page = page.map_err(|err| {
            let code = err.as_service_error().and_then(|e| e.code()).unwrap_or("no AWS error code");
            let status = err.raw_response().map(|r| r.status().as_u16().to_string()).unwrap_or_else(|| "no response".into());
            Error(StatusCode::SERVICE_UNAVAILABLE, format!(
                "S3 ListObjectsV2 for v2/due/ failed ({err}, {code}, HTTP {status}). Cleanup will retry in 60 seconds."
            ))
        })?;
        for key in page.contents().iter().filter_map(|x| x.key()) {
            let Some((stamp, id)) = key.trim_start_matches("v2/due/").split_once('/') else {
                continue;
            };
            let Ok(due) = stamp.parse::<u64>() else {
                continue;
            };
            if due > now() {
                return Ok(());
            }
            if valid_id(id) {
                if let Err(e) = expire(s, id, key).await {
                    eprintln!("Cleanup retry for {id}: {}", e.1)
                }
            }
        }
    }
    Ok(())
}
async fn expire(s: &AppState, id: &str, key: &str) -> Result<()> {
    let _guard = lock(id).await;
    let read = store::json::<Control>(s, &control(id)).await;
    match read {
        Ok((mut c, tag)) if c.deadline <= now() => {
            let mut current_tag = tag;
            if c.phase != Phase::Expired {
                c.phase = Phase::Expired;
                c.owner.clear();
                c.manifest_hash.clear();
                if !store::put_json(s, &control(id), &c, Some(&current_tag)).await? {
                    return Ok(());
                }
                (c, current_tag) = store::json(s, &control(id)).await?;
            }
            store::purge(s, &data(id, "")).await?;
            if c.cleaned_at.is_none() {
                c.cleaned_at = Some(now());
                if !store::put_json(s, &control(id), &c, Some(&current_tag)).await? {
                    return Ok(());
                }
            }
        }
        Ok(_) => (),
        Err(e) if e.0 == StatusCode::NOT_FOUND => (),
        Err(e) => return Err(e),
    }
    store::purge(s, key).await
}
