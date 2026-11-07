//! `GET /ws`: one socket per client. `hello` within the timeout, identity from
//! userinfo, then the relay protocol until the socket closes or misses a ping.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use prism_proto::chat;
use prism_proto::relay::{ClientMsg, CustomStatus, PresenceState, ServerMsg, TYPING_TTL_MS};
use tokio::sync::mpsc;

use crate::hub::{PresenceRecord, Session};
use crate::names::valid_space;
use crate::state::AppState;
use crate::subs;

const PING_INTERVAL: Duration = Duration::from_secs(30);

pub async fn handle(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> Response {
    ws.on_upgrade(move |socket| run(socket, state))
}

fn frame(msg: &ServerMsg) -> Message {
    Message::Text(serde_json::to_string(msg).unwrap_or_default().into())
}

fn error(code: &str, message: impl Into<String>) -> ServerMsg {
    ServerMsg::Error {
        code: code.into(),
        message: message.into(),
    }
}

async fn reject(sink: &mut SplitSink<WebSocket, Message>, code: &str, message: &str) {
    let _ = sink.send(frame(&error(code, message))).await;
    let _ = sink
        .send(Message::Close(Some(CloseFrame {
            code: 1008,
            reason: code.into(),
        })))
        .await;
}

async fn first_client_msg(
    stream: &mut SplitStream<WebSocket>,
    timeout: Duration,
) -> Result<ClientMsg, &'static str> {
    match tokio::time::timeout(timeout, stream.next()).await {
        Err(_) => Err("hello_timeout"),
        Ok(None) | Ok(Some(Err(_))) => Err("closed"),
        Ok(Some(Ok(Message::Text(t)))) => {
            serde_json::from_str::<ClientMsg>(t.as_str()).map_err(|_| "protocol")
        }
        Ok(Some(Ok(_))) => Err("protocol"),
    }
}

async fn run(socket: WebSocket, state: Arc<AppState>) {
    let (mut sink, mut stream) = socket.split();

    let (token, device_id, client_version) =
        match first_client_msg(&mut stream, state.config.hello_timeout).await {
            Ok(ClientMsg::Hello {
                token,
                device_id,
                client_version,
            }) => (token, device_id, client_version),
            Ok(_) => return reject(&mut sink, "protocol", "first frame must be hello").await,
            Err("closed") => return,
            Err(code) => return reject(&mut sink, code, "hello expected within the timeout").await,
        };

    let info = match state.google.userinfo(&token).await {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(error = %e, "hello rejected");
            return reject(&mut sink, "unauthorized", "token rejected by userinfo").await;
        }
    };
    let user = format!("users/{}", info.sub);
    let session_id = uuid::Uuid::new_v4().to_string();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let session = Arc::new(Session::new(
        session_id.clone(),
        user.clone(),
        info.sub.clone(),
        device_id,
        token,
        tx,
    ));
    state.hub.add(session.clone());
    tracing::info!(session = %session_id, user, client_version, "session open");

    let custom = state.db.custom_status(&user).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "custom status read failed");
        None
    });
    set_presence(&state, &session, PresenceState::Active, custom);
    subs::on_hello(state.clone(), session.clone());

    let welcome = ServerMsg::Welcome {
        user: chat::User {
            name: user.clone(),
            display_name: info.name,
            email: info.email,
            avatar_url: info.picture,
            user_type: Some("HUMAN".into()),
            ..Default::default()
        },
        session_id,
        online: state.hub.online_users(),
    };
    if sink.send(frame(&welcome)).await.is_err() {
        close(&state, &session);
        return;
    }

    let mut ping =
        tokio::time::interval_at(tokio::time::Instant::now() + PING_INTERVAL, PING_INTERVAL);
    let mut awaiting_pong = false;
    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Text(t))) => {
                        awaiting_pong = false;
                        match serde_json::from_str::<ClientMsg>(t.as_str()) {
                            Ok(m) => handle_msg(&state, &session, m).await,
                            Err(e) => session.send(error("bad_frame", e.to_string())),
                        }
                    }
                    Some(Ok(_)) => awaiting_pong = false,
                }
            }
            outgoing = rx.recv() => {
                match outgoing {
                    Some(m) => {
                        if sink.send(frame(&m)).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            _ = ping.tick() => {
                if awaiting_pong {
                    tracing::info!(session = %session.id, "no pong in {}s; dropping", PING_INTERVAL.as_secs() * 2);
                    break;
                }
                if sink.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
                awaiting_pong = true;
            }
        }
    }
    let _ = sink.send(Message::Close(None)).await;
    close(&state, &session);
}

