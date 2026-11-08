//! Delivery of optimistic sends. A row enters the outbox before the command
//! returns; this task posts it with `messageId=client-{id}` so a retry after a
//! crash or a 409 can never double-post, then tells the web view via `shell`.

use crate::cache::OutboxState;
use crate::shell::{OutboxStatus, ShellEvent};
use crate::AppState;
use std::sync::Arc;
use tauri::AppHandle;

pub fn spawn_deliver(app: AppHandle, state: Arc<AppState>, client_id: String) {
    tauri::async_runtime::spawn(deliver(app, state, client_id));
}

/// Re-drive every pending/sending row (after launch or sign-in).
pub fn resume_unsent(app: AppHandle, state: Arc<AppState>) {
    match state.cache.outbox_unsent() {
        Ok(rows) => {
            if !rows.is_empty() {
                tracing::info!("outbox: resuming {} unsent message(s)", rows.len());
            }
            for row in rows {
                spawn_deliver(app.clone(), state.clone(), row.client_id);
            }
        }
        Err(e) => tracing::warn!("{e}"),
    }
}

async fn deliver(app: AppHandle, state: Arc<AppState>, client_id: String) {
    let row = match state.cache.outbox_get(&client_id) {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::warn!("outbox: no row for {client_id}");
            return;
        }
        Err(e) => {
            tracing::warn!("{e}");
            return;
        }
    };
    if row.state == OutboxState::Sent {
        return;
    }
    if let Err(e) = state
        .cache
        .outbox_transition(&client_id, OutboxState::Sending, None)
    {
        tracing::warn!("{e}");
        return;
    }
    let result = state
        .google
        .create_message(
            &row.space,
            &client_id,
            &row.text,
            row.thread.as_deref(),
            &row.attachments,
        )
        .await;
    let result = match result {
        Err(e) if e.status == 409 => {
            // Already accepted (a retry after a crash or a dropped response): fetch it by its client id.
            state
                .google
                .get_message(&format!("{}/messages/client-{client_id}", row.space))
                .await
        }
        other => other,
    };
    let event = match result {
        Ok(message) => {
            if let Err(e) = state.cache.upsert_message(&message) {
                tracing::warn!("{e}");
            }
            if let Err(e) = state
                .cache
                .outbox_transition(&client_id, OutboxState::Sent, None)
            {
                tracing::warn!("{e}");
            }
            ShellEvent::Outbox {
                client_id: client_id.clone(),
                status: OutboxStatus::Sent,
                message: Some(proxied(message, &state)),
                error: None,
            }
        }
        Err(e) => {
            tracing::warn!("outbox: {client_id} failed: {e}");
            if let Err(e2) =
                state
                    .cache
                    .outbox_transition(&client_id, OutboxState::Failed, Some(&e.message))
            {
                tracing::warn!("{e2}");
            }
            ShellEvent::Outbox {
                client_id: client_id.clone(),
                status: OutboxStatus::Failed,
                message: None,
                error: Some(e.message),
            }
        }
    };
    crate::accounts::emit_shell(&app, &state, &event);
}

fn proxied(
    mut message: confab_proto::chat::Message,
    state: &AppState,
) -> confab_proto::chat::Message {
    crate::shell::proxy_media(std::slice::from_mut(&mut message), &state.profile.endpoints);
    message
}
