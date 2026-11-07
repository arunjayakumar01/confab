//! `POST /events/push`: the Cloud Pub/Sub push endpoint (the twin posts the same
//! envelope). Authenticate, decode, dedupe, hydrate if the resource is missing,
//! fan out to the sessions subscribed to the event's space. Always answer fast.
//! Everything after authentication lives in [`ingest`], which the pull loop
//! (`pubsub_pull`) feeds with the same envelope.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context};
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use prism_proto::events::PubSubPush;
use prism_proto::relay::{event_types, ChatEvent, ServerMsg};
use serde::Deserialize;

use crate::names::{valid_message, valid_space};
use crate::state::{AppState, JwksCache};

const JWKS_TTL: Duration = Duration::from_secs(3600);
const JWKS_REFETCH_FLOOR: Duration = Duration::from_secs(60);
const GOOGLE_ISSUERS: [&str; 2] = ["accounts.google.com", "https://accounts.google.com"];

#[derive(Deserialize, Default)]
pub struct PushQuery {
    /// Twin mode alternative to the header: push subscriptions cannot set custom headers.
    pub secret: Option<String>,
}

pub async fn handle(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PushQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(status) = authorize(&state, &headers, query.secret.as_deref()).await {
        return status.into_response();
    }
    let push: PubSubPush = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "push body is not JSON");
            return (StatusCode::BAD_REQUEST, "unparsable JSON").into_response();
        }
    };
    ingest(&state, push).await;
    StatusCode::NO_CONTENT.into_response()
}

/// Everything the push endpoint does once the sender is trusted: decode the
/// envelope, check the space name, dedupe, hydrate a name-only message, fan
/// out. Shared with the pull loop so both delivery paths behave identically.
/// A rejected or duplicate envelope is dropped (logged, never retried), which
/// is why `/events/push` still answers 204 and the puller still acks.
/// Returns true when an event reached the hub.
pub async fn ingest(state: &AppState, push: PubSubPush) -> bool {
    let event = match push.into_chat_event() {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "push envelope rejected");
            return false;
        }
    };
    if !valid_space(&event.space) {
        tracing::warn!(space = %event.space, event_type = %event.event_type, "push without a usable space");
        return false;
    }
    let key = dedupe_key(&event);
    if !state.first_seen(key) {
        tracing::debug!(event_type = %event.event_type, space = %event.space, "duplicate push dropped");
        return false;
    }
    let Some(event) = hydrate(state, event).await else {
        return false;
    };
    tracing::debug!(event_type = %event.event_type, space = %event.space, "fan out");
    state
        .hub
        .broadcast_space(&event.space.clone(), &ServerMsg::Event { event }, None);
    true
}

/// Dedupe identity: the changed resource plus the event type, so a created and
/// a deleted event for the same message are never collapsed into one.
pub fn dedupe_key(ev: &ChatEvent) -> String {
    if let Some(m) = &ev.message {
        return format!(
            "{}|{}|{}",
            ev.event_type,
            m.name,
            m.last_update_time.as_deref().unwrap_or("")
        );
    }
    if let Some(name) = ev.reaction.as_ref().and_then(|r| r.name.as_deref()) {
        return format!("{}|{}", ev.event_type, name);
    }
    if let Some(m) = &ev.membership {
        return format!("{}|{}", ev.event_type, m.name);
    }
    format!("{}|{}|{}", ev.event_type, ev.space, ev.event_time)
}

/// A message event that carries only the resource name (includeResource false)
/// is completed with a `GET /v1/{name}` on behalf of any subscribed session.
async fn hydrate(state: &AppState, mut event: ChatEvent) -> Option<ChatEvent> {
    let message_event = matches!(
        event.event_type.as_str(),
        event_types::MESSAGE_CREATED | event_types::MESSAGE_UPDATED
    );
    let name_only = event
        .message
        .as_ref()
        .is_some_and(|m| m.create_time.is_none() && m.text.is_none() && m.sender.is_none());
    if !(message_event && name_only) {
        return Some(event);
    }
    let name = event.message.as_ref()?.name.clone();
    if !valid_message(&name) {
        tracing::warn!(name, "event names a message outside the expected shape");
        return None;
    }
    let Some(token) = state.hub.any_token_for_space(&event.space) else {
        tracing::debug!(space = %event.space, "no subscriber to fetch the message with");
        return None;
    };
    match state.google.get_message(&token, &name).await {
        Ok(m) => {
            event.message = Some(m);
            Some(event)
        }
        Err(e) => {
            tracing::warn!(name, error = %e, "could not fetch event message");
            None
        }
    }
}

