//! Relay WebSocket protocol. One socket per client, JSON text frames,
//! `{"type": "...", ...}` tagged. The relay carries only what Google cannot:
//! typing, presence, read receipts, draft sync, and fan-out of Google events.
//! Messages themselves never pass through here; the client talks to Google directly.

use crate::chat;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    Active,
    Away,
    Dnd,
    Offline,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct CustomStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    pub text: String,
    /// RFC 3339; absent = until cleared
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// Client → relay.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// First frame on every socket. `token` is the Google (or twin) access token;
    /// the relay resolves identity by calling the userinfo endpoint, never by trusting a user id.
    Hello {
        token: String,
        device_id: String,
        client_version: String,
    },
    /// Spaces the client wants events for (its sidebar). Replaces the previous set.
    Subscribe {
        spaces: Vec<String>,
    },
    /// Sent on first keystroke, every 3 s while typing, and with `active: false` on send or clear.
    Typing {
        space: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<String>,
        active: bool,
    },
    Presence {
        state: PresenceState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom_status: Option<CustomStatus>,
    },
    /// The newest message the user has seen in a space or thread.
    Read {
        space: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<String>,
        message: String,
    },
    /// Draft text for a space or thread, synced across the user's own devices only.
    Draft {
        space: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<String>,
        text: String,
    },
    /// Join the space's huddle (in-app audio between Prism clients, WebRTC signalled through the relay).
    HuddleJoin {
        space: String,
    },
    HuddleLeave {
        space: String,
    },
    /// One peer's SDP or ICE for another peer in the same huddle; the relay only forwards it.
    HuddleSignal {
        space: String,
        to: String,
        payload: serde_json::Value,
    },
    Ping {
        t: u64,
    },
}

/// A Google Workspace event, normalised. `event_type` is the CloudEvents type,
/// e.g. `google.workspace.chat.message.v1.created`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ChatEvent {
    pub event_type: String,
    /// `spaces/{id}`
    pub space: String,
    /// RFC 3339
    pub event_time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<chat::Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reaction: Option<chat::Reaction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membership: Option<chat::Membership>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_resource: Option<chat::Space>,
}

pub mod event_types {
    pub const MESSAGE_CREATED: &str = "google.workspace.chat.message.v1.created";
    pub const MESSAGE_UPDATED: &str = "google.workspace.chat.message.v1.updated";
    pub const MESSAGE_DELETED: &str = "google.workspace.chat.message.v1.deleted";
    pub const REACTION_CREATED: &str = "google.workspace.chat.reaction.v1.created";
    pub const REACTION_DELETED: &str = "google.workspace.chat.reaction.v1.deleted";
    pub const MEMBERSHIP_CREATED: &str = "google.workspace.chat.membership.v1.created";
    pub const MEMBERSHIP_DELETED: &str = "google.workspace.chat.membership.v1.deleted";
    pub const SPACE_UPDATED: &str = "google.workspace.chat.space.v1.updated";
}

/// Relay → client.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // note: Event carries a full Message; boxing would ripple through every consumer
pub enum ServerMsg {
    /// Answer to `Hello`. `online` = Prism users currently connected (user names `users/{id}`).
    Welcome {
        user: chat::User,
        session_id: String,
        online: Vec<String>,
    },
    Typing {
        space: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<String>,
        user: String,
        active: bool,
        /// Client hides the indicator after this many ms with no refresh.
        ttl_ms: u64,
    },
    Presence {
        user: String,
        state: PresenceState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom_status: Option<CustomStatus>,
        at: String,
    },
    Receipt {
        space: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<String>,
        message: String,
        user: String,
        at: String,
    },
    Draft {
        space: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<String>,
        text: String,
        device_id: String,
        at: String,
    },
    Event {
        event: ChatEvent,
    },
    /// Who is in the space's huddle now; sent to every subscriber on each change (empty = over).
    Huddle {
        space: String,
        users: Vec<String>,
        at: String,
    },
    /// SDP or ICE from `from`, for the receiving user only.
    HuddleSignal {
        space: String,
        from: String,
        payload: serde_json::Value,
    },
    Pong {
        t: u64,
    },
    Error {
        code: String,
        message: String,
    },
}

/// How long a typing indicator stays visible without a refresh.
pub const TYPING_TTL_MS: u64 = 5_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagged_json_shape() {
        let m = ClientMsg::Typing {
            space: "spaces/A".into(),
            thread: None,
            active: true,
        };
        let j = serde_json::to_string(&m).unwrap();
        assert_eq!(j, r#"{"type":"typing","space":"spaces/A","active":true}"#);
        let s: ServerMsg = serde_json::from_str(
            r#"{"type":"presence","user":"users/1","state":"dnd","at":"2026-11-06T10:00:00Z"}"#,
        )
        .unwrap();
        assert!(matches!(
            s,
            ServerMsg::Presence {
                state: PresenceState::Dnd,
                ..
            }
        ));
    }
}
