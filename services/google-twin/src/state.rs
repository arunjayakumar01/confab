//! In-memory store behind one RwLock. Reseeded on every start; never persisted.

use crate::config::Config;
use crate::events::Pusher;
use crate::util::{fmt_time, now_str, random_id, random_token, user_id, utf16_len};
use chrono::{DateTime, Duration, Local, SecondsFormat, Utc};
use prism_proto::availability::{Availability, CustomStatusWire};
use prism_proto::calendar::CalEvent;
use prism_proto::chat::{
    Annotation, EmojiReactionSummary, Membership, MembershipCount, Message, Reaction, Space,
    SpaceDetails, TokenResponse, User, UserMentionMetadata,
};
use prism_proto::tasks::Task;
use std::collections::{BTreeMap, HashMap};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

pub struct AppState {
    pub config: Config,
    pub store: RwLock<Store>,
    pub pusher: Pusher,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let pusher = Pusher::from_config(&config);
        let store = crate::seed::seed(&config, Utc::now());
        Self {
            config,
            store: RwLock::new(store),
            pusher,
        }
    }

    pub fn read(&self) -> RwLockReadGuard<'_, Store> {
        self.store.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, Store> {
        self.store.write().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Debug, Clone)]
pub struct TokenRec {
    pub user: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AuthCode {
    pub user: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: Option<String>,
    pub challenge_method: String,
    pub scope: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Blob {
    pub space: String,
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// One Google Tasks task: the shared `Task` plus the `completed` timestamp, which
/// the Tasks API sets when the status flips and which the client never reads.
#[derive(Debug, Clone)]
pub struct TaskRec {
    pub task: Task,
    /// RFC 3339, set while `status` is `completed`.
    pub completed: Option<String>,
}

impl TaskRec {
    /// The task as the Tasks API prints it: `kind`, the shared fields, `completed`.
    pub fn to_json(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(&self.task).unwrap_or_default();
        v["kind"] = serde_json::json!("tasks#task");
        if let Some(c) = &self.completed {
            v["completed"] = serde_json::json!(c);
        }
        v
    }
}

/// One Meet space the twin created: the room behind a meeting code. Its conference
/// counts as active for `MEET_ACTIVE_SECS` after creation.
#[derive(Debug, Clone)]
pub struct MeetRec {
    /// `spaces/meet-{n}`
    pub name: String,
    /// `abc-defg-hij`
    pub code: String,
    pub created_at: DateTime<Utc>,
    /// `users/{id}` of the caller who created it; the first participant.
    pub creator: String,
}

/// How long a twin Meet space reports an `activeConference` after creation.
pub const MEET_ACTIVE_SECS: i64 = 15 * 60;

/// The caller's own calendar id.
pub const CAL_PRIMARY: &str = "primary";
/// The shared read-only calendar every twin user sees.
pub const CAL_TEAM: &str = "team@example.test";
/// How many changes back a calendar sync token stays valid; older ones answer 410.
pub const CAL_SYNC_HISTORY: u64 = 100;

/// One calendar event, plus the value of the change counter when it last changed.
/// Cancelled events stay so an incremental sync can report them.
#[derive(Debug, Clone)]
pub struct CalEventRec {
    /// `primary` or `team@example.test`
    pub calendar: String,
    /// `self` on attendees is not stored: it is relative to the caller and set on output.
    pub event: CalEvent,
    /// `Store::cal_seq` at the last create, patch or delete.
    pub seq: u64,
}

/// One user's presence as `users.availability` keeps it: a state with, for
/// `ACTIVE` and `DO_NOT_DISTURB`, the instant it lapses, and a custom status with
/// its own expiry. Absent means the default, `ACTIVE` with no status.
#[derive(Debug, Clone, Default)]
pub struct AvailabilityRec {
    /// `ACTIVE` | `AWAY` | `DO_NOT_DISTURB`
    pub state: String,
    /// `markAsActive` keeps the user active until here, then `IDLE`; do-not-disturb
    /// ends here and the user is `ACTIVE` again.
    pub until: Option<DateTime<Utc>>,
    pub custom_status: Option<CustomStatusWire>,
    pub status_until: Option<DateTime<Utc>>,
}

impl AvailabilityRec {
    /// What the resource looks like at `now`, lapsed windows applied.
    pub fn to_json(&self, name: &str, now: DateTime<Utc>) -> serde_json::Value {
        let lapsed = self.until.is_some_and(|t| t <= now);
        let state = match self.state.as_str() {
            Availability::ACTIVE if lapsed => Availability::IDLE,
            Availability::DO_NOT_DISTURB if lapsed => Availability::ACTIVE,
            "" => Availability::ACTIVE,
            s => s,
        };
        let mut v = serde_json::json!({ "name": name, "state": state });
        if state == Availability::DO_NOT_DISTURB {
            if let Some(t) = self.until {
                v["doNotDisturbMetadata"] =
                    serde_json::json!({ "expirationTime": Store::task_time(t) });
            }
        }
        if let Some(status) = &self.custom_status {
            if self.status_until.is_none_or(|t| t > now) {
                v["customStatus"] = serde_json::to_value(status).unwrap_or_default();
            }
        }
        v
    }
}

/// One file in the Drive app-data folder of the caller who created it.
#[derive(Debug, Clone)]
pub struct DriveFile {
    /// `file-{n}`
    pub id: String,
    /// `users/{id}` of the owner; app data is per account.
    pub owner: String,
    pub name: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    /// RFC 3339 with milliseconds, the way Drive prints `modifiedTime`.
    pub modified_time: String,
}

impl DriveFile {
    /// `{kind, id, name, mimeType, modifiedTime}`, the metadata Drive answers with.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": "drive#file",
            "id": self.id,
            "name": self.name,
            "mimeType": self.content_type,
            "modifiedTime": self.modified_time,
        })
    }
}

