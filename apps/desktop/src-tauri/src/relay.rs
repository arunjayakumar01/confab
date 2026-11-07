//! Relay WebSocket client. One task per signed-in session: hello → welcome →
//! subscribe, then forward every `ServerMsg` to the web view as the `relay`
//! event after folding `event` frames into the cache. Reconnects with backoff
//! 1 s → 30 s; re-hellos with a fresh token after a refresh.

use crate::accounts::{emit_relay, emit_shell};
use crate::cache::Cache;
use crate::shell::{RelayState, ShellEvent};
use crate::AppState;
use futures::{SinkExt, StreamExt};
use prism_proto::relay::{event_types, ChatEvent, ClientMsg, ServerMsg};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::AppHandle;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message as WsMessage;

pub const CLIENT_VERSION: &str = concat!("prism-desktop/", env!("CARGO_PKG_VERSION"));
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
const PING_EVERY: Duration = Duration::from_secs(30);
const WELCOME_TIMEOUT: Duration = Duration::from_secs(15);

struct Running {
    tx: mpsc::UnboundedSender<ClientMsg>,
    stop: watch::Sender<bool>,
}

/// Starts and stops the relay task; hands outbound frames to it.
#[derive(Default)]
pub struct RelayManager {
    inner: Mutex<Option<Running>>,
    /// The last state emitted, so an account switch can replay it to the web view.
    last: Mutex<RelayState>,
}

