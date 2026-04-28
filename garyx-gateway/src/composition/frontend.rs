use axum::Router;
use axum::handler::HandlerWithoutStateExt;
use std::path::PathBuf;
use tower_http::services::ServeDir;

use crate::routes;

fn frontend_static_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(from_env) = std::env::var("GARYX_WEB_FRONTEND_DIR") {
        let trimmed = from_env.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }

    candidates.push(PathBuf::from("desktop/garyx-desktop/out/web"));
    candidates.push(PathBuf::from("../desktop/garyx-desktop/out/web"));
    candidates.push(PathBuf::from("../../desktop/garyx-desktop/out/web"));

    candidates
}

fn web_frontend_static_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(from_env) = std::env::var("GARYX_WEB_FRONTEND_DIR") {
        let trimmed = from_env.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }

    candidates.push(PathBuf::from("desktop/garyx-desktop/out/web"));
    candidates.push(PathBuf::from("../desktop/garyx-desktop/out/web"));
    candidates.push(PathBuf::from("../../desktop/garyx-desktop/out/web"));

    candidates
}

pub(crate) fn mount_frontend_routes(router: Router) -> Router {
    let mut router = router;

    let web_static_candidates = web_frontend_static_candidates();
    let web_static_dir = web_static_candidates
        .iter()
        .find(|candidate| candidate.is_dir())
        .cloned();

    if let Some(web_static_dir) = web_static_dir {
        tracing::info!("Serving web shell from {}", web_static_dir.display());
        router = router.nest_service(
            "/web",
            ServeDir::new(web_static_dir).append_index_html_on_directories(true),
        );
    } else {
        let searched = web_static_candidates
            .iter()
            .map(|candidate| candidate.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        tracing::info!(
            "Web shell static dir not found, /web disabled. searched: {}",
            searched
        );
    }

    let static_candidates = frontend_static_candidates();
    let static_dir = static_candidates
        .iter()
        .find(|candidate| candidate.is_dir())
        .cloned();

    if let Some(static_dir) = static_dir {
        tracing::info!("Serving root frontend from {}", static_dir.display());
        router.fallback_service(
            ServeDir::new(static_dir)
                .append_index_html_on_directories(true)
                .not_found_service(routes::fallback.into_service()),
        )
    } else {
        let searched = static_candidates
            .iter()
            .map(|candidate| candidate.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        tracing::warn!(
            "Frontend static dir not found, serving API-only. searched: {}",
            searched
        );
        router.fallback(routes::fallback)
    }
}