#[derive(Default)]
pub struct Store {
    pub public_base: String,
    pub token_ttl_secs: i64,
    /// `users/{id}` → user
    pub users: BTreeMap<String, User>,
    /// `spaces/{id}` → space
    pub spaces: BTreeMap<String, Space>,
    /// `spaces/{s}/members/{uid}` → membership
    pub memberships: BTreeMap<String, Membership>,
    /// `spaces/{s}/messages/{m}` → message (summaries kept current here)
    pub messages: BTreeMap<String, Message>,
    /// Insertion order = creation order.
    pub reactions: Vec<Reaction>,
    /// (`users/{id}`, `spaces/{s}`) → lastReadTime
    pub read_states: HashMap<(String, String), String>,
    /// attachment resourceName → bytes
    pub blobs: HashMap<String, Blob>,
    /// `{space}|{client-id}` → message name
    pub client_ids: HashMap<String, String>,
    pub codes: HashMap<String, AuthCode>,
    pub access_tokens: HashMap<String, TokenRec>,
    /// refresh token → `users/{id}`
    pub refresh_tokens: HashMap<String, String>,
    /// Google Tasks, one default list shared by every twin user: `task-{n}` → task.
    // note: real Google keeps a list per account; the twin's single list keeps the seed small.
    pub tasks: BTreeMap<String, TaskRec>,
    /// Next `task-{n}`; seeded tasks take the first ids.
    pub next_task_id: u64,
    /// Meet spaces created through `POST /v2/spaces`: `spaces/meet-{n}` → space.
    pub meets: BTreeMap<String, MeetRec>,
    /// Next `meet-{n}`.
    pub next_meet_id: u64,
    /// Calendar events of every calendar: `ev-{n}` → event. Cancelled ones stay.
    pub cal_events: BTreeMap<String, CalEventRec>,
    /// Next `ev-{n}`; seeded events take the first ids.
    pub next_event_id: u64,
    /// The calendar change counter: the value of the latest change. A sync token is
    /// `change + 1`, so a fetch with token `t` returns what changed at `t` or later.
    pub cal_seq: u64,
    /// The oldest sync token still honoured (`CAL_SYNC_HISTORY` changes back).
    pub cal_min_sync: u64,
    /// Drive app-data files: `file-{n}` → file.
    pub drive_files: BTreeMap<String, DriveFile>,
    /// Next `file-{n}`.
    pub next_drive_id: u64,
    /// `users/{id}` → presence; absent means `ACTIVE` with no custom status.
    pub availability: HashMap<String, AvailabilityRec>,
}