impl RelayManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Running>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Idempotent: a second call while running is a no-op.
    pub fn start(&self, app: AppHandle, state: Arc<AppState>) {
        let mut g = self.lock();
        if g.is_some() {
            return;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let (stop, stop_rx) = watch::channel(false);
        tauri::async_runtime::spawn(run(app, state, rx, stop_rx));
        *g = Some(Running { tx, stop });
    }

    /// Closes the socket (the task emits `relay_state: closed` and exits).
    pub fn stop(&self) {
        if let Some(r) = self.lock().take() {
            let _ = r.stop.send(true);
        }
    }

    pub fn is_running(&self) -> bool {
        self.lock().is_some()
    }

    /// The socket state as last reported (`closed` before the first connect).
    pub fn last_state(&self) -> RelayState {
        *self.last.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn set_state(&self, state: RelayState) {
        *self.last.lock().unwrap_or_else(|p| p.into_inner()) = state;
    }

    /// Queue a frame; it goes out after the next `welcome` if the socket is down.
    pub fn send(&self, msg: ClientMsg) -> Result<(), String> {
        match self.lock().as_ref() {
            Some(r) => {
                r.tx.send(msg)
                    .map_err(|_| "relay task has stopped".to_string())
            }
            None => Err("relay not running (not signed in)".to_string()),
        }
    }
}

/// Remembers the state on the account's manager and tells the web view when the
/// account is the active one.
fn emit_state(app: &AppHandle, account: &AppState, state: RelayState) {
    account.relay.set_state(state);
    emit_shell(app, account, &ShellEvent::RelayState { state });
}

/// `spaces/{s}/messages/{m}` of a reaction name.
pub fn message_of_reaction(reaction_name: &str) -> Option<String> {
    let parts: Vec<&str> = reaction_name.split('/').collect();
    if parts.len() >= 6 && parts[0] == "spaces" && parts[2] == "messages" && parts[4] == "reactions"
    {
        Some(parts[..4].join("/"))
    } else {
        None
    }
}

/// Fold a Google event into the cache. Returns a message name that needs a
/// refetch (reactions change counts the event does not carry).
pub fn apply_event(cache: &Cache, ev: &ChatEvent) -> Option<String> {
    let result = match ev.event_type.as_str() {
        event_types::MESSAGE_CREATED | event_types::MESSAGE_UPDATED => match &ev.message {
            Some(m) => cache.upsert_message(m),
            None => Ok(()),
        },
        event_types::MESSAGE_DELETED => match &ev.message {
            Some(m) => cache.delete_message(&m.name),
            None => Ok(()),
        },
        event_types::REACTION_CREATED | event_types::REACTION_DELETED => {
            return ev
                .reaction
                .as_ref()
                .and_then(|r| r.name.as_deref())
                .and_then(message_of_reaction);
        }
        event_types::MEMBERSHIP_CREATED => match &ev.membership {
            Some(m) => cache.upsert_members(&ev.space, std::slice::from_ref(m)),
            None => Ok(()),
        },
        event_types::MEMBERSHIP_DELETED => match &ev.membership {
            Some(m) => cache.delete_member(&m.name),
            None => Ok(()),
        },
        event_types::SPACE_UPDATED => match &ev.space_resource {
            Some(s) => cache.upsert_spaces(std::slice::from_ref(s)),
            None => Ok(()),
        },
        _ => Ok(()),
    };
    if let Err(e) = result {
        tracing::warn!("cache update for {} failed: {e}", ev.event_type);
    }
    None
}

enum Exit {
    /// Reconnect after backoff.
    Retry,
    /// Reconnect right away (token changed).
    Rehello,
    /// Signed out.
    Stop,
}

async fn run(
    app: AppHandle,
    state: Arc<AppState>,
    mut rx: mpsc::UnboundedReceiver<ClientMsg>,
    mut stop_rx: watch::Receiver<bool>,
) {
    let mut token_rx = state.auth.subscribe_token_changes();
    let mut backoff = BACKOFF_MIN;
    loop {
        if *stop_rx.borrow() {
            break;
        }
        emit_state(&app, &state, RelayState::Connecting);
        let exit = session(
            &app,
            &state,
            &mut rx,
            &mut stop_rx,
            &mut token_rx,
            &mut backoff,
        )
        .await;
        emit_state(&app, &state, RelayState::Closed);
        match exit {
            Exit::Stop => break,
            Exit::Rehello => continue,
            Exit::Retry => {
                tracing::info!("relay: reconnecting in {backoff:?}");
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = stop_rx.changed() => break,
                    _ = token_rx.changed() => {}
                }
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
        }
    }
    tracing::info!("relay task stopped");
}

async fn session(
    app: &AppHandle,
    state: &Arc<AppState>,
    rx: &mut mpsc::UnboundedReceiver<ClientMsg>,
    stop_rx: &mut watch::Receiver<bool>,
    token_rx: &mut watch::Receiver<u64>,
    backoff: &mut Duration,
) -> Exit {
    let token = match state.auth.access_token().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("relay: no token ({e})");
            return if state.auth.is_signed_in() {
                Exit::Retry
            } else {
                Exit::Stop
            };
        }
    };
    // Anything the refresh above produced is the token we are about to use.
    token_rx.borrow_and_update();

    let url = state.profile.endpoints.relay_ws.clone();
    let (ws, _) = match tokio_tungstenite::connect_async(url.as_str()).await {
        Ok(ok) => ok,
        Err(e) => {
            tracing::warn!("relay: connect {url} failed: {e}");
            return Exit::Retry;
        }
    };
    let (mut write, mut read) = ws.split();

    let hello = ClientMsg::Hello {
        token: token.clone(),
        device_id: state.profile.device_id.clone(),
        client_version: CLIENT_VERSION.to_string(),
    };
    if send_frame(&mut write, &hello).await.is_err() {
        return Exit::Retry;
    }

    // First frame must be welcome (or error + close).
    let first = tokio::time::timeout(WELCOME_TIMEOUT, read.next()).await;
    match first {
        Ok(Some(Ok(WsMessage::Text(t)))) => match serde_json::from_str::<ServerMsg>(&t) {
            Ok(ServerMsg::Welcome { ref user, .. }) => {
                tracing::info!("relay: welcome as {}", user.name);
                let msg = serde_json::from_str::<ServerMsg>(&t).expect("parsed once already");
                emit_relay(app, state, &msg);
            }
            Ok(ServerMsg::Error { code, message }) => {
                tracing::warn!("relay: rejected hello: {code} {message}");
                emit_relay(
                    app,
                    state,
                    &ServerMsg::Error {
                        code: code.clone(),
                        message,
                    },
                );
                // note: any rejection is treated as a stale token; one refresh then backoff.
                if let Err(e) = state.auth.force_refresh(&token).await {
                    tracing::warn!("relay: refresh after rejection failed: {e}");
                }
                return Exit::Retry;
            }
            Ok(other) => {
                tracing::warn!("relay: expected welcome, got {other:?}");
                return Exit::Retry;
            }
            Err(e) => {
                tracing::warn!("relay: unreadable first frame: {e}");
                return Exit::Retry;
            }
        },
        Ok(Some(Ok(WsMessage::Close(_)))) | Ok(None) => return Exit::Retry,
        Ok(Some(Ok(_))) => return Exit::Retry,
        Ok(Some(Err(e))) => {
            tracing::warn!("relay: {e}");
            return Exit::Retry;
        }
        Err(_) => {
            tracing::warn!("relay: no welcome within {WELCOME_TIMEOUT:?}");
            return Exit::Retry;
        }
    }
    *backoff = BACKOFF_MIN;
    emit_state(app, state, RelayState::Open);

    let spaces = state.cache.space_names().unwrap_or_default();
    if send_frame(&mut write, &ClientMsg::Subscribe { spaces })
        .await
        .is_err()
    {
        return Exit::Retry;
    }

    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await; // the first tick fires immediately; skip it
    loop {
        tokio::select! {
            frame = read.next() => match frame {
                Some(Ok(WsMessage::Text(t))) => handle_frame(app, state, &t),
                Some(Ok(WsMessage::Close(_))) | None => return Exit::Retry,
                Some(Ok(_)) => {}
                Some(Err(e)) => { tracing::warn!("relay: read: {e}"); return Exit::Retry; }
            },
            out = rx.recv() => match out {
                Some(msg) => { if send_frame(&mut write, &msg).await.is_err() { return Exit::Retry; } }
                None => { let _ = write.send(WsMessage::Close(None)).await; return Exit::Stop; }
            },
            _ = ping.tick() => {
                let t = chrono::Utc::now().timestamp_millis() as u64;
                if send_frame(&mut write, &ClientMsg::Ping { t }).await.is_err() { return Exit::Retry; }
            },
            changed = token_rx.changed() => {
                if changed.is_ok() {
                    tracing::info!("relay: token changed, re-hello");
                    let _ = write.send(WsMessage::Close(None)).await;
                    return Exit::Rehello;
                }
            },
            _ = stop_rx.changed() => {
                let _ = write.send(WsMessage::Close(None)).await;
                return Exit::Stop;
            },
        }
    }
}

