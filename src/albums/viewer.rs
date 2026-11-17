use super::*;
use axum::{body::Body, extract::Query};
use std::io::{Seek, Write};
use tokio_util::io::ReaderStream;

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    id: String,
    name: String,
    tags: Vec<String>,
}
pub(super) async fn publish(
    State(s): State<AppState>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> Api {
    let _guard = lock(&id).await;
    let (mut c, etag) = owned(&s, &id, &h).await?;
    if c.phase == Phase::Draft {
        let d = draft(&s, &id).await?;
        let ready: HashSet<_> = store::list(&s, &data(&id, "ready/"))
            .await?
            .into_iter()
            .collect();
        if d.photos
            .iter()
            .any(|p| !ready.contains(&data(&id, &format!("ready/{}", p.id))))
        {
            return Err(conflict("Finish all photo uploads before publishing"));
        }
        let entries: Vec<_> = d
            .photos
            .into_iter()
            .map(|p| Entry {
                id: p.id,
                name: p.name,
                tags: p.tags,
            })
            .collect();
        for (n, page) in entries.chunks(PAGE).enumerate() {
            store::put_json(&s, &data(&id, &format!("pages/{n}")), &page, None).await?;
        }
        let terms: Vec<_> = entries
            .iter()
            .map(|e| {
                e.tags
                    .iter()
                    .chain(std::iter::once(&e.name))
                    .map(|v| v.to_lowercase())
                    .collect::<Vec<_>>()
            })
            .collect();
        store::put_json(&s, &data(&id, "search"), &terms, None).await?;
        store::put_json(&s, &data(&id, "title"), &d.title, None).await?;
        if c.deadline <= now() {
            return Err(gone());
        }
        c.deadline = now() + d.days * 86400;
        store::put(&s, &marker(&id, c.deadline), vec![], None).await?;
        c.phase = Phase::Published;
        if !store::put_json(&s, &control(&id), &c, Some(&etag)).await? {
            return Err(conflict("Publication changed. Retry to recover the link."));
        }
    }
    Ok(Json(
        json!({"url":format!("/g/v2-{id}"),"expires_at":c.deadline}),
    ))
}
async fn published(s: &AppState, id: &str) -> Result<Control> {
    let (c, _) = state(s, id).await?;
    if c.phase != Phase::Published {
        return Err(Error(StatusCode::NOT_FOUND, "Gallery not published".into()));
    }
    Ok(c)
}
pub(super) async fn metadata(State(s): State<AppState>, Path(id): Path<String>) -> Api {
    let c = published(&s, &id).await?;
    let (title, _): (String, _) = store::json(&s, &data(&id, "title")).await?;
    Ok(Json(
        json!({"title":title,"count":c.count,"expires_at":c.deadline}),
    ))
}
#[derive(Deserialize, Default)]
pub(super) struct Params {
    q: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
}
fn cursor(id: &str, q: &str, n: usize, rev: &str) -> String {
    format!("{n}.{}", hash(format!("{id}/{rev}/{q}/{n}").as_bytes()))
}
pub(super) async fn photos(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<Params>,
) -> Api {
    let c = published(&s, &id).await?;
    let q = params.q.unwrap_or_default().trim().to_lowercase();
    let limit = params.limit.unwrap_or(PAGE);
    if !(1..=100).contains(&limit) || q.len() > 512 {
        return Err(bad("Invalid page parameters"));
    }
    let start = match params.cursor {
        None => 0,
        Some(raw) => {
            let n = raw
                .split('.')
                .next()
                .and_then(|v| v.parse::<usize>().ok())
                .ok_or_else(|| bad("Invalid cursor"))?;
            if raw != cursor(&id, &q, n, &c.manifest_hash) || n >= c.count {
                return Err(bad("Invalid cursor"));
            }
            n
        }
    };
    let positions: Vec<usize> = if q.is_empty() {
        (start..c.count).take(limit + 1).collect()
    } else {
        let (terms, _): (Vec<Vec<String>>, _) = store::json(&s, &data(&id, "search")).await?;
        terms
            .iter()
            .enumerate()
            .skip(start)
            .filter(|(_, row)| row.iter().any(|v| v.contains(&q)))
            .map(|(i, _)| i)
            .take(limit + 1)
            .collect()
    };
    let next = positions
        .get(limit)
        .map(|&n| cursor(&id, &q, n, &c.manifest_hash));
    let mut items = Vec::new();
    let mut loaded = None;
    let mut page = Vec::<Entry>::new();
    for &pos in positions.iter().take(limit) {
        let index = pos / PAGE;
        if loaded != Some(index) {
            page = store::json(&s, &data(&id, &format!("pages/{index}")))
                .await?
                .0;
            loaded = Some(index);
        }
        items.push(
            page.get(pos % PAGE)
                .ok_or_else(|| fault("Invalid page index"))?
                .clone(),
        );
    }
    Ok(Json(json!({"items":items,"next_cursor":next})))
}
pub(super) async fn image(
    State(s): State<AppState>,
    Path((id, pid)): Path<(String, String)>,
) -> Result<Response> {
    published(&s, &id).await?;
    if !valid_id(&pid) {
        return Err(bad("Invalid photo ID"));
    }
    let (mime, _): (String, _) = store::json(&s, &data(&id, &format!("ready/{pid}"))).await?;
    let obj = s
        .s3_client
        .get_object()
        .bucket(&s.bucket)
        .key(data(&id, &format!("images/{pid}")))
        .send()
        .await
        .map_err(fault)?;
    Ok((
        [
            ("content-type", mime),
            ("x-content-type-options", "nosniff".to_string()),
        ],
        Body::from_stream(ReaderStream::new(obj.body.into_async_read())),
    )
        .into_response())
}

static ZIP_SLOTS: Semaphore = Semaphore::const_new(2);
pub(super) async fn download(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response> {
    let permit = ZIP_SLOTS.try_acquire().map_err(|_| {
        Error(
            StatusCode::TOO_MANY_REQUESTS,
            "ZIP service busy. Retry shortly.".into(),
        )
    })?;
    published(&s, &id).await?;
    let path = std::env::temp_dir().join(format!("album-{}.zip", Uuid::new_v4()));
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .mode(0o600)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(fault)?;
    std::fs::remove_file(path).map_err(fault)?;
    let mut zip = zip::ZipWriter::new(file);
    let d = draft(&s, &id).await?;
    for (i, p) in d.photos.iter().enumerate() {
        let bytes = store::read(&s, &data(&id, &format!("images/{}", p.id)))
            .await?
            .0;
        let name: String = p
            .name
            .chars()
            .map(|c| {
                if c.is_control() || "/\\:".contains(c) {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        zip = tokio::task::spawn_blocking(move || -> Result<_> {
            zip.start_file(
                format!("{:04}-{}", i + 1, name),
                zip::write::FileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .map_err(fault)?;
            zip.write_all(&bytes).map_err(fault)?;
            Ok(zip)
        })
        .await
        .map_err(fault)??;
    }
    let mut file = zip.finish().map_err(fault)?;
    file.rewind().map_err(fault)?;
    published(&s, &id).await?;
    let stream = ReaderStream::new(tokio::fs::File::from_std(file));
    // Hold the slot until the client drops the response body and the spool file closes.
    use futures_util::StreamExt;
    let stream = stream.map(move |value| {
        let _ = &permit;
        value
    });
    Ok((
        [
            ("content-type", "application/zip"),
            ("content-disposition", "attachment; filename=album.zip"),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}
