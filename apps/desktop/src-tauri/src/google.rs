//! Typed reqwest client over the Chat API table in docs/ARCHITECTURE.md. Bearer injected from
//! `Auth`; 401 → refresh once and retry; 429/503 → exponential backoff (3 tries,
//! capped at 32 s); Google error bodies become `ApiError`.

use crate::auth::Auth;
use crate::shell::space_of;
use confab_proto::availability::Availability;
use confab_proto::calendar::{Attendee, CalEvent, CalendarEntry, EventInput, EventsPage};
use confab_proto::chat::*;
use confab_proto::meet::{MeetParticipant, MeetSpace, MeetStatus};
use confab_proto::tasks::{ListTasksResponse, Task};
use reqwest::{Client, RequestBuilder};
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_RETRIES: u32 = 3;
const BACKOFF_CAP: Duration = Duration::from_secs(32);
/// 12 reads per second per space.
const READ_INTERVAL: Duration = Duration::from_millis(84);
/// 1 write per second per space.
const WRITE_INTERVAL: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    /// HTTP status, 0 for transport failures.
    pub status: u16,
    pub message: String,
}

impl ApiError {
    fn from_google(status: u16, body: &str) -> Self {
        let message = match serde_json::from_str::<ErrorBody>(body) {
            Ok(e) if !e.error.message.is_empty() => {
                format!("{} ({})", e.error.message, e.error.status)
            }
            _ => {
                let snippet: String = body.chars().take(200).collect();
                if snippet.trim().is_empty() {
                    format!("HTTP {status}")
                } else {
                    format!("HTTP {status}: {snippet}")
                }
            }
        };
        Self { status, message }
    }
    fn transport(e: reqwest::Error) -> Self {
        Self {
            status: 0,
            message: format!("network: {e}"),
        }
    }
    pub fn is_not_found(&self) -> bool {
        self.status == 404
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

impl From<ApiError> for String {
    fn from(e: ApiError) -> String {
        e.message
    }
}

struct Lanes {
    next_read: Instant,
    next_write: Instant,
}

pub struct Google {
    http: Client,
    auth: Arc<Auth>,
    base: String,
    /// People API base (directory search); the twin serves it on its own base.
    people_base: String,
    /// Tasks API base (the default task list); the twin serves it on its own base.
    tasks_base: String,
    /// Meet REST API base (spaces and conference records); the twin serves it on its own base.
    meet_base: String,
    /// Calendar API v3 base including `/calendar/v3`; the twin serves it under its own base.
    calendar_base: String,
    /// Drive API v3 base including `/drive/v3`, for the app-data notes file.
    drive_base: String,
    /// Drive upload base (`/upload/drive/v3`), derived from `drive_base`.
    drive_upload_base: String,
    /// note: a min-interval reservation per space (reads and writes on separate
    /// lanes) stands in for a real token bucket; bursts queue in order, nothing is dropped.
    throttle: Mutex<HashMap<String, Lanes>>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateMessageBody<'a> {
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread: Option<Thread>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachment: Vec<Attachment>,
    /// Only a huddle's `RICH_LINK` is ever sent; mentions are Google's to compute.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    annotations: Vec<Annotation>,
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(1u64 << attempt.min(6)).min(BACKOFF_CAP)
}

impl Google {
    // note: one base per Google API keeps the call sites explicit; a bases struct would hide which host each call hits.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        http: Client,
        auth: Arc<Auth>,
        chat_api: &str,
        people_api: &str,
        tasks_api: &str,
        meet_api: &str,
        calendar_api: &str,
        drive_api: &str,
    ) -> Self {
        Self {
            http,
            auth,
            base: chat_api.trim_end_matches('/').to_string(),
            people_base: people_api.trim_end_matches('/').to_string(),
            tasks_base: tasks_api.trim_end_matches('/').to_string(),
            meet_base: meet_api.trim_end_matches('/').to_string(),
            calendar_base: calendar_api.trim_end_matches('/').to_string(),
            drive_base: drive_api.trim_end_matches('/').to_string(),
            drive_upload_base: confab_proto::drive_upload_api(drive_api),
            throttle: Mutex::new(HashMap::new()),
        }
    }

    async fn throttle(&self, space: &str, write: bool) {
        let wait = {
            let mut lanes = self.throttle.lock().unwrap_or_else(|p| p.into_inner());
            let now = Instant::now();
            let lane = lanes.entry(space.to_string()).or_insert(Lanes {
                next_read: now,
                next_write: now,
            });
            let (slot, interval) = if write {
                (&mut lane.next_write, WRITE_INTERVAL)
            } else {
                (&mut lane.next_read, READ_INTERVAL)
            };
            let at = (*slot).max(now);
            *slot = at + interval;
            at.saturating_duration_since(now)
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }

    /// One call with the retry policy. `build` is re-invoked per attempt.
    async fn call_raw<F>(
        &self,
        space: Option<&str>,
        write: bool,
        build: F,
    ) -> Result<Vec<u8>, ApiError>
    where
        F: Fn(&Client) -> RequestBuilder,
    {
        let resp = self.call_ok(space, write, build).await?;
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(ApiError::transport)
    }

    /// The retry loop itself; returns the successful response unread.
    async fn call_ok<F>(
        &self,
        space: Option<&str>,
        write: bool,
        build: F,
    ) -> Result<reqwest::Response, ApiError>
    where
        F: Fn(&Client) -> RequestBuilder,
    {
        if let Some(s) = space {
            self.throttle(s, write).await;
        }
        let mut refreshed = false;
        let mut attempt = 0u32;
        loop {
            let token = self.auth.access_token().await.map_err(|m| ApiError {
                status: 401,
                message: m,
            })?;
            let resp = match build(&self.http).bearer_auth(&token).send().await {
                Ok(r) => r,
                Err(e) => {
                    if attempt < MAX_RETRIES && (e.is_connect() || e.is_timeout()) {
                        attempt += 1;
                        tokio::time::sleep(backoff(attempt)).await;
                        continue;
                    }
                    return Err(ApiError::transport(e));
                }
            };
            let status = resp.status();
            if status.is_success() {
                return Ok(resp);
            }
            let body = resp.text().await.unwrap_or_default();
            if status.as_u16() == 401 && !refreshed {
                refreshed = true;
                self.auth
                    .force_refresh(&token)
                    .await
                    .map_err(|m| ApiError {
                        status: 401,
                        message: m,
                    })?;
                continue;
            }
            if matches!(status.as_u16(), 429 | 503) && attempt < MAX_RETRIES {
                attempt += 1;
                let wait = backoff(attempt);
                tracing::warn!("google {status}, retry {attempt}/{MAX_RETRIES} in {wait:?}");
                tokio::time::sleep(wait).await;
                continue;
            }
            return Err(ApiError::from_google(status.as_u16(), &body));
        }
    }

    async fn call_json<T, F>(
        &self,
        space: Option<&str>,
        write: bool,
        build: F,
    ) -> Result<T, ApiError>
    where
        T: DeserializeOwned,
        F: Fn(&Client) -> RequestBuilder,
    {
        let bytes = self.call_raw(space, write, build).await?;
        let bytes = if bytes.iter().all(u8::is_ascii_whitespace) {
            b"{}".to_vec()
        } else {
            bytes
        };
        serde_json::from_slice(&bytes).map_err(|e| ApiError {
            status: 0,
            message: format!("bad response from Google: {e}"),
        })
    }

    fn v1(&self, path: &str) -> String {
        format!("{}/v1/{}", self.base, path.trim_start_matches('/'))
    }

    // ----- people -----

    /// People API directory search. A person's `people/{id}` is Chat's `users/{id}`.
    pub async fn search_people(&self, query: &str) -> Result<Vec<User>, ApiError> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/v1/people:searchDirectoryPeople", self.people_base);
        let r: SearchPeopleResponse = self
            .call_json(None, false, |c| {
                c.get(&url).query(&[
                    ("query", query),
                    ("readMask", "names,emailAddresses,photos"),
                    ("sources", "DIRECTORY_SOURCE_TYPE_DOMAIN_PROFILE"),
                    ("pageSize", "20"),
                ])
            })
            .await?;
        Ok(r.people.into_iter().filter_map(person_to_user).collect())
    }

    /// `spaces:setup`: the DM with one user (existing or new), or a new named space
    /// with the caller as manager and `members` (`users/{id}`) joined.
    pub async fn setup_space(
        &self,
        space_type: &str,
        display_name: Option<&str>,
        members: &[String],
    ) -> Result<Space, ApiError> {
        let url = self.v1("spaces:setup");
        let mut space = serde_json::json!({ "spaceType": space_type });
        if let Some(n) = display_name {
            space["displayName"] = serde_json::json!(n);
        }
        let body = serde_json::json!({
            "space": space,
            "memberships": members
                .iter()
                .map(|m| serde_json::json!({ "member": { "name": m, "type": "HUMAN" } }))
                .collect::<Vec<_>>(),
        });
        self.call_json(None, true, |c| c.post(&url).json(&body))
            .await
    }

    // ----- tasks (Google Tasks API, the default list) -----

    fn tasks_url(&self, id: Option<&str>) -> String {
        let list = format!("{}/tasks/v1/lists/@default/tasks", self.tasks_base);
        match id {
            Some(id) => format!("{list}/{id}"),
            None => list,
        }
    }

    /// Every task in the default list, completed ones included, hidden ones not.
    // note: one page of 100; the default list of one person rarely has more.
    pub async fn list_tasks(&self) -> Result<Vec<Task>, ApiError> {
        let url = self.tasks_url(None);
        let r: ListTasksResponse = self
            .call_json(None, false, |c| {
                c.get(&url).query(&[
                    ("showCompleted", "true"),
                    ("showHidden", "false"),
                    ("maxResults", "100"),
                ])
            })
            .await?;
        Ok(r.items)
    }

    /// `due` is RFC 3339; Google keeps the date and drops the time.
    pub async fn add_task(
        &self,
        title: &str,
        notes: Option<&str>,
        due: Option<&str>,
    ) -> Result<Task, ApiError> {
        let url = self.tasks_url(None);
        let mut body = serde_json::json!({ "title": title });
        if let Some(n) = notes.filter(|n| !n.trim().is_empty()) {
            body["notes"] = serde_json::json!(n);
        }
        if let Some(d) = due.filter(|d| !d.trim().is_empty()) {
            body["due"] = serde_json::json!(d);
        }
        self.call_json(None, true, |c| c.post(&url).json(&body))
            .await
    }

    /// `completed` with its timestamp set by Google, or back to `needsAction`
    /// (an explicit null `completed` clears the timestamp).
    pub async fn set_task_done(&self, id: &str, done: bool) -> Result<Task, ApiError> {
        let url = self.tasks_url(Some(id));
        let body = if done {
            serde_json::json!({ "status": Task::COMPLETED })
        } else {
            serde_json::json!({ "status": Task::NEEDS_ACTION, "completed": null })
        };
        self.call_json(None, true, |c| c.patch(&url).json(&body))
            .await
    }

    /// 204 with no body.
    pub async fn delete_task(&self, id: &str) -> Result<(), ApiError> {
        let url = self.tasks_url(Some(id));
        self.call_raw(None, true, |c| c.delete(&url))
            .await
            .map(|_| ())
    }

    // ----- meet (Google Meet REST API v2) -----

    fn meet_url(&self, path: &str) -> String {
        format!("{}/v2/{}", self.meet_base, path.trim_start_matches('/'))
    }

    /// `spaces.create` with `accessType: TRUSTED` (anyone in the org joins without knocking).
    /// Google answers `{name, meetingUri, meetingCode, config}`; `config` is dropped.
    pub async fn create_meet(&self) -> Result<MeetSpace, ApiError> {
        let url = self.meet_url("spaces");
        let body = serde_json::json!({ "config": { "accessType": "TRUSTED" } });
        self.call_json(None, true, |c| c.post(&url).json(&body))
            .await
    }

    /// `spaces.get` by meeting code (`abc-defg-hij`) or full `spaces/{id}` name, then,
    /// while a conference is active, `conferenceRecords.participants.list` filtered to
    /// the people still in the call (no `latestEndTime`).
    // note: one page of 100 participants; a huddle never has more.
    pub async fn meet_status(&self, code: &str) -> Result<MeetStatus, ApiError> {
        let code = code.trim().trim_matches('/');
        if code.is_empty() {
            return Err(ApiError {
                status: 400,
                message: "a meeting code is needed".into(),
            });
        }
        let path = if code.starts_with("spaces/") {
            code.to_string()
        } else {
            format!("spaces/{code}")
        };
        let url = self.meet_url(&path);
        let space: MeetSpaceResponse =
            self.call_json(None, false, |c| c.get(&url))
                .await
                .map_err(|e| {
                    if e.is_not_found() {
                        ApiError {
                            status: 404,
                            message: format!("No Meet space for {code}."),
                        }
                    } else {
                        e
                    }
                })?;
        let Some(record) = space
            .active_conference
            .and_then(|a| a.conference_record)
            .filter(|r| !r.trim().is_empty())
        else {
            return Ok(MeetStatus::default());
        };
        let url = self.meet_url(&format!("{record}/participants"));
        let r: ParticipantsResponse = self
            .call_json(None, false, |c| c.get(&url).query(&[("pageSize", "100")]))
            .await?;
        Ok(MeetStatus {
            active: true,
            conference_record: Some(record),
            participants: present_participants(r.participants),
        })
    }

    // ----- availability (Chat `users.availability`, the signed-in user) -----

    /// `GET v1/users/me/availability`.
    pub async fn availability_get(&self) -> Result<Availability, ApiError> {
        let url = self.v1("users/me/availability");
        self.call_json(None, false, |c| c.get(&url)).await
    }

    /// `markAsActive` (optional `ttl`, Google's default is thirty minutes), `markAsAway`
    /// (no body) or `markAsDoNotDisturb` (`ttl`, an hour when none is given).
    pub async fn availability_mark(
        &self,
        state: &str,
        ttl_seconds: Option<u64>,
    ) -> Result<Availability, ApiError> {
        let (verb, body) = match state.trim().to_ascii_lowercase().as_str() {
            "active" => (
                "markAsActive",
                match ttl_seconds {
                    Some(s) => serde_json::json!({ "ttl": duration_field(s) }),
                    None => serde_json::json!({}),
                },
            ),
            "away" => ("markAsAway", serde_json::json!({})),
            "dnd" | "do_not_disturb" => (
                "markAsDoNotDisturb",
                serde_json::json!({ "ttl": duration_field(ttl_seconds.unwrap_or(DND_DEFAULT_SECS)) }),
            ),
            other => {
                return Err(ApiError {
                    status: 400,
                    message: format!("state must be active, away or dnd, not {other:?}"),
                })
            }
        };
        let url = self.v1(&format!("users/me/availability:{verb}"));
        self.call_json(None, true, |c| c.post(&url).json(&body))
            .await
    }

    /// `PATCH v1/users/me/availability?updateMask=customStatus`. Blank `text` clears
    /// the status (`{customStatus: {}}`; if Google refuses that shape with a 400 the
    /// mask is sent with an empty body instead). `emoji` defaults to a speech balloon.
    pub async fn availability_status(
        &self,
        text: Option<&str>,
        emoji: Option<&str>,
        ttl_seconds: Option<u64>,
    ) -> Result<Availability, ApiError> {
        let url = self.v1("users/me/availability");
        let query = [("updateMask", "customStatus")];
        let text = text.map(str::trim).filter(|t| !t.is_empty());
        let Some(text) = text else {
            let clear = serde_json::json!({ "customStatus": {} });
            let first = self
                .call_json(None, true, |c| c.patch(&url).query(&query).json(&clear))
                .await;
            return match first {
                Err(e) if e.status == 400 => {
                    tracing::info!(
                        "clearing with an empty customStatus refused ({e}), sending the mask alone"
                    );
                    let empty = serde_json::json!({});
                    self.call_json(None, true, |c| c.patch(&url).query(&query).json(&empty))
                        .await
                }
                other => other,
            };
        };
        if text.chars().count() > Availability::STATUS_TEXT_MAX {
            return Err(ApiError {
                status: 400,
                message: format!(
                    "a status is at most {} characters",
                    Availability::STATUS_TEXT_MAX
                ),
            });
        }
        let emoji = emoji
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .unwrap_or(DEFAULT_STATUS_EMOJI);
        let mut status = serde_json::json!({ "text": text, "emoji": { "unicode": emoji } });
        if let Some(s) = ttl_seconds.filter(|s| *s > 0) {
            status["ttl"] = serde_json::json!(duration_field(s));
        }
        let body = serde_json::json!({ "customStatus": status });
        self.call_json(None, true, |c| c.patch(&url).query(&query).json(&body))
            .await
    }

    // ----- calendar (Google Calendar API v3) -----

    /// `{calendar_base}/calendars/{id}/events[/{event}]` with the ids percent-encoded.
    fn events_url(&self, calendar_id: &str, event_id: Option<&str>) -> String {
        let list = format!(
            "{}/calendars/{}/events",
            self.calendar_base,
            path_segment(calendar_id)
        );
        match event_id {
            Some(e) => format!("{list}/{}", path_segment(e)),
            None => list,
        }
    }

    /// `calendarList.list` with `minAccessRole=reader`: every calendar the user can read.
    pub async fn calendar_list(&self) -> Result<Vec<CalendarEntry>, ApiError> {
        let url = format!("{}/users/me/calendarList", self.calendar_base);
        let mut out = Vec::new();
        let mut page = String::new();
        loop {
            let r: CalendarListResponse = self
                .call_json(None, false, |c| {
                    c.get(&url).query(&[
                        ("minAccessRole", "reader"),
                        ("maxResults", "250"),
                        ("pageToken", page.as_str()),
                    ])
                })
                .await?;
            out.extend(r.items);
            match r.next_page_token.filter(|t| !t.is_empty()) {
                Some(t) => page = t,
                None => return Ok(out),
            }
        }
    }

    /// `events.list` as single instances, deleted ones included: a window
    /// (`timeMin`/`timeMax`, ordered by start) or, with a sync token, only what changed
    /// since (no bounds, no ordering). Pages are followed and merged; every item gets
    /// `calendarId`. A 410 means the token expired: the error message starts with
    /// `sync_expired` so the caller refetches the window from scratch.
    pub async fn calendar_events(
        &self,
        calendar_id: &str,
        time_min: Option<&str>,
        time_max: Option<&str>,
        sync_token: Option<&str>,
    ) -> Result<EventsPage, ApiError> {
        let url = self.events_url(calendar_id, None);
        let mut query: Vec<(&str, String)> = vec![
            ("singleEvents", "true".into()),
            ("maxResults", "250".into()),
            ("showDeleted", "true".into()),
        ];
        match sync_token.filter(|t| !t.trim().is_empty()) {
            Some(t) => query.push(("syncToken", t.to_string())),
            None => {
                if let Some(t) = time_min.filter(|t| !t.trim().is_empty()) {
                    query.push(("timeMin", t.to_string()));
                }
                if let Some(t) = time_max.filter(|t| !t.trim().is_empty()) {
                    query.push(("timeMax", t.to_string()));
                }
                query.push(("orderBy", "startTime".into()));
            }
        }
        let mut merged = EventsPage::default();
        let mut page = String::new();
        loop {
            let r: EventsPage = self
                .call_json(None, false, |c| {
                    let r = c.get(&url).query(&query);
                    if page.is_empty() {
                        r
                    } else {
                        r.query(&[("pageToken", page.as_str())])
                    }
                })
                .await
                .map_err(|e| {
                    if e.status == 410 {
                        ApiError {
                            status: 410,
                            message: format!("sync_expired: {}", e.message),
                        }
                    } else {
                        e
                    }
                })?;
            merged.items.extend(r.items.into_iter().map(|mut ev| {
                ev.calendar_id = Some(calendar_id.to_string());
                ev
            }));
            if let Some(t) = r.next_sync_token.filter(|t| !t.is_empty()) {
                merged.next_sync_token = Some(t);
            }
            match r.next_page_token.filter(|t| !t.is_empty()) {
                Some(t) => page = t,
                None => return Ok(merged),
            }
        }
    }

    /// `events.insert` with `sendUpdates=all`; `addMeet` asks Google for a Meet
    /// conference (`conferenceDataVersion=1` plus a `createRequest`).
    pub async fn calendar_create_event(
        &self,
        calendar_id: &str,
        event: &EventInput,
    ) -> Result<CalEvent, ApiError> {
        let url = self.events_url(calendar_id, None);
        let mut body = event_body(event);
        let mut query = vec![("sendUpdates", "all")];
        if event.add_meet {
            body["conferenceData"] = meet_create_request();
            query.push(("conferenceDataVersion", "1"));
        }
        let mut created: CalEvent = self
            .call_json(None, true, |c| c.post(&url).query(&query).json(&body))
            .await?;
        created.calendar_id = Some(calendar_id.to_string());
        Ok(created)
    }

    /// `events.patch` with `sendUpdates=all`: the input's fields replace the event's
    /// (attendees as a whole list, recurrence only when given). `addMeet` on an event
    /// without a conference adds one the way `calendar_create_event` does.
    pub async fn calendar_update_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        event: &EventInput,
    ) -> Result<CalEvent, ApiError> {
        let url = self.events_url(calendar_id, Some(event_id));
        let mut body = event_body(event);
        let mut query = vec![("sendUpdates", "all")];
        if event.add_meet {
            let current: CalEvent = self.call_json(None, false, |c| c.get(&url)).await?;
            let has_meet = current
                .hangout_link
                .as_deref()
                .is_some_and(|l| !l.trim().is_empty())
                || current
                    .conference_data
                    .as_ref()
                    .is_some_and(|c| !c.entry_points.is_empty());
            if !has_meet {
                body["conferenceData"] = meet_create_request();
                query.push(("conferenceDataVersion", "1"));
            }
        }
        let mut updated: CalEvent = self
            .call_json(None, true, |c| c.patch(&url).query(&query).json(&body))
            .await?;
        updated.calendar_id = Some(calendar_id.to_string());
        Ok(updated)
    }

