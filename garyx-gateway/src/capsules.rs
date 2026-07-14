use std::fmt;
use std::path::{Path as StdPath, PathBuf};
use std::sync::{Arc, LazyLock};

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use garyx_models::local_paths::default_capsules_dir;
use regex::Regex;
use serde_json::json;
use tokio::fs;
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::garyx_db::{CapsuleRecord, GaryxDbError};
use crate::server::AppState;

pub(crate) const CAPSULE_MAX_HTML_BYTES: usize = 5 * 1024 * 1024;
pub(crate) const CAPSULE_MCP_BODY_LIMIT_BYTES: usize = CAPSULE_MAX_HTML_BYTES + 1024 * 1024;
pub(crate) const CAPSULE_CSP: &str = "default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' https: blob: data:; style-src 'unsafe-inline' https:; img-src https: data: blob:; font-src https: data:; connect-src https:; media-src https: data: blob:; frame-src https:; object-src 'none'; base-uri 'none'; form-action 'none'";

static RESOURCE_ATTR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)\b(src|href|poster|data|action|srcset)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#,
    )
    .expect("resource attribute regex compiles")
});
static CSS_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)url\(\s*(?:"([^"]*)"|'([^']*)'|([^\)\s]+))\s*\)"#)
        .expect("css url regex compiles")
});
static HEAD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<head\b[^>]*>").expect("head regex compiles"));

#[derive(Debug)]
pub(crate) struct CapsuleError {
    status: StatusCode,
    message: String,
}

impl CapsuleError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl IntoResponse for CapsuleError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
}

impl fmt::Display for CapsuleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl From<GaryxDbError> for CapsuleError {
    fn from(error: GaryxDbError) -> Self {
        match error {
            GaryxDbError::BadRequest(message) => CapsuleError::bad_request(message),
            GaryxDbError::ThreadArchived(thread_id) => {
                CapsuleError::bad_request(format!("thread is archived: {thread_id}"))
            }
            GaryxDbError::LockPoisoned
            | GaryxDbError::Join(_)
            | GaryxDbError::Configuration(_)
            | GaryxDbError::Io(_)
            | GaryxDbError::Sqlite(_) => CapsuleError::internal(error.to_string()),
        }
    }
}

pub(crate) fn capsules_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_CAPSULES_DIR
        .lock()
        .expect("test capsules dir lock poisoned")
        .clone()
    {
        return path;
    }

    default_capsules_dir()
}

#[cfg(test)]
static TEST_CAPSULES_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
#[cfg(test)]
static TEST_CAPSULES_DIR_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
fn set_test_capsules_dir(path: PathBuf) -> impl Drop {
    struct Guard {
        _serial: std::sync::MutexGuard<'static, ()>,
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            *TEST_CAPSULES_DIR
                .lock()
                .expect("test capsules dir lock poisoned") = None;
        }
    }

    let serial = TEST_CAPSULES_DIR_SERIAL
        .lock()
        .expect("test capsules dir serial lock poisoned");
    *TEST_CAPSULES_DIR
        .lock()
        .expect("test capsules dir lock poisoned") = Some(path);
    Guard { _serial: serial }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use std::path::PathBuf;

    pub(crate) fn set_test_capsules_dir_for_test(path: PathBuf) -> impl Drop {
        super::set_test_capsules_dir(path)
    }
}

pub(crate) fn parse_capsule_uuid(id: &str) -> Result<String, CapsuleError> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(CapsuleError::bad_request("capsule id must not be empty"));
    }
    Uuid::parse_str(trimmed)
        .map(|uuid| uuid.to_string())
        .map_err(|_| CapsuleError::bad_request("capsule id must be a UUID"))
}

pub(crate) fn capsule_file_path(id: &str) -> Result<PathBuf, CapsuleError> {
    let id = parse_capsule_uuid(id)?;
    Ok(capsules_dir().join(format!("{id}.html")))
}

