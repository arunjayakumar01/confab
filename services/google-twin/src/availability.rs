//! Google Chat `users.availability` under `/v1/users/{u}/availability`: the signed-in
//! user's presence, in memory per caller (default `ACTIVE`, no custom status).
//! `markAsActive` and `markAsDoNotDisturb` take a protobuf `ttl` (`"1800s"`), the
//! custom status patch takes `updateMask=customStatus`. Bearer required,
//! Google-shaped errors.

use crate::auth::Caller;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::user_id;
use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use prism_proto::availability::{Availability, CustomStatusWire, EmojiWire};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

type Params = Query<HashMap<String, String>>;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/v1/users/{u}/availability",
            get(get_availability).patch(patch_availability),
        )
        .route("/v1/users/{u}/availability:markAsActive", post(mark_active))
        .route("/v1/users/{u}/availability:markAsAway", post(mark_away))
        .route(
            "/v1/users/{u}/availability:markAsDoNotDisturb",
            post(mark_dnd),
        )
}

/// `markAsActive` keeps a user active this long when no `ttl` is given.
const ACTIVE_DEFAULT_SECS: f64 = 30.0 * 60.0;
/// The longest do-not-disturb window Google accepts: one year.
const DND_MAX_SECS: f64 = 365.0 * 24.0 * 3600.0;

/// `{"ttl": "1800s"}` bodies; the field is a protobuf `Duration` on the wire.
#[derive(Deserialize, Default)]
struct TtlBody {
    #[serde(default)]
    ttl: Option<String>,
}

/// The patch body: `customStatus` absent or empty clears the status.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PatchBody {
    #[serde(default)]
    custom_status: Option<StatusBody>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct StatusBody {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    emoji: Option<EmojiWire>,
    #[serde(default)]
    ttl: Option<String>,
    #[serde(default)]
    expire_time: Option<String>,
}

/// Seconds of a protobuf `Duration` string such as `1800s` or `0.5s`.
fn parse_duration(s: &str) -> Option<f64> {
    let secs: f64 = s.trim().strip_suffix('s')?.trim().parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then_some(secs)
}

fn ttl_of(field: Option<&str>, name: &str) -> ApiResult<Option<f64>> {
    match field.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => parse_duration(s).map(Some).ok_or_else(|| {
            ApiError::invalid_argument(format!(
                "Invalid value for {name}: {s:?} (expected a duration such as \"1800s\")"
            ))
        }),
    }
}

fn after(now: DateTime<Utc>, secs: f64) -> DateTime<Utc> {
    now + Duration::milliseconds((secs * 1000.0).round() as i64)
}

/// `me`, the caller's bare id or `users/{id}`; anyone else's presence is Google's to refuse.
fn resolve_user(caller: &str, u: &str) -> ApiResult<String> {
    let u = u.trim();
    if u == "me" || u == user_id(caller) || u == caller {
        Ok(format!("users/{u}/availability"))
    } else {
        Err(ApiError::permission_denied(
            "Only the authenticated user's availability can be read or changed.",
        ))
    }
}

async fn get_availability(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(u): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let name = resolve_user(&caller.user, &u)?;
    let now = Utc::now();
    let st = state.read();
    let rec = st
        .availability
        .get(&caller.user)
        .cloned()
        .unwrap_or_default();
    Ok(Json(rec.to_json(&name, now)))
}

async fn patch_availability(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(u): Path<String>,
    Query(q): Params,
    body: Option<Json<PatchBody>>,
) -> ApiResult<Json<serde_json::Value>> {
    let name = resolve_user(&caller.user, &u)?;
    let mask: Vec<&str> = q
        .get("updateMask")
        .map(|m| m.split(',').map(str::trim).collect())
        .unwrap_or_default();
    if !mask.iter().any(|m| *m == "customStatus" || *m == "*") {
        return Err(ApiError::invalid_argument(
            "updateMask must name customStatus.",
        ));
    }
    let now = Utc::now();
    let status = body.and_then(|Json(b)| b.custom_status);
    let (custom_status, status_until) = match status {
        Some(s) if s.text.as_deref().is_some_and(|t| !t.trim().is_empty()) => {
            let text = s.text.unwrap_or_default();
            if text.chars().count() > Availability::STATUS_TEXT_MAX {
                return Err(ApiError::invalid_argument(format!(
                    "customStatus.text must be at most {} characters.",
                    Availability::STATUS_TEXT_MAX
                )));
            }
            let emoji = s
                .emoji
                .filter(|e| e.unicode.as_deref().is_some_and(|u| !u.trim().is_empty()))
                .ok_or_else(|| ApiError::invalid_argument("customStatus.emoji is required."))?;
            if s.ttl.is_some() && s.expire_time.is_some() {
                return Err(ApiError::invalid_argument(
                    "customStatus.ttl and customStatus.expireTime are exclusive.",
                ));
            }
            let until = match ttl_of(s.ttl.as_deref(), "customStatus.ttl")? {
                Some(secs) => Some(after(now, secs)),
                None => match s
                    .expire_time
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                {
                    Some(t) => Some(crate::util::parse_time(t).ok_or_else(|| {
                        ApiError::invalid_argument(format!(
                            "Invalid value for customStatus.expireTime: {t:?}"
                        ))
                    })?),
                    None => None,
                },
            };
            (
                Some(CustomStatusWire {
                    text,
                    emoji: Some(emoji),
                    expire_time: until.map(crate::state::Store::task_time),
                }),
                until,
            )
        }
        _ => (None, None),
    };
    let mut st = state.write();
    let rec = st.availability_mut(&caller.user);
    rec.custom_status = custom_status;
    rec.status_until = status_until;
    Ok(Json(rec.to_json(&name, now)))
}