    /// RSVP: the event is fetched, the attendee marked `self` gets `responseStatus`
    /// (`accepted` | `declined` | `tentative`), and the whole attendee list is patched
    /// back with `sendUpdates=all`.
    pub async fn calendar_respond(
        &self,
        calendar_id: &str,
        event_id: &str,
        response: &str,
    ) -> Result<CalEvent, ApiError> {
        let response = response.trim();
        if !Attendee::is_response(response) {
            return Err(ApiError {
                status: 400,
                message: format!(
                    "response must be accepted, declined or tentative, not {response:?}"
                ),
            });
        }
        let url = self.events_url(calendar_id, Some(event_id));
        let current: CalEvent = self.call_json(None, false, |c| c.get(&url)).await?;
        let attendees = respond_as_self(&current.attendees, response).ok_or_else(|| ApiError {
            status: 400,
            message: "you are not an attendee of this event".into(),
        })?;
        let body = serde_json::json!({ "attendees": attendees });
        let mut updated: CalEvent = self
            .call_json(None, true, |c| {
                c.patch(&url).query(&[("sendUpdates", "all")]).json(&body)
            })
            .await?;
        updated.calendar_id = Some(calendar_id.to_string());
        Ok(updated)
    }

    /// `events.delete` with `sendUpdates=all`; 204 with no body.
    pub async fn calendar_delete_event(
        &self,
        calendar_id: &str,
        event_id: &str,
    ) -> Result<(), ApiError> {
        let url = self.events_url(calendar_id, Some(event_id));
        self.call_raw(None, true, |c| {
            c.delete(&url).query(&[("sendUpdates", "all")])
        })
        .await
        .map(|_| ())
    }