async fn send_frame<S>(write: &mut S, msg: &ClientMsg) -> Result<(), ()>
where
    S: SinkExt<WsMessage> + Unpin,
    S::Error: std::fmt::Display,
{
    let json = serde_json::to_string(msg).map_err(|_| ())?;
    write.send(WsMessage::Text(json)).await.map_err(|e| {
        tracing::warn!("relay: send: {e}");
    })
}

fn handle_frame(app: &AppHandle, state: &Arc<AppState>, text: &str) {
    let mut msg: ServerMsg = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(
                "relay: unreadable frame ({e}): {}",
                text.chars().take(120).collect::<String>()
            );
            return;
        }
    };
    if let ServerMsg::Event { event } = &msg {
        if let Some(name) = apply_event(&state.cache, event) {
            // note: the refetch runs off the socket loop so a reaction burst cannot stall pings.
            let state = state.clone();
            tauri::async_runtime::spawn(async move {
                match state.google.get_message(&name).await {
                    Ok(m) => {
                        if let Err(e) = state.cache.upsert_message(&m) {
                            tracing::warn!("{e}");
                        }
                    }
                    Err(e) if e.is_not_found() => {
                        let _ = state.cache.delete_message(&name);
                    }
                    Err(e) => tracing::warn!("refetch {name} after reaction: {e}"),
                }
            });
        }
    }
    if let ServerMsg::Event { event } = &mut msg {
        if let Some(m) = event.message.as_mut() {
            crate::shell::proxy_media(std::slice::from_mut(m), &state.profile.endpoints);
        }
    }
    emit_relay(app, state, &msg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_proto::chat::{Message, Reaction};

    #[test]
    fn frames_round_trip_with_prism_proto() {
        let hello = ClientMsg::Hello {
            token: "t".into(),
            device_id: "d".into(),
            client_version: CLIENT_VERSION.into(),
        };
        let v = serde_json::to_value(&hello).unwrap();
        assert_eq!(v["type"], "hello");
        assert_eq!(v["device_id"], "d");
        assert!(v["client_version"]
            .as_str()
            .unwrap()
            .starts_with("prism-desktop/"));

        let welcome: ServerMsg = serde_json::from_str(
            r#"{"type":"welcome","user":{"name":"users/1001","displayName":"Priya"},"session_id":"s","online":["users/1002"]}"#,
        )
        .unwrap();
        assert!(
            matches!(welcome, ServerMsg::Welcome { ref online, .. } if online == &["users/1002".to_string()])
        );

        let event: ServerMsg = serde_json::from_str(
            r#"{"type":"event","event":{"event_type":"google.workspace.chat.message.v1.created","space":"spaces/A","event_time":"2026-11-06T10:00:00Z","message":{"name":"spaces/A/messages/B","text":"hi","createTime":"2026-11-06T10:00:00Z"}}}"#,
        )
        .unwrap();
        let back = serde_json::to_value(&event).unwrap();
        assert_eq!(
            back["event"]["message"]["createTime"],
            "2026-11-06T10:00:00Z"
        );
        assert_eq!(back["event"]["event_type"], event_types::MESSAGE_CREATED);
    }

    #[test]
    fn events_fold_into_cache() {
        let cache = Cache::in_memory().unwrap();
        let created = ChatEvent {
            event_type: event_types::MESSAGE_CREATED.into(),
            space: "spaces/A".into(),
            event_time: "2026-11-06T10:00:00Z".into(),
            message: Some(Message {
                name: "spaces/A/messages/B".into(),
                text: Some("budget".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(apply_event(&cache, &created), None);
        assert_eq!(cache.search("budget").unwrap().len(), 1);

        let reaction = ChatEvent {
            event_type: event_types::REACTION_CREATED.into(),
            reaction: Some(Reaction {
                name: Some("spaces/A/messages/B/reactions/R".into()),
                ..Default::default()
            }),
            ..created.clone()
        };
        assert_eq!(
            apply_event(&cache, &reaction).as_deref(),
            Some("spaces/A/messages/B")
        );

        let deleted = ChatEvent {
            event_type: event_types::MESSAGE_DELETED.into(),
            ..created
        };
        assert_eq!(apply_event(&cache, &deleted), None);
        assert!(cache.search("budget").unwrap().is_empty());
        assert_eq!(message_of_reaction("spaces/A/messages/B"), None);
    }
}
