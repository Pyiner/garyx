use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, Request, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Json, http};
use serde_json::json;

use crate::server::AppState;

pub const MCP_AUTH_SEGMENT: &str = "auth";
const UNAUTHORIZED_MESSAGE: &str = "valid gateway authorization token required";
const TOKEN_NOT_CONFIGURED_MESSAGE: &str = "gateway authorization token is not configured; run `garyx gateway token` on the gateway host, then paste the token into the client";

pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max_len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    for index in 0..max_len {
        let lhs = *left.get(index).unwrap_or(&0);
        let rhs = *right.get(index).unwrap_or(&0);
        diff |= (lhs ^ rhs) as usize;
    }
    diff == 0
}

fn configured_tokens(state: &AppState) -> Vec<String> {
    let mut tokens = Vec::new();
    let config_token = state.config_snapshot().gateway.auth_token.trim().to_owned();
    if !config_token.is_empty() {
        tokens.push(config_token);
    }
    tokens
}

pub fn gateway_auth_enabled(state: &AppState) -> bool {
    !configured_tokens(state).is_empty()
}

fn token_from_authorization(headers: &HeaderMap) -> Option<String> {
    headers
        .get(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.strip_prefix("Bearer ").unwrap_or(value).trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            headers
                .get("x-garyx-token")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            headers
                .get("x-mcp-token")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn token_from_query(uri: &Uri) -> Option<String> {
    uri.query().and_then(|query| {
        query.split('&').find_map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            if name != "token" {
                return None;
            }
            let decoded = urlencoding::decode(value)
                .map(|item| item.into_owned())
                .unwrap_or_else(|_| value.to_owned());
            let trimmed = decoded.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        })
    })
}

pub fn token_from_mcp_path(path: &str) -> Option<String> {
    let mut segments = path
        .strip_prefix("/mcp/")
        .unwrap_or("")
        .split('/')
        .filter(|segment| !segment.trim().is_empty());
    let first = segments.next()?;
    if first != MCP_AUTH_SEGMENT {
        return None;
    }
    let token = segments.next()?.trim();
    if token.is_empty() {
        return None;
    }
    Some(
        urlencoding::decode(token)
            .map(|value| value.into_owned())
            .unwrap_or_else(|_| token.to_owned()),
    )
}

pub fn extract_request_token(headers: &HeaderMap, uri: &Uri) -> Option<String> {
    token_from_authorization(headers)
        .or_else(|| token_from_query(uri))
        .or_else(|| token_from_mcp_path(uri.path()))
}

fn authorization_failure_message(
    state: &AppState,
    headers: &HeaderMap,
    uri: &Uri,
) -> Option<&'static str> {
    let configured = configured_tokens(state);
    if configured.is_empty() {
        return Some(TOKEN_NOT_CONFIGURED_MESSAGE);
    }
    let Some(provided) = extract_request_token(headers, uri) else {
        return Some(UNAUTHORIZED_MESSAGE);
    };
    if configured
        .iter()
        .any(|token| constant_time_eq(token.as_bytes(), provided.as_bytes()))
    {
        None
    } else {
        Some(UNAUTHORIZED_MESSAGE)
    }
}

pub fn request_authorized(state: &AppState, headers: &HeaderMap, uri: &Uri) -> bool {
    authorization_failure_message(state, headers, uri).is_none()
}

fn request_from_loopback(req: &Request<Body>) -> bool {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip().is_loopback())
        .unwrap_or(false)
}

pub fn unauthorized_gateway_response(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "ok": false,
            "error": "unauthorized",
            "message": message,
        })),
    )
        .into_response()
}

pub async fn enforce_gateway_auth(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if request_from_loopback(&req) {
        return next.run(req).await;
    }

    if let Some(message) = authorization_failure_message(&state, req.headers(), req.uri()) {
        unauthorized_gateway_response(message)
    } else {
        next.run(req).await
    }
}

#[cfg(test)]
mod tests;
