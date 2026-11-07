//! Google twin: a local stand-in for Google's OAuth endpoints, the Google Chat
//! API v1 subset Prism uses, the Google Tasks default list, the Google Meet
//! spaces a huddle needs, the Calendar v3 events and calendar list, the Drive
//! app-data folder, the Chat `users.availability` presence, and Workspace Events
//! pushed in the Pub/Sub shape.
//! Everything lives in memory and is reseeded deterministically on every start.

pub mod auth;
pub mod availability;
pub mod calendar;
pub mod chat;
pub mod config;
pub mod drive;
pub mod error;
pub mod events;
pub mod meet;
pub mod seed;
pub mod state;
pub mod tasks;
pub mod twin;
pub mod util;

use axum::{extract::DefaultBodyLimit, Router};
use std::sync::Arc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

pub use config::Config;
pub use state::AppState;

/// The whole twin as one router: OAuth, Chat API, Tasks API, Meet API, Calendar API,
/// Drive app data, twin helpers.
pub fn app(state: Arc<AppState>) -> Router {
    Router::new()
        .merge(auth::routes())
        .merge(chat::routes())
        .merge(tasks::routes())
        .merge(meet::routes())
        .merge(calendar::routes())
        .merge(drive::routes())
        .merge(availability::routes())
        .merge(twin::routes())
        .fallback(fallback)
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        // Permissive on purpose: the web view calls the twin straight from a browser.
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn fallback(uri: axum::http::Uri) -> error::ApiError {
    error::ApiError::not_found(format!("Requested entity was not found: {}", uri.path()))
}