impl Store {
    pub fn avatar_url(&self, id: &str) -> String {
        format!("{}/twin/avatars/{}.svg", self.public_base, user_id(id))
    }

    pub fn user(&self, name: &str) -> Option<&User> {
        self.users.get(name)
    }

    pub fn membership_name(space: &str, user: &str) -> String {
        format!("{space}/members/{}", user_id(user))
    }

    pub fn is_member(&self, space: &str, user: &str) -> bool {
        self.memberships
            .contains_key(&Self::membership_name(space, user))
    }

    pub fn members_of(&self, space: &str) -> Vec<Membership> {
        let prefix = format!("{space}/members/");
        self.memberships
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .map(|(_, m)| m.clone())
            .collect()
    }

    pub fn spaces_for(&self, user: &str) -> Vec<Space> {
        self.spaces
            .values()
            .filter(|s| self.is_member(&s.name, user))
            .cloned()
            .collect()
    }

    /// Canonical DM name, the seed's `spaces/AAAAdm{a}x{b}` with the lower id first.
    pub fn dm_name(a: &str, b: &str) -> String {
        let (a, b) = (user_id(a), user_id(b));
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        format!("spaces/AAAAdm{lo}x{hi}")
    }

    /// Insert an empty space in the seed's shape; members arrive via `add_member`.
    pub fn new_space(
        &mut self,
        name: &str,
        space_type: &str,
        display_name: Option<&str>,
        description: &str,
        create_time: &str,
    ) -> Space {
        let s = Space {
            name: name.to_string(),
            space_type: Some(space_type.into()),
            display_name: display_name.map(str::to_string),
            space_threading_state: Some("GROUPED_MESSAGES".into()),
            space_details: (!description.is_empty()).then(|| SpaceDetails {
                description: Some(description.into()),
                guidelines: None,
            }),
            create_time: Some(create_time.to_string()),
            last_active_time: None,
            membership_count: Some(MembershipCount {
                joined_direct_human_user_count: Some(0),
                joined_group_count: Some(0),
            }),
            space_uri: Some(format!(
                "https://chat.google.com/room/{}",
                name.trim_start_matches("spaces/")
            )),
            external_user_allowed: Some(false),
        };
        self.spaces.insert(s.name.clone(), s.clone());
        s
    }

    /// Insert a membership (and its read state) for an existing user.
    pub fn add_member(
        &mut self,
        space: &str,
        user: &str,
        role: &str,
        create_time: &str,
    ) -> Membership {
        let member = self.users.get(user).cloned().unwrap_or_else(|| User {
            name: user.to_string(),
            user_type: Some("HUMAN".into()),
            ..Default::default()
        });
        let m = Membership {
            name: Self::membership_name(space, user),
            state: Some("JOINED".into()),
            role: Some(role.into()),
            member: Some(member),
            create_time: Some(create_time.to_string()),
        };
        self.memberships.insert(m.name.clone(), m.clone());
        self.read_states
            .entry((user.to_string(), space.to_string()))
            .or_insert_with(|| create_time.to_string());
        self.refresh_membership_count(space);
        m
    }

    pub fn remove_member(&mut self, space: &str, user: &str) -> Option<Membership> {
        let removed = self.memberships.remove(&Self::membership_name(space, user));
        self.read_states
            .remove(&(user.to_string(), space.to_string()));
        self.refresh_membership_count(space);
        removed
    }

    fn refresh_membership_count(&mut self, space: &str) {
        let n = self.members_of(space).len() as i64;
        if let Some(s) = self.spaces.get_mut(space) {
            s.membership_count = Some(prism_proto::chat::MembershipCount {
                joined_direct_human_user_count: Some(n),
                joined_group_count: Some(0),
            });
        }
    }