pub(crate) fn validate_capsule_html_bytes(bytes: &[u8]) -> Result<String, CapsuleError> {
    if bytes.is_empty() {
        return Err(CapsuleError::bad_request("capsule HTML must not be empty"));
    }
    if bytes.len() > CAPSULE_MAX_HTML_BYTES {
        return Err(CapsuleError::bad_request(format!(
            "capsule HTML exceeds {} bytes",
            CAPSULE_MAX_HTML_BYTES
        )));
    }
    let html = std::str::from_utf8(bytes)
        .map_err(|_| CapsuleError::bad_request("capsule HTML must be valid UTF-8"))?;
    for capture in RESOURCE_ATTR_RE.captures_iter(html) {
        let attr = capture
            .get(1)
            .map(|value| value.as_str().to_ascii_lowercase())
            .unwrap_or_default();
        let value = capture
            .get(2)
            .or_else(|| capture.get(3))
            .or_else(|| capture.get(4))
            .map(|value| value.as_str())
            .unwrap_or_default();
        if attr == "srcset" {
            validate_srcset(value)?;
        } else {
            validate_resource_reference(value)?;
        }
    }
    for capture in CSS_URL_RE.captures_iter(html) {
        let value = capture
            .get(1)
            .or_else(|| capture.get(2))
            .or_else(|| capture.get(3))
            .map(|value| value.as_str())
            .unwrap_or_default();
        validate_resource_reference(value)?;
    }

    Ok(html.to_owned())
}

fn validate_srcset(value: &str) -> Result<(), CapsuleError> {
    let trimmed = value.trim();
    if trimmed.to_ascii_lowercase().starts_with("data:") {
        return Ok(());
    }
    for candidate in trimmed.split(',') {
        let Some(url) = candidate.split_whitespace().next() else {
            continue;
        };
        validate_resource_reference(url)?;
    }
    Ok(())
}

fn validate_resource_reference(value: &str) -> Result<(), CapsuleError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
        return Ok(());
    }
    let lower = trimmed.to_ascii_lowercase();
    for scheme in ["data:", "blob:", "https:"] {
        if lower.starts_with(scheme) {
            return Ok(());
        }
    }
    Err(CapsuleError::bad_request(format!(
        "capsule HTML must be self-contained; relative or local resource reference rejected: {trimmed}"
    )))
}

pub(crate) async fn read_capsule_html_path(path: &str) -> Result<Vec<u8>, CapsuleError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(CapsuleError::bad_request("html_path must not be empty"));
    }
    let raw = PathBuf::from(trimmed);
    if !raw.is_absolute() {
        return Err(CapsuleError::bad_request("html_path must be absolute"));
    }
    let canonical = fs::canonicalize(&raw)
        .await
        .map_err(|error| CapsuleError::bad_request(format!("failed to read html_path: {error}")))?;
    let metadata = fs::metadata(&canonical).await.map_err(|error| {
        CapsuleError::bad_request(format!("failed to inspect html_path: {error}"))
    })?;
    if !metadata.is_file() {
        return Err(CapsuleError::bad_request(
            "html_path must point to a regular file",
        ));
    }
    if metadata.len() > (CAPSULE_MAX_HTML_BYTES as u64) {
        return Err(CapsuleError::bad_request(format!(
            "capsule HTML exceeds {} bytes",
            CAPSULE_MAX_HTML_BYTES
        )));
    }

    let file = fs::File::open(&canonical)
        .await
        .map_err(|error| CapsuleError::bad_request(format!("failed to open html_path: {error}")))?;
    let mut bytes = Vec::new();
    file.take((CAPSULE_MAX_HTML_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| CapsuleError::bad_request(format!("failed to read html_path: {error}")))?;
    Ok(bytes)
}

