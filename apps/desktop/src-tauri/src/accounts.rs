//! Google accounts side by side. Every profile directory
//! under the app data dir is one account with its own `AppState` (tokens, cache,
//! relay socket). One account is active: its commands, `relay` and `shell` events
//! and media proxy are what the web view sees. The others keep their relay sockets
//! open; a new message there bumps an unread counter and, when someone else sent it,
//! raises one system notification per account per minute.

use crate::profile::{AccountInfo, Profile};
use crate::shell::ShellEvent;
use crate::{outbox, AppState};
use dashmap::DashMap;
use prism_proto::relay::{event_types, ServerMsg};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// At most one notification per account in this window.
pub const NOTIFY_EVERY: Duration = Duration::from_secs(60);

/// One row of `accounts_list`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// The profile directory name (`default`, `two`, `acct-3`).
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<String>,
    pub signed_in: bool,
    pub active: bool,
    /// Messages that arrived while the account was not the active one.
    pub unread: u32,
}

/// Where a relay frame of one account goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// To the web view: the account is active.
    Emit,
    /// Counted for an inactive account; `notify` when someone else sent the message.
    Unread { notify: bool },
    /// Nothing: an inactive account's presence, typing or socket chatter.
    Drop,
}

/// Pure half of the routing: the active account's frames all go through, an inactive
/// account only counts new messages (`me` is that account's `users/{id}`).
pub fn route_frame(active: bool, msg: &ServerMsg, me: Option<&str>) -> Route {
    if active {
        return Route::Emit;
    }
    match msg {
        ServerMsg::Event { event } if event.event_type == event_types::MESSAGE_CREATED => {
            let sender = event
                .message
                .as_ref()
                .and_then(|m| m.sender.as_ref())
                .map(|u| u.name.as_str());
            let mine = sender.is_some() && sender == me;
            Route::Unread { notify: !mine }
        }
        _ => Route::Drop,
    }
}

/// The first free `acct-<n>` given the profile directories that exist.
pub fn next_account_id(existing: &[String]) -> String {
    let taken: BTreeSet<u64> = existing
        .iter()
        .filter_map(|id| id.strip_prefix("acct-"))
        .filter_map(|n| n.parse().ok())
        .collect();
    let mut n = 1;
    while taken.contains(&n) {
        n += 1;
    }
    format!("acct-{n}")
}

/// Active first, then by id.
pub fn sort_accounts(accounts: &mut [Account]) {
    accounts.sort_by(|a, b| b.active.cmp(&a.active).then_with(|| a.id.cmp(&b.id)));
}

/// One notification per account per `NOTIFY_EVERY`.
#[derive(Default)]
pub struct NotifyThrottle {
    last: Mutex<HashMap<String, Instant>>,
}

impl NotifyThrottle {
    /// True when `account` may notify at `now`; the slot is then taken.
    pub fn allow(&self, account: &str, now: Instant) -> bool {
        let mut last = self.last.lock().unwrap_or_else(|p| p.into_inner());
        match last.get(account) {
            Some(at) if now.duration_since(*at) < NOTIFY_EVERY => false,
            _ => {
                last.insert(account.to_string(), now);
                true
            }
        }
    }
}

pub struct Accounts {
    app_data_dir: PathBuf,
    /// Built states by profile name; a listed profile may not be built yet.
    states: RwLock<BTreeMap<String, Arc<AppState>>>,
    active: RwLock<String>,
    unread: DashMap<String, u32>,
    throttle: NotifyThrottle,
}

impl Accounts {
    /// Builds the account `PRISM_PROFILE` names (`default` when unset) and makes it
    /// active; the other profiles are listed and built by `boot_others`.
    pub fn boot(app_data_dir: PathBuf) -> Result<Self, String> {
        let initial = Profile::initial_name();
        let accounts = Self {
            app_data_dir,
            states: RwLock::new(BTreeMap::new()),
            active: RwLock::new(initial.clone()),
            unread: DashMap::new(),
            throttle: NotifyThrottle::default(),
        };
        accounts.build(&initial)?;
        Ok(accounts)
    }

    fn states(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<String, Arc<AppState>>> {
        self.states.read().unwrap_or_else(|p| p.into_inner())
    }

    fn states_mut(&self) -> std::sync::RwLockWriteGuard<'_, BTreeMap<String, Arc<AppState>>> {
        self.states.write().unwrap_or_else(|p| p.into_inner())
    }

