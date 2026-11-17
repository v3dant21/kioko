mod store;
mod viewer;
mod worker;

use crate::AppState;
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    sync::LazyLock,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, MutexGuard, Semaphore};
use uuid::Uuid;
pub use worker::start;

const CHUNK: usize = 4 * 1024 * 1024;
const MAX_PHOTO: usize = 32 * 1024 * 1024;
const PAGE: usize = 24;
static LOCKS: LazyLock<Vec<Mutex<()>>> =
    LazyLock::new(|| (0..128).map(|_| Mutex::new(())).collect());
static ASSEMBLY: Semaphore = Semaphore::const_new(2);
async fn lock(id: &str) -> MutexGuard<'static, ()> {
    let byte = Sha256::digest(id.as_bytes())[0] as usize;
    LOCKS[byte % LOCKS.len()].lock().await
}
type Result<T> = std::result::Result<T, Error>;
type Api = Result<Json<Value>>;
#[derive(Debug)]
pub struct Error(StatusCode, String);
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}
fn fault(e: impl std::fmt::Display) -> Error {
    eprintln!("Album storage: {e}");
    Error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Storage operation failed".into(),
    )
}
fn bad(s: &str) -> Error {
    Error(StatusCode::BAD_REQUEST, s.into())
}
fn conflict(s: &str) -> Error {
    Error(StatusCode::CONFLICT, s.into())
}
fn gone() -> Error {
    Error(StatusCode::GONE, "Album expired".into())
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn hash(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}
fn valid_id(id: &str) -> bool {
    Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id)
}
fn control(id: &str) -> String {
    format!("v2/control/{id}")
}
fn data(id: &str, path: &str) -> String {
    format!("v2/data/{id}/{path}")
}
fn marker(id: &str, at: u64) -> String {
    format!("v2/due/{at:012}/{id}")
}
fn token(h: &HeaderMap) -> Result<String> {
    let raw = h
        .get("authorization")
        .and_then(|x| x.to_str().ok())
        .and_then(|x| x.strip_prefix("Bearer "))
        .filter(|x| x.len() == 64 && x.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| Error(StatusCode::UNAUTHORIZED, "Owner credential required".into()))?;
    Ok(hash(raw.as_bytes()))
}
#[derive(Serialize, Deserialize, Clone)]
struct Photo {
    id: String,
    name: String,
    tags: Vec<String>,
    size: usize,
    digest: String,
}
#[derive(Serialize, Deserialize)]
struct Draft {
    id: String,
    title: String,
    days: u64,
    photos: Vec<Photo>,
}
#[derive(Serialize, Deserialize, PartialEq)]
enum Phase {
    Draft,
    Published,
    Expired,
}
#[derive(Serialize, Deserialize)]
struct Control {
    phase: Phase,
    owner: String,
    manifest_hash: String,
    deadline: u64,
    count: usize,
    #[serde(default)]
    cleaned_at: Option<u64>,
}
fn validate(d: &Draft) -> Result<()> {
    if !valid_id(&d.id)
        || d.title.trim().is_empty()
        || d.title.len() > 240
        || !matches!(d.days, 1 | 3 | 7)
        || d.photos.is_empty()
        || d.photos.len() > 2000
    {
        return Err(bad("Invalid album name, lifetime or photo count (1–2000)"));
    }
    let mut ids = HashSet::new();
    for p in &d.photos {
        if !valid_id(&p.id)
            || !ids.insert(&p.id)
            || p.size == 0
            || p.size > MAX_PHOTO
            || p.name.is_empty()
            || p.name.len() > 512
            || p.tags.len() > 32
            || p.tags.iter().any(|t| t.len() > 128)
            || p.digest.len() != 64
            || !p.digest.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(bad("Invalid photo metadata. Maximum photo size is 32 MiB."));
        }
    }
    if d.photos.iter().map(|p| p.size as u64).sum::<u64>() > 1024 * 1024 * 1024 {
        return Err(bad("Album exceeds 1 GiB"));
    }
    Ok(())
}
async fn state(s: &AppState, id: &str) -> Result<(Control, String)> {
    if !valid_id(id) {
        return Err(bad("Invalid album ID"));
    }
    let (c, t): (Control, _) = store::json(s, &control(id)).await?;
    if c.deadline <= now() || c.phase == Phase::Expired {
        return Err(gone());
    }
    Ok((c, t))
}
async fn owned(s: &AppState, id: &str, h: &HeaderMap) -> Result<(Control, String)> {
    let owner = token(h)?;
    let (c, t) = state(s, id).await?;
    if c.owner != owner {
        return Err(Error(
            StatusCode::FORBIDDEN,
            "Invalid owner credential".into(),
        ));
    }
    Ok((c, t))
}
async fn draft(s: &AppState, id: &str) -> Result<Draft> {
    Ok(store::json(s, &data(id, "manifest")).await?.0)
}
async fn slot(s: &AppState, id: &str, pid: &str, h: &HeaderMap) -> Result<Photo> {
    let (c, _) = owned(s, id, h).await?;
    if c.phase != Phase::Draft {
        return Err(conflict("Album already published"));
    }
    if !valid_id(pid) {
        return Err(bad("Invalid photo ID"));
    }
    Ok(store::json(s, &data(id, &format!("slots/{pid}"))).await?.0)
}
async fn create(State(s): State<AppState>, h: HeaderMap, Json(d): Json<Draft>) -> Api {
    validate(&d)?;
    let owner = token(&h)?;
    let bytes = serde_json::to_vec(&d).map_err(fault)?;
    let _guard = lock(&d.id).await;
    let c = Control {
        owner,
        manifest_hash: hash(&bytes),
        deadline: now() + 86400,
        count: d.photos.len(),
        cleaned_at: None,
        phase: Phase::Draft,
    };
    let existing = if exists(&s, &control(&d.id)).await? {
        true
    } else {
        store::put(&s, &marker(&d.id, c.deadline), vec![], None).await?;
        !store::put_json(&s, &control(&d.id), &c, None).await?
    };
    if existing {
        let (old, _) = owned(&s, &d.id, &h).await?;
        if old.manifest_hash != c.manifest_hash {
            return Err(conflict("Different draft already exists"));
        }
        if old.phase == Phase::Published {
            return Ok(Json(json!({"published":true,"expires_at":old.deadline})));
        }
    }
    // Manifest is written last so its existence signals that all slots were persisted.
    if !exists(&s, &data(&d.id, "manifest")).await? {
        for p in &d.photos {
            store::put_json(&s, &data(&d.id, &format!("slots/{}", p.id)), p, None).await?;
        }
        store::put(&s, &data(&d.id, "manifest"), bytes, None).await?;
    }
    Ok(Json(json!({"chunk_size":CHUNK,"published":false})))
}
async fn exists(s: &AppState, key: &str) -> Result<bool> {
    let result = s
        .s3_client
        .head_object()
        .bucket(&s.bucket)
        .key(key)
        .send()
        .await;
    match result {
        Ok(_) => Ok(true),
        Err(e) if e.raw_response().is_some_and(|r| r.status().as_u16() == 404) => Ok(false),
        Err(e) => Err(fault(e)),
    }
}
fn part_len(size: usize, n: usize) -> Option<usize> {
    size.checked_sub(n.checked_mul(CHUNK)?)
        .filter(|x| *x > 0)
        .map(|x| x.min(CHUNK))
}
async fn receipts(
    State(s): State<AppState>,
    Path((id, pid)): Path<(String, String)>,
    h: HeaderMap,
) -> Api {
    let p = slot(&s, &id, &pid, &h).await?;
    let keys = store::list(&s, &data(&id, &format!("parts/{pid}/"))).await?;
    let accepted: Vec<usize> = keys
        .iter()
        .filter_map(|k| k.rsplit('/').next()?.parse().ok())
        .collect();
    Ok(Json(
        json!({"accepted":accepted,"ready":exists(&s,&data(&id,&format!("ready/{pid}"))).await?,"size":p.size,"chunk_size":CHUNK}),
    ))
}
async fn part(
    State(s): State<AppState>,
    Path((id, pid, n)): Path<(String, String, usize)>,
    h: HeaderMap,
    body: Bytes,
) -> Api {
    let _guard = lock(&id).await;
    let p = slot(&s, &id, &pid, &h).await?;
    if part_len(p.size, n) != Some(body.len()) {
        return Err(bad("Incorrect part length"));
    }
    let digest = hash(&body);
    if h.get("x-part-sha256").and_then(|v| v.to_str().ok()) != Some(&digest) {
        return Err(bad("Part checksum mismatch"));
    }
    let key = data(&id, &format!("parts/{pid}/{n}"));
    if !store::put(&s, &key, body.to_vec(), None).await?
        && hash(&store::read(&s, &key).await?.0) != digest
    {
        return Err(conflict("Part already contains different bytes"));
    }
    Ok(Json(json!({"part":n,"bytes":body.len()})))
}
fn media(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if b.starts_with(&[255, 216, 255]) {
        Some("image/jpeg")
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}
async fn complete(
    State(s): State<AppState>,
    Path((id, pid)): Path<(String, String)>,
    h: HeaderMap,
) -> Api {
    let _guard = lock(&id).await;
    let p = slot(&s, &id, &pid, &h).await?;
    let ready = data(&id, &format!("ready/{pid}"));
    if !exists(&s, &ready).await? {
        let _permit = ASSEMBLY.acquire().await.map_err(fault)?;
        let mut bytes = Vec::with_capacity(p.size);
        for n in 0..p.size.div_ceil(CHUNK) {
            let chunk = store::read(&s, &data(&id, &format!("parts/{pid}/{n}")))
                .await
                .map_err(|e| {
                    if e.0 == StatusCode::NOT_FOUND {
                        conflict("Upload missing parts")
                    } else {
                        e
                    }
                })?
                .0;
            bytes.extend_from_slice(&chunk);
        }
        let mime = media(&bytes).ok_or_else(|| bad("Supported photos: JPEG, PNG, GIF, WebP"))?;
        if bytes.len() != p.size || hash(&bytes) != p.digest {
            return Err(bad("Photo checksum mismatch"));
        }
        state(&s, &id).await?;
        store::put(&s, &data(&id, &format!("images/{pid}")), bytes, None).await?;
        store::put_json(&s, &ready, &mime, None).await?;
    }
    store::purge(&s, &data(&id, &format!("parts/{pid}/"))).await?;
    Ok(Json(json!({"ready":true})))
}
async fn status(State(s): State<AppState>, Path(id): Path<String>, h: HeaderMap) -> Api {
    let (c, _) = owned(&s, &id, &h).await?;
    let ready = store::list(&s, &data(&id, "ready/")).await?.len();
    Ok(Json(
        json!({"count":c.count,"ready":ready,"published":c.phase==Phase::Published,"deadline":c.deadline}),
    ))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", post(create))
        .route("/:id/uploads/:pid", get(receipts))
        .route("/:id/status", get(status))
        .route("/:id/photos/:pid/uploads", post(receipts))
        .route("/:id/uploads/:pid/parts/:n", put(part))
        .route("/:id/uploads/:pid/complete", post(complete))
        .route("/:id/publish", post(viewer::publish))
        .route("/:id", get(viewer::metadata))
        .route("/:id/photos", get(viewer::photos))
        .route("/:id/photos/:pid", get(viewer::image))
        .route("/:id/download", get(viewer::download))
        .layer(DefaultBodyLimit::max(CHUNK))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn album() -> Draft {
        Draft {
            id: Uuid::new_v4().to_string(),
            title: "Trip".into(),
            days: 3,
            photos: vec![Photo {
                id: Uuid::new_v4().to_string(),
                name: "trip.png".into(),
                size: CHUNK + 1,
                digest: hash(b"fixture"),
                tags: vec![],
            }],
        }
    }

    #[test]
    fn part_geometry_handles_tail_and_out_of_range() {
        assert_eq!(part_len(1, 0), Some(1));
        assert_eq!(part_len(CHUNK, 0), Some(CHUNK));
        assert_eq!(part_len(CHUNK, 1), None);
        assert_eq!(part_len(CHUNK + 1, 1), Some(1));
        assert_eq!(part_len(CHUNK + 1, usize::MAX), None);
    }

    #[test]
    fn manifest_rejects_duplicate_slots_and_invalid_retention() {
        let mut draft = album();
        assert!(validate(&draft).is_ok());
        draft.photos.push(draft.photos[0].clone());
        assert!(validate(&draft).is_err());
        draft.photos.pop();
        draft.days = 0;
        assert!(validate(&draft).is_err());
    }

    #[test]
    fn upload_ids_cannot_select_another_storage_prefix() {
        assert!(!valid_id("../control"));
        assert!(!valid_id("a/b"));
        assert!(valid_id(&Uuid::new_v4().to_string()));
        assert!(crate::s3::validate_gallery_id("../v2/data").is_err());
    }

    #[test]
    fn reject_unsupported_bytes_before_publication() {
        assert_eq!(media(b"RIFF1234WEBP"), Some("image/webp"));
        assert_eq!(media(b"RIFF1234WAVE"), None);
        assert_eq!(media(b"<svg/>"), None);
    }
}