pub(crate) async fn atomic_write_capsule_file(
    id: &str,
    html: &[u8],
) -> Result<PathBuf, CapsuleError> {
    let id = parse_capsule_uuid(id)?;
    let dir = capsules_dir();
    fs::create_dir_all(&dir).await.map_err(|error| {
        CapsuleError::internal(format!("failed to create capsules directory: {error}"))
    })?;
    let final_path = dir.join(format!("{id}.html"));
    let tmp_path = dir.join(format!(".{id}.{}.tmp", Uuid::new_v4()));
    write_then_rename(&tmp_path, &final_path, html).await?;
    Ok(final_path)
}

async fn write_then_rename(
    tmp_path: &StdPath,
    final_path: &StdPath,
    bytes: &[u8],
) -> Result<(), CapsuleError> {
    if let Err(error) = fs::write(tmp_path, bytes).await {
        let _ = fs::remove_file(tmp_path).await;
        return Err(CapsuleError::internal(format!(
            "failed to write capsule HTML: {error}"
        )));
    }
    if let Err(error) = fs::rename(tmp_path, final_path).await {
        let _ = fs::remove_file(tmp_path).await;
        return Err(CapsuleError::internal(format!(
            "failed to publish capsule HTML: {error}"
        )));
    }
    Ok(())
}

pub(crate) fn inject_csp_meta(html: &str) -> String {
    let meta = format!(r#"<meta http-equiv="Content-Security-Policy" content="{CAPSULE_CSP}">"#);
    if let Some(mat) = HEAD_RE.find(html) {
        let mut output = String::with_capacity(html.len() + meta.len());
        output.push_str(&html[..mat.end()]);
        output.push_str(&meta);
        output.push_str(&html[mat.end()..]);
        output
    } else {
        format!("{meta}{html}")
    }
}

fn capsule_response(record: CapsuleRecord) -> serde_json::Value {
    json!(record)
}

fn capsule_list_response(records: Vec<CapsuleRecord>) -> serde_json::Value {
    json!({ "capsules": records })
}

pub async fn list_capsules(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state
        .ops
        .garyx_db
        .run_blocking(|db| db.list_capsules())
        .await
    {
        Ok(records) => (StatusCode::OK, Json(capsule_list_response(records))).into_response(),
        Err(error) => CapsuleError::from(error).into_response(),
    }
}

pub async fn get_capsule(
    AxumPath(id): AxumPath<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let id = match parse_capsule_uuid(&id) {
        Ok(id) => id,
        Err(error) => return error.into_response(),
    };
    match state
        .ops
        .garyx_db
        .run_blocking(move |db| db.get_capsule(&id))
        .await
    {
        Ok(Some(record)) => (
            StatusCode::OK,
            Json(json!({ "capsule": capsule_response(record) })),
        )
            .into_response(),
        Ok(None) => CapsuleError::not_found("capsule not found").into_response(),
        Err(error) => CapsuleError::from(error).into_response(),
    }
}

pub async fn serve_capsule(
    AxumPath(id): AxumPath<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let id = match parse_capsule_uuid(&id) {
        Ok(id) => id,
        Err(error) => return error.into_response(),
    };
    let lookup_id = id.clone();
    match state
        .ops
        .garyx_db
        .run_blocking(move |db| db.get_capsule(&lookup_id))
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return CapsuleError::not_found("capsule not found").into_response(),
        Err(error) => return CapsuleError::from(error).into_response(),
    }
    let path = match capsule_file_path(&id) {
        Ok(path) => path,
        Err(error) => return error.into_response(),
    };
    let bytes = match fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CapsuleError::not_found("capsule HTML file not found").into_response();
        }
        Err(error) => {
            return CapsuleError::internal(format!("failed to read capsule HTML: {error}"))
                .into_response();
        }
    };
    let html = match validate_capsule_html_bytes(&bytes) {
        Ok(html) => html,
        Err(error) => return error.into_response(),
    };
    let body = inject_csp_meta(&html);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CAPSULE_CSP),
    );
    (StatusCode::OK, headers, body).into_response()
}