async fn mark_active(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(u): Path<String>,
    body: Option<Json<TtlBody>>,
) -> ApiResult<Json<serde_json::Value>> {
    let name = resolve_user(&caller.user, &u)?;
    let ttl =
        ttl_of(body.and_then(|Json(b)| b.ttl).as_deref(), "ttl")?.unwrap_or(ACTIVE_DEFAULT_SECS);
    let now = Utc::now();
    let mut st = state.write();
    let rec = st.availability_mut(&caller.user);
    rec.state = Availability::ACTIVE.into();
    rec.until = Some(after(now, ttl));
    Ok(Json(rec.to_json(&name, now)))
}

async fn mark_away(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(u): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let name = resolve_user(&caller.user, &u)?;
    let now = Utc::now();
    let mut st = state.write();
    let rec = st.availability_mut(&caller.user);
    rec.state = Availability::AWAY.into();
    rec.until = None;
    Ok(Json(rec.to_json(&name, now)))
}

async fn mark_dnd(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(u): Path<String>,
    body: Option<Json<TtlBody>>,
) -> ApiResult<Json<serde_json::Value>> {
    let name = resolve_user(&caller.user, &u)?;
    let ttl = ttl_of(body.and_then(|Json(b)| b.ttl).as_deref(), "ttl")?
        .ok_or_else(|| ApiError::invalid_argument("ttl is required."))?;
    if ttl <= 0.0 || ttl > DND_MAX_SECS {
        return Err(ApiError::invalid_argument(
            "ttl must be positive and at most one year.",
        ));
    }
    let now = Utc::now();
    let mut st = state.write();
    let rec = st.availability_mut(&caller.user);
    rec.state = Availability::DO_NOT_DISTURB.into();
    rec.until = Some(after(now, ttl));
    Ok(Json(rec.to_json(&name, now)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AvailabilityRec;

    #[test]
    fn durations_parse_and_windows_lapse() {
        assert_eq!(parse_duration("1800s"), Some(1800.0));
        assert_eq!(parse_duration(" 0.5s "), Some(0.5));
        assert!(parse_duration("1800").is_none());
        assert!(parse_duration("-1s").is_none());
        assert!(ttl_of(Some("x"), "ttl").is_err());
        assert_eq!(ttl_of(Some(""), "ttl").unwrap(), None);

        assert_eq!(
            resolve_user("users/1001", "me").unwrap(),
            "users/me/availability"
        );
        assert_eq!(
            resolve_user("users/1001", "1001").unwrap(),
            "users/1001/availability"
        );
        assert!(resolve_user("users/1001", "1002").is_err());

        let now = Utc::now();
        let fresh = AvailabilityRec::default();
        assert_eq!(
            fresh.to_json("users/me/availability", now)["state"],
            "ACTIVE"
        );
        let dnd = AvailabilityRec {
            state: Availability::DO_NOT_DISTURB.into(),
            until: Some(now + Duration::minutes(5)),
            custom_status: Some(CustomStatusWire {
                text: "Heads down".into(),
                emoji: Some(EmojiWire {
                    unicode: Some("🎧".into()),
                }),
                expire_time: None,
            }),
            status_until: Some(now - Duration::seconds(1)),
        };
        let v = dnd.to_json("users/me/availability", now);
        assert_eq!(v["state"], "DO_NOT_DISTURB");
        assert!(v["doNotDisturbMetadata"]["expirationTime"].is_string());
        assert!(v.get("customStatus").is_none(), "expired status dropped");
        let v = dnd.to_json("users/me/availability", now + Duration::minutes(6));
        assert_eq!(v["state"], "ACTIVE");
        assert!(v.get("doNotDisturbMetadata").is_none());
        let idle = AvailabilityRec {
            state: Availability::ACTIVE.into(),
            until: Some(now - Duration::seconds(1)),
            ..Default::default()
        };
        assert_eq!(idle.to_json("n", now)["state"], "IDLE");
    }
}