    pub fn active_id(&self) -> String {
        self.active
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn is_active(&self, id: &str) -> bool {
        *self.active.read().unwrap_or_else(|p| p.into_inner()) == id
    }

    /// The active account's state; `boot` and every switch make sure it is built.
    pub fn active(&self) -> Arc<AppState> {
        let id = self.active_id();
        if let Some(st) = self.states().get(&id) {
            return st.clone();
        }
        // note: unreachable after boot; a failed build here would mean the data dir vanished.
        self.build(&id)
            .unwrap_or_else(|e| panic!("active account {id} cannot be built: {e}"))
    }

    pub fn state(&self, id: &str) -> Option<Arc<AppState>> {
        self.states().get(id).cloned()
    }

    /// Profile directories on disk: those holding an `auth.json` or an `account.json`
    /// (a keyring-backed profile has no token file but gets `account.json` on sign-in).
    fn profile_dirs(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.app_data_dir) else {
            return Vec::new();
        };
        let mut out: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter(|e| {
                e.path().join("auth.json").exists() || e.path().join("account.json").exists()
            })
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|name| Profile::sanitize(name) == *name)
            .collect();
        out.sort();
        out
    }

    /// Every account id: built states, profile directories and the active one.
    fn ids(&self) -> Vec<String> {
        let mut ids: BTreeSet<String> = self.states().keys().cloned().collect();
        ids.extend(self.profile_dirs());
        ids.insert(self.active_id());
        ids.into_iter().collect()
    }

    /// Listed profiles whose state is not built yet.
    pub fn pending_ids(&self) -> Vec<String> {
        let built = self.states();
        self.profile_dirs()
            .into_iter()
            .filter(|id| !built.contains_key(id))
            .collect()
    }

    /// The state for `id`, built now when the profile is new or not yet loaded.
    pub fn build(&self, id: &str) -> Result<Arc<AppState>, String> {
        if let Some(st) = self.states().get(id) {
            return Ok(st.clone());
        }
        let st = Arc::new(AppState::new(self.app_data_dir.clone(), id)?);
        let mut states = self.states_mut();
        Ok(states.entry(id.to_string()).or_insert(st).clone())
    }

    fn account_of(&self, id: &str, built: Option<&Arc<AppState>>) -> Account {
        let dir = self.app_data_dir.join(id);
        let (info, signed_in) = match built {
            Some(st) => (
                st.auth
                    .user()
                    .map(|u| AccountInfo::from_user(&u))
                    .or_else(|| AccountInfo::read(&st.profile.account_file())),
                st.auth.is_signed_in(),
            ),
            None => (
                AccountInfo::read(&dir.join("account.json")),
                dir.join("auth.json").exists(),
            ),
        };
        let info = info.unwrap_or_default();
        Account {
            id: id.to_string(),
            email: info.email,
            display_name: info.display_name,
            photo: info.photo,
            signed_in,
            active: self.is_active(id),
            unread: self.unread.get(id).map(|n| *n).unwrap_or(0),
        }
    }

    /// Every account, active first, then by id.
    pub fn list(&self) -> Vec<Account> {
        let ids = self.ids();
        let states = self.states();
        let mut out: Vec<Account> = ids
            .iter()
            .map(|id| self.account_of(id, states.get(id)))
            .collect();
        drop(states);
        sort_accounts(&mut out);
        out
    }

    /// Makes `id` active (building it when needed) and clears its unread count.
    pub fn switch(&self, id: &str) -> Result<Account, String> {
        let id = Profile::sanitize(id);
        if self.state(&id).is_none() && !self.app_data_dir.join(&id).is_dir() {
            return Err(format!("no account {id}"));
        }
        let st = self.build(&id)?;
        *self.active.write().unwrap_or_else(|p| p.into_inner()) = id.clone();
        self.unread.remove(&id);
        tracing::info!("active account: {id}");
        Ok(self.account_of(&id, Some(&st)))
    }

    /// A fresh signed-out profile `acct-<n>`, made active.
    pub fn add(&self) -> Result<Account, String> {
        let id = next_account_id(&self.ids());
        let dir = self.app_data_dir.join(&id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        self.switch(&id)
    }

    /// Signs the account out, drops its state and deletes its directory. The active
    /// account is refused: switch first.
    pub fn remove(&self, id: &str) -> Result<(), String> {
        let id = Profile::sanitize(id);
        if self.is_active(&id) {
            return Err("the active account cannot be removed; switch to another first".into());
        }
        let dir = self.app_data_dir.join(&id);
        let removed = self.states_mut().remove(&id);
        if removed.is_none() && !dir.is_dir() {
            return Err(format!("no account {id}"));
        }
        if let Some(st) = removed {
            st.relay.stop();
            if let Err(e) = st.auth.sign_out() {
                tracing::warn!("sign out {id}: {e}");
            }
        }
        self.unread.remove(&id);
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| format!("cannot remove {}: {e}", dir.display()))?;
        }
        tracing::info!("account {id} removed");
        Ok(())
    }

    /// One more unread message for an inactive account; the new count.
    pub fn bump_unread(&self, id: &str) -> u32 {
        let mut n = self.unread.entry(id.to_string()).or_insert(0);
        *n = n.saturating_add(1);
        *n
    }
}

