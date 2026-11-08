//! Twin-only helpers: seeded user list, token minting for scripts, avatars, health.

use crate::error::ApiError;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use confab_proto::chat::TokenResponse;
use std::collections::HashMap;
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/twin/users", get(list_users))
        .route("/twin/token", post(mint_token))
        .route("/twin/avatars/{file}", get(avatar))
        .route(
            "/healthz",
            get(|| async { Json(serde_json::json!({"ok": true, "service": "google-twin"})) }),
        )
}

async fn list_users(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let st = state.read();
    let users: Vec<_> = st.users.values().cloned().collect();
    Json(serde_json::json!({ "domain": crate::seed::DOMAIN, "users": users }))
}

async fn mint_token(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<TokenResponse>, ApiError> {
    let user = q
        .get("user")
        .map(|u| {
            if u.starts_with("users/") {
                u.clone()
            } else {
                format!("users/{u}")
            }
        })
        .ok_or_else(|| ApiError::invalid_argument("Missing query parameter: user=users/{id}"))?;
    let mut st = state.write();
    if !st.users.contains_key(&user) {
        return Err(ApiError::not_found(format!("Unknown user {user}")));
    }
    Ok(Json(st.mint_token(&user, None)))
}

async fn avatar(Path(file): Path<String>) -> Response {
    let id = file.strip_suffix(".svg").unwrap_or(&file);
    match crate::seed::avatar_svg(id) {
        Some(svg) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "image/svg+xml"),
                (header::CACHE_CONTROL, "public, max-age=86400"),
            ],
            svg,
        )
            .into_response(),
        None => ApiError::not_found(format!("No avatar for {id}")).into_response(),
    }
}