    /// `{m}` in a path is either the server id or a `client-...` alias.
    pub fn resolve_message(&self, space: &str, id: &str) -> Option<String> {
        let direct = format!("{space}/messages/{id}");
        if self.messages.contains_key(&direct) {
            return Some(direct);
        }
        if id.starts_with("client-") {
            return self.client_ids.get(&format!("{space}|{id}")).cloned();
        }
        None
    }

    /// Messages of one space, oldest first (RFC 3339 strings sort chronologically).
    pub fn messages_in(&self, space: &str) -> Vec<&Message> {
        let prefix = format!("{space}/messages/");
        let mut v: Vec<&Message> = self
            .messages
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .map(|(_, m)| m)
            .collect();
        v.sort_by(|a, b| {
            a.create_time
                .cmp(&b.create_time)
                .then_with(|| a.name.cmp(&b.name))
        });
        v
    }

    pub fn latest_message_time(&self, space: &str) -> Option<String> {
        self.messages_in(space)
            .last()
            .and_then(|m| m.create_time.clone())
    }

    /// Replies in a thread other than the head itself.
    pub fn thread_replies(&self, space: &str, thread_name: &str, head: &str) -> Vec<String> {
        self.messages_in(space)
            .into_iter()
            .filter(|m| {
                m.name != head
                    && m.thread.as_ref().and_then(|t| t.name.as_deref()) == Some(thread_name)
            })
            .map(|m| m.name.clone())
            .collect()
    }

    pub fn reactions_of(&self, message: &str) -> Vec<Reaction> {
        let prefix = format!("{message}/reactions/");
        self.reactions
            .iter()
            .filter(|r| r.name.as_deref().is_some_and(|n| n.starts_with(&prefix)))
            .cloned()
            .collect()
    }

    /// Rebuild `emojiReactionSummaries` on a message from the reactions list.
    pub fn recompute_summaries(&mut self, message: &str) {
        let mut order: Vec<String> = Vec::new();
        let mut counts: HashMap<String, i64> = HashMap::new();
        for r in self.reactions_of(message) {
            let u = r.emoji.unicode.clone().unwrap_or_default();
            if !counts.contains_key(&u) {
                order.push(u.clone());
            }
            *counts.entry(u).or_insert(0) += 1;
        }
        if let Some(m) = self.messages.get_mut(message) {
            m.emoji_reaction_summaries = order
                .into_iter()
                .map(|u| EmojiReactionSummary {
                    reaction_count: counts.get(&u).copied(),
                    emoji: Some(prism_proto::chat::Emoji {
                        unicode: Some(u),
                        custom_emoji: None,
                    }),
                })
                .collect();
        }
    }