    // ----- notes (one JSON file in the Drive app-data folder) -----

    /// The notes file's metadata when it exists: `files.list` in `appDataFolder` by name.
    async fn notes_file(&self) -> Result<Option<DriveFile>, ApiError> {
        let url = format!("{}/files", self.drive_base);
        let r: DriveFileList = self
            .call_json(None, false, |c| {
                c.get(&url).query(&[
                    ("spaces", "appDataFolder"),
                    ("q", NOTES_QUERY),
                    ("fields", "files(id,name,modifiedTime)"),
                ])
            })
            .await?;
        Ok(r.files.into_iter().find(|f| !f.id.is_empty()))
    }

    /// The notes JSON as stored, `None` before the first save.
    pub async fn notes_load(&self) -> Result<Option<String>, ApiError> {
        let Some(file) = self.notes_file().await? else {
            return Ok(None);
        };
        let url = format!("{}/files/{}", self.drive_base, path_segment(&file.id));
        let bytes = self
            .call_raw(None, false, |c| c.get(&url).query(&[("alt", "media")]))
            .await?;
        Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// Writes the notes JSON: a media `PATCH` over the existing file, else a
    /// `multipart/related` create (`{name, parents: [appDataFolder]}` plus the content).
    pub async fn notes_save(&self, json: &str) -> Result<(), ApiError> {
        match self.notes_file().await? {
            Some(file) => {
                let url = format!(
                    "{}/files/{}",
                    self.drive_upload_base,
                    path_segment(&file.id)
                );
                self.call_raw(None, true, |c| {
                    c.patch(&url)
                        .query(&[("uploadType", "media")])
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .body(json.to_string())
                })
                .await
                .map(|_| ())
            }
            None => {
                let url = format!("{}/files", self.drive_upload_base);
                let boundary = format!("prism-notes-{}", uuid::Uuid::new_v4().simple());
                let body = multipart_related(
                    &boundary,
                    &serde_json::json!({ "name": NOTES_FILE, "parents": ["appDataFolder"] })
                        .to_string(),
                    json,
                );
                let content_type = format!("multipart/related; boundary={boundary}");
                self.call_raw(None, true, |c| {
                    c.post(&url)
                        .query(&[("uploadType", "multipart")])
                        .header(reqwest::header::CONTENT_TYPE, content_type.as_str())
                        .body(body.clone())
                })
                .await
                .map(|_| ())
            }
        }
    }

    // ----- spaces -----

    pub async fn list_spaces(&self) -> Result<Vec<Space>, ApiError> {
        let url = self.v1("spaces");
        let mut out = Vec::new();
        let mut page = String::new();
        loop {
            let r: ListSpacesResponse = self
                .call_json(None, false, |c| {
                    c.get(&url)
                        .query(&[("pageSize", "100"), ("pageToken", page.as_str())])
                })
                .await?;
            out.extend(r.spaces);
            match r.next_page_token.filter(|t| !t.is_empty()) {
                Some(t) => page = t,
                None => return Ok(out),
            }
        }
    }

    pub async fn get_space(&self, name: &str) -> Result<Space, ApiError> {
        let url = self.v1(name);
        self.call_json(Some(name), false, |c| c.get(&url)).await
    }

    // ----- members -----

    pub async fn list_members(&self, space: &str) -> Result<Vec<Membership>, ApiError> {
        let url = self.v1(&format!("{space}/members"));
        let mut out = Vec::new();
        let mut page = String::new();
        loop {
            let r: ListMembershipsResponse = self
                .call_json(Some(space), false, |c| {
                    c.get(&url)
                        .query(&[("pageSize", "100"), ("pageToken", page.as_str())])
                })
                .await?;
            out.extend(r.memberships);
            match r.next_page_token.filter(|t| !t.is_empty()) {
                Some(t) => page = t,
                None => return Ok(out),
            }
        }
    }

    /// `user` is `users/{id}`. 403/404 when not a member.
    /// `spaces.members.create`: adds one human user to a space (the caller must be a member).
    pub async fn add_member(&self, space: &str, user: &str) -> Result<Membership, ApiError> {
        let url = self.v1(&format!("{space}/members"));
        let body = serde_json::json!({ "member": { "name": user, "type": "HUMAN" } });
        self.call_json(Some(space), true, |c| c.post(&url).json(&body))
            .await
    }

    /// `spaces.members.delete` on the caller's own membership: leaves the space.
    pub async fn leave_space(&self, space: &str, me: &str) -> Result<(), ApiError> {
        let mine = self.my_membership(space, me).await?;
        let url = self.v1(&mine.name);
        self.call_raw(Some(space), true, |c| c.delete(&url))
            .await
            .map(|_| ())
    }

    pub async fn my_membership(&self, space: &str, user: &str) -> Result<Membership, ApiError> {
        let url = self.v1(&format!("{space}/members/{user}"));
        self.call_json(Some(space), false, |c| c.get(&url)).await
    }

    // ----- messages -----

    pub async fn list_messages(
        &self,
        space: &str,
        page_token: Option<&str>,
        filter: Option<&str>,
        page_size: u32,
    ) -> Result<ListMessagesResponse, ApiError> {
        let url = self.v1(&format!("{space}/messages"));
        let size = page_size.to_string();
        // showDeleted keeps Google's tombstones in the page, so "Message deleted" survives a reload.
        let mut q: Vec<(&str, &str)> = vec![
            ("pageSize", size.as_str()),
            ("orderBy", "createTime desc"),
            ("showDeleted", "true"),
        ];
        if let Some(t) = page_token.filter(|t| !t.is_empty()) {
            q.push(("pageToken", t));
        }
        if let Some(f) = filter.filter(|f| !f.is_empty()) {
            q.push(("filter", f));
        }
        self.call_json(Some(space), false, |c| c.get(&url).query(&q))
            .await
    }

    pub async fn get_message(&self, name: &str) -> Result<Message, ApiError> {
        let url = self.v1(name);
        let space = space_of(name);
        self.call_json(Some(&space), false, |c| c.get(&url)).await
    }

    /// `client_id` is the bare id; Google sees `messageId=client-{client_id}`.
    pub async fn create_message(
        &self,
        space: &str,
        client_id: &str,
        text: &str,
        thread: Option<&str>,
        attachments: &[Attachment],
    ) -> Result<Message, ApiError> {
        let url = self.v1(&format!("{space}/messages"));
        let message_id = format!("client-{client_id}");
        let body = CreateMessageBody {
            text,
            thread: thread.map(|t| Thread {
                name: Some(t.to_string()),
                thread_key: None,
            }),
            // note: only the attachmentDataRef matters to Google; the rest of the Attachment is ignored.
            attachment: attachments
                .iter()
                .map(|a| Attachment {
                    attachment_data_ref: a.attachment_data_ref.clone(),
                    ..Default::default()
                })
                .collect(),
            annotations: Vec::new(),
        };
        // Google rejects the reply option without a thread ("does not specify which message
        // to reply to"), so a top-level message sends only the client id.
        let mut query = vec![("messageId", message_id.as_str())];
        if thread.is_some() {
            query.push(("messageReplyOption", "REPLY_MESSAGE_FALLBACK_TO_NEW_THREAD"));
        }
        self.call_json(Some(space), true, |c| {
            c.post(&url).query(&query).json(&body)
        })
        .await
    }

    /// The message Google itself posts for a huddle: no text, one `RICH_LINK` annotation
    /// to the Meet space with `huddleStatus: STARTED`. Sent with `messageId=client-{client_id}`
    /// like any other send. Google may treat `annotations` as output only and answer 400;
    /// then a plain "Started a huddle" message with the link goes out under the same id
    /// (a 409 on that retry means the first one was accepted after all, so it is fetched).
    pub async fn create_huddle_message(
        &self,
        space: &str,
        client_id: &str,
        meeting_uri: &str,
        meeting_code: &str,
        mention_all: bool,
    ) -> Result<Message, ApiError> {
        let url = self.v1(&format!("{space}/messages"));
        let message_id = format!("client-{client_id}");
        let query = [("messageId", message_id.as_str())];
        let body = CreateMessageBody {
            text: "",
            thread: None,
            attachment: Vec::new(),
            annotations: vec![Annotation::huddle(
                meeting_uri,
                meeting_code,
                Annotation::HUDDLE_STARTED,
            )],
        };
        let first = self
            .call_json(Some(space), true, |c| {
                c.post(&url).query(&query).json(&body)
            })
            .await;
        let err = match first {
            Ok(m) => return Ok(m),
            Err(e) if e.status == 400 => e,
            Err(e) => return Err(e),
        };
        tracing::info!("huddle annotation refused ({err}), posting the link as text");
        // No public API rings anyone, so the message is what reaches people on Google Chat: a mention makes
        // Google notify the members of a space, while a direct message notifies by itself.
        let mut text = huddle_fallback_text(meeting_uri);
        if mention_all {
            text.push_str(" <users/all>");
        }
        match self
            .create_message(space, client_id, &text, None, &[])
            .await
        {
            Err(e) if e.status == 409 => {
                self.get_message(&format!("{space}/messages/{message_id}"))
                    .await
            }
            other => other,
        }
    }

    pub async fn update_message(&self, name: &str, text: &str) -> Result<Message, ApiError> {
        let url = self.v1(name);
        let space = space_of(name);
        let body = serde_json::json!({ "text": text });
        self.call_json(Some(&space), true, |c| {
            c.patch(&url).query(&[("updateMask", "text")]).json(&body)
        })
        .await
    }

    /// `force` also deletes the thread's replies; Google refuses a parent with replies without it.
    pub async fn delete_message(&self, name: &str, force: bool) -> Result<(), ApiError> {
        let url = self.v1(name);
        let space = space_of(name);
        self.call_raw(Some(&space), true, |c| {
            let r = c.delete(&url);
            if force {
                r.query(&[("force", "true")])
            } else {
                r
            }
        })
        .await
        .map(|_| ())
    }

    // ----- reactions -----

    pub async fn list_reactions(
        &self,
        message: &str,
        filter: Option<&str>,
    ) -> Result<Vec<Reaction>, ApiError> {
        let url = self.v1(&format!("{message}/reactions"));
        let space = space_of(message);
        let filter = filter.unwrap_or("");
        let r: ListReactionsResponse = self
            .call_json(Some(&space), false, |c| {
                c.get(&url).query(&[("filter", filter)])
            })
            .await?;
        Ok(r.reactions)
    }

    pub async fn create_reaction(
        &self,
        message: &str,
        unicode: &str,
    ) -> Result<Reaction, ApiError> {
        let url = self.v1(&format!("{message}/reactions"));
        let space = space_of(message);
        let body = serde_json::json!({ "emoji": { "unicode": unicode } });
        self.call_json(Some(&space), true, |c| c.post(&url).json(&body))
            .await
    }

    pub async fn delete_reaction(&self, name: &str) -> Result<(), ApiError> {
        let url = self.v1(name);
        let space = space_of(name);
        self.call_raw(Some(&space), true, |c| c.delete(&url))
            .await
            .map(|_| ())
    }

    // ----- read state -----

    pub async fn get_read_state(&self, space: &str) -> Result<SpaceReadState, ApiError> {
        let url = self.v1(&format!("users/me/{space}/spaceReadState"));
        self.call_json(Some(space), false, |c| c.get(&url)).await
    }

    pub async fn update_read_state(
        &self,
        space: &str,
        last_read_time: &str,
    ) -> Result<SpaceReadState, ApiError> {
        let url = self.v1(&format!("users/me/{space}/spaceReadState"));
        let body = serde_json::json!({ "lastReadTime": last_read_time });
        self.call_json(Some(space), true, |c| {
            c.patch(&url)
                .query(&[("updateMask", "lastReadTime")])
                .json(&body)
        })
        .await
    }

    // ----- search (twin only; the desktop searches its FTS cache) -----

    pub async fn search(&self, query: &str) -> Result<Vec<Message>, ApiError> {
        let url = self.v1("spaces/-/messages:search");
        let r: SearchMessagesResponse = self
            .call_json(None, false, |c| {
                c.get(&url).query(&[("query", query), ("pageSize", "50")])
            })
            .await?;
        Ok(r.messages)
    }

    // ----- media -----

    /// `GET /v1/{attachment name}`: the full resource. A message list carries a Drive
    /// attachment with only `name` and `source`; this call fills in the rest.
    pub async fn get_attachment(&self, name: &str) -> Result<Attachment, ApiError> {
        let url = self.v1(name);
        self.call_json(None, false, |c| c.get(&url)).await
    }

    /// Upload a file by path: reads it, guesses the mime type, then `upload_bytes`.
    pub async fn upload_attachment(
        &self,
        space: &str,
        path: &Path,
    ) -> Result<UploadAttachmentResponse, ApiError> {
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        let bytes = tokio::fs::read(path).await.map_err(|e| ApiError {
            status: 0,
            message: format!("cannot read {}: {e}", path.display()),
        })?;
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();
        self.upload_bytes(space, &filename, &mime, bytes).await
    }

    /// `POST /upload/v1/{space}/attachments:upload?uploadType=multipart`:
    /// `metadata` JSON `{filename}` plus the file part.
    pub async fn upload_bytes(
        &self,
        space: &str,
        filename: &str,
        mime: &str,
        bytes: Vec<u8>,
    ) -> Result<UploadAttachmentResponse, ApiError> {
        let metadata = serde_json::json!({ "filename": filename }).to_string();
        let url = format!("{}/upload/v1/{space}/attachments:upload", self.base);
        // note: the file is held in memory and cloned per attempt; fine below a few hundred MB.
        self.call_json(Some(space), true, |c| {
            let form = reqwest::multipart::Form::new()
                .part(
                    "metadata",
                    reqwest::multipart::Part::text(metadata.clone())
                        .mime_str("application/json")
                        .expect("static mime"),
                )
                .part(
                    "file",
                    reqwest::multipart::Part::bytes(bytes.clone())
                        .file_name(filename.to_string())
                        .mime_str(mime)
                        .unwrap_or_else(|_| {
                            reqwest::multipart::Part::bytes(bytes.clone())
                                .file_name(filename.to_string())
                        }),
                );
            c.post(&url)
                .query(&[("uploadType", "multipart")])
                .multipart(form)
        })
        .await
    }

    /// `GET /v1/media/{resourceName}?alt=media`
    pub async fn download_media(&self, resource_name: &str) -> Result<Vec<u8>, ApiError> {
        let url = self.v1(&format!("media/{resource_name}"));
        self.call_raw(None, false, |c| c.get(&url).query(&[("alt", "media")]))
            .await
    }

    /// Authenticated GET of an absolute URL for the `prism-media` proxy:
    /// `(content_type, bytes)`, with the same Bearer, refresh and retry policy.
    pub async fn get_bytes(&self, url: &str) -> Result<(String, Vec<u8>), ApiError> {
        let resp = self.call_ok(None, false, |c| c.get(url)).await?;
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        // note: the whole body is buffered; attachments are bounded by Google's 200 MB upload cap.
        let bytes = resp
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(ApiError::transport)?;
        Ok((content_type, bytes))
    }
}

/// What the huddle message says when Google refuses the annotation.
pub fn huddle_fallback_text(meeting_uri: &str) -> String {
    format!("Started a huddle\n{meeting_uri}")
}

// ----- availability -----

/// `markAsDoNotDisturb` without a duration from the caller: one hour.
const DND_DEFAULT_SECS: u64 = 3600;
/// The emoji a custom status gets when the caller gives text alone.
const DEFAULT_STATUS_EMOJI: &str = "💬";

/// A protobuf `Duration` on the JSON wire: seconds with an `s` suffix.
fn duration_field(seconds: u64) -> String {
    format!("{seconds}s")
}

// ----- Calendar API shapes -----

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CalendarListResponse {
    #[serde(default)]
    items: Vec<CalendarEntry>,
    #[serde(default)]
    next_page_token: Option<String>,
}

/// Percent-encodes one path segment: everything but RFC 3986 unreserved characters,
/// so `team@example.test` becomes `team%40example.test` and `/` cannot split the path.
fn path_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The `events.insert` / `events.patch` body for an `EventInput`: `start` and `end`
/// as given (`dateTime` or all-day `date`), attendees as `{email}`, `recurrence` only
/// when the input has lines, blank description and location left out.
fn event_body(event: &EventInput) -> serde_json::Value {
    let mut body = serde_json::json!({
        "summary": event.summary,
        "start": event.start,
        "end": event.end,
        "attendees": event
            .attendees
            .iter()
            .map(|e| e.trim())
            .filter(|e| !e.is_empty())
            .map(|e| serde_json::json!({ "email": e }))
            .collect::<Vec<_>>(),
    });
    if let Some(d) = event
        .description
        .as_deref()
        .filter(|d| !d.trim().is_empty())
    {
        body["description"] = serde_json::json!(d);
    }
    if let Some(l) = event.location.as_deref().filter(|l| !l.trim().is_empty()) {
        body["location"] = serde_json::json!(l);
    }
    let recurrence: Vec<&str> = event
        .recurrence
        .iter()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .collect();
    if !recurrence.is_empty() {
        body["recurrence"] = serde_json::json!(recurrence);
    }
    body
}

/// `conferenceData` asking Google to attach a Meet; needs `conferenceDataVersion=1`.
fn meet_create_request() -> serde_json::Value {
    serde_json::json!({
        "createRequest": {
            "requestId": uuid::Uuid::new_v4().to_string(),
            "conferenceSolutionKey": { "type": "hangoutsMeet" }
        }
    })
}

/// The attendee list to patch back with the caller's answer set, `None` when the caller
/// is not an attendee. Only writable fields are sent; `self` and `organizer` are Google's.
fn respond_as_self(attendees: &[Attendee], response: &str) -> Option<Vec<serde_json::Value>> {
    if !attendees.iter().any(|a| a.is_self) {
        return None;
    }
    Some(
        attendees
            .iter()
            .map(|a| {
                let mut v = serde_json::json!({ "email": a.email });
                if let Some(n) = &a.display_name {
                    v["displayName"] = serde_json::json!(n);
                }
                let status = if a.is_self {
                    Some(response)
                } else {
                    a.response_status.as_deref()
                };
                if let Some(s) = status {
                    v["responseStatus"] = serde_json::json!(s);
                }
                if a.optional {
                    v["optional"] = serde_json::json!(true);
                }
                v
            })
            .collect(),
    )
}

// ----- Drive API shapes -----

/// The one file Confab keeps in the app-data folder.
const NOTES_FILE: &str = "prism-notes.json";
/// `files.list` filter for it.
const NOTES_QUERY: &str = "name='prism-notes.json'";

#[derive(serde::Deserialize, Default)]
struct DriveFileList {
    #[serde(default)]
    files: Vec<DriveFile>,
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DriveFile {
    #[serde(default)]
    id: String,
    #[serde(default)]
    #[allow(dead_code)]
    name: String,
    #[serde(default)]
    #[allow(dead_code)]
    modified_time: Option<String>,
}

/// Google's `multipart/related` upload: the metadata JSON part, then the content part.
fn multipart_related(boundary: &str, metadata: &str, content: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(metadata.len() + content.len() + 256);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Type: application/json\r\n\r\n{content}\r\n--{boundary}--\r\n"
        )
        .as_bytes(),
    );
    body
}