/// Builds every listed profile the boot left out, on the async runtime, and
/// starts the relay and the outbox for the signed-in ones.
pub fn boot_others(app: AppHandle, accounts: Arc<Accounts>) {
    tauri::async_runtime::spawn(async move {
        for id in accounts.pending_ids() {
            // note: AppState::new opens SQLite and reads the token file synchronously; a few profiles take milliseconds.
            match accounts.build(&id) {
                Ok(st) => {
                    if st.auth.is_signed_in() {
                        if let Some(u) = st.auth.status().user {
                            match st.cache.ensure_owner(&u.name) {
                                Ok(true) => tracing::info!("cache reset for {} ({id})", u.name),
                                Ok(false) => {}
                                Err(e) => tracing::warn!("cache owner check failed ({id}): {e}"),
                            }
                        }
                        st.relay.start(app.clone(), st.clone());
                        outbox::resume_unsent(app.clone(), st.clone());
                    }
                    tracing::info!("account {id} ready");
                }
                Err(e) => tracing::warn!("account {id} failed to load: {e}"),
            }
        }
    });
}

/// A `relay` frame from `account`: to the web view when active, else counted (and
/// possibly notified) when it is a new message, dropped otherwise.
pub fn emit_relay(app: &AppHandle, account: &AppState, msg: &ServerMsg) {
    let id = account.profile.name.as_str();
    let Some(accounts) = app.try_state::<Arc<Accounts>>() else {
        emit(app, "relay", msg);
        return;
    };
    match route_frame(accounts.is_active(id), msg, account.auth.me().as_deref()) {
        Route::Emit => emit(app, "relay", msg),
        Route::Unread { notify } => {
            let n = accounts.bump_unread(id);
            tracing::debug!("account {id}: {n} unread while inactive");
            if notify && accounts.throttle.allow(id, Instant::now()) {
                notify_new_message(app, account, msg);
            }
        }
        Route::Drop => {}
    }
}

/// A `shell` event from `account`: only the active account's reach the web view.
pub fn emit_shell(app: &AppHandle, account: &AppState, event: &ShellEvent) {
    let id = account.profile.name.as_str();
    let active = app
        .try_state::<Arc<Accounts>>()
        .is_none_or(|a| a.is_active(id));
    if active {
        emit(app, "shell", event);
    } else {
        tracing::debug!("account {id}: shell event dropped while inactive");
    }
}

fn emit<T: Serialize + Clone>(app: &AppHandle, name: &str, payload: &T) {
    if let Err(e) = app.emit(name, payload) {
        tracing::warn!("emit {name}: {e}");
    }
}

/// What the account is called in a notification: display name, else email, else id.
pub fn account_label(account: &AppState) -> String {
    account
        .auth
        .user()
        .and_then(|u| u.name.filter(|n| !n.trim().is_empty()).or(u.email))
        .unwrap_or_else(|| account.profile.name.clone())
}

