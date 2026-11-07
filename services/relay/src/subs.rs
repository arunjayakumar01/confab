//! Workspace Events subscription manager. Real mode only: one subscription per
//! user over all their Chat spaces, created on first hello, renewed before it
//! expires with a token from a currently connected session. A no-op in twin mode.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::hub::Session;
use crate::state::AppState;

const RENEW_WINDOW: Duration = Duration::from_secs(30 * 60);
const RENEW_EVERY: Duration = Duration::from_secs(5 * 60);

fn expires_within(expire_time: &str, window: Duration) -> bool {
    match DateTime::parse_from_rfc3339(expire_time) {
        Ok(t) => {
            t.with_timezone(&Utc)
                <= Utc::now() + chrono::Duration::from_std(window).unwrap_or_default()
        }
        Err(_) => true,
    }
}

/// Called on every hello; creates the user's subscription if none is live.
pub fn on_hello(state: Arc<AppState>, session: Arc<Session>) {
    if !state.config.subscriptions_enabled() {
        return;
    }
    tokio::spawn(async move { ensure(&state, &session).await });
}

pub async fn ensure(state: &AppState, session: &Session) {
    let Some(topic) = state.config.pubsub_topic.as_deref() else {
        return;
    };
    match state.db.subscription(&session.user) {
        Ok(Some(row)) if !expires_within(&row.expire_time, Duration::ZERO) => return,
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(error = %e, "subscription read failed");
            return;
        }
    }
    match state
        .google
        .create_subscription(&session.token, topic)
        .await
    {
        Ok(info) => {
            tracing::info!(user = %session.user, name = %info.name, expires = %info.expire_time, "subscription created");
            if let Err(e) = state.db.upsert_subscription(
                &session.user,
                &info.name,
                &info.expire_time,
                &AppState::now_rfc3339(),
            ) {
                tracing::warn!(error = %e, "subscription write failed");
            }
        }
        Err(e) => tracing::warn!(user = %session.user, error = %e, "subscription create failed"),
    }
}

pub fn spawn_renewer(state: Arc<AppState>) {
    if !state.config.subscriptions_enabled() {
        tracing::info!("subscription manager disabled (twin mode or no PRISM_PUBSUB_TOPIC)");
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(RENEW_EVERY);
        loop {
            tick.tick().await;
            renew_due(&state).await;
        }
    });
}

/// Renew every stored subscription that expires within the window, using a
/// token of a connected session of its user; offline users are skipped.
pub async fn renew_due(state: &AppState) {
    let rows = match state.db.all_subscriptions() {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "subscription list failed");
            return;
        }
    };
    for row in rows {
        if !expires_within(&row.expire_time, RENEW_WINDOW) {
            continue;
        }
        let Some(token) = state.hub.any_token_for_user(&row.user) else {
            tracing::info!(user = %row.user, name = %row.name, "renewal due but user offline; skipping");
            continue;
        };
        match state.google.renew_subscription(&token, &row.name).await {
            Ok(info) => {
                tracing::info!(user = %row.user, name = %info.name, expires = %info.expire_time, "subscription renewed");
                if let Err(e) = state.db.upsert_subscription(
                    &row.user,
                    &info.name,
                    &info.expire_time,
                    &AppState::now_rfc3339(),
                ) {
                    tracing::warn!(error = %e, "subscription write failed");
                }
            }
            Err(e) if e.to_string() == "gone" => {
                tracing::warn!(user = %row.user, name = %row.name, "subscription gone; forgetting it");
                let _ = state.db.delete_subscription(&row.user);
            }
            Err(e) => {
                tracing::warn!(user = %row.user, name = %row.name, error = %e, "renewal failed")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_window() {
        let soon = (Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
        let later = (Utc::now() + chrono::Duration::hours(3)).to_rfc3339();
        assert!(expires_within(&soon, RENEW_WINDOW));
        assert!(!expires_within(&later, RENEW_WINDOW));
        assert!(!expires_within(&soon, Duration::ZERO));
        assert!(expires_within("garbage", Duration::ZERO));
    }
}
