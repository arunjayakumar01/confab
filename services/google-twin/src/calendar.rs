//! Google Calendar API v3 subset under `/calendar/v3`: the calendar list and
//! `events.list|get|insert|patch|delete` on two calendars, in memory. Bearer required,
//! Google-shaped errors. `self` on attendees is computed per caller; a sync token is
//! the change counter plus one, deleted events stay as `cancelled` so a sync reports
//! them, and a token older than the kept history answers 410.

use crate::auth::Caller;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, CalEventRec, Store, CAL_PRIMARY, CAL_TEAM};
use crate::util::parse_time;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use prism_proto::calendar::{Attendee, CalEvent, ConferenceData, EntryPoint, EventTime};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

type Params = Query<HashMap<String, String>>;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/calendar/v3/users/me/calendarList", get(calendar_list))
        .route(
            "/calendar/v3/calendars/{id}/events",
            get(list_events).post(create_event),
        )
        .route(
            "/calendar/v3/calendars/{id}/events/{event_id}",
            get(get_event)
                .patch(patch_event)
                .put(patch_event)
                .delete(delete_event),
        )
}

/// `events.insert` and `events.patch` bodies: everything optional, a missing field
/// is left alone on patch and defaulted on insert.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct EventBody {
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    start: Option<EventTime>,
    #[serde(default)]
    end: Option<EventTime>,
    #[serde(default)]
    attendees: Option<Vec<AttendeeBody>>,
    #[serde(default)]
    recurrence: Option<Vec<String>>,
    #[serde(default)]
    transparency: Option<String>,
    #[serde(default)]
    color_id: Option<String>,
    #[serde(default)]
    conference_data: Option<ConferenceBody>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct AttendeeBody {
    #[serde(default)]
    email: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    response_status: Option<String>,
    #[serde(default)]
    optional: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ConferenceBody {
    #[serde(default)]
    create_request: Option<serde_json::Value>,
}

/// `primary`, the caller's own email, or the team calendar; `(canonical id, writable)`.
fn resolve_calendar(st: &Store, caller: &str, id: &str) -> ApiResult<(String, bool)> {
    let id = id.trim();
    if id == CAL_PRIMARY || id.eq_ignore_ascii_case(&st.email_of(caller)) {
        return Ok((CAL_PRIMARY.to_string(), true));
    }
    if id.eq_ignore_ascii_case(CAL_TEAM) {
        return Ok((CAL_TEAM.to_string(), false));
    }
    Err(ApiError::not_found("Not Found"))
}

fn writable(st: &Store, caller: &str, id: &str) -> ApiResult<String> {
    let (calendar, writable) = resolve_calendar(st, caller, id)?;
    if !writable {
        return Err(ApiError::permission_denied("Forbidden"));
    }
    Ok(calendar)
}

fn not_found() -> ApiError {
    ApiError::not_found("Not Found")
}

/// The instant an event boundary stands for: `dateTime` as given, an all-day `date`
/// at local midnight (the end date is exclusive, so midnight is right for both).
fn instant(t: &EventTime) -> Option<DateTime<Utc>> {
    if let Some(dt) = t.date_time.as_deref() {
        return parse_time(dt);
    }
    let date = NaiveDate::parse_from_str(t.date.as_deref()?, "%Y-%m-%d").ok()?;
    let naive = date.and_hms_opt(0, 0, 0)?;
    let local = Local
        .from_local_datetime(&naive)
        .earliest()
        .unwrap_or_else(|| Local.from_utc_datetime(&naive));
    Some(local.with_timezone(&Utc))
}

fn mark_self(a: &mut serde_json::Value, caller_email: &str) {
    let Some(obj) = a.as_object_mut() else {
        return;
    };
    let me = !caller_email.is_empty()
        && obj
            .get("email")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e.eq_ignore_ascii_case(caller_email));
    if me {
        obj.insert("self".into(), serde_json::json!(true));
    } else {
        obj.remove("self");
    }
    for flag in ["organizer", "optional"] {
        if obj.get(flag).and_then(|v| v.as_bool()) == Some(false) {
            obj.remove(flag);
        }
    }
}