async fn set_capsule_favorite(id: String, state: Arc<AppState>, favorited: bool) -> Response {
    let id = match parse_capsule_uuid(&id) {
        Ok(id) => id,
        Err(error) => return error.into_response(),
    };
    match state
        .ops
        .garyx_db
        .run_blocking(move |db| db.set_capsule_favorite(&id, favorited))
        .await
    {
        Ok(Some(record)) => {
            let favorited = record.favorited_at.is_some();
            (
                StatusCode::OK,
                Json(json!({
                    "favorited": favorited,
                    "capsule": capsule_response(record),
                })),
            )
                .into_response()
        }
        Ok(None) => CapsuleError::not_found("capsule not found").into_response(),
        Err(error) => CapsuleError::from(error).into_response(),
    }
}

pub async fn favorite_capsule(
    AxumPath(id): AxumPath<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    set_capsule_favorite(id, state, true).await
}

pub async fn unfavorite_capsule(
    AxumPath(id): AxumPath<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    set_capsule_favorite(id, state, false).await
}

pub async fn delete_capsule(
    AxumPath(id): AxumPath<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let id = match parse_capsule_uuid(&id) {
        Ok(id) => id,
        Err(error) => return error.into_response(),
    };
    let path = match capsule_file_path(&id) {
        Ok(path) => path,
        Err(error) => return error.into_response(),
    };
    match state
        .ops
        .garyx_db
        .run_blocking(move |db| db.delete_capsule(&id))
        .await
    {
        Ok(true) => {
            let _ = fs::remove_file(path).await;
            (StatusCode::OK, Json(json!({ "deleted": true }))).into_response()
        }
        Ok(false) => CapsuleError::not_found("capsule not found").into_response(),
        Err(error) => CapsuleError::from(error).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use garyx_models::config::GaryxConfig;
    use tower::ServiceExt;

    use crate::garyx_db::CapsuleCreateDraft;
    use crate::route_graph::build_router;
    use crate::server::AppStateBuilder;

    fn valid_html() -> &'static str {
        r##"<!doctype html><html><head><title>Demo</title></head><body><img src="data:image/png;base64,AA=="><a href="#local">jump</a></body></html>"##
    }

    fn create_record(state: &Arc<AppState>, id: &str) {
        state
            .ops
            .garyx_db
            .create_capsule(CapsuleCreateDraft {
                id: id.to_owned(),
                title: "Demo".to_owned(),
                description: "Synthetic demo".to_owned(),
                thread_id: Some("thread::capsule-http".to_owned()),
                run_id: Some("run::capsule-http".to_owned()),
                agent_id: Some("agent::capsule".to_owned()),
                provider_type: Some("codex_app_server".to_owned()),
                html_sha256: "c".repeat(64),
                byte_size: valid_html().len() as i64,
            })
            .expect("create capsule record");
    }

    #[test]
    fn validate_capsule_html_rejects_oversize_and_local_references() {
        validate_capsule_html_bytes(valid_html().as_bytes()).expect("valid html");
        validate_capsule_html_bytes(
            br#"<html><body><script src="https://cdn.example.test/app.js"></script><img src="blob:https://example.test/id"><a href="//example.test/path">cdn</a></body></html>"#,
        )
        .expect("allowed remote and inline schemes");
        validate_capsule_html_bytes(br#"<p>Do not use file:// URLs in Capsules.</p>"#)
            .expect("file scheme prose is not a resource reference");
        assert!(validate_capsule_html_bytes(&vec![b'a'; CAPSULE_MAX_HTML_BYTES + 1]).is_err());
        assert!(validate_capsule_html_bytes(br#"<img src="file:///Users/test/a.png">"#).is_err());
        assert!(validate_capsule_html_bytes(br#"<img src="asset.png">"#).is_err());
        assert!(
            validate_capsule_html_bytes(br#"<style>body{background:url(./asset.png)}</style>"#)
                .is_err()
        );
    }

    #[test]
    fn inject_csp_meta_inserts_into_head_or_prepends() {
        let with_head = inject_csp_meta("<html><head><title>x</title></head><body></body></html>");
        assert!(with_head.contains("<head><meta http-equiv=\"Content-Security-Policy\""));
        let without_head = inject_csp_meta("<main>demo</main>");
        assert!(without_head.starts_with("<meta http-equiv=\"Content-Security-Policy\""));
    }

    #[test]
    fn capsule_file_path_rejects_invalid_id_before_join() {
        assert!(capsule_file_path("../escape").is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn capsule_http_routes_are_protected_and_serve_headers() {
        let temp = tempfile::tempdir().expect("temp dir");
        let _guard = set_test_capsules_dir(temp.path().join("capsules"));
        let state = AppStateBuilder::new(crate::test_support::with_gateway_auth(
            GaryxConfig::default(),
        ))
        .build();
        let id = Uuid::new_v4().to_string();
        create_record(&state, &id);
        atomic_write_capsule_file(&id, valid_html().as_bytes())
            .await
            .expect("write capsule file");
        let router = build_router(state.clone());

        let unauth = axum::http::Request::builder()
            .uri("/api/capsules")
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(unauth).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let list = crate::test_support::authed_request()
            .uri("/api/capsules")
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(list).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["capsules"][0]["id"], id);
        assert!(payload["capsules"][0]["favorited_at"].is_null());

        let get = crate::test_support::authed_request()
            .uri(format!("/api/capsules/{id}"))
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(get).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["capsule"]["id"], id);

        let serve = crate::test_support::authed_request()
            .uri(format!("/api/capsules/{id}/serve"))
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(serve).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            response.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_SECURITY_POLICY)
                .unwrap(),
            CAPSULE_CSP
        );
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("http-equiv=\"Content-Security-Policy\""));
        assert!(html.contains("<title>Demo</title>"));

        let unauth_favorite = axum::http::Request::builder()
            .method("PUT")
            .uri(format!("/api/capsules/{id}/favorite"))
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(unauth_favorite).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let favorite = crate::test_support::authed_request()
            .method("PUT")
            .uri(format!("/api/capsules/{id}/favorite"))
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(favorite).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["favorited"], true);
        assert_eq!(payload["capsule"]["id"], id);
        assert!(payload["capsule"]["favorited_at"].is_string());

        let unfavorite = crate::test_support::authed_request()
            .method("DELETE")
            .uri(format!("/api/capsules/{id}/favorite"))
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(unfavorite).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["favorited"], false);
        assert!(payload["capsule"]["favorited_at"].is_null());

        let unknown_id = Uuid::new_v4();
        for method in ["PUT", "DELETE"] {
            let missing = crate::test_support::authed_request()
                .method(method)
                .uri(format!("/api/capsules/{unknown_id}/favorite"))
                .body(Body::empty())
                .unwrap();
            let response = router.clone().oneshot(missing).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(payload["error"], "capsule not found");
        }

        fs::remove_file(capsule_file_path(&id).unwrap())
            .await
            .unwrap();
        let missing_file = crate::test_support::authed_request()
            .uri(format!("/api/capsules/{id}/serve"))
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(missing_file).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let bad_id = crate::test_support::authed_request()
            .uri("/api/capsules/not-a-uuid/serve")
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(bad_id).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        atomic_write_capsule_file(&id, valid_html().as_bytes())
            .await
            .expect("rewrite capsule file");
        let delete = crate::test_support::authed_request()
            .method("DELETE")
            .uri(format!("/api/capsules/{id}"))
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(delete).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(state.ops.garyx_db.get_capsule(&id).unwrap().is_none());
        assert!(!capsule_file_path(&id).unwrap().exists());

        let delete_again = crate::test_support::authed_request()
            .method("DELETE")
            .uri(format!("/api/capsules/{id}"))
            .body(Body::empty())
            .unwrap();
        let response = router.oneshot(delete_again).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
