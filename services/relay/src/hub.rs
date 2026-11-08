//! Live sessions and presence. Fan-out is a loop over per-session mpsc senders.

use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use dashmap::DashMap;
use confab_proto::relay::{CustomStatus, PresenceState, ServerMsg};
use tokio::sync::mpsc;

pub struct Session {
    pub id: String,
    /// `users/{sub}`
    pub user: String,
    /// The bare `sub`, for membership URLs.
    pub user_id: String,
    pub device_id: String,
    /// Access token, in memory for the life of the socket only.
    pub token: String,
    spaces: RwLock<HashSet<String>>,
    tx: mpsc::UnboundedSender<ServerMsg>,
}

impl Session {
    pub fn new(
        id: String,
        user: String,
        user_id: String,
        device_id: String,
        token: String,
        tx: mpsc::UnboundedSender<ServerMsg>,
    ) -> Self {
        Self {
            id,
            user,
            user_id,
            device_id,
            token,
            spaces: RwLock::new(HashSet::new()),
            tx,
        }
    }

    pub fn send(&self, msg: ServerMsg) {
        // A closed receiver means the socket task is already gone; nothing to do.
        let _ = self.tx.send(msg);
    }

    pub fn spaces(&self) -> HashSet<String> {
        self.spaces
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn set_spaces(&self, spaces: HashSet<String>) {
        *self.spaces.write().unwrap_or_else(|p| p.into_inner()) = spaces;
    }

    pub fn subscribed(&self, space: &str) -> bool {
        self.spaces
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .contains(space)
    }
}

#[derive(Clone, Debug)]
pub struct PresenceRecord {
    pub state: PresenceState,
    pub custom_status: Option<CustomStatus>,
    pub at: String,
}

impl PresenceRecord {
    pub fn to_msg(&self, user: &str) -> ServerMsg {
        ServerMsg::Presence {
            user: user.to_string(),
            state: self.state,
            custom_status: self.custom_status.clone(),
            at: self.at.clone(),
        }
    }
}

// note: in-process fan-out; Redis pub/sub when the relay runs more than one instance
#[derive(Default)]
pub struct Hub {
    sessions: DashMap<String, Arc<Session>>,
    presence: DashMap<String, PresenceRecord>,
    /// space -> users in its huddle (in-app audio); gone when the last one leaves.
    huddles: DashMap<String, HashSet<String>>,
}

impl Hub {
    pub fn add(&self, session: Arc<Session>) {
        self.sessions.insert(session.id.clone(), session);
    }

    pub fn remove(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.remove(id).map(|(_, s)| s)
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn online_users(&self) -> Vec<String> {
        let mut users: Vec<String> = self
            .sessions
            .iter()
            .map(|s| s.user.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        users.sort();
        users
    }

    pub fn sessions_of_user(&self, user: &str) -> Vec<Arc<Session>> {
        self.sessions
            .iter()
            .filter(|s| s.user == user)
            .map(|s| s.clone())
            .collect()
    }

    pub fn sessions_in_space(&self, space: &str) -> Vec<Arc<Session>> {
        self.sessions
            .iter()
            .filter(|s| s.subscribed(space))
            .map(|s| s.clone())
            .collect()
    }

    /// Union of a user's subscribed spaces across devices.
    pub fn user_spaces(&self, user: &str) -> HashSet<String> {
        let mut out = HashSet::new();
        for s in self.sessions.iter().filter(|s| s.user == user) {
            out.extend(s.spaces());
        }
        out
    }

    /// Any token of a session subscribed to `space` (for resource fetches on push).
    pub fn any_token_for_space(&self, space: &str) -> Option<String> {
        self.sessions
            .iter()
            .find(|s| s.subscribed(space))
            .map(|s| s.token.clone())
    }

    pub fn any_token_for_user(&self, user: &str) -> Option<String> {
        self.sessions
            .iter()
            .find(|s| s.user == user)
            .map(|s| s.token.clone())
    }

    /// Every session subscribed to `space`, except `exclude`.
    pub fn broadcast_space(&self, space: &str, msg: &ServerMsg, exclude: Option<&str>) {
        for s in self.sessions.iter() {
            if Some(s.id.as_str()) != exclude && s.subscribed(space) {
                s.send(msg.clone());
            }
        }
    }

    /// Every session of `user`, except `exclude`.
    pub fn broadcast_user(&self, user: &str, msg: &ServerMsg, exclude: Option<&str>) {
        for s in self.sessions.iter() {
            if Some(s.id.as_str()) != exclude && s.user == user {
                s.send(msg.clone());
            }
        }
    }

    /// Every session subscribed to at least one of `spaces`, once each, except `exclude`.
    pub fn broadcast_sharing(
        &self,
        spaces: &HashSet<String>,
        msg: &ServerMsg,
        exclude: Option<&str>,
    ) {
        if spaces.is_empty() {
            return;
        }
        for s in self.sessions.iter() {
            if Some(s.id.as_str()) == exclude {
                continue;
            }
            let theirs = s.spaces();
            if spaces.iter().any(|sp| theirs.contains(sp)) {
                s.send(msg.clone());
            }
        }
    }

    /// Adds `user` to the space's huddle; returns the members after the change.
    pub fn huddle_join(&self, space: &str, user: &str) -> Vec<String> {
        let mut set = self.huddles.entry(space.to_string()).or_default();
        set.insert(user.to_string());
        let mut v: Vec<String> = set.iter().cloned().collect();
        v.sort();
        v
    }

    /// Removes `user` from the space's huddle; returns the members after the change (empty = over).
    pub fn huddle_leave(&self, space: &str, user: &str) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(mut set) = self.huddles.get_mut(space) {
            set.remove(user);
            v = set.iter().cloned().collect();
            v.sort();
        }
        if v.is_empty() {
            self.huddles.remove(space);
        }
        v
    }

    /// Every huddle the user is in, for the session-close sweep.
    pub fn huddles_of(&self, user: &str) -> Vec<String> {
        self.huddles
            .iter()
            .filter(|e| e.value().contains(user))
            .map(|e| e.key().clone())
            .collect()
    }

    pub fn huddle_members(&self, space: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .huddles
            .get(space)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    pub fn set_presence(&self, user: &str, record: PresenceRecord) {
        self.presence.insert(user.to_string(), record);
    }

    pub fn presence(&self, user: &str) -> Option<PresenceRecord> {
        self.presence.get(user).map(|p| p.clone())
    }

    pub fn clear_presence(&self, user: &str) {
        self.presence.remove(user);
    }
}
