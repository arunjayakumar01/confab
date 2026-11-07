//! Prism relay: one WebSocket per client for typing, presence, read receipts and
//! draft sync; a Pub/Sub push endpoint that fans Google Workspace events out to
//! subscribed sessions; and, against real Google, the Workspace Events
//! subscription manager. With `PRISM_PUBSUB_SUBSCRIPTION` set it also pulls
//! events from Pub/Sub itself, for hosts that cannot receive a push. Message
//! bodies never pass through here except as part of the live event fan-out.

pub mod config;
pub mod db;
pub mod google;
pub mod hub;
pub mod names;
pub mod pubsub_pull;
pub mod push;
pub mod state;
pub mod subs;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::{Json, Router};
use tower_http::cors::CorsLayer;

pub use config::Config;
pub use state::AppState;

/// The HTTP surface: `/ws`, `/events/push`, `/health`.
pub fn app(state: Arc<AppState>) -> Router {
    let open = Router::new()
        .route("/events/push", post(push::handle))
        .route("/health", get(health))
        .layer(CorsLayer::permissive());
    Router::new()
        .route("/ws", get(ws::handle))
        .merge(open)
        .with_state(state)
}

async fn health(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "ok": true,
        "sessions": state.hub.session_count(),
        "users_online": state.hub.online_users().len(),
        "mode": if state.config.real_mode { "real" } else { "twin" },
        "pull": state.config.pubsub_subscription.as_deref().unwrap_or("none"),
    }))
}

/// Background work: subscription renewal (real mode), the Pub/Sub puller
/// (when a subscription is configured) and cache sweeps.
pub fn spawn_background(state: Arc<AppState>) {
    subs::spawn_renewer(state.clone());
    pubsub_pull::spawn(state.clone());
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            state.sweep_caches();
        }
    });
}

/// Bind and serve until the process ends.
pub async fn run(config: Config) -> anyhow::Result<()> {
    let state = AppState::new(config)?;
    let addr = SocketAddr::from(([0, 0, 0, 0], state.config.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(
        %addr,
        mode = if state.config.real_mode { "real" } else { "twin" },
        subscriptions = state.config.subscriptions_enabled(),
        db = %state.config.db_path,
        "relay listening"
    );
    spawn_background(state.clone());
    axum::serve(listener, app(state)).await?;
    Ok(())
}