/// The event as Calendar prints it for `caller_email`: `kind` and `etag` added,
/// `self` set on the caller's attendee entry, false flags dropped, Prism's
/// `calendarId` left out. A cancelled event is the trimmed `{kind, etag, id, status}`.
fn event_json(rec: &CalEventRec, caller_email: &str) -> serde_json::Value {
    let etag = format!("\"{}\"", rec.seq);
    if rec.event.is_cancelled() {
        return serde_json::json!({
            "kind": "calendar#event",
            "etag": etag,
            "id": rec.event.id,
            "status": CalEvent::CANCELLED,
        });
    }
    let mut v = serde_json::to_value(&rec.event).unwrap_or_default();
    let Some(obj) = v.as_object_mut() else {
        return v;
    };
    obj.insert("kind".into(), serde_json::json!("calendar#event"));
    obj.insert("etag".into(), serde_json::json!(etag));
    obj.remove("calendarId");
    match obj.get_mut("attendees").and_then(|a| a.as_array_mut()) {
        Some(list) if !list.is_empty() => {
            for a in list.iter_mut() {
                mark_self(a, caller_email);
            }
        }
        _ => {
            obj.remove("attendees");
        }
    }
    if let Some(org) = obj.get_mut("organizer").and_then(|o| o.as_object_mut()) {
        org.retain(|k, _| matches!(k.as_str(), "email" | "displayName"));
        let me = org
            .get("email")
            .and_then(|e| e.as_str())
            .is_some_and(|e| !caller_email.is_empty() && e.eq_ignore_ascii_case(caller_email));
        if me {
            org.insert("self".into(), serde_json::json!(true));
        }
    }
    v
}

fn calendar_summary(st: &Store, caller: &str, calendar: &str) -> String {
    if calendar == CAL_TEAM {
        "Team".to_string()
    } else {
        st.user(caller)
            .and_then(|u| u.display_name.clone())
            .unwrap_or_else(|| caller.to_string())
    }
}

async fn calendar_list(
    State(state): State<Arc<AppState>>,
    caller: Caller,
) -> Json<serde_json::Value> {
    let st = state.read();
    let mine = calendar_summary(&st, &caller.user, CAL_PRIMARY);
    Json(serde_json::json!({
        "kind": "calendar#calendarList",
        "items": [
            {
                "kind": "calendar#calendarListEntry",
                "id": CAL_PRIMARY,
                "summary": mine,
                "primary": true,
                "backgroundColor": "#7986cb",
                "foregroundColor": "#000000",
                "accessRole": "owner",
                "selected": true,
            },
            {
                "kind": "calendar#calendarListEntry",
                "id": CAL_TEAM,
                "summary": "Team",
                "backgroundColor": "#33b679",
                "foregroundColor": "#000000",
                "accessRole": "reader",
                "selected": true,
            }
        ]
    }))
}

// note: one page; maxResults and pageToken are accepted and ignored, the seed is small.
async fn list_events(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(id): Path<String>,
    Query(q): Params,
) -> ApiResult<Json<serde_json::Value>> {
    let st = state.read();
    let (calendar, writable) = resolve_calendar(&st, &caller.user, &id)?;
    let caller_email = st.email_of(&caller.user);
    let get = |k: &str| q.get(k).map(|s| s.trim()).filter(|s| !s.is_empty());
    let show_deleted = get("showDeleted") == Some("true");
    let mut items: Vec<&CalEventRec> = if let Some(token) = get("syncToken") {
        for k in ["timeMin", "timeMax", "orderBy", "q", "updatedMin"] {
            if get(k).is_some() {
                return Err(ApiError::invalid_argument(format!(
                    "syncToken cannot be specified together with {k}."
                )));
            }
        }
        let since = st.cal_sync_token_valid(token).ok_or_else(|| {
            ApiError::gone(
                "fullSyncRequired",
                "Sync token is no longer valid, a full sync is required.",
            )
        })?;
        st.cal_events_in(&calendar)
            .into_iter()
            .filter(|r| r.seq >= since)
            .collect()
    } else {
        let bound = |k: &str| -> ApiResult<Option<DateTime<Utc>>> {
            match get(k) {
                None => Ok(None),
                Some(v) => parse_time(v).map(Some).ok_or_else(|| {
                    ApiError::invalid_argument(format!("Invalid value for {k}: {v}"))
                }),
            }
        };
        let time_min = bound("timeMin")?;
        let time_max = bound("timeMax")?;
        let mut v: Vec<&CalEventRec> = st
            .cal_events_in(&calendar)
            .into_iter()
            .filter(|r| show_deleted || !r.event.is_cancelled())
            .filter(|r| {
                let start = instant(&r.event.start);
                let end = instant(&r.event.end).or(start);
                time_min.is_none_or(|min| end.is_some_and(|e| e > min))
                    && time_max.is_none_or(|max| start.is_some_and(|s| s < max))
            })
            .collect();
        match get("orderBy") {
            Some("startTime") => v.sort_by_key(|r| instant(&r.event.start)),
            Some("updated") => v.sort_by(|a, b| a.event.updated.cmp(&b.event.updated)),
            Some(other) => {
                return Err(ApiError::invalid_argument(format!(
                    "Invalid value for orderBy: {other}"
                )))
            }
            None => {}
        }
        v
    };
    items.sort_by_key(|r| if r.event.is_cancelled() { 1 } else { 0 });
    let items: Vec<serde_json::Value> =
        items.iter().map(|r| event_json(r, &caller_email)).collect();
    Ok(Json(serde_json::json!({
        "kind": "calendar#events",
        "summary": calendar_summary(&st, &caller.user, &calendar),
        "updated": Store::task_time(Utc::now()),
        "accessRole": if writable { "owner" } else { "reader" },
        "items": items,
        "nextSyncToken": st.cal_next_sync_token(),
    })))
}