fn set_presence(
    state: &AppState,
    session: &Session,
    ps: PresenceState,
    custom: Option<CustomStatus>,
) {
    let record = PresenceRecord {
        state: ps,
        custom_status: custom,
        at: AppState::now_rfc3339(),
    };
    state.hub.set_presence(&session.user, record.clone());
    let spaces = state.hub.user_spaces(&session.user);
    state
        .hub
        .broadcast_sharing(&spaces, &record.to_msg(&session.user), Some(&session.id));
}

/// Tells every subscriber of `space` who is in its huddle now.
fn announce_huddle(state: &AppState, space: &str, users: Vec<String>) {
    let msg = ServerMsg::Huddle {
        space: space.to_string(),
        users,
        at: AppState::now_rfc3339(),
    };
    state.hub.broadcast_space(space, &msg, None);
}

fn close(state: &AppState, session: &Arc<Session>) {
    state.hub.remove(&session.id);
    tracing::info!(session = %session.id, user = %session.user, "session closed");
    // A dropped socket leaves every huddle the user was in, unless another session of theirs is still up.
    if state.hub.sessions_of_user(&session.user).is_empty() {
        for space in state.hub.huddles_of(&session.user) {
            let users = state.hub.huddle_leave(&space, &session.user);
            announce_huddle(state, &space, users);
        }
    }
    if state.hub.sessions_of_user(&session.user).is_empty() {
        state.hub.clear_presence(&session.user);
        let msg = ServerMsg::Presence {
            user: session.user.clone(),
            state: PresenceState::Offline,
            custom_status: None,
            at: AppState::now_rfc3339(),
        };
        state.hub.broadcast_sharing(&session.spaces(), &msg, None);
    }
}