async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    query_secret: Option<&str>,
) -> Result<(), StatusCode> {
    if !state.config.real_mode {
        let presented = headers
            .get("x-prism-push-secret")
            .and_then(|v| v.to_str().ok())
            .or(query_secret);
        return match presented {
            Some(s) if constant_eq(s.as_bytes(), state.config.push_secret.as_bytes()) => Ok(()),
            _ => {
                tracing::warn!("push rejected: bad or missing shared secret");
                Err(StatusCode::UNAUTHORIZED)
            }
        };
    }
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let Some(token) = token else {
        tracing::warn!("push rejected: no bearer token");
        return Err(StatusCode::UNAUTHORIZED);
    };
    verify_google_oidc(state, token).await.map_err(|e| {
        tracing::warn!(error = %e, "push rejected: OIDC token invalid");
        StatusCode::UNAUTHORIZED
    })
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[derive(Deserialize)]
struct Claims {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    email_verified: Option<bool>,
}

/// RS256 signature against Google's JWKS, `aud`, `iss`, `exp`, then the
/// service account email and `email_verified`.
async fn verify_google_oidc(state: &AppState, token: &str) -> anyhow::Result<()> {
    let aud = state
        .config
        .push_audience
        .as_deref()
        .ok_or_else(|| anyhow!("PRISM_PUSH_AUDIENCE is not set"))?;
    let sa = state
        .config
        .push_service_account
        .as_deref()
        .ok_or_else(|| anyhow!("PRISM_PUSH_SERVICE_ACCOUNT is not set"))?;
    let header = decode_header(token).context("jwt header")?;
    if header.alg != Algorithm::RS256 {
        bail!("unexpected alg {:?}", header.alg);
    }
    let kid = header.kid.ok_or_else(|| anyhow!("jwt without kid"))?;
    let jwk = jwk_for(state, &kid).await?;
    let key = DecodingKey::from_jwk(&jwk).context("jwk to key")?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[aud]);
    validation.set_issuer(&GOOGLE_ISSUERS);
    validation.set_required_spec_claims(&["exp", "aud", "iss"]);
    let data = decode::<Claims>(token, &key, &validation).context("jwt verify")?;
    if data.claims.email.as_deref() != Some(sa) {
        bail!(
            "email {:?} is not the push service account",
            data.claims.email
        );
    }
    if data.claims.email_verified != Some(true) {
        bail!("email not verified");
    }
    Ok(())
}

async fn jwk_for(state: &AppState, kid: &str) -> anyhow::Result<Jwk> {
    {
        let cache = state.jwks.read().await;
        if let Some(c) = cache.as_ref() {
            if c.fetched_at.elapsed() < JWKS_TTL {
                if let Some(j) = c.keys.find(kid) {
                    return Ok(j.clone());
                }
            }
        }
    }
    let mut cache = state.jwks.write().await;
    if let Some(c) = cache.as_ref() {
        if c.fetched_at.elapsed() < JWKS_REFETCH_FLOOR {
            return c
                .keys
                .find(kid)
                .cloned()
                .ok_or_else(|| anyhow!("unknown kid {kid}"));
        }
    }
    let keys: JwkSet = state
        .http
        .get(&state.config.jwks_url)
        .send()
        .await
        .context("fetch jwks")?
        .error_for_status()
        .context("jwks status")?
        .json()
        .await
        .context("jwks body")?;
    let found = keys.find(kid).cloned();
    *cache = Some(JwksCache {
        fetched_at: Instant::now(),
        keys,
    });
    found.ok_or_else(|| anyhow!("unknown kid {kid}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_proto::chat::{Membership, Message, Reaction};

    #[test]
    fn dedupe_keys_distinguish_type_and_version() {
        let mut ev = ChatEvent {
            event_type: event_types::MESSAGE_CREATED.into(),
            space: "spaces/A".into(),
            event_time: "t1".into(),
            message: Some(Message {
                name: "spaces/A/messages/1".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let created = dedupe_key(&ev);
        ev.event_type = event_types::MESSAGE_DELETED.into();
        assert_ne!(created, dedupe_key(&ev));
        ev.event_type = event_types::MESSAGE_UPDATED.into();
        let u1 = dedupe_key(&ev);
        ev.message.as_mut().unwrap().last_update_time = Some("t2".into());
        assert_ne!(u1, dedupe_key(&ev));

        let r = ChatEvent {
            event_type: event_types::REACTION_CREATED.into(),
            reaction: Some(Reaction {
                name: Some("spaces/A/messages/1/reactions/9".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(dedupe_key(&r).ends_with("reactions/9"));
        let m = ChatEvent {
            event_type: event_types::MEMBERSHIP_DELETED.into(),
            membership: Some(Membership {
                name: "spaces/A/members/u".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(dedupe_key(&m).ends_with("members/u"));
        let s = ChatEvent {
            event_type: event_types::SPACE_UPDATED.into(),
            space: "spaces/A".into(),
            event_time: "t".into(),
            ..Default::default()
        };
        assert_eq!(
            dedupe_key(&s),
            "google.workspace.chat.space.v1.updated|spaces/A|t"
        );
    }

    #[test]
    fn constant_eq_basics() {
        assert!(constant_eq(b"dev", b"dev"));
        assert!(!constant_eq(b"dev", b"dev2"));
        assert!(!constant_eq(b"dev", b"dex"));
    }
}