fn notify_new_message(app: &AppHandle, account: &AppState, msg: &ServerMsg) {
    let body = match msg {
        ServerMsg::Event { event } => event.message.as_ref().map(|m| {
            let who = m
                .sender
                .as_ref()
                .and_then(|s| s.display_name.clone())
                .unwrap_or_else(|| "Someone".into());
            let text: String = m.text.as_deref().unwrap_or("").chars().take(140).collect();
            if text.trim().is_empty() {
                who
            } else {
                format!("{who}: {text}")
            }
        }),
        _ => None,
    };
    let title = format!("{}: new message", account_label(account));
    let mut builder = app.notification().builder().title(title);
    if let Some(b) = body {
        builder = builder.body(b);
    }
    if let Err(e) = builder.show() {
        tracing::debug!("notification for {}: {e}", account.profile.name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_proto::chat::{Message, User};
    use prism_proto::relay::ChatEvent;

    #[test]
    fn account_ids_take_the_first_free_number() {
        assert_eq!(next_account_id(&[]), "acct-1");
        assert_eq!(next_account_id(&["default".into(), "two".into()]), "acct-1");
        assert_eq!(
            next_account_id(&["acct-1".into(), "acct-2".into()]),
            "acct-3"
        );
        assert_eq!(
            next_account_id(&["acct-1".into(), "acct-3".into()]),
            "acct-2"
        );
        assert_eq!(next_account_id(&["acct-x".into()]), "acct-1");
    }

    fn created_by(sender: &str) -> ServerMsg {
        ServerMsg::Event {
            event: ChatEvent {
                event_type: event_types::MESSAGE_CREATED.into(),
                space: "spaces/A".into(),
                event_time: "2026-11-07T10:00:00Z".into(),
                message: Some(Message {
                    name: "spaces/A/messages/B".into(),
                    sender: Some(User {
                        name: sender.into(),
                        ..Default::default()
                    }),
                    text: Some("hi".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        }
    }

    #[test]
    fn relay_frames_route_by_active_account() {
        let theirs = created_by("users/1002");
        let mine = created_by("users/1001");
        let me = Some("users/1001");
        assert_eq!(route_frame(true, &theirs, me), Route::Emit);
        assert_eq!(route_frame(true, &mine, me), Route::Emit);
        assert_eq!(
            route_frame(false, &theirs, me),
            Route::Unread { notify: true }
        );
        assert_eq!(
            route_frame(false, &mine, me),
            Route::Unread { notify: false }
        );
        assert_eq!(
            route_frame(false, &theirs, None),
            Route::Unread { notify: true }
        );
        let updated = ServerMsg::Event {
            event: ChatEvent {
                event_type: event_types::MESSAGE_UPDATED.into(),
                ..match &theirs {
                    ServerMsg::Event { event } => event.clone(),
                    _ => unreachable!(),
                }
            },
        };
        assert_eq!(route_frame(false, &updated, me), Route::Drop);
        let welcome: ServerMsg = serde_json::from_str(
            r#"{"type":"welcome","user":{"name":"users/1001"},"session_id":"s","online":[]}"#,
        )
        .unwrap();
        assert_eq!(route_frame(false, &welcome, me), Route::Drop);
        assert_eq!(route_frame(true, &welcome, me), Route::Emit);
    }

    #[test]
    fn accounts_sort_active_first_then_by_id() {
        let mk = |id: &str, active: bool| Account {
            id: id.into(),
            active,
            ..Default::default()
        };
        let mut v = vec![mk("two", false), mk("acct-1", false), mk("default", true)];
        sort_accounts(&mut v);
        let ids: Vec<&str> = v.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["default", "acct-1", "two"]);
        let json = serde_json::to_value(&v[0]).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "id": "default", "signedIn": false, "active": true, "unread": 0 })
        );
        let full = Account {
            id: "two".into(),
            email: Some("priya@prism.test".into()),
            display_name: Some("Priya Nair".into()),
            photo: None,
            signed_in: true,
            active: false,
            unread: 3,
        };
        let json = serde_json::to_value(&full).unwrap();
        assert_eq!(json["displayName"], "Priya Nair");
        assert_eq!(json["signedIn"], true);
        assert_eq!(json["unread"], 3);
        assert!(json.get("photo").is_none());
    }

    #[test]
    fn notifications_are_throttled_per_account() {
        let t = NotifyThrottle::default();
        let now = Instant::now();
        assert!(t.allow("two", now));
        assert!(!t.allow("two", now + Duration::from_secs(30)));
        assert!(t.allow("acct-1", now + Duration::from_secs(30)));
        assert!(t.allow("two", now + NOTIFY_EVERY));
        assert!(!t.allow("two", now + NOTIFY_EVERY + Duration::from_secs(1)));
    }
}