async fn handle_msg(state: &Arc<AppState>, session: &Arc<Session>, msg: ClientMsg) {
    match msg {
        ClientMsg::Hello { .. } => session.send(error("protocol", "hello already received")),
        ClientMsg::Subscribe { spaces } => subscribe(state, session, spaces).await,
        ClientMsg::Typing {
            space,
            thread,
            active,
        } => {
            if !session.subscribed(&space) {
                return session.send(error("not_subscribed", space));
            }
            let msg = ServerMsg::Typing {
                space: space.clone(),
                thread,
                user: session.user.clone(),
                active,
                ttl_ms: TYPING_TTL_MS,
            };
            state.hub.broadcast_space(&space, &msg, Some(&session.id));
        }
        ClientMsg::Presence {
            state: ps,
            custom_status,
        } => {
            if let Err(e) = state.db.set_custom_status(
                &session.user,
                custom_status.as_ref(),
                &AppState::now_rfc3339(),
            ) {
                tracing::warn!(error = %e, "custom status write failed");
            }
            set_presence(state, session, ps, custom_status);
        }
        ClientMsg::Read {
            space,
            thread,
            message,
        } => {
            if !session.subscribed(&space) {
                return session.send(error("not_subscribed", space));
            }
            let at = AppState::now_rfc3339();
            if let Err(e) =
                state
                    .db
                    .upsert_receipt(&session.user, &space, thread.as_deref(), &message, &at)
            {
                tracing::warn!(error = %e, "receipt write failed");
            }
            let msg = ServerMsg::Receipt {
                space: space.clone(),
                thread,
                message,
                user: session.user.clone(),
                at,
            };
            state.hub.broadcast_space(&space, &msg, Some(&session.id));
        }
        ClientMsg::Draft {
            space,
            thread,
            text,
        } => {
            let at = AppState::now_rfc3339();
            if let Err(e) = state.db.upsert_draft(
                &session.user,
                &space,
                thread.as_deref(),
                &session.device_id,
                &text,
                &at,
            ) {
                tracing::warn!(error = %e, "draft write failed");
            }
            let msg = ServerMsg::Draft {
                space,
                thread,
                text,
                device_id: session.device_id.clone(),
                at,
            };
            state
                .hub
                .broadcast_user(&session.user, &msg, Some(&session.id));
        }
        ClientMsg::HuddleJoin { space } => {
            if !session.subscribed(&space) {
                return session.send(error("not_subscribed", space));
            }
            let users = state.hub.huddle_join(&space, &session.user);
            announce_huddle(state, &space, users);
        }
        ClientMsg::HuddleLeave { space } => {
            let users = state.hub.huddle_leave(&space, &session.user);
            announce_huddle(state, &space, users);
        }
        ClientMsg::HuddleSignal { space, to, payload } => {
            if !session.subscribed(&space) {
                return session.send(error("not_subscribed", space));
            }
            let msg = ServerMsg::HuddleSignal {
                space,
                from: session.user.clone(),
                payload,
            };
            state.hub.broadcast_user(&to, &msg, None);
        }
        ClientMsg::Ping { t } => session.send(ServerMsg::Pong { t }),
    }
}

/// Replace the session's space set with the authorised subset of `spaces`;
/// then catch the session up on drafts and presence for what was added, and
/// tell the newly shared spaces about this user's presence.
async fn subscribe(state: &Arc<AppState>, session: &Arc<Session>, spaces: Vec<String>) {
    let mut allowed = HashSet::new();
    let mut seen = HashSet::new();
    for space in spaces {
        if !seen.insert(space.clone()) {
            continue;
        }
        if !valid_space(&space) {
            tracing::debug!(space, "subscribe: malformed space name dropped");
            continue;
        }
        // note: sequential membership checks; fine for a sidebar, slow for hundreds of spaces
        if state
            .authorize_space(&session.user, &session.user_id, &session.token, &space)
            .await
        {
            allowed.insert(space);
        }
    }
    let previous = session.spaces();
    let added: Vec<String> = allowed.difference(&previous).cloned().collect();
    session.set_spaces(allowed);
    if added.is_empty() {
        return;
    }
    for space in &added {
        let users = state.hub.huddle_members(space);
        if !users.is_empty() {
            session.send(ServerMsg::Huddle {
                space: space.clone(),
                users,
                at: AppState::now_rfc3339(),
            });
        }
    }

    match state.db.drafts_for(&session.user, &added) {
        Ok(rows) => {
            for d in rows {
                session.send(ServerMsg::Draft {
                    space: d.space,
                    thread: d.thread,
                    text: d.text,
                    device_id: d.device_id,
                    at: d.at,
                });
            }
        }
        Err(e) => tracing::warn!(error = %e, "draft read failed"),
    }

    let mine = state
        .hub
        .presence(&session.user)
        .map(|p| p.to_msg(&session.user));
    let mut users_told_about = HashSet::from([session.user.clone()]);
    let mut sessions_told = HashSet::new();
    for space in &added {
        for other in state.hub.sessions_in_space(space) {
            if other.id == session.id {
                continue;
            }
            if users_told_about.insert(other.user.clone()) {
                if let Some(p) = state.hub.presence(&other.user) {
                    session.send(p.to_msg(&other.user));
                }
            }
            if let Some(m) = &mine {
                if sessions_told.insert(other.id.clone()) {
                    other.send(m.clone());
                }
            }
        }
    }
}