// ----- Meet REST API shapes -----

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MeetSpaceResponse {
    #[serde(default)]
    active_conference: Option<ActiveConference>,
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ActiveConference {
    #[serde(default)]
    conference_record: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct ParticipantsResponse {
    #[serde(default)]
    participants: Vec<RawParticipant>,
}

/// `conferenceRecords.participants`: one of `signedinUser`, `anonymousUser`, `phoneUser`.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawParticipant {
    #[serde(default)]
    signedin_user: Option<SignedinUser>,
    #[serde(default)]
    anonymous_user: Option<NamedUser>,
    #[serde(default)]
    phone_user: Option<NamedUser>,
    #[serde(default)]
    earliest_start_time: Option<String>,
    #[serde(default)]
    latest_end_time: Option<String>,
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SignedinUser {
    /// `users/{id}`, the same id as Chat's.
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct NamedUser {
    #[serde(default)]
    display_name: Option<String>,
}

/// The participants still in the call, in Google's order (earliest joiner first).
fn present_participants(raw: Vec<RawParticipant>) -> Vec<MeetParticipant> {
    raw.into_iter()
        .filter(|p| {
            p.latest_end_time
                .as_deref()
                .is_none_or(|t| t.trim().is_empty())
        })
        .map(|p| {
            let (user, signed_name) = match p.signedin_user {
                Some(u) => (
                    u.user.and_then(non_empty),
                    u.display_name.and_then(non_empty),
                ),
                None => (None, None),
            };
            let display_name = signed_name
                .or_else(|| {
                    p.anonymous_user
                        .and_then(|u| u.display_name)
                        .and_then(non_empty)
                })
                .or_else(|| {
                    p.phone_user
                        .and_then(|u| u.display_name)
                        .and_then(non_empty)
                });
            MeetParticipant {
                user,
                display_name,
                since: p.earliest_start_time.and_then(non_empty),
            }
        })
        .collect()
}

// ----- People API shapes -----

#[derive(serde::Deserialize, Default)]
struct SearchPeopleResponse {
    #[serde(default)]
    people: Vec<Person>,
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Person {
    #[serde(default)]
    resource_name: String,
    #[serde(default)]
    names: Vec<Named>,
    #[serde(default)]
    email_addresses: Vec<Valued>,
    #[serde(default)]
    photos: Vec<Photo>,
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Named {
    #[serde(default)]
    display_name: String,
}

#[derive(serde::Deserialize, Default)]
struct Valued {
    #[serde(default)]
    value: String,
}

#[derive(serde::Deserialize, Default)]
struct Photo {
    #[serde(default)]
    url: String,
}

fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

fn person_to_user(p: Person) -> Option<User> {
    let id = p.resource_name.strip_prefix("people/")?;
    Some(User {
        name: format!("users/{id}"),
        display_name: p
            .names
            .into_iter()
            .next()
            .and_then(|n| non_empty(n.display_name)),
        user_type: Some("HUMAN".into()),
        avatar_url: p.photos.into_iter().next().and_then(|p| non_empty(p.url)),
        email: p
            .email_addresses
            .into_iter()
            .next()
            .and_then(|e| non_empty(e.value)),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_directory_person_becomes_a_chat_user() {
        let raw = r#"{"people":[{"resourceName":"people/42","names":[{"displayName":"Priya N"}],
            "emailAddresses":[{"value":"priya@example.test"}],"photos":[{"url":"https://p/42"}]},
            {"resourceName":"groups/9","names":[{"displayName":"Not a person"}]}]}"#;
        let r: SearchPeopleResponse = serde_json::from_str(raw).unwrap();
        let users: Vec<User> = r.people.into_iter().filter_map(person_to_user).collect();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].name, "users/42");
        assert_eq!(users[0].display_name.as_deref(), Some("Priya N"));
        assert_eq!(users[0].email.as_deref(), Some("priya@example.test"));
        assert_eq!(users[0].avatar_url.as_deref(), Some("https://p/42"));
    }

    use super::*;

    #[test]
    fn meet_participants_keep_only_those_still_in_the_call() {
        let raw = r#"{"participants":[
            {"name":"conferenceRecords/x/participants/1","signedinUser":{"user":"users/1001","displayName":"Arun"},
             "earliestStartTime":"2026-11-07T10:00:00Z"},
            {"name":"conferenceRecords/x/participants/2","anonymousUser":{"displayName":"Guest"},
             "earliestStartTime":"2026-11-07T10:00:05Z"},
            {"name":"conferenceRecords/x/participants/3","phoneUser":{"displayName":"+91 98"},
             "earliestStartTime":"2026-11-07T10:00:06Z"},
            {"name":"conferenceRecords/x/participants/4","signedinUser":{"user":"users/1003","displayName":"Rahul"},
             "earliestStartTime":"2026-11-07T10:00:01Z","latestEndTime":"2026-11-07T10:00:03Z"}]}"#;
        let r: ParticipantsResponse = serde_json::from_str(raw).unwrap();
        let present = present_participants(r.participants);
        assert_eq!(present.len(), 3);
        assert_eq!(present[0].user.as_deref(), Some("users/1001"));
        assert_eq!(present[0].display_name.as_deref(), Some("Arun"));
        assert_eq!(present[0].since.as_deref(), Some("2026-11-07T10:00:00Z"));
        assert_eq!(present[1].user, None);
        assert_eq!(present[1].display_name.as_deref(), Some("Guest"));
        assert_eq!(present[2].display_name.as_deref(), Some("+91 98"));

        let idle: MeetSpaceResponse =
            serde_json::from_str(r#"{"name":"spaces/x","meetingCode":"abc-defg-hij"}"#).unwrap();
        assert!(idle.active_conference.is_none());
        let live: MeetSpaceResponse = serde_json::from_str(
            r#"{"name":"spaces/x","activeConference":{"conferenceRecord":"conferenceRecords/x"}}"#,
        )
        .unwrap();
        assert_eq!(
            live.active_conference.unwrap().conference_record.as_deref(),
            Some("conferenceRecords/x")
        );
        assert_eq!(
            huddle_fallback_text("https://meet.google.com/abc-defg-hij"),
            "Started a huddle\nhttps://meet.google.com/abc-defg-hij"
        );
    }

    #[test]
    fn durations_are_protobuf_strings() {
        assert_eq!(duration_field(1800), "1800s");
        assert_eq!(DND_DEFAULT_SECS, 3600);
    }

    #[test]
    fn calendar_bodies_follow_google_shapes() {
        assert_eq!(path_segment("primary"), "primary");
        assert_eq!(path_segment("team@example.test"), "team%40example.test");
        assert_eq!(path_segment("a/b c"), "a%2Fb%20c");

        let input: EventInput = serde_json::from_str(
            r#"{"summary":"Sync","description":"  ","location":"Room 4",
                "start":{"dateTime":"2026-11-08T09:00:00+05:30"},"end":{"dateTime":"2026-11-08T09:30:00+05:30"},
                "attendees":["priya@prism.test"," ","rahul@prism.test"],"addMeet":true}"#,
        )
        .unwrap();
        let body = event_body(&input);
        assert_eq!(
            body,
            serde_json::json!({
                "summary": "Sync",
                "location": "Room 4",
                "start": { "dateTime": "2026-11-08T09:00:00+05:30" },
                "end": { "dateTime": "2026-11-08T09:30:00+05:30" },
                "attendees": [{ "email": "priya@prism.test" }, { "email": "rahul@prism.test" }]
            })
        );
        let all_day: EventInput = serde_json::from_str(
            r#"{"summary":"Offsite","start":{"date":"2026-11-20"},"end":{"date":"2026-11-21"},
                "recurrence":["RRULE:FREQ=WEEKLY;BYDAY=MO,WE"]}"#,
        )
        .unwrap();
        let body = event_body(&all_day);
        assert_eq!(body["start"], serde_json::json!({ "date": "2026-11-20" }));
        assert_eq!(
            body["recurrence"],
            serde_json::json!(["RRULE:FREQ=WEEKLY;BYDAY=MO,WE"])
        );
        assert_eq!(body["attendees"], serde_json::json!([]));

        let meet = meet_create_request();
        assert_eq!(
            meet["createRequest"]["conferenceSolutionKey"]["type"],
            "hangoutsMeet"
        );
        assert_eq!(
            meet["createRequest"]["requestId"].as_str().unwrap().len(),
            36
        );

        let attendees = vec![
            Attendee {
                email: "arun@prism.test".into(),
                display_name: Some("Arun".into()),
                response_status: Some("needsAction".into()),
                is_self: true,
                ..Default::default()
            },
            Attendee {
                email: "priya@prism.test".into(),
                response_status: Some("accepted".into()),
                organizer: true,
                optional: true,
                ..Default::default()
            },
        ];
        let patched = respond_as_self(&attendees, "declined").unwrap();
        assert_eq!(
            patched,
            vec![
                serde_json::json!({ "email": "arun@prism.test", "displayName": "Arun", "responseStatus": "declined" }),
                serde_json::json!({ "email": "priya@prism.test", "responseStatus": "accepted", "optional": true }),
            ]
        );
        assert!(respond_as_self(&attendees[1..], "accepted").is_none());

        let body = multipart_related("b", r#"{"name":"n"}"#, "{}");
        let text = String::from_utf8(body).unwrap();
        assert!(text.starts_with("--b\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{\"name\":\"n\"}\r\n--b\r\n"));
        assert!(text.ends_with("\r\n\r\n{}\r\n--b--\r\n"));
    }

    #[test]
    fn google_error_bodies_become_messages() {
        let e = ApiError::from_google(
            409,
            r#"{"error":{"code":409,"message":"already exists","status":"ALREADY_EXISTS"}}"#,
        );
        assert_eq!(e.status, 409);
        assert_eq!(e.message, "already exists (ALREADY_EXISTS)");
        let e = ApiError::from_google(502, "<html>bad gateway</html>");
        assert!(e.message.starts_with("HTTP 502"));
        assert!(ApiError::from_google(404, "").is_not_found());
    }

    #[test]
    fn backoff_is_exponential_and_capped() {
        assert_eq!(backoff(1), Duration::from_secs(2));
        assert_eq!(backoff(2), Duration::from_secs(4));
        assert_eq!(backoff(3), Duration::from_secs(8));
        assert_eq!(backoff(9), BACKOFF_CAP);
    }
}
