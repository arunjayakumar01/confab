//! Google Meet REST API v2 subset under `/v2`: `spaces.create`, `spaces.get` and
//! `conferenceRecords.participants.list`, in memory. Bearer required, Google-shaped
//! errors. A space counts as having an active conference for fifteen minutes after
//! creation; its participants are the creator, Priya (joins five seconds in) and
//! Rahul (already left).

use crate::auth::Caller;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, MeetRec, MEET_ACTIVE_SECS};
use crate::util::fmt_time;
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v2/spaces", post(create_space))
        .route("/v2/spaces/{code}", get(get_space))
        .route(
            "/v2/conferenceRecords/{id}/participants",
            get(list_participants),
        )
}

/// Seconds after creation at which Priya joins.
const PRIYA_JOINS_AFTER_SECS: i64 = 5;
/// Rahul's short visit: joined at creation, gone two seconds later.
const RAHUL_LEAVES_AFTER_SECS: i64 = 2;

fn space_json(rec: &MeetRec, now: DateTime<Utc>) -> serde_json::Value {
    let mut v = serde_json::json!({
        "name": rec.name,
        "meetingUri": format!("https://meet.google.com/{}", rec.code),
        "meetingCode": rec.code,
        "config": { "accessType": "TRUSTED", "entryPointAccess": "ALL" },
    });
    if now - rec.created_at < Duration::seconds(MEET_ACTIVE_SECS) {
        v["activeConference"] = serde_json::json!({ "conferenceRecord": record_name(rec) });
    }
    v
}

/// `conferenceRecords/meet-{n}`: the record id is the space id.
fn record_name(rec: &MeetRec) -> String {
    format!(
        "conferenceRecords/{}",
        rec.name.trim_start_matches("spaces/")
    )
}

async fn create_space(
    State(state): State<Arc<AppState>>,
    caller: Caller,
) -> Json<serde_json::Value> {
    // note: the request body (`config`) is accepted and ignored; every twin space is TRUSTED.
    let now = Utc::now();
    let mut st = state.write();
    let rec = st.new_meet(&caller.user, now);
    Json(space_json(&rec, now))
}

async fn get_space(
    State(state): State<Arc<AppState>>,
    _caller: Caller,
    Path(code): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let st = state.read();
    let rec = st
        .meet(&code)
        .ok_or_else(|| ApiError::not_found(format!("Space {code} not found.")))?;
    Ok(Json(space_json(rec, Utc::now())))
}

async fn list_participants(
    State(state): State<Arc<AppState>>,
    _caller: Caller,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let st = state.read();
    let rec = st.meet_by_record(&id).ok_or_else(|| {
        ApiError::not_found(format!(
            "Conference record conferenceRecords/{id} not found."
        ))
    })?;
    let record = record_name(rec);
    let now = Utc::now();
    let participant = |n: u32, user: &str, display: &str, start: DateTime<Utc>| {
        serde_json::json!({
            "name": format!("{record}/participants/{n}"),
            "signedinUser": { "user": user, "displayName": display },
            "earliestStartTime": fmt_time(start),
        })
    };
    let creator_name = st
        .user(&rec.creator)
        .and_then(|u| u.display_name.clone())
        .unwrap_or_else(|| rec.creator.clone());
    let mut participants = vec![participant(1, &rec.creator, &creator_name, rec.created_at)];
    if now - rec.created_at >= Duration::seconds(PRIYA_JOINS_AFTER_SECS) {
        participants.push(participant(
            2,
            "users/1002",
            "Priya Nair",
            rec.created_at + Duration::seconds(PRIYA_JOINS_AFTER_SECS),
        ));
    }
    let mut rahul = participant(3, "users/1003", "Rahul Mehta", rec.created_at);
    rahul["latestEndTime"] = serde_json::json!(fmt_time(
        rec.created_at + Duration::seconds(RAHUL_LEAVES_AFTER_SECS)
    ));
    participants.push(rahul);
    Ok(Json(serde_json::json!({ "participants": participants })))
}