async fn get_event(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((id, event_id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let st = state.read();
    let (calendar, _) = resolve_calendar(&st, &caller.user, &id)?;
    let rec = st.cal_event(&calendar, &event_id).ok_or_else(not_found)?;
    Ok(Json(event_json(rec, &st.email_of(&caller.user))))
}

fn check_time(t: &EventTime, field: &str) -> ApiResult<()> {
    if instant(t).is_none() {
        return Err(ApiError::invalid_argument(format!(
            "Invalid value for {field}: a dateTime (RFC 3339) or a date (YYYY-MM-DD) is required."
        )));
    }
    Ok(())
}

/// Attendees from a body: `needsAction` unless given, the caller `accepted` unless
/// given, the organizer (by email) flagged. The caller is added as organizer when the
/// list is not empty and does not carry them, the way Calendar does.
fn attendees_from(
    st: &Store,
    caller: &str,
    organizer_email: &str,
    body: Vec<AttendeeBody>,
) -> ApiResult<Vec<Attendee>> {
    let caller_email = st.email_of(caller);
    let mut out = Vec::with_capacity(body.len() + 1);
    for a in body {
        let email = a.email.trim().to_string();
        if email.is_empty() || !email.contains('@') {
            return Err(ApiError::invalid_argument(format!(
                "Invalid attendee email: {email:?}"
            )));
        }
        if let Some(s) = a.response_status.as_deref() {
            if s != Attendee::NEEDS_ACTION && !Attendee::is_response(s) {
                return Err(ApiError::invalid_argument(format!(
                    "Invalid value for responseStatus: {s}"
                )));
            }
        }
        let is_caller = email.eq_ignore_ascii_case(&caller_email);
        let known_name = st
            .users
            .values()
            .find(|u| {
                u.email
                    .as_deref()
                    .is_some_and(|e| e.eq_ignore_ascii_case(&email))
            })
            .and_then(|u| u.display_name.clone());
        out.push(Attendee {
            organizer: email.eq_ignore_ascii_case(organizer_email),
            response_status: Some(a.response_status.unwrap_or_else(|| {
                if is_caller {
                    Attendee::ACCEPTED.into()
                } else {
                    Attendee::NEEDS_ACTION.into()
                }
            })),
            display_name: a.display_name.or(known_name),
            optional: a.optional.unwrap_or(false),
            is_self: false,
            email,
        });
    }
    if !out.is_empty()
        && !caller_email.is_empty()
        && !out
            .iter()
            .any(|a| a.email.eq_ignore_ascii_case(&caller_email))
    {
        out.insert(
            0,
            Attendee {
                email: caller_email.clone(),
                display_name: st.user(caller).and_then(|u| u.display_name.clone()),
                response_status: Some(Attendee::ACCEPTED.into()),
                organizer: caller_email.eq_ignore_ascii_case(organizer_email),
                ..Default::default()
            },
        );
    }
    Ok(out)
}

/// Attach a Meet when `conferenceDataVersion=1` and the body asks for one; an event
/// that already has a link keeps it.
fn attach_meet(event: &mut CalEvent, conference_version: Option<&str>, body: &EventBody) {
    let asked = conference_version == Some("1")
        && body
            .conference_data
            .as_ref()
            .is_some_and(|c| c.create_request.is_some());
    if !asked || event.hangout_link.is_some() {
        return;
    }
    let code = Store::cal_meet_code(&event.id);
    let uri = format!("https://meet.google.com/{code}");
    event.hangout_link = Some(uri.clone());
    event.conference_data = Some(ConferenceData {
        entry_points: vec![EntryPoint {
            entry_point_type: Some("video".into()),
            uri: Some(uri),
            label: Some(format!("meet.google.com/{code}")),
        }],
    });
}

async fn create_event(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(id): Path<String>,
    Query(q): Params,
    Json(body): Json<EventBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let start = body
        .start
        .clone()
        .ok_or_else(|| ApiError::invalid_argument("Missing start time."))?;
    let end = body
        .end
        .clone()
        .ok_or_else(|| ApiError::invalid_argument("Missing end time."))?;
    check_time(&start, "start")?;
    check_time(&end, "end")?;
    if start.is_all_day() != end.is_all_day() {
        return Err(ApiError::invalid_argument(
            "Start and end times must either both be date or both be dateTime.",
        ));
    }
    if instant(&end) < instant(&start) {
        return Err(ApiError::invalid_argument(
            "The specified time range is empty.",
        ));
    }
    let now = Utc::now();
    let mut st = state.write();
    let calendar = writable(&st, &caller.user, &id)?;
    let caller_email = st.email_of(&caller.user);
    let organizer = Attendee {
        email: caller_email.clone(),
        display_name: st.user(&caller.user).and_then(|u| u.display_name.clone()),
        response_status: Some(Attendee::ACCEPTED.into()),
        organizer: true,
        ..Default::default()
    };
    let attendees = attendees_from(
        &st,
        &caller.user,
        &caller_email,
        body.attendees.clone().unwrap_or_default(),
    )?;
    let event = CalEvent {
        summary: body.summary.clone(),
        description: body.description.clone(),
        location: body.location.clone(),
        start,
        end,
        attendees,
        organizer: Some(organizer),
        recurrence: body.recurrence.clone().unwrap_or_default(),
        transparency: body.transparency.clone(),
        color_id: body.color_id.clone(),
        ..Default::default()
    };
    let created = st.new_cal_event(&calendar, event, now);
    let conference_version = q.get("conferenceDataVersion").map(String::as_str);
    let rec = st.cal_events.get_mut(&created.id).expect("just inserted");
    attach_meet(&mut rec.event, conference_version, &body);
    Ok(Json(event_json(rec, &caller_email)))
}

async fn patch_event(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((id, event_id)): Path<(String, String)>,
    Query(q): Params,
    Json(body): Json<EventBody>,
) -> ApiResult<Json<serde_json::Value>> {
    if let Some(t) = &body.start {
        check_time(t, "start")?;
    }
    if let Some(t) = &body.end {
        check_time(t, "end")?;
    }
    let now = Utc::now();
    let mut st = state.write();
    let calendar = writable(&st, &caller.user, &id)?;
    let caller_email = st.email_of(&caller.user);
    let organizer_email = st
        .cal_event(&calendar, &event_id)
        .ok_or_else(not_found)?
        .event
        .organizer
        .as_ref()
        .map(|o| o.email.clone())
        .unwrap_or_else(|| caller_email.clone());
    let attendees = match body.attendees.clone() {
        Some(list) => Some(attendees_from(&st, &caller.user, &organizer_email, list)?),
        None => None,
    };
    let conference_version = q.get("conferenceDataVersion").map(String::as_str);
    let rec = st
        .cal_event_mut(&calendar, &event_id, now)
        .ok_or_else(not_found)?;
    let ev = &mut rec.event;
    if let Some(s) = body.summary.clone() {
        ev.summary = Some(s);
    }
    if let Some(d) = body.description.clone() {
        ev.description = Some(d);
    }
    if let Some(l) = body.location.clone() {
        ev.location = Some(l);
    }
    if let Some(t) = body.start.clone() {
        ev.start = t;
    }
    if let Some(t) = body.end.clone() {
        ev.end = t;
    }
    if let Some(a) = attendees {
        ev.attendees = a;
    }
    if let Some(r) = body.recurrence.clone() {
        ev.recurrence = r;
    }
    if let Some(t) = body.transparency.clone() {
        ev.transparency = Some(t);
    }
    if let Some(c) = body.color_id.clone() {
        ev.color_id = Some(c);
    }
    attach_meet(ev, conference_version, &body);
    Ok(Json(event_json(rec, &caller_email)))
}

/// Marks the event cancelled and keeps it, so a sync reports the deletion; a second
/// delete answers 410 the way Calendar does.
async fn delete_event(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((id, event_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let now = Utc::now();
    let mut st = state.write();
    let calendar = writable(&st, &caller.user, &id)?;
    if st
        .cal_event(&calendar, &event_id)
        .ok_or_else(not_found)?
        .event
        .is_cancelled()
    {
        return Err(ApiError::gone("deleted", "Resource has been deleted"));
    }
    let rec = st
        .cal_event_mut(&calendar, &event_id, now)
        .ok_or_else(not_found)?;
    rec.event.status = Some(CalEvent::CANCELLED.into());
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_boundaries_become_instants() {
        let timed = EventTime {
            date_time: Some("2026-11-07T10:00:00+05:30".into()),
            ..Default::default()
        };
        assert_eq!(
            instant(&timed).unwrap(),
            Utc.with_ymd_and_hms(2026, 11, 7, 4, 30, 0).unwrap()
        );
        let all_day = EventTime {
            date: Some("2026-11-20".into()),
            ..Default::default()
        };
        let midnight = instant(&all_day).unwrap().with_timezone(&Local);
        assert_eq!(midnight.date_naive().to_string(), "2026-11-20");
        assert!(instant(&EventTime::default()).is_none());
        assert!(instant(&EventTime {
            date: Some("tomorrow".into()),
            ..Default::default()
        })
        .is_none());
    }

    #[test]
    fn self_is_marked_per_caller_and_cancelled_events_are_trimmed() {
        let rec = CalEventRec {
            calendar: CAL_PRIMARY.into(),
            event: CalEvent {
                id: "ev-9".into(),
                status: Some(CalEvent::CONFIRMED.into()),
                attendees: vec![
                    Attendee {
                        email: "Arun@prism.test".into(),
                        organizer: true,
                        ..Default::default()
                    },
                    Attendee {
                        email: "priya@prism.test".into(),
                        optional: true,
                        ..Default::default()
                    },
                ],
                organizer: Some(Attendee {
                    email: "arun@prism.test".into(),
                    display_name: Some("Arun".into()),
                    response_status: Some("accepted".into()),
                    organizer: true,
                    ..Default::default()
                }),
                calendar_id: Some("leak".into()),
                ..Default::default()
            },
            seq: 4,
        };
        let v = event_json(&rec, "priya@prism.test");
        assert_eq!(v["kind"], "calendar#event");
        assert_eq!(v["etag"], "\"4\"");
        assert!(v.get("calendarId").is_none());
        assert_eq!(v["attendees"][0]["organizer"], true);
        assert!(v["attendees"][0].get("self").is_none());
        assert!(v["attendees"][0].get("optional").is_none());
        assert_eq!(v["attendees"][1]["self"], true);
        assert_eq!(v["attendees"][1]["optional"], true);
        assert_eq!(
            v["organizer"],
            serde_json::json!({ "email": "arun@prism.test", "displayName": "Arun" })
        );
        let v = event_json(&rec, "arun@prism.test");
        assert_eq!(v["attendees"][0]["self"], true);
        assert_eq!(v["organizer"]["self"], true);

        let mut gone = rec.clone();
        gone.event.status = Some(CalEvent::CANCELLED.into());
        assert_eq!(
            event_json(&gone, "arun@prism.test"),
            serde_json::json!({ "kind": "calendar#event", "etag": "\"4\"", "id": "ev-9", "status": "cancelled" })
        );
    }
}