    /// `<users/1002>` spans become USER_MENTION annotations. Indices are UTF-16 code
    /// units, the index space the web view slices in.
    // note: Google counts in its own index space; UTF-16 matches JavaScript consumers,
    // which is all Prism has.
    pub fn annotate(&self, text: &str) -> Vec<Annotation> {
        let mut out = Vec::new();
        for (byte_idx, _) in text.match_indices("<users/") {
            let rest = &text[byte_idx..];
            let Some(close) = rest.find('>') else {
                continue;
            };
            let token = &rest[..=close];
            let id = &token[7..close];
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
                continue;
            }
            let name = format!("users/{id}");
            let Some(user) = self.users.get(&name) else {
                continue;
            };
            out.push(Annotation {
                annotation_type: Some("USER_MENTION".into()),
                start_index: Some(utf16_len(&text[..byte_idx]) as i64),
                length: Some(utf16_len(token) as i64),
                user_mention: Some(UserMentionMetadata {
                    user: Some(user.clone()),
                    mention_type: Some("MENTION".into()),
                }),
                rich_link_metadata: None,
            });
        }
        out
    }

    pub fn touch_space(&mut self, space: &str, time: &str) {
        if let Some(s) = self.spaces.get_mut(space) {
            s.last_active_time = Some(time.to_string());
        }
    }

    // ---- tokens -------------------------------------------------------------

    pub fn issue_access(&mut self, user: &str) -> (String, i64) {
        let token = format!("ya29.twin.{}", random_token());
        let ttl = self.token_ttl_secs;
        self.access_tokens.insert(
            token.clone(),
            TokenRec {
                user: user.to_string(),
                expires_at: Utc::now() + Duration::seconds(ttl),
            },
        );
        (token, ttl)
    }

    pub fn mint_token(&mut self, user: &str, scope: Option<String>) -> TokenResponse {
        let (access_token, expires_in) = self.issue_access(user);
        let refresh = format!("1//twin.{}", random_token());
        self.refresh_tokens
            .insert(refresh.clone(), user.to_string());
        TokenResponse {
            access_token,
            refresh_token: Some(refresh),
            expires_in,
            id_token: None,
            scope: scope.or_else(|| Some(prism_proto::SCOPES.join(" "))),
            token_type: "Bearer".into(),
        }
    }

    /// Resolve a bearer token to `users/{id}`; expired tokens are dropped.
    pub fn caller_for(&self, token: &str) -> Option<String> {
        let rec = self.access_tokens.get(token)?;
        if rec.expires_at < Utc::now() {
            return None;
        }
        Some(rec.user.clone())
    }

    pub fn revoke(&mut self, token: &str) -> bool {
        self.access_tokens.remove(token).is_some() || self.refresh_tokens.remove(token).is_some()
    }

    pub fn issue_code(&mut self, code: AuthCode) -> String {
        let value = format!("4/0twin-{}", random_token());
        self.codes.insert(value.clone(), code);
        value
    }

    // ---- messages -----------------------------------------------------------

    /// Server-assigned message id in Google's `{thread}.{message}` shape.
    pub fn new_message_name(&self, space: &str, thread_id: Option<&str>) -> (String, String) {
        let id = random_id();
        let thread_id = thread_id.map(str::to_string).unwrap_or_else(|| id.clone());
        (format!("{space}/messages/{thread_id}.{id}"), thread_id)
    }

    pub fn now_string(&self) -> String {
        now_str()
    }

    // ---- tasks --------------------------------------------------------------

    /// RFC 3339 with milliseconds, the way the Tasks API prints `updated` and `due`.
    pub fn task_time(t: DateTime<Utc>) -> String {
        t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    /// Insert a `needsAction` task with the next `task-{n}` id, `updated` at `now`.
    pub fn new_task(
        &mut self,
        title: &str,
        notes: Option<&str>,
        due: Option<&str>,
        now: DateTime<Utc>,
    ) -> Task {
        self.next_task_id += 1;
        let task = Task {
            id: format!("task-{}", self.next_task_id),
            title: title.to_string(),
            notes: notes.map(str::to_string),
            status: Task::NEEDS_ACTION.into(),
            due: due.map(str::to_string),
            updated: Some(Self::task_time(now)),
            web_view_link: Some("https://tasks.google.com/".into()),
        };
        self.tasks.insert(
            task.id.clone(),
            TaskRec {
                task: task.clone(),
                completed: None,
            },
        );
        task
    }

    /// Tasks in id order, which is creation order.
    pub fn tasks_sorted(&self) -> Vec<&TaskRec> {
        let mut v: Vec<&TaskRec> = self.tasks.values().collect();
        v.sort_by_key(|r| {
            r.task
                .id
                .strip_prefix("task-")
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(u64::MAX)
        });
        v
    }

    // ---- meet ---------------------------------------------------------------

    /// A meeting code in Google's `abc-defg-hij` shape, unique per `n`: `n` is mixed by
    /// an odd multiplier coprime with 26^10, so the map onto ten base-26 letters is a bijection.
    pub fn meet_code(n: u64) -> String {
        const LETTERS: u128 = 26;
        const SPACE: u128 = 141_167_095_653_376; // 26^10
        const MIX: u128 = 2_654_435_761; // odd, not a multiple of 13
        let mut x = (n as u128 * MIX + 7) % SPACE;
        let mut letters = [b'a'; 10];
        for slot in letters.iter_mut().rev() {
            *slot = b'a' + (x % LETTERS) as u8;
            x /= LETTERS;
        }
        let s = std::str::from_utf8(&letters).expect("ascii letters");
        format!("{}-{}-{}", &s[..3], &s[3..7], &s[7..])
    }

    /// Insert a Meet space for `creator` with the next `spaces/meet-{n}` name.
    pub fn new_meet(&mut self, creator: &str, now: DateTime<Utc>) -> MeetRec {
        self.next_meet_id += 1;
        let rec = MeetRec {
            name: format!("spaces/meet-{}", self.next_meet_id),
            code: Self::meet_code(self.next_meet_id),
            created_at: now,
            creator: creator.to_string(),
        };
        self.meets.insert(rec.name.clone(), rec.clone());
        rec
    }

    /// Look a Meet space up by meeting code, `meet-{n}` or `spaces/meet-{n}`.
    pub fn meet(&self, key: &str) -> Option<&MeetRec> {
        let key = key.trim().trim_matches('/');
        let id = key.strip_prefix("spaces/").unwrap_or(key);
        self.meets
            .get(&format!("spaces/{id}"))
            .or_else(|| self.meets.values().find(|m| m.code == key))
    }

    /// The space behind `conferenceRecords/{id}`: the record id is the space id.
    pub fn meet_by_record(&self, record_id: &str) -> Option<&MeetRec> {
        self.meets.get(&format!("spaces/{record_id}"))
    }

    pub fn fmt(&self, t: DateTime<Utc>) -> String {
        fmt_time(t)
    }

    // ---- calendar ----------------------------------------------------------

    /// RFC 3339 to the second with the twin's local offset, the way Calendar prints
    /// `start.dateTime` and `end.dateTime`.
    pub fn cal_time(t: DateTime<Local>) -> String {
        t.to_rfc3339_opts(SecondsFormat::Secs, false)
    }

    /// Advance the change counter and return its new value. Tokens older than
    /// `CAL_SYNC_HISTORY` changes expire here.
    pub fn cal_bump(&mut self) -> u64 {
        self.cal_seq += 1;
        let floor = (self.cal_seq + 1).saturating_sub(CAL_SYNC_HISTORY);
        self.cal_min_sync = self.cal_min_sync.max(floor).max(1);
        self.cal_seq
    }

    /// The token a client should use next: one past the latest change.
    pub fn cal_next_sync_token(&self) -> String {
        (self.cal_seq + 1).to_string()
    }

    /// A token parses, is not older than the history kept and not from the future.
    pub fn cal_sync_token_valid(&self, token: &str) -> Option<u64> {
        let t: u64 = token.trim().parse().ok()?;
        (t >= 1 && t >= self.cal_min_sync && t <= self.cal_seq + 1).then_some(t)
    }

    /// Insert a confirmed event with the next `ev-{n}` id, its `htmlLink`, `created`
    /// and `updated` at `now`, and the change counter advanced.
    pub fn new_cal_event(
        &mut self,
        calendar: &str,
        mut event: CalEvent,
        now: DateTime<Utc>,
    ) -> CalEvent {
        self.next_event_id += 1;
        let id = format!("ev-{}", self.next_event_id);
        event.id = id.clone();
        event.status = Some(CalEvent::CONFIRMED.into());
        event.html_link = Some(format!("https://calendar.google.com/event?eid={id}"));
        event.created = Some(Self::task_time(now));
        event.updated = Some(Self::task_time(now));
        event.calendar_id = None;
        let seq = self.cal_bump();
        self.cal_events.insert(
            id,
            CalEventRec {
                calendar: calendar.to_string(),
                event: event.clone(),
                seq,
            },
        );
        event
    }

    /// Every event of one calendar in id order, cancelled ones included.
    pub fn cal_events_in(&self, calendar: &str) -> Vec<&CalEventRec> {
        let mut v: Vec<&CalEventRec> = self
            .cal_events
            .values()
            .filter(|r| r.calendar == calendar)
            .collect();
        v.sort_by_key(|r| Self::event_number(&r.event.id));
        v
    }

    fn event_number(id: &str) -> u64 {
        id.strip_prefix("ev-")
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or(u64::MAX)
    }

    pub fn cal_event(&self, calendar: &str, id: &str) -> Option<&CalEventRec> {
        self.cal_events.get(id).filter(|r| r.calendar == calendar)
    }

    /// Mutable access with the change counter advanced and `updated` set to `now`:
    /// every caller changes the event.
    pub fn cal_event_mut(
        &mut self,
        calendar: &str,
        id: &str,
        now: DateTime<Utc>,
    ) -> Option<&mut CalEventRec> {
        self.cal_event(calendar, id)?;
        let seq = self.cal_bump();
        let rec = self.cal_events.get_mut(id)?;
        rec.seq = seq;
        rec.event.updated = Some(Self::task_time(now));
        Some(rec)
    }

    /// A Meet code for a calendar event, in a range the huddle spaces never use.
    pub fn cal_meet_code(event_id: &str) -> String {
        Self::meet_code(100_000 + Self::event_number(event_id).min(u64::MAX / 2))
    }

    /// The caller's email, the key that marks `self` on attendees.
    pub fn email_of(&self, user: &str) -> String {
        self.user(user)
            .and_then(|u| u.email.clone())
            .unwrap_or_default()
    }

    // ---- availability -------------------------------------------------------

    /// The caller's presence record, created as the default when first touched.
    pub fn availability_mut(&mut self, user: &str) -> &mut AvailabilityRec {
        self.availability
            .entry(user.to_string())
            .or_insert_with(|| AvailabilityRec {
                state: Availability::ACTIVE.into(),
                ..Default::default()
            })
    }

    // ---- drive --------------------------------------------------------------

    /// Insert an app-data file for `owner` with the next `file-{n}` id.
    pub fn new_drive_file(
        &mut self,
        owner: &str,
        name: &str,
        content_type: &str,
        bytes: Vec<u8>,
        now: DateTime<Utc>,
    ) -> DriveFile {
        self.next_drive_id += 1;
        let file = DriveFile {
            id: format!("file-{}", self.next_drive_id),
            owner: owner.to_string(),
            name: name.to_string(),
            content_type: content_type.to_string(),
            bytes,
            modified_time: Self::task_time(now),
        };
        self.drive_files.insert(file.id.clone(), file.clone());
        file
    }

    /// One of `owner`'s files; another account's file is as good as absent.
    pub fn drive_file(&self, owner: &str, id: &str) -> Option<&DriveFile> {
        self.drive_files.get(id).filter(|f| f.owner == owner)
    }

    pub fn drive_file_mut(&mut self, owner: &str, id: &str) -> Option<&mut DriveFile> {
        self.drive_files.get_mut(id).filter(|f| f.owner == owner)
    }

    /// `owner`'s files in id order, optionally only those named `name`.
    pub fn drive_files_of(&self, owner: &str, name: Option<&str>) -> Vec<&DriveFile> {
        let mut v: Vec<&DriveFile> = self
            .drive_files
            .values()
            .filter(|f| f.owner == owner && name.is_none_or(|n| f.name == n))
            .collect();
        v.sort_by_key(|f| {
            f.id.strip_prefix("file-")
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(u64::MAX)
        });
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let mut s = Store {
            public_base: "http://t".into(),
            token_ttl_secs: 60,
            ..Default::default()
        };
        s.users.insert(
            "users/1002".into(),
            User {
                name: "users/1002".into(),
                display_name: Some("Priya Nair".into()),
                ..Default::default()
            },
        );
        s
    }

    #[test]
    fn mentions_use_utf16_offsets() {
        let s = store();
        let a = s.annotate("👍 hey <users/1002> and <users/9999>");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].start_index, Some(7));
        assert_eq!(a[0].length, Some(12));
        assert_eq!(
            a[0].user_mention
                .as_ref()
                .unwrap()
                .user
                .as_ref()
                .unwrap()
                .name,
            "users/1002"
        );
    }

    #[test]
    fn meet_codes_are_shaped_and_distinct() {
        let codes: Vec<String> = (1..=500).map(Store::meet_code).collect();
        for c in &codes {
            let parts: Vec<&str> = c.split('-').collect();
            assert_eq!(parts.len(), 3, "{c}");
            assert_eq!(parts[0].len(), 3, "{c}");
            assert_eq!(parts[1].len(), 4, "{c}");
            assert_eq!(parts[2].len(), 3, "{c}");
            assert!(
                c.chars().all(|ch| ch.is_ascii_lowercase() || ch == '-'),
                "{c}"
            );
        }
        let distinct: std::collections::HashSet<&String> = codes.iter().collect();
        assert_eq!(distinct.len(), codes.len());
        assert_eq!(Store::meet_code(1), Store::meet_code(1));

        let mut s = store();
        let a = s.new_meet("users/1002", Utc::now());
        assert_eq!(a.name, "spaces/meet-1");
        assert_eq!(
            s.meet(&a.code).map(|m| m.name.as_str()),
            Some("spaces/meet-1")
        );
        assert_eq!(
            s.meet("meet-1").map(|m| m.creator.as_str()),
            Some("users/1002")
        );
        assert_eq!(
            s.meet("spaces/meet-1").map(|m| m.code.as_str()),
            Some(a.code.as_str())
        );
        assert!(s.meet("zzz-zzzz-zzz").is_none());
        assert!(s.meet_by_record("meet-1").is_some());
        assert!(s.meet_by_record("meet-9").is_none());
    }

    #[test]
    fn calendar_sync_tokens_follow_the_change_counter() {
        let mut s = store();
        assert_eq!(s.cal_next_sync_token(), "1", "nothing changed yet");
        assert_eq!(s.cal_sync_token_valid("1"), Some(1));
        assert!(s.cal_sync_token_valid("2").is_none());
        let now = Utc::now();
        let ev = s.new_cal_event(CAL_PRIMARY, CalEvent::default(), now);
        assert_eq!(ev.id, "ev-1");
        assert_eq!(ev.status.as_deref(), Some("confirmed"));
        assert_eq!(s.cal_seq, 1);
        assert_eq!(s.cal_next_sync_token(), "2");
        assert_eq!(s.cal_sync_token_valid("1"), Some(1));
        assert_eq!(s.cal_sync_token_valid("2"), Some(2));
        assert!(s.cal_sync_token_valid("3").is_none(), "from the future");
        assert!(s.cal_sync_token_valid("0").is_none());
        assert!(s.cal_sync_token_valid("abc").is_none());
        assert!(s.cal_event_mut(CAL_TEAM, "ev-1", now).is_none());
        assert!(s.cal_event_mut(CAL_PRIMARY, "ev-1", now).is_some());
        assert_eq!(s.cal_events["ev-1"].seq, 2);
        for _ in 0..CAL_SYNC_HISTORY {
            s.cal_bump();
        }
        assert!(s.cal_sync_token_valid("2").is_none(), "too old");
        assert_eq!(
            s.cal_sync_token_valid(&s.cal_next_sync_token()),
            Some(s.cal_seq + 1)
        );
        assert_ne!(Store::cal_meet_code("ev-1"), Store::meet_code(1));

        let f = s.new_drive_file(
            "users/1002",
            "prism-notes.json",
            "application/json",
            b"{}".to_vec(),
            now,
        );
        assert_eq!(f.id, "file-1");
        assert!(s.drive_file("users/1002", "file-1").is_some());
        assert!(s.drive_file("users/1001", "file-1").is_none());
        assert_eq!(
            s.drive_files_of("users/1002", Some("prism-notes.json"))
                .len(),
            1
        );
        assert!(s.drive_files_of("users/1002", Some("other")).is_empty());
    }

    #[test]
    fn tokens_expire() {
        let mut s = store();
        let (t, _) = s.issue_access("users/1002");
        assert_eq!(s.caller_for(&t).as_deref(), Some("users/1002"));
        s.access_tokens.get_mut(&t).unwrap().expires_at = Utc::now() - Duration::seconds(1);
        assert!(s.caller_for(&t).is_none());
    }
}
