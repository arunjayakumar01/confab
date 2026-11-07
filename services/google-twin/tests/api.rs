//! End-to-end tests against the router (tower oneshot) and, for events, a real
//! listener standing in for the relay or the Pub/Sub emulator.

use axum::{
    body::{Body, Bytes},
    http::{header, HeaderMap, Request, StatusCode},
    routing::post,
    Router,
};
use google_twin::{app, AppState, Config};
use http_body_util::BodyExt;
use prism_proto::chat::{Message, TokenResponse, UserInfo};
use prism_proto::events::PubSubPush;
use prism_proto::relay::event_types;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

fn twin() -> (Router, Arc<AppState>) {
    twin_with(Config::for_tests("http://127.0.0.1:9/events/push"))
}

fn twin_with(config: Config) -> (Router, Arc<AppState>) {
    let state = Arc::new(AppState::new(config));
    (app(state.clone()), state)
}

fn token(state: &AppState, user: &str) -> String {
    state.write().mint_token(user, None).access_token
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value, HeaderMap) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, body, headers)
}

fn req(method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    match body {
        Some(v) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&v).unwrap()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    }
}

fn form(path: &str, fields: &[(&str, &str)]) -> Request<Body> {
    let body: String = fields
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                k,
                url::form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap()
}

const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const REDIRECT: &str = "http://127.0.0.1:4242/callback";

fn auth_url(twin_user: Option<&str>) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    s.append_pair("client_id", "prism-dev")
        .append_pair("redirect_uri", REDIRECT)
        .append_pair("response_type", "code")
        .append_pair("state", "st-123")
        .append_pair("code_challenge", CHALLENGE)
        .append_pair("code_challenge_method", "S256")
        .append_pair("scope", "openid email profile")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");
    if let Some(u) = twin_user {
        s.append_pair("twin_user", u);
    }
    format!("/o/oauth2/v2/auth?{}", s.finish())
}

async fn code_for(app: &Router, user: &str) -> String {
    let (status, _, headers) = send(app, req("GET", &auth_url(Some(user)), None, None)).await;
    assert_eq!(status, StatusCode::FOUND);
    let loc = url::Url::parse(headers[header::LOCATION].to_str().unwrap()).unwrap();
    assert!(loc.as_str().starts_with(REDIRECT));
    let pairs: std::collections::HashMap<_, _> = loc.query_pairs().into_owned().collect();
    assert_eq!(pairs["state"], "st-123");
    pairs["code"].clone()
}

#[tokio::test]
async fn pkce_round_trip_to_userinfo() {
    let (app, _) = twin();

    // The sign-in page lists every seeded user.
    let (status, body, _) = send(&app, req("GET", &auth_url(None), None, None)).await;
    assert_eq!(status, StatusCode::OK);
    let html = body.as_str().unwrap();
    assert!(html.contains("<title>Sign in to Prism (twin)</title>"));
    assert_eq!(html.matches("&twin_user=users/").count(), 5);

    // Wrong verifier: 400 invalid_grant, and the code is spent.
    let code = code_for(&app, "users/1002").await;
    let (status, body, _) = send(
        &app,
        form(
            "/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("code_verifier", "wrong"),
                ("redirect_uri", REDIRECT),
                ("client_id", "prism-dev"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_grant");
    let (status, _, _) = send(
        &app,
        form(
            "/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("code_verifier", VERIFIER),
                ("redirect_uri", REDIRECT),
                ("client_id", "prism-dev"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Right verifier: a token with a refresh token.
    let code = code_for(&app, "users/1002").await;
    let (status, body, _) = send(
        &app,
        form(
            "/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("code_verifier", VERIFIER),
                ("redirect_uri", REDIRECT),
                ("client_id", "prism-dev"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tok: TokenResponse = serde_json::from_value(body).unwrap();
    assert_eq!(tok.expires_in, 3600);
    assert_eq!(tok.token_type, "Bearer");
    let refresh = tok.refresh_token.clone().expect("refresh token");

    let (status, body, _) = send(
        &app,
        req("GET", "/oauth2/v3/userinfo", Some(&tok.access_token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let info: UserInfo = serde_json::from_value(body).unwrap();
    assert_eq!(info.sub, "1002");
    assert_eq!(info.email.as_deref(), Some("priya@prism.test"));
    assert_eq!(info.hd.as_deref(), Some("prism.test"));
    assert!(info.picture.unwrap().ends_with("/twin/avatars/1002.svg"));

    // Refresh grant issues a fresh access token that also resolves.
    let (status, body, _) = send(
        &app,
        form(
            "/token",
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", &refresh),
                ("client_id", "prism-dev"),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let fresh: TokenResponse = serde_json::from_value(body).unwrap();
    assert_ne!(fresh.access_token, tok.access_token);
    let (status, body, _) = send(
        &app,
        req(
            "GET",
            "/oauth2/v3/userinfo",
            Some(&fresh.access_token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sub"], "1002");

    // Unknown token: Google-shaped 401.
    let (status, body, headers) = send(
        &app,
        req("GET", "/oauth2/v3/userinfo", Some("ya29.nope"), None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["status"], "UNAUTHENTICATED");
    assert_eq!(body["error"]["code"], 401);
    assert!(headers.contains_key(header::WWW_AUTHENTICATE));
    let (status, _, _) = send(&app, req("GET", "/v1/spaces", None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_spaces_filtered_by_membership() {
    let (app, state) = twin();
    let meera = token(&state, "users/1004");
    let arun = token(&state, "users/1001");

    let (status, body, _) = send(
        &app,
        req("GET", "/v1/spaces?pageSize=100", Some(&meera), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let spaces = body["spaces"].as_array().unwrap();
    assert_eq!(spaces.len(), 4, "general, design, random, announcements");
    assert!(spaces.iter().all(|s| s["spaceType"] == "SPACE"));
    assert!(spaces.iter().all(|s| s["name"] != "spaces/AAAAengineering"));
    assert!(body.get("nextPageToken").is_none());

    let (_, body, _) = send(&app, req("GET", "/v1/spaces", Some(&arun), None)).await;
    assert_eq!(body["spaces"].as_array().unwrap().len(), 7);

    // Pagination by token.
    let (_, body, _) = send(&app, req("GET", "/v1/spaces?pageSize=3", Some(&arun), None)).await;
    assert_eq!(body["spaces"].as_array().unwrap().len(), 3);
    let next = body["nextPageToken"].as_str().unwrap().to_string();
    let (_, body, _) = send(
        &app,
        req(
            "GET",
            &format!("/v1/spaces?pageSize=3&pageToken={next}"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(body["spaces"].as_array().unwrap().len(), 3);

    // Not a member: 403; unknown: 404.
    let (status, body, _) = send(
        &app,
        req("GET", "/v1/spaces/AAAAdm1001x1002", Some(&meera), None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["status"], "PERMISSION_DENIED");
    let (status, body, _) = send(
        &app,
        req("GET", "/v1/spaces/AAAAnothing", Some(&meera), None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["status"], "NOT_FOUND");

    // My membership, both spellings.
    let (status, _, _) = send(
        &app,
        req(
            "GET",
            "/v1/spaces/AAAAengineering/members/users/me",
            Some(&meera),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body, _) = send(
        &app,
        req(
            "GET",
            "/v1/spaces/AAAAengineering/members/users/me",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "spaces/AAAAengineering/members/1001");
    assert_eq!(body["role"], "ROLE_MANAGER");
    assert_eq!(body["member"]["name"], "users/1001");
    let (status, body, _) = send(
        &app,
        req(
            "GET",
            "/v1/spaces/AAAAengineering/members/users/1002",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["member"]["email"], "priya@prism.test");
    assert!(body["member"]["avatarUrl"]
        .as_str()
        .unwrap()
        .contains("/twin/avatars/1002.svg"));
    let (status, _, _) = send(
        &app,
        req(
            "GET",
            "/v1/spaces/AAAAengineering/members/users/1004",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, body, _) = send(
        &app,
        req(
            "GET",
            "/v1/spaces/AAAAengineering/members?pageSize=100",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(body["memberships"].as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn reply_option_without_a_thread_is_invalid_argument() {
    let (app, state) = twin();
    let arun = token(&state, "users/1001");
    let (status, body, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces/AAAAgeneral/messages?messageReplyOption=REPLY_MESSAGE_FALLBACK_TO_NEW_THREAD",
            Some(&arun),
            Some(json!({"text": "top level"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["status"], "INVALID_ARGUMENT");
}

#[tokio::test]
async fn create_message_with_client_id_then_409() {
    let (app, state) = twin();
    let arun = token(&state, "users/1001");
    let path = "/v1/spaces/AAAAgeneral/messages?messageId=client-abc-123";
    let (status, body, _) = send(
        &app,
        req(
            "POST",
            path,
            Some(&arun),
            Some(json!({"text": "hello <users/1002>, see **this**"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let msg: Message = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(
        msg.client_assigned_message_id.as_deref(),
        Some("client-abc-123")
    );
    assert!(msg.name.starts_with("spaces/AAAAgeneral/messages/"));
    assert!(
        !msg.message_id().starts_with("client-"),
        "name is server-assigned: {}",
        msg.name
    );
    assert_eq!(msg.thread_reply, Some(false));
    assert_eq!(
        msg.thread.as_ref().unwrap().name.as_deref(),
        Some(
            &format!(
                "spaces/AAAAgeneral/threads/{}",
                msg.message_id().split('.').next().unwrap()
            )[..]
        )
    );
    assert_eq!(msg.sender.as_ref().unwrap().name, "users/1001");
    assert_eq!(msg.annotations.len(), 1);
    assert_eq!(
        msg.annotations[0].annotation_type.as_deref(),
        Some("USER_MENTION")
    );
    assert_eq!(msg.annotations[0].start_index, Some(6));
    assert_eq!(msg.annotations[0].length, Some(12));
    assert_eq!(
        msg.annotations[0]
            .user_mention
            .as_ref()
            .unwrap()
            .user
            .as_ref()
            .unwrap()
            .name,
        "users/1002"
    );

    let (status, body, _) = send(
        &app,
        req("POST", path, Some(&arun), Some(json!({"text": "again"}))),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["status"], "ALREADY_EXISTS");

    // Alias lookup, edit, delete, 404 after.
    let (status, body, _) = send(
        &app,
        req(
            "GET",
            "/v1/spaces/AAAAgeneral/messages/client-abc-123",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], msg.name);
    let m_path = format!("/v1/spaces/AAAAgeneral/messages/{}", msg.message_id());
    let (status, body, _) = send(
        &app,
        req(
            "PATCH",
            &format!("{m_path}?updateMask=text"),
            Some(&arun),
            Some(json!({"text": "edited"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["text"], "edited");
    assert!(body["lastUpdateTime"].is_string());
    let priya = token(&state, "users/1002");
    let (status, _, _) = send(
        &app,
        req(
            "PATCH",
            &format!("{m_path}?updateMask=text"),
            Some(&priya),
            Some(json!({"text": "not mine"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = send(&app, req("DELETE", &m_path, Some(&arun), None)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(&app, req("GET", &m_path, Some(&arun), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Non-member cannot post; empty text rejected; bad id rejected.
    let meera = token(&state, "users/1004");
    let (status, _, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces/AAAAengineering/messages",
            Some(&meera),
            Some(json!({"text": "hi"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces/AAAAgeneral/messages",
            Some(&arun),
            Some(json!({"text": "  "})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["status"], "INVALID_ARGUMENT");
    let (status, _, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces/AAAAgeneral/messages?messageId=Nope",
            Some(&arun),
            Some(json!({"text": "x"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn thread_filter_returns_replies() {
    let (app, state) = twin();
    let arun = token(&state, "users/1001");
    let (status, body, _) = send(
        &app,
        req(
            "GET",
            "/v1/spaces/AAAAengineering/messages?pageSize=1000&orderBy=createTime%20desc",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let all: Vec<Message> = serde_json::from_value(body["messages"].clone()).unwrap();
    assert!(all.len() > 50);
    assert!(
        all.windows(2).all(|w| w[0].create_time >= w[1].create_time),
        "newest first"
    );
    let reply = all
        .iter()
        .find(|m| m.thread_reply == Some(true))
        .expect("a seeded reply");
    let thread = reply.thread.as_ref().unwrap().name.clone().unwrap();

    let filter =
        url::form_urlencoded::byte_serialize(format!("thread.name = \"{thread}\"").as_bytes())
            .collect::<String>();
    let (status, body, _) = send(
        &app,
        req(
            "GET",
            &format!("/v1/spaces/AAAAengineering/messages?pageSize=100&filter={filter}"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let in_thread: Vec<Message> = serde_json::from_value(body["messages"].clone()).unwrap();
    assert!(in_thread.len() >= 3, "head plus replies");
    assert!(in_thread
        .iter()
        .all(|m| m.thread.as_ref().unwrap().name.as_deref() == Some(thread.as_str())));
    assert_eq!(
        in_thread
            .iter()
            .filter(|m| m.thread_reply == Some(false))
            .count(),
        1
    );
    assert!(
        in_thread
            .windows(2)
            .all(|w| w[0].create_time <= w[1].create_time),
        "default order is ascending"
    );

    // Replying into the thread via the API lands there with threadReply true.
    let (status, body, _) = send(
        &app,
        req("POST", "/v1/spaces/AAAAengineering/messages?messageReplyOption=REPLY_MESSAGE_FALLBACK_TO_NEW_THREAD", Some(&arun), Some(json!({"text": "late reply", "thread": {"name": thread}}))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["threadReply"], true);
    assert_eq!(body["thread"]["name"], thread);
    let (_, body, _) = send(
        &app,
        req(
            "GET",
            &format!("/v1/spaces/AAAAengineering/messages?pageSize=100&filter={filter}"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(
        body["messages"].as_array().unwrap().len(),
        in_thread.len() + 1
    );

    // REPLY_MESSAGE_OR_FAIL on an unknown thread is 404; unspecified ignores the thread.
    let (status, _, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces/AAAAengineering/messages?messageReplyOption=REPLY_MESSAGE_OR_FAIL",
            Some(&arun),
            Some(json!({"text": "x", "thread": {"name": "spaces/AAAAengineering/threads/nope"}})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces/AAAAengineering/messages",
            Some(&arun),
            Some(json!({"text": "x", "thread": {"name": thread}})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["threadReply"], false);
    assert_ne!(body["thread"]["name"], thread);
}

#[tokio::test]
async fn reaction_toggle_updates_summaries() {
    let (app, state) = twin();
    let arun = token(&state, "users/1001");
    let priya = token(&state, "users/1002");
    let (_, body, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces/AAAAgeneral/messages",
            Some(&arun),
            Some(json!({"text": "react to me"})),
        ),
    )
    .await;
    let msg: Message = serde_json::from_value(body).unwrap();
    let m_path = format!("/v1/spaces/AAAAgeneral/messages/{}", msg.message_id());

    let (status, body, _) = send(
        &app,
        req(
            "POST",
            &format!("{m_path}/reactions"),
            Some(&priya),
            Some(json!({"emoji": {"unicode": "👍"}})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let reaction_name = body["name"].as_str().unwrap().to_string();
    assert!(reaction_name.starts_with(&format!("{}/reactions/", msg.name)));
    assert_eq!(body["user"]["name"], "users/1002");
    let (_, body, _) = send(
        &app,
        req(
            "POST",
            &format!("{m_path}/reactions"),
            Some(&arun),
            Some(json!({"emoji": {"unicode": "👍"}})),
        ),
    )
    .await;
    assert_eq!(body["user"]["name"], "users/1001");
    let (_, body, _) = send(
        &app,
        req(
            "POST",
            &format!("{m_path}/reactions"),
            Some(&arun),
            Some(json!({"emoji": {"unicode": "🎉"}})),
        ),
    )
    .await;
    assert_eq!(body["emoji"]["unicode"], "🎉");

    let (status, body, _) = send(
        &app,
        req(
            "POST",
            &format!("{m_path}/reactions"),
            Some(&priya),
            Some(json!({"emoji": {"unicode": "👍"}})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["status"], "ALREADY_EXISTS");

    let (_, body, _) = send(&app, req("GET", &m_path, Some(&arun), None)).await;
    let summaries = body["emojiReactionSummaries"].as_array().unwrap();
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0]["emoji"]["unicode"], "👍");
    assert_eq!(summaries[0]["reactionCount"], 2);
    assert_eq!(summaries[1]["reactionCount"], 1);

    let filter =
        url::form_urlencoded::byte_serialize(b"user.name = \"users/me\"").collect::<String>();
    let (_, body, _) = send(
        &app,
        req(
            "GET",
            &format!("{m_path}/reactions?filter={filter}"),
            Some(&priya),
            None,
        ),
    )
    .await;
    let mine = body["reactions"].as_array().unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0]["name"], reaction_name);
    let (_, body, _) = send(
        &app,
        req("GET", &format!("{m_path}/reactions"), Some(&priya), None),
    )
    .await;
    assert_eq!(body["reactions"].as_array().unwrap().len(), 3);

    // Only the owner can remove it.
    let r_id = reaction_name.rsplit('/').next().unwrap();
    let (status, _, _) = send(
        &app,
        req(
            "DELETE",
            &format!("{m_path}/reactions/{r_id}"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = send(
        &app,
        req(
            "DELETE",
            &format!("{m_path}/reactions/{r_id}"),
            Some(&priya),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(
        &app,
        req(
            "DELETE",
            &format!("{m_path}/reactions/{r_id}"),
            Some(&priya),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, body, _) = send(&app, req("GET", &m_path, Some(&arun), None)).await;
    let summaries = body["emojiReactionSummaries"].as_array().unwrap();
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0]["reactionCount"], 1);
}

/// A tiny receiver standing in for the relay (`/events/push`) and the Pub/Sub
/// emulator (`/v1/{topic}:publish`). Each request is forwarded as (path, headers, body).
async fn receiver() -> (
    String,
    tokio::sync::mpsc::UnboundedReceiver<(String, HeaderMap, Bytes)>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let handler = {
        let tx = tx.clone();
        move |uri: axum::http::Uri, headers: HeaderMap, body: Bytes| {
            let tx = tx.clone();
            async move {
                tx.send((uri.path().to_string(), headers, body)).unwrap();
                StatusCode::OK
            }
        }
    };
    let router = Router::new()
        .route("/events/push", post(handler.clone()))
        .route(
            "/v1/projects/floci-local/topics/prism-events:publish",
            post(handler),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("127.0.0.1:{}", addr.port()), rx)
}

#[tokio::test]
async fn message_create_pushes_event_to_relay() {
    let (host, mut rx) = receiver().await;
    let (app, state) = twin_with(Config::for_tests(&format!("http://{host}/events/push")));
    let arun = token(&state, "users/1001");
    let (status, body, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces/AAAAgeneral/messages",
            Some(&arun),
            Some(json!({"text": "ping"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (path, headers, bytes) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(path, "/events/push");
    assert_eq!(headers["x-prism-push-secret"], "dev");
    let push: PubSubPush = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        push.message.attributes["ce-subject"],
        "//chat.googleapis.com/spaces/AAAAgeneral"
    );
    assert!(push.message.message_id.is_some());
    let ev = push.into_chat_event().unwrap();
    assert_eq!(ev.event_type, event_types::MESSAGE_CREATED);
    assert_eq!(ev.space, "spaces/AAAAgeneral");
    let m = ev.message.unwrap();
    assert_eq!(m.name, body["name"]);
    assert_eq!(m.text.as_deref(), Some("ping"));

    // Edit, react, delete each push too, in order.
    let m_path = format!("/v1/spaces/AAAAgeneral/messages/{}", m.message_id());
    send(
        &app,
        req(
            "PATCH",
            &format!("{m_path}?updateMask=text"),
            Some(&arun),
            Some(json!({"text": "pong"})),
        ),
    )
    .await;
    send(
        &app,
        req(
            "POST",
            &format!("{m_path}/reactions"),
            Some(&arun),
            Some(json!({"emoji": {"unicode": "👀"}})),
        ),
    )
    .await;
    send(&app, req("DELETE", &m_path, Some(&arun), None)).await;
    let mut types = Vec::new();
    for _ in 0..3 {
        let (_, _, bytes) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let ev = serde_json::from_slice::<PubSubPush>(&bytes)
            .unwrap()
            .into_chat_event()
            .unwrap();
        if ev.event_type == event_types::MESSAGE_DELETED {
            let d = ev.message.as_ref().unwrap();
            assert!(d.delete_time.is_some());
            assert!(d.text.is_none());
        }
        if ev.event_type == event_types::REACTION_CREATED {
            assert_eq!(ev.reaction.unwrap().emoji.unicode.as_deref(), Some("👀"));
        }
        types.push(ev.event_type);
    }
    // Each push runs on its own task, so arrival order is not fixed, as with real Pub/Sub.
    types.sort();
    let mut expected = vec![
        event_types::MESSAGE_UPDATED,
        event_types::REACTION_CREATED,
        event_types::MESSAGE_DELETED,
    ];
    expected.sort();
    assert_eq!(types, expected);
}

#[tokio::test]
async fn publish_mode_posts_to_pubsub_emulator() {
    let (host, mut rx) = receiver().await;
    let mut config = Config::for_tests("http://127.0.0.1:9/never");
    config.pubsub_emulator_host = Some(host);
    config.pubsub_topic = Some("projects/floci-local/topics/prism-events".into());
    let (app, state) = twin_with(config);
    let arun = token(&state, "users/1001");
    send(
        &app,
        req(
            "POST",
            "/v1/spaces/AAAAgeneral/messages",
            Some(&arun),
            Some(json!({"text": "to the emulator"})),
        ),
    )
    .await;
    let (path, headers, bytes) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(path, "/v1/projects/floci-local/topics/prism-events:publish");
    assert!(!headers.contains_key("x-prism-push-secret"));
    assert!(!headers.contains_key(header::AUTHORIZATION));
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].get("messageId").is_none());
    let push = PubSubPush {
        message: serde_json::from_value(msgs[0].clone()).unwrap(),
        subscription: String::new(),
    };
    let ev = push.into_chat_event().unwrap();
    assert_eq!(ev.event_type, event_types::MESSAGE_CREATED);
    assert_eq!(ev.message.unwrap().text.as_deref(), Some("to the emulator"));
}

#[tokio::test]
async fn upload_then_attach_then_download() {
    let (app, state) = twin();
    let arun = token(&state, "users/1001");
    let png = b"\x89PNG\r\n\x1a\nfake-bytes".to_vec();
    let boundary = "prism-test-boundary";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json\r\n\r\n{{\"filename\":\"chart.png\"}}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"chart.png\"\r\nContent-Type: image/png\r\n\r\n").as_bytes());
    body.extend_from_slice(&png);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let request = Request::builder()
        .method("POST")
        .uri("/upload/v1/spaces/AAAAgeneral/attachments:upload?uploadType=multipart")
        .header(header::AUTHORIZATION, format!("Bearer {arun}"))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let (status, body, _) = send(&app, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let resource = body["attachmentDataRef"]["resourceName"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(body["attachmentDataRef"]["attachmentUploadToken"].is_string());

    let (status, body, _) = send(
        &app,
        req("POST", "/v1/spaces/AAAAgeneral/messages", Some(&arun), Some(json!({"text": "chart attached", "attachment": [{"attachmentDataRef": {"resourceName": resource}}]}))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let msg: Message = serde_json::from_value(body).unwrap();
    assert_eq!(msg.attachment.len(), 1);
    let att = &msg.attachment[0];
    assert_eq!(att.content_name.as_deref(), Some("chart.png"));
    assert_eq!(att.content_type.as_deref(), Some("image/png"));
    assert_eq!(att.source.as_deref(), Some("UPLOADED_CONTENT"));
    assert!(att
        .name
        .as_ref()
        .unwrap()
        .starts_with(&format!("{}/attachments/", msg.name)));
    let download = att.download_uri.clone().unwrap();
    assert!(download.contains(&format!("/v1/media/{resource}?alt=media")));

    let (status, _, headers) = send(
        &app,
        req(
            "GET",
            &format!("/v1/media/{resource}?alt=media"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/png");
    let resp = app
        .clone()
        .oneshot(req(
            "GET",
            &format!("/v1/media/{resource}?alt=media&access_token={arun}"),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
        png
    );

    // Seeded image attachments are downloadable and member-gated.
    let (status, body, _) = send(
        &app,
        req(
            "GET",
            "/v1/spaces/AAAAengineering/messages?pageSize=1000",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let seeded: Vec<Message> = serde_json::from_value(body["messages"].clone()).unwrap();
    let with_image = seeded
        .iter()
        .find(|m| !m.attachment.is_empty())
        .expect("seeded attachment");
    let seeded_resource = with_image.attachment[0]
        .attachment_data_ref
        .as_ref()
        .unwrap()
        .resource_name
        .clone()
        .unwrap();
    let (status, _, headers) = send(
        &app,
        req(
            "GET",
            &format!("/v1/media/{seeded_resource}?alt=media"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/svg+xml");
    let meera = token(&state, "users/1004");
    let (status, _, _) = send(
        &app,
        req(
            "GET",
            &format!("/v1/media/{seeded_resource}?alt=media"),
            Some(&meera),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = send(
        &app,
        req(
            "GET",
            "/v1/media/spaces/AAAAgeneral/attachments/nope?alt=media",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn search_read_state_and_avatars() {
    let (app, state) = twin();
    let arun = token(&state, "users/1001");
    let meera = token(&state, "users/1004");

    let (status, body, _) = send(
        &app,
        req(
            "GET",
            "/v1/spaces/-/messages:search?query=Streaming&pageSize=50",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let hits: Vec<Message> = serde_json::from_value(body["messages"].clone()).unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|m| m
        .text
        .as_ref()
        .unwrap()
        .to_lowercase()
        .contains("streaming")));
    assert!(hits
        .windows(2)
        .all(|w| w[0].create_time >= w[1].create_time));
    assert!(hits
        .iter()
        .any(|m| m.space_name() == "spaces/AAAAengineering"));
    let (_, body, _) = send(
        &app,
        req(
            "GET",
            "/v1/spaces/-/messages:search?query=Streaming",
            Some(&meera),
            None,
        ),
    )
    .await;
    let hits: Vec<Message> = serde_json::from_value(body["messages"].clone()).unwrap();
    assert!(
        hits.iter()
            .all(|m| m.space_name() != "spaces/AAAAengineering"),
        "search is membership-scoped"
    );
    let (status, _, _) = send(
        &app,
        req("GET", "/v1/spaces/-/messages:search", Some(&arun), None),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, body, _) = send(
        &app,
        req(
            "GET",
            "/v1/users/me/spaces/AAAAgeneral/spaceReadState",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "users/1001/spaces/AAAAgeneral/spaceReadState");
    assert!(body["lastReadTime"].is_string());
    let (status, body, _) = send(
        &app,
        req(
            "PATCH",
            "/v1/users/me/spaces/AAAAgeneral/spaceReadState?updateMask=lastReadTime",
            Some(&arun),
            Some(json!({"lastReadTime": "2099-01-01T00:00:00Z"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let coerced = body["lastReadTime"].as_str().unwrap();
    assert!(
        coerced.starts_with("20") && !coerced.starts_with("2099"),
        "coerced to the latest message: {coerced}"
    );
    let (_, body, _) = send(
        &app,
        req(
            "GET",
            "/v1/users/1001/spaces/AAAAgeneral/spaceReadState",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(body["lastReadTime"], coerced);
    let (status, _, _) = send(
        &app,
        req(
            "GET",
            "/v1/users/1002/spaces/AAAAgeneral/spaceReadState",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = send(
        &app,
        req(
            "PATCH",
            "/v1/users/me/spaces/AAAAgeneral/spaceReadState",
            Some(&arun),
            Some(json!({"lastReadTime": "2026-01-01T00:00:00Z"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, body, headers) =
        send(&app, req("GET", "/twin/avatars/1003.svg", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/svg+xml");
    assert!(body.as_str().unwrap().contains(">RM<"));
    let (_, body, _) = send(&app, req("GET", "/twin/users", None, None)).await;
    assert_eq!(body["users"].as_array().unwrap().len(), 5);
    let (status, body, _) =
        send(&app, req("POST", "/twin/token?user=users/1005", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    let (_, info, _) = send(
        &app,
        req(
            "GET",
            "/oauth2/v3/userinfo",
            body["access_token"].as_str(),
            None,
        ),
    )
    .await;
    assert_eq!(info["sub"], "1005");

    // CORS preflight from a browser origin is allowed.
    let preflight = Request::builder()
        .method("OPTIONS")
        .uri("/v1/spaces")
        .header(header::ORIGIN, "http://localhost:1420")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
        .body(Body::empty())
        .unwrap();
    let (status, _, headers) = send(&app, preflight).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
}

#[tokio::test]
async fn membership_changes_and_thread_delete_semantics() {
    let (host, mut rx) = receiver().await;
    let (app, state) = twin_with(Config::for_tests(&format!("http://{host}/events/push")));
    let arun = token(&state, "users/1001");
    let meera = token(&state, "users/1004");

    let (status, body, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces/AAAAengineering/members",
            Some(&arun),
            Some(json!({"member": {"name": "users/1004", "type": "HUMAN"}})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "spaces/AAAAengineering/members/1004");
    assert_eq!(body["state"], "JOINED");
    let (status, _, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces/AAAAengineering/members",
            Some(&arun),
            Some(json!({"member": {"name": "users/1004"}})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _, _) = send(
        &app,
        req("GET", "/v1/spaces/AAAAengineering", Some(&meera), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, _, bytes) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let ev = serde_json::from_slice::<PubSubPush>(&bytes)
        .unwrap()
        .into_chat_event()
        .unwrap();
    assert_eq!(ev.event_type, event_types::MEMBERSHIP_CREATED);
    assert_eq!(ev.membership.unwrap().member.unwrap().name, "users/1004");

    let (status, body, _) = send(
        &app,
        req(
            "DELETE",
            "/v1/spaces/AAAAengineering/members/1004",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "NOT_A_MEMBER");
    let (status, _, _) = send(
        &app,
        req("GET", "/v1/spaces/AAAAengineering", Some(&meera), None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (_, _, bytes) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let ev = serde_json::from_slice::<PubSubPush>(&bytes)
        .unwrap()
        .into_chat_event()
        .unwrap();
    assert_eq!(ev.event_type, event_types::MEMBERSHIP_DELETED);

    // A thread head with replies needs force=true, and the replies go with it.
    let (_, head, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces/AAAAgeneral/messages",
            Some(&arun),
            Some(json!({"text": "head"})),
        ),
    )
    .await;
    let thread = head["thread"]["name"].as_str().unwrap().to_string();
    let (_, reply, _) = send(&app, req("POST", "/v1/spaces/AAAAgeneral/messages?messageReplyOption=REPLY_MESSAGE_FALLBACK_TO_NEW_THREAD", Some(&arun), Some(json!({"text": "reply", "thread": {"name": thread}})))).await;
    assert_eq!(reply["threadReply"], true);
    let head_id = head["name"].as_str().unwrap().rsplit('/').next().unwrap();
    let reply_id = reply["name"].as_str().unwrap().rsplit('/').next().unwrap();
    let (status, body, _) = send(
        &app,
        req(
            "DELETE",
            &format!("/v1/spaces/AAAAgeneral/messages/{head_id}"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["status"], "FAILED_PRECONDITION");
    let (status, _, _) = send(
        &app,
        req(
            "DELETE",
            &format!("/v1/spaces/AAAAgeneral/messages/{head_id}?force=true"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(
        &app,
        req(
            "GET",
            &format!("/v1/spaces/AAAAgeneral/messages/{reply_id}"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn directory_search_finds_users_and_excludes_caller() {
    let (app, state) = twin();
    let arun = token(&state, "users/1001");

    let (status, _, _) = send(
        &app,
        req(
            "GET",
            "/v1/people:searchDirectoryPeople?query=priya",
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body, _) = send(
        &app,
        req(
            "GET",
            "/v1/people:searchDirectoryPeople?query=PRIYA&readMask=names,emailAddresses,photos&sources=DIRECTORY_SOURCE_TYPE_DOMAIN_PROFILE",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let people = body["people"].as_array().unwrap();
    assert_eq!(people.len(), 1);
    assert_eq!(people[0]["resourceName"], "people/1002");
    assert_eq!(people[0]["names"][0]["displayName"], "Priya Nair");
    assert_eq!(people[0]["emailAddresses"][0]["value"], "priya@prism.test");
    assert!(people[0]["photos"][0]["url"]
        .as_str()
        .unwrap()
        .ends_with("/twin/avatars/1002.svg"));
    assert!(body.get("nextPageToken").is_none());

    // The caller never appears, even on an exact match.
    let (_, body, _) = send(
        &app,
        req(
            "GET",
            "/v1/people:searchDirectoryPeople?query=arun",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert!(body["people"].as_array().unwrap().is_empty());

    // Blank query: empty page. Missing query: 400.
    let (status, body, _) = send(
        &app,
        req(
            "GET",
            "/v1/people:searchDirectoryPeople?query=%20",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["people"].as_array().unwrap().is_empty());
    let (status, body, _) = send(
        &app,
        req("GET", "/v1/people:searchDirectoryPeople", Some(&arun), None),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["status"], "INVALID_ARGUMENT");

    // Email domain matches everyone but the caller; pageSize caps it.
    let (_, body, _) = send(
        &app,
        req(
            "GET",
            "/v1/people:searchDirectoryPeople?query=prism.test&pageSize=2",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(body["people"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn spaces_setup_direct_message_is_idempotent() {
    let (app, state) = twin();
    let rahul = token(&state, "users/1003");
    let meera = token(&state, "users/1004");
    let body = json!({
        "space": {"spaceType": "DIRECT_MESSAGE"},
        "memberships": [{"member": {"name": "users/1004", "type": "HUMAN"}}]
    });

    let (status, first, _) = send(
        &app,
        req("POST", "/v1/spaces:setup", Some(&rahul), Some(body.clone())),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["name"], "spaces/AAAAdm1003x1004");
    assert_eq!(first["spaceType"], "DIRECT_MESSAGE");
    assert!(first.get("displayName").is_none());
    assert_eq!(first["membershipCount"]["joinedDirectHumanUserCount"], 2);

    let (status, second, _) = send(
        &app,
        req("POST", "/v1/spaces:setup", Some(&rahul), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["name"], first["name"]);

    // The other side asking gets the same space, not a mirror of it.
    let (_, third, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces:setup",
            Some(&meera),
            Some(json!({
                "space": {"spaceType": "DIRECT_MESSAGE"},
                "memberships": [{"member": {"name": "users/1003"}}]
            })),
        ),
    )
    .await;
    assert_eq!(third["name"], first["name"]);

    // A seeded DM is found rather than duplicated.
    let arun = token(&state, "users/1001");
    let (_, seeded, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces:setup",
            Some(&arun),
            Some(json!({
                "space": {"spaceType": "DIRECT_MESSAGE"},
                "memberships": [{"member": {"name": "users/1002"}}]
            })),
        ),
    )
    .await;
    assert_eq!(seeded["name"], "spaces/AAAAdm1001x1002");

    for tok in [&rahul, &meera] {
        let (_, list, _) = send(&app, req("GET", "/v1/spaces", Some(tok), None)).await;
        assert!(list["spaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "spaces/AAAAdm1003x1004"));
        let (status, _, _) = send(
            &app,
            req(
                "POST",
                "/v1/spaces/AAAAdm1003x1004/messages",
                Some(tok),
                Some(json!({"text": "hi"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    // Shape errors: not exactly one membership, unknown member.
    let (status, err, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces:setup",
            Some(&rahul),
            Some(json!({"space": {"spaceType": "DIRECT_MESSAGE"}, "memberships": []})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err["error"]["status"], "INVALID_ARGUMENT");
    let (status, err, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces:setup",
            Some(&rahul),
            Some(json!({
                "space": {"spaceType": "DIRECT_MESSAGE"},
                "memberships": [{"member": {"name": "users/9999"}}]
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(err["error"]["status"], "NOT_FOUND");
}

#[tokio::test]
async fn spaces_setup_group_chat_adds_everyone() {
    let (app, state) = twin();
    let priya = token(&state, "users/1002");
    let body = json!({
        "space": {"spaceType": "GROUP_CHAT"},
        "memberships": [
            {"member": {"name": "users/1003", "type": "HUMAN"}},
            {"member": {"name": "users/1004", "type": "HUMAN"}},
            {"member": {"name": "users/1005", "type": "HUMAN"}}
        ]
    });

    let (status, space, _) = send(
        &app,
        req("POST", "/v1/spaces:setup", Some(&priya), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{space}");
    let name = space["name"].as_str().unwrap().to_string();
    assert!(name.starts_with("spaces/AAAAg"), "{name}");
    assert_eq!(space["spaceType"], "GROUP_CHAT");
    assert!(space.get("displayName").is_none());
    assert!(space["createTime"].as_str().is_some());
    assert_eq!(space["membershipCount"]["joinedDirectHumanUserCount"], 4);

    // Caller plus the three listed users, every one a plain member.
    let (status, list, _) = send(
        &app,
        req(
            "GET",
            &format!("/v1/{name}/members?pageSize=100"),
            Some(&priya),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let members = list["memberships"].as_array().unwrap();
    assert_eq!(members.len(), 4);
    let mut users: Vec<&str> = members
        .iter()
        .map(|m| m["member"]["name"].as_str().unwrap())
        .collect();
    users.sort_unstable();
    assert_eq!(
        users,
        ["users/1002", "users/1003", "users/1004", "users/1005"]
    );
    assert!(members.iter().all(|m| m["role"] == "ROLE_MEMBER"));
    for user in ["users/1003", "users/1004", "users/1005"] {
        let tok = token(&state, user);
        let (_, list, _) = send(&app, req("GET", "/v1/spaces", Some(&tok), None)).await;
        assert!(list["spaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == name));
    }

    // One member is a DM, not a group chat: 400.
    let (status, err, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces:setup",
            Some(&priya),
            Some(json!({
                "space": {"spaceType": "GROUP_CHAT"},
                "memberships": [{"member": {"name": "users/1003"}}]
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err["error"]["status"], "INVALID_ARGUMENT");
    assert_eq!(
        err["error"]["message"],
        "A group chat requires at least two memberships."
    );
}

#[tokio::test]
async fn spaces_setup_named_space_then_409() {
    let (host, mut rx) = receiver().await;
    let (app, state) = twin_with(Config::for_tests(&format!("http://{host}/events/push")));
    let priya = token(&state, "users/1002");
    let dev = token(&state, "users/1005");
    let body = json!({
        "space": {"spaceType": "SPACE", "displayName": "Launch plan"},
        "memberships": [{"member": {"name": "users/1005", "type": "HUMAN"}}]
    });

    let (status, space, _) = send(
        &app,
        req("POST", "/v1/spaces:setup", Some(&priya), Some(body.clone())),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{space}");
    let name = space["name"].as_str().unwrap().to_string();
    assert!(name.starts_with("spaces/AAAA"));
    assert_eq!(space["spaceType"], "SPACE");
    assert_eq!(space["displayName"], "Launch plan");
    assert!(space["createTime"].as_str().is_some());
    assert_eq!(space["membershipCount"]["joinedDirectHumanUserCount"], 2);
    let id = name.trim_start_matches("spaces/");

    // One membership.created per member, same event as create_member.
    for _ in 0..2 {
        let (_, _, bytes) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let ev = serde_json::from_slice::<PubSubPush>(&bytes)
            .unwrap()
            .into_chat_event()
            .unwrap();
        assert_eq!(ev.event_type, event_types::MEMBERSHIP_CREATED);
        assert_eq!(ev.membership.unwrap().name.rsplit('/').nth(2), Some(id));
    }

    // Caller is manager, the listed user a member; both see it listed and can post.
    let (_, m, _) = send(
        &app,
        req(
            "GET",
            &format!("/v1/spaces/{id}/members/users/me"),
            Some(&priya),
            None,
        ),
    )
    .await;
    assert_eq!(m["role"], "ROLE_MANAGER");
    let (_, m, _) = send(
        &app,
        req(
            "GET",
            &format!("/v1/spaces/{id}/members/users/me"),
            Some(&dev),
            None,
        ),
    )
    .await;
    assert_eq!(m["role"], "ROLE_MEMBER");
    for tok in [&priya, &dev] {
        let (_, list, _) = send(&app, req("GET", "/v1/spaces", Some(tok), None)).await;
        assert!(list["spaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == name));
    }
    let (status, msg, _) = send(
        &app,
        req(
            "POST",
            &format!("/v1/spaces/{id}/messages"),
            Some(&dev),
            Some(json!({"text": "kickoff"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{msg}");
    assert_eq!(msg["space"]["name"], name);

    // Same displayName again: 409 ALREADY_EXISTS in the Google envelope. Blank: 400.
    let (status, err, _) = send(
        &app,
        req("POST", "/v1/spaces:setup", Some(&priya), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(err["error"]["status"], "ALREADY_EXISTS");
    assert_eq!(err["error"]["code"], 409);
    let (status, err, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces:setup",
            Some(&priya),
            Some(json!({"space": {"spaceType": "SPACE", "displayName": "  "}})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err["error"]["message"], "displayName is required");
}

#[tokio::test]
async fn tasks_list_add_complete_delete() {
    let (app, state) = twin();
    let arun = token(&state, "users/1001");
    const LIST: &str = "/tasks/v1/lists/@default/tasks";

    // Bearer required.
    let (status, _, _) = send(&app, req("GET", LIST, None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The seed: two tasks, the second due tomorrow at midnight UTC.
    let (status, body, _) = send(
        &app,
        req(
            "GET",
            &format!("{LIST}?showCompleted=true&showHidden=false&maxResults=100"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], "task-1");
    assert_eq!(items[0]["title"], "Review the relay reconnect PR");
    assert_eq!(items[0]["notes"], "spaces/AAAAgeneral thread from Priya");
    assert_eq!(items[0]["status"], "needsAction");
    assert_eq!(items[0]["webViewLink"], "https://tasks.google.com/");
    assert!(items[0].get("completed").is_none());
    assert_eq!(items[1]["id"], "task-2");
    assert_eq!(items[1]["title"], "Send the design spec to Priya");
    let due = items[1]["due"].as_str().unwrap();
    assert!(due.ends_with("T00:00:00.000Z"), "{due}");
    let expected = (chrono::Utc::now() + chrono::Duration::days(1))
        .date_naive()
        .to_string();
    assert!(due.starts_with(&expected), "{due} vs {expected}");

    // Add: 201, task-3, needsAction, updated set.
    let (status, created, _) = send(
        &app,
        req(
            "POST",
            LIST,
            Some(&arun),
            Some(json!({ "title": "Book the offsite room", "notes": "Thursday 4pm" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["id"], "task-3");
    assert_eq!(created["title"], "Book the offsite room");
    assert_eq!(created["notes"], "Thursday 4pm");
    assert_eq!(created["status"], "needsAction");
    assert!(created["updated"].as_str().unwrap().ends_with('Z'));
    assert!(created.get("due").is_none());
    let (_, body, _) = send(&app, req("GET", LIST, Some(&arun), None)).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 3);

    // Patch to completed sets `completed`; back to needsAction clears it.
    let (status, done, _) = send(
        &app,
        req(
            "PATCH",
            &format!("{LIST}/task-3"),
            Some(&arun),
            Some(json!({ "status": "completed" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["status"], "completed");
    assert!(done["completed"].as_str().unwrap().ends_with('Z'));
    assert_eq!(done["title"], "Book the offsite room");
    let (status, undone, _) = send(
        &app,
        req(
            "PATCH",
            &format!("{LIST}/task-3"),
            Some(&arun),
            Some(
                json!({ "status": "needsAction", "completed": null, "title": "Book the big room" }),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{undone}");
    assert_eq!(undone["status"], "needsAction");
    assert!(undone.get("completed").is_none());
    assert_eq!(undone["title"], "Book the big room");
    let (status, body, _) = send(
        &app,
        req(
            "PATCH",
            &format!("{LIST}/task-3"),
            Some(&arun),
            Some(json!({ "status": "done" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["status"], "INVALID_ARGUMENT");

    // Delete: 204 with no body, then 404 everywhere.
    let (status, body, _) = send(
        &app,
        req("DELETE", &format!("{LIST}/task-3"), Some(&arun), None),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(body, Value::String(String::new()));
    for method in ["GET", "PATCH", "DELETE"] {
        let body = (method == "PATCH").then(|| json!({ "status": "completed" }));
        let (status, body, _) = send(
            &app,
            req(method, &format!("{LIST}/task-3"), Some(&arun), body),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}: {body}");
        assert_eq!(body["error"]["status"], "NOT_FOUND");
    }
    let (_, body, _) = send(&app, req("GET", LIST, Some(&arun), None)).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn meet_create_get_participants_and_404() {
    let (app, state) = twin();
    let arun = token(&state, "users/1001");

    // Bearer required.
    let (status, _, _) = send(&app, req("POST", "/v2/spaces", None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Create: Google's shape, a TRUSTED config, a code like abc-defg-hij.
    let (status, space, _) = send(
        &app,
        req(
            "POST",
            "/v2/spaces",
            Some(&arun),
            Some(json!({ "config": { "accessType": "TRUSTED" } })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{space}");
    assert_eq!(space["name"], "spaces/meet-1");
    let code = space["meetingCode"].as_str().unwrap().to_string();
    let parts: Vec<&str> = code.split('-').collect();
    assert_eq!(parts.len(), 3, "{code}");
    assert_eq!(
        (parts[0].len(), parts[1].len(), parts[2].len()),
        (3, 4, 3),
        "{code}"
    );
    assert!(
        code.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
        "{code}"
    );
    assert_eq!(
        space["meetingUri"],
        format!("https://meet.google.com/{code}")
    );
    assert_eq!(space["config"]["accessType"], "TRUSTED");
    assert_eq!(space["config"]["entryPointAccess"], "ALL");
    assert_eq!(
        space["activeConference"]["conferenceRecord"],
        "conferenceRecords/meet-1"
    );

    // Get by code and by name: the same space, still active.
    for key in [code.as_str(), "meet-1"] {
        let (status, got, _) = send(
            &app,
            req("GET", &format!("/v2/spaces/{key}"), Some(&arun), None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{key}: {got}");
        assert_eq!(got["name"], "spaces/meet-1");
        assert_eq!(got["meetingCode"], code);
        assert_eq!(
            got["activeConference"]["conferenceRecord"],
            "conferenceRecords/meet-1"
        );
    }

    // Participants right away: the creator is in, Rahul already left, Priya not yet.
    const PARTICIPANTS: &str = "/v2/conferenceRecords/meet-1/participants?pageSize=100";
    let (status, _, _) = send(&app, req("GET", PARTICIPANTS, None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body, _) = send(&app, req("GET", PARTICIPANTS, Some(&arun), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let people = body["participants"].as_array().unwrap();
    let creator = people
        .iter()
        .find(|p| p["signedinUser"]["user"] == "users/1001")
        .expect("creator listed");
    assert_eq!(creator["signedinUser"]["displayName"], "Arun Jayakumar");
    assert_eq!(creator["name"], "conferenceRecords/meet-1/participants/1");
    assert!(creator["earliestStartTime"]
        .as_str()
        .unwrap()
        .ends_with('Z'));
    assert!(creator.get("latestEndTime").is_none());
    let rahul = people
        .iter()
        .find(|p| p["signedinUser"]["user"] == "users/1003")
        .expect("Rahul listed");
    assert_eq!(rahul["signedinUser"]["displayName"], "Rahul Mehta");
    assert!(rahul["latestEndTime"].as_str().unwrap().ends_with('Z'));

    // Ten seconds in (the clock is rewound in the store): Priya has joined and stays.
    state
        .write()
        .meets
        .get_mut("spaces/meet-1")
        .unwrap()
        .created_at -= chrono::Duration::seconds(10);
    let (_, body, _) = send(&app, req("GET", PARTICIPANTS, Some(&arun), None)).await;
    let people = body["participants"].as_array().unwrap();
    assert_eq!(people.len(), 3);
    let priya = people
        .iter()
        .find(|p| p["signedinUser"]["user"] == "users/1002")
        .expect("Priya listed");
    assert_eq!(priya["signedinUser"]["displayName"], "Priya Nair");
    assert!(priya.get("latestEndTime").is_none());
    let creator_now = people
        .iter()
        .find(|p| p["signedinUser"]["user"] == "users/1001")
        .expect("creator still listed");
    assert!(
        priya["earliestStartTime"].as_str().unwrap()
            > creator_now["earliestStartTime"].as_str().unwrap(),
        "Priya joined after the creator"
    );

    // Twenty minutes in: the conference is over, the space remains.
    state
        .write()
        .meets
        .get_mut("spaces/meet-1")
        .unwrap()
        .created_at -= chrono::Duration::minutes(20);
    let (status, got, _) = send(
        &app,
        req("GET", &format!("/v2/spaces/{code}"), Some(&arun), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got["meetingCode"], code);
    assert!(got.get("activeConference").is_none(), "{got}");

    // A second space gets its own code.
    let (_, second, _) = send(&app, req("POST", "/v2/spaces", Some(&arun), None)).await;
    assert_eq!(second["name"], "spaces/meet-2");
    assert_ne!(second["meetingCode"], code);

    // Unknown code and unknown record: Google-shaped 404s.
    let (status, body, _) = send(
        &app,
        req("GET", "/v2/spaces/zzz-zzzz-zzz", Some(&arun), None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["status"], "NOT_FOUND");
    assert_eq!(body["error"]["code"], 404);
    let (status, body, _) = send(
        &app,
        req(
            "GET",
            "/v2/conferenceRecords/meet-9/participants",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["status"], "NOT_FOUND");
}

#[tokio::test]
async fn huddle_message_keeps_its_rich_link() {
    let (app, state) = twin();
    let arun = token(&state, "users/1001");
    let annotation = json!({
        "type": "RICH_LINK",
        "startIndex": 0,
        "richLinkMetadata": {
            "uri": "https://meet.google.com/kyf-vogr-zmm",
            "richLinkType": "MEET_SPACE",
            "meetSpaceLinkData": {
                "meetingCode": "kyf-vogr-zmm",
                "type": "HUDDLE",
                "huddleStatus": "STARTED"
            }
        }
    });
    let (status, body, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces/AAAAgeneral/messages?messageId=client-huddle-1",
            Some(&arun),
            Some(json!({ "text": "", "annotations": [annotation] })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["annotations"][0], annotation, "{body}");
    assert_eq!(body["clientAssignedMessageId"], "client-huddle-1");
    assert_eq!(body["sender"]["name"], "users/1001");
    let msg: Message = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(msg.text.as_deref(), Some(""));
    assert_eq!(
        msg.annotations[0].huddle_link(),
        Some(("kyf-vogr-zmm", "STARTED"))
    );

    // It reads back the same, and a list of the space carries it.
    let (_, got, _) = send(
        &app,
        req("GET", &format!("/v1/{}", msg.name), Some(&arun), None),
    )
    .await;
    assert_eq!(got["annotations"][0], annotation);

    // No text, no attachment, no rich link: still refused.
    let (status, body, _) = send(
        &app,
        req(
            "POST",
            "/v1/spaces/AAAAgeneral/messages",
            Some(&arun),
            Some(
                json!({ "text": "", "annotations": [{ "type": "USER_MENTION", "startIndex": 0 }] }),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

// ----- calendar -----

/// `days` from today at `h:m` in the twin's local zone, as the twin prints `dateTime`.
fn local_time(days: i64, h: u32, m: u32) -> String {
    use chrono::TimeZone;
    let date = chrono::Local::now().date_naive() + chrono::Duration::days(days);
    let naive = date.and_hms_opt(h, m, 0).unwrap();
    chrono::Local
        .from_local_datetime(&naive)
        .earliest()
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// A query value: the `+` of an offset must not arrive as a space.
fn enc(s: &str) -> String {
    s.replace('+', "%2B")
}

fn instant(v: &Value) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(v.as_str().expect("a time"))
        .unwrap()
        .with_timezone(&chrono::Utc)
}

#[tokio::test]
async fn calendar_list_is_per_caller() {
    let (app, state) = twin();
    const LIST: &str = "/calendar/v3/users/me/calendarList?minAccessRole=reader";
    let (status, _, _) = send(&app, req("GET", LIST, None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let arun = token(&state, "users/1001");
    let (status, body, _) = send(&app, req("GET", LIST, Some(&arun), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["kind"], "calendar#calendarList");
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], "primary");
    assert_eq!(items[0]["summary"], "Arun Jayakumar");
    assert_eq!(items[0]["primary"], true);
    assert_eq!(items[0]["backgroundColor"], "#7986cb");
    assert_eq!(items[0]["accessRole"], "owner");
    assert_eq!(items[1]["id"], "team@example.test");
    assert_eq!(items[1]["summary"], "Team");
    assert!(items[1].get("primary").is_none());
    assert_eq!(items[1]["backgroundColor"], "#33b679");
    assert_eq!(items[1]["accessRole"], "reader");

    let priya = token(&state, "users/1002");
    let (_, body, _) = send(&app, req("GET", LIST, Some(&priya), None)).await;
    assert_eq!(body["items"][0]["summary"], "Priya Nair");
}

#[tokio::test]
async fn calendar_events_window_and_ordering() {
    let (app, state) = twin();
    let arun = token(&state, "users/1001");
    const EVENTS: &str = "/calendar/v3/calendars/primary/events";
    let (status, _, _) = send(&app, req("GET", EVENTS, None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Yesterday to the day after tomorrow, ordered by start: the three primary events.
    let path = format!(
        "{EVENTS}?singleEvents=true&maxResults=250&showDeleted=true&orderBy=startTime&timeMin={}&timeMax={}",
        enc(&local_time(-1, 0, 0)),
        enc(&local_time(2, 0, 0))
    );
    let (status, body, _) = send(&app, req("GET", &path, Some(&arun), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["kind"], "calendar#events");
    assert_eq!(body["accessRole"], "owner");
    assert_eq!(body["nextSyncToken"], "5", "four seeded changes");
    let items = body["items"].as_array().unwrap();
    let mut ids: Vec<&str> = items.iter().map(|e| e["id"].as_str().unwrap()).collect();
    ids.sort();
    assert_eq!(ids, ["ev-1", "ev-2", "ev-3"]);
    let starts: Vec<_> = items
        .iter()
        .map(|e| instant(&e["start"]["dateTime"]))
        .collect();
    assert!(starts.windows(2).all(|w| w[0] <= w[1]), "{starts:?}");

    let standup = items.iter().find(|e| e["id"] == "ev-1").unwrap();
    assert_eq!(standup["kind"], "calendar#event");
    assert_eq!(standup["summary"], "Standup");
    assert_eq!(standup["status"], "confirmed");
    assert_eq!(
        standup["htmlLink"],
        "https://calendar.google.com/event?eid=ev-1"
    );
    assert_eq!(
        standup["hangoutLink"],
        "https://meet.google.com/abc-defg-hij"
    );
    assert_eq!(standup["start"]["dateTime"], local_time(0, 10, 0));
    assert_eq!(standup["end"]["dateTime"], local_time(0, 10, 15));
    assert!(standup["created"].as_str().unwrap().ends_with('Z'));
    assert!(standup["updated"].as_str().unwrap().ends_with('Z'));
    assert!(standup["etag"].is_string());
    assert!(standup.get("calendarId").is_none());
    let attendees = standup["attendees"].as_array().unwrap();
    assert_eq!(attendees.len(), 3);
    assert_eq!(attendees[0]["email"], "arun@prism.test");
    assert_eq!(attendees[0]["displayName"], "Arun Jayakumar");
    assert_eq!(attendees[0]["self"], true);
    assert_eq!(attendees[0]["organizer"], true);
    assert_eq!(attendees[0]["responseStatus"], "accepted");
    assert_eq!(attendees[1]["email"], "priya@prism.test");
    assert_eq!(attendees[1]["responseStatus"], "accepted");
    assert!(attendees[1].get("self").is_none());
    assert!(attendees[1].get("organizer").is_none());
    assert_eq!(attendees[2]["email"], "rahul@prism.test");
    assert_eq!(attendees[2]["responseStatus"], "needsAction");
    assert_eq!(standup["organizer"]["email"], "arun@prism.test");
    assert_eq!(standup["organizer"]["self"], true);

    let review = items.iter().find(|e| e["id"] == "ev-2").unwrap();
    assert_eq!(review["summary"], "Design review");
    assert_eq!(review["location"], "Room 2B");
    assert!(review.get("hangoutLink").is_none());
    assert_eq!(
        review["conferenceData"]["entryPoints"][0]["entryPointType"],
        "video"
    );
    assert!(review["conferenceData"]["entryPoints"][0]["uri"]
        .as_str()
        .unwrap()
        .starts_with("https://zoom.example.test/"));
    let review_len = instant(&review["end"]["dateTime"]) - instant(&review["start"]["dateTime"]);
    assert_eq!(review_len, chrono::Duration::minutes(45));
    let deploy = items.iter().find(|e| e["id"] == "ev-3").unwrap();
    assert_eq!(deploy["summary"], "Relay deploy window");
    assert!(deploy.get("attendees").is_none());
    assert_eq!(deploy["start"]["dateTime"], local_time(1, 15, 0));
    assert_eq!(deploy["end"]["dateTime"], local_time(1, 16, 0));

    // Tomorrow afternoon holds the deploy window alone; a window inside it overlaps.
    for (min, max) in [((1, 12, 0), (1, 23, 59)), ((1, 15, 30), (1, 15, 45))] {
        let path = format!(
            "{EVENTS}?orderBy=startTime&timeMin={}&timeMax={}",
            enc(&local_time(min.0, min.1, min.2)),
            enc(&local_time(max.0, max.1, max.2))
        );
        let (status, body, _) = send(&app, req("GET", &path, Some(&arun), None)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let items = body["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "{min:?}..{max:?}: {body}");
        assert_eq!(items[0]["id"], "ev-3");
    }
    // timeMax is exclusive of an event's start.
    let path = format!(
        "{EVENTS}?timeMin={}&timeMax={}",
        enc(&local_time(1, 14, 0)),
        enc(&local_time(1, 15, 0))
    );
    let (_, body, _) = send(&app, req("GET", &path, Some(&arun), None)).await;
    assert!(body["items"].as_array().unwrap().is_empty(), "{body}");

    // The team calendar, by both spellings of its id.
    for id in ["team@example.test", "team%40example.test"] {
        let path = format!("/calendar/v3/calendars/{id}/events?orderBy=startTime");
        let (status, body, _) = send(&app, req("GET", &path, Some(&arun), None)).await;
        assert_eq!(status, StatusCode::OK, "{id}: {body}");
        assert_eq!(body["accessRole"], "reader");
        assert_eq!(body["summary"], "Team");
        let items = body["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], "ev-4");
        assert_eq!(items[0]["summary"], "All hands");
        assert_eq!(items[0]["start"]["dateTime"], local_time(3, 11, 0));
        assert_eq!(items[0]["organizer"]["email"], "meera@prism.test");
        let me = items[0]["attendees"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["email"] == "arun@prism.test")
            .unwrap();
        assert_eq!(me["self"], true);
        assert_eq!(me["responseStatus"], "needsAction");
    }

    // As Priya, `self` moves to her entry.
    let priya = token(&state, "users/1002");
    let (_, body, _) = send(&app, req("GET", EVENTS, Some(&priya), None)).await;
    let standup = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "ev-1")
        .unwrap();
    let attendees = standup["attendees"].as_array().unwrap();
    assert!(attendees[0].get("self").is_none());
    assert_eq!(attendees[1]["self"], true);
    assert!(standup["organizer"].get("self").is_none());
    let (_, by_email, _) = send(
        &app,
        req(
            "GET",
            "/calendar/v3/calendars/priya@prism.test/events",
            Some(&priya),
            None,
        ),
    )
    .await;
    assert_eq!(by_email["items"].as_array().unwrap().len(), 3);

    // Bad input: unknown calendar, bad bounds, a sync token with bounds.
    let (status, body, _) = send(
        &app,
        req(
            "GET",
            "/calendar/v3/calendars/nobody@prism.test/events",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["status"], "NOT_FOUND");
    let (status, body, _) = send(
        &app,
        req(
            "GET",
            &format!("{EVENTS}?timeMin=yesterday"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["status"], "INVALID_ARGUMENT");
    let (status, body, _) = send(
        &app,
        req(
            "GET",
            &format!("{EVENTS}?syncToken=5&timeMin={}", enc(&local_time(0, 0, 0))),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn calendar_create_update_respond_delete_and_sync() {
    let (app, state) = twin();
    let arun = token(&state, "users/1001");
    let priya = token(&state, "users/1002");
    const EVENTS: &str = "/calendar/v3/calendars/primary/events";

    // A full fetch first: its token is what the client syncs from later.
    let (_, body, _) = send(
        &app,
        req(
            "GET",
            &format!("{EVENTS}?singleEvents=true&showDeleted=true&orderBy=startTime"),
            Some(&arun),
            None,
        ),
    )
    .await;
    let first_token = body["nextSyncToken"].as_str().unwrap().to_string();
    assert_eq!(first_token, "5");

    // Create with a Meet: the twin mints a link and an entry point.
    let (status, _, _) = send(&app, req("POST", EVENTS, None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, created, _) = send(
        &app,
        req(
            "POST",
            &format!("{EVENTS}?sendUpdates=all&conferenceDataVersion=1"),
            Some(&arun),
            Some(json!({
                "summary": "Pairing",
                "description": "Relay reconnect",
                "start": { "dateTime": local_time(1, 9, 0) },
                "end": { "dateTime": local_time(1, 9, 30) },
                "attendees": [{ "email": "priya@prism.test" }],
                "conferenceData": {
                    "createRequest": {
                        "requestId": "r1",
                        "conferenceSolutionKey": { "type": "hangoutsMeet" }
                    }
                }
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["id"], "ev-5");
    assert_eq!(created["status"], "confirmed");
    assert_eq!(created["summary"], "Pairing");
    assert_eq!(created["description"], "Relay reconnect");
    assert_eq!(created["start"]["dateTime"], local_time(1, 9, 0));
    assert_eq!(
        created["htmlLink"],
        "https://calendar.google.com/event?eid=ev-5"
    );
    let link = created["hangoutLink"].as_str().unwrap().to_string();
    assert!(link.starts_with("https://meet.google.com/"), "{link}");
    assert_eq!(created["conferenceData"]["entryPoints"][0]["uri"], link);
    assert_eq!(
        created["conferenceData"]["entryPoints"][0]["entryPointType"],
        "video"
    );
    assert!(created["created"].as_str().unwrap().ends_with('Z'));
    let attendees = created["attendees"].as_array().unwrap();
    assert_eq!(attendees.len(), 2, "the organizer is added");
    assert_eq!(attendees[0]["email"], "arun@prism.test");
    assert_eq!(attendees[0]["self"], true);
    assert_eq!(attendees[0]["organizer"], true);
    assert_eq!(attendees[0]["responseStatus"], "accepted");
    assert_eq!(attendees[1]["email"], "priya@prism.test");
    assert_eq!(attendees[1]["displayName"], "Priya Nair");
    assert_eq!(attendees[1]["responseStatus"], "needsAction");
    assert_eq!(created["organizer"]["email"], "arun@prism.test");
    assert_eq!(created["organizer"]["self"], true);

    // An all-day series with no Meet.
    let (status, offsite, _) = send(
        &app,
        req(
            "POST",
            &format!("{EVENTS}?sendUpdates=all"),
            Some(&arun),
            Some(json!({
                "summary": "Offsite",
                "start": { "date": "2026-11-20" },
                "end": { "date": "2026-11-21" },
                "recurrence": ["RRULE:FREQ=WEEKLY;BYDAY=MO,WE"],
                "transparency": "transparent",
                "attendees": []
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{offsite}");
    assert_eq!(offsite["id"], "ev-6");
    assert_eq!(offsite["start"], json!({ "date": "2026-11-20" }));
    assert_eq!(offsite["end"], json!({ "date": "2026-11-21" }));
    assert_eq!(
        offsite["recurrence"],
        json!(["RRULE:FREQ=WEEKLY;BYDAY=MO,WE"])
    );
    assert_eq!(offsite["transparency"], "transparent");
    assert!(offsite.get("hangoutLink").is_none());
    assert!(offsite.get("attendees").is_none());

    // Google-shaped 400s: no end, mixed shapes.
    for body in [
        json!({ "summary": "x", "start": { "dateTime": local_time(1, 9, 0) } }),
        json!({ "summary": "x", "start": { "date": "2026-11-20" }, "end": { "dateTime": local_time(1, 9, 0) } }),
        json!({ "summary": "x", "start": { "date": "soon" }, "end": { "date": "2026-11-21" } }),
    ] {
        let (status, err, _) = send(&app, req("POST", EVENTS, Some(&arun), Some(body))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");
        assert_eq!(err["error"]["status"], "INVALID_ARGUMENT");
    }

    // Update: fields merge, a Meet is added once and kept.
    let (status, updated, _) = send(
        &app,
        req(
            "PATCH",
            &format!("{EVENTS}/ev-6?sendUpdates=all&conferenceDataVersion=1"),
            Some(&arun),
            Some(json!({
                "summary": "Offsite planning",
                "location": "Hill",
                "recurrence": ["RRULE:FREQ=MONTHLY"],
                "conferenceData": {
                    "createRequest": {
                        "requestId": "r2",
                        "conferenceSolutionKey": { "type": "hangoutsMeet" }
                    }
                }
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["summary"], "Offsite planning");
    assert_eq!(updated["location"], "Hill");
    assert_eq!(updated["recurrence"], json!(["RRULE:FREQ=MONTHLY"]));
    assert_eq!(updated["start"], json!({ "date": "2026-11-20" }));
    assert_eq!(updated["transparency"], "transparent");
    let offsite_link = updated["hangoutLink"].as_str().unwrap().to_string();
    assert!(offsite_link.starts_with("https://meet.google.com/"));
    assert_ne!(offsite_link, link, "each event gets its own code");
    assert_ne!(updated["etag"], offsite["etag"]);
    let (_, again, _) = send(
        &app,
        req(
            "PATCH",
            &format!("{EVENTS}/ev-6?conferenceDataVersion=1"),
            Some(&arun),
            Some(json!({ "conferenceData": { "createRequest": { "requestId": "r3" } } })),
        ),
    )
    .await;
    assert_eq!(again["hangoutLink"], offsite_link);
    let (_, with_people, _) = send(
        &app,
        req(
            "PATCH",
            &format!("{EVENTS}/ev-6?sendUpdates=all"),
            Some(&arun),
            Some(json!({ "attendees": [{ "email": "dev@prism.test" }] })),
        ),
    )
    .await;
    let people = with_people["attendees"].as_array().unwrap();
    assert_eq!(people.len(), 2);
    assert_eq!(people[0]["email"], "arun@prism.test");
    assert_eq!(people[0]["organizer"], true);
    assert_eq!(people[1]["email"], "dev@prism.test");
    assert_eq!(people[1]["responseStatus"], "needsAction");

    // Respond as Priya: the whole list patched back, her entry changes.
    let (status, answered, _) = send(
        &app,
        req(
            "PATCH",
            &format!("{EVENTS}/ev-5?sendUpdates=all"),
            Some(&priya),
            Some(json!({ "attendees": [
                { "email": "arun@prism.test", "responseStatus": "accepted" },
                { "email": "priya@prism.test", "responseStatus": "declined" }
            ] })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answered}");
    let attendees = answered["attendees"].as_array().unwrap();
    assert_eq!(attendees[1]["email"], "priya@prism.test");
    assert_eq!(attendees[1]["self"], true);
    assert_eq!(attendees[1]["responseStatus"], "declined");
    assert_eq!(attendees[0]["organizer"], true);
    assert!(attendees[0].get("self").is_none());
    let (_, seen, _) = send(
        &app,
        req("GET", &format!("{EVENTS}/ev-5"), Some(&arun), None),
    )
    .await;
    assert_eq!(seen["attendees"][0]["self"], true);
    assert_eq!(seen["attendees"][1]["responseStatus"], "declined");
    let (status, err, _) = send(
        &app,
        req(
            "PATCH",
            &format!("{EVENTS}/ev-5"),
            Some(&arun),
            Some(json!({ "attendees": [{ "email": "priya@prism.test", "responseStatus": "maybe" }] })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");

    // The team calendar is read-only.
    let (status, err, _) = send(
        &app,
        req(
            "POST",
            "/calendar/v3/calendars/team@example.test/events",
            Some(&arun),
            Some(json!({ "summary": "x", "start": { "date": "2026-11-20" }, "end": { "date": "2026-11-21" } })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(err["error"]["status"], "PERMISSION_DENIED");

    // Delete: 204, kept as cancelled, a second delete is 410.
    let (status, body, _) = send(
        &app,
        req(
            "DELETE",
            &format!("{EVENTS}/ev-5?sendUpdates=all"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(body, Value::String(String::new()));
    let (status, err, _) = send(
        &app,
        req("DELETE", &format!("{EVENTS}/ev-5"), Some(&arun), None),
    )
    .await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(err["error"]["status"], "deleted");
    let (status, gone, _) = send(
        &app,
        req("GET", &format!("{EVENTS}/ev-5"), Some(&arun), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(gone["status"], "cancelled");
    assert!(gone.get("summary").is_none(), "{gone}");
    let window = format!(
        "{EVENTS}?timeMin={}&timeMax={}",
        enc(&local_time(1, 8, 0)),
        enc(&local_time(1, 10, 0))
    );
    let (_, hidden, _) = send(&app, req("GET", &window, Some(&arun), None)).await;
    assert!(hidden["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["id"] != "ev-5"));
    let (_, shown, _) = send(
        &app,
        req(
            "GET",
            &format!("{window}&showDeleted=true"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert!(shown["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["id"] == "ev-5" && e["status"] == "cancelled"));

    // Incremental sync from the first token: the two new events, one cancelled.
    let (status, sync, _) = send(
        &app,
        req(
            "GET",
            &format!("{EVENTS}?singleEvents=true&showDeleted=true&syncToken={first_token}"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sync}");
    let items = sync["items"].as_array().unwrap();
    let mut ids: Vec<&str> = items.iter().map(|e| e["id"].as_str().unwrap()).collect();
    ids.sort();
    assert_eq!(ids, ["ev-5", "ev-6"]);
    let ev5 = items.iter().find(|e| e["id"] == "ev-5").unwrap();
    assert_eq!(ev5["status"], "cancelled");
    let ev6 = items.iter().find(|e| e["id"] == "ev-6").unwrap();
    assert_eq!(ev6["summary"], "Offsite planning");
    let next = sync["nextSyncToken"].as_str().unwrap().to_string();
    assert!(next.parse::<u64>().unwrap() > first_token.parse::<u64>().unwrap());
    let (_, quiet, _) = send(
        &app,
        req(
            "GET",
            &format!("{EVENTS}?syncToken={next}"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert!(quiet["items"].as_array().unwrap().is_empty(), "{quiet}");
    assert_eq!(quiet["nextSyncToken"], next);

    // Unknown, stale and future tokens: 410 asking for a full sync.
    state.write().cal_min_sync = next.parse::<u64>().unwrap();
    for t in ["abc", "0", "999999", first_token.as_str()] {
        let (status, err, _) = send(
            &app,
            req("GET", &format!("{EVENTS}?syncToken={t}"), Some(&arun), None),
        )
        .await;
        assert_eq!(status, StatusCode::GONE, "{t}: {err}");
        assert_eq!(err["error"]["code"], 410);
        assert_eq!(err["error"]["status"], "fullSyncRequired");
    }
    let (status, _, _) = send(
        &app,
        req(
            "GET",
            &format!("{EVENTS}?syncToken={next}"),
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the newest token survives");

    // Unknown events are 404 everywhere.
    for method in ["GET", "PATCH", "DELETE"] {
        let body = (method == "PATCH").then(|| json!({ "summary": "x" }));
        let (status, err, _) = send(
            &app,
            req(method, &format!("{EVENTS}/ev-99"), Some(&arun), body),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}: {err}");
        assert_eq!(err["error"]["status"], "NOT_FOUND");
    }
}

// ----- drive app data -----

#[tokio::test]
async fn drive_app_data_notes_round_trip() {
    let (app, state) = twin();
    let arun = token(&state, "users/1001");
    const LIST: &str = "/drive/v3/files?spaces=appDataFolder&q=name%3D%27prism-notes.json%27&fields=files(id,name,modifiedTime)";
    let (status, _, _) = send(&app, req("GET", LIST, None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Nothing saved yet.
    let (status, body, _) = send(&app, req("GET", LIST, Some(&arun), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["kind"], "drive#fileList");
    assert_eq!(body["files"], json!([]));

    // Create: Google's multipart/related, metadata part then content part.
    let notes = json!({ "notes": [{ "id": 1, "text": "hello" }] });
    let boundary = "prism-notes-test";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{{\"name\":\"prism-notes.json\",\"parents\":[\"appDataFolder\"]}}\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Type: application/json\r\n\r\n{notes}\r\n--{boundary}--\r\n"
        )
        .as_bytes(),
    );
    let request = Request::builder()
        .method("POST")
        .uri("/upload/drive/v3/files?uploadType=multipart")
        .header(header::AUTHORIZATION, format!("Bearer {arun}"))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/related; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let (status, created, _) = send(&app, request).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["id"], "file-1");
    assert_eq!(created["name"], "prism-notes.json");
    assert_eq!(created["mimeType"], "application/json");
    assert!(created["modifiedTime"].as_str().unwrap().ends_with('Z'));

    // Media round trip and the metadata get.
    let (status, got, headers) = send(
        &app,
        req("GET", "/drive/v3/files/file-1?alt=media", Some(&arun), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    assert_eq!(got, notes);
    let (_, meta, _) = send(
        &app,
        req("GET", "/drive/v3/files/file-1", Some(&arun), None),
    )
    .await;
    assert_eq!(meta["name"], "prism-notes.json");

    // Update in place with a media upload.
    let replaced = json!({ "notes": [] });
    let request = Request::builder()
        .method("PATCH")
        .uri("/upload/drive/v3/files/file-1?uploadType=media")
        .header(header::AUTHORIZATION, format!("Bearer {arun}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(replaced.to_string()))
        .unwrap();
    let (status, updated, _) = send(&app, request).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["id"], "file-1");
    assert_eq!(updated["name"], "prism-notes.json");
    let (_, got, _) = send(
        &app,
        req("GET", "/drive/v3/files/file-1?alt=media", Some(&arun), None),
    )
    .await;
    assert_eq!(got, replaced);

    // The list finds it by name, and only by that name.
    let (_, body, _) = send(&app, req("GET", LIST, Some(&arun), None)).await;
    let files = body["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["id"], "file-1");
    assert_eq!(files[0]["name"], "prism-notes.json");
    assert!(files[0]["modifiedTime"].is_string());
    let (_, other, _) = send(
        &app,
        req(
            "GET",
            "/drive/v3/files?spaces=appDataFolder&q=name%3D%27other.json%27",
            Some(&arun),
            None,
        ),
    )
    .await;
    assert_eq!(other["files"], json!([]));

    // App data is per account; unknown ids and upload types are refused.
    let priya = token(&state, "users/1002");
    let (_, body, _) = send(&app, req("GET", LIST, Some(&priya), None)).await;
    assert_eq!(body["files"], json!([]));
    let (status, err, _) = send(
        &app,
        req(
            "GET",
            "/drive/v3/files/file-1?alt=media",
            Some(&priya),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(err["error"]["status"], "NOT_FOUND");
    let (status, _, _) = send(
        &app,
        req("GET", "/drive/v3/files/file-9?alt=media", Some(&arun), None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let request = Request::builder()
        .method("PATCH")
        .uri("/upload/drive/v3/files/file-9?uploadType=media")
        .header(header::AUTHORIZATION, format!("Bearer {arun}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let (status, _, _) = send(&app, request).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, err, _) = send(
        &app,
        req(
            "POST",
            "/upload/drive/v3/files?uploadType=resumable",
            Some(&arun),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err["error"]["status"], "INVALID_ARGUMENT");
}

// ----- availability -----

#[tokio::test]
async fn availability_get_mark_and_custom_status() {
    let (app, state) = twin();
    let arun = token(&state, "users/1001");
    const ME: &str = "/v1/users/me/availability";
    let (status, _, _) = send(&app, req("GET", ME, None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The default: active, no status, no do-not-disturb window.
    let (status, body, _) = send(&app, req("GET", ME, Some(&arun), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "users/me/availability");
    assert_eq!(body["state"], "ACTIVE");
    assert!(body.get("customStatus").is_none());
    assert!(body.get("doNotDisturbMetadata").is_none());

    // Away, then do not disturb for ten minutes, then active again.
    let (status, body, _) = send(
        &app,
        req("POST", &format!("{ME}:markAsAway"), Some(&arun), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "AWAY");
    let (status, body, _) = send(
        &app,
        req(
            "POST",
            &format!("{ME}:markAsDoNotDisturb"),
            Some(&arun),
            Some(json!({ "ttl": "600s" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "DO_NOT_DISTURB");
    let until = instant(&body["doNotDisturbMetadata"]["expirationTime"]);
    let left = until - chrono::Utc::now();
    assert!(
        left > chrono::Duration::seconds(590) && left <= chrono::Duration::seconds(600),
        "{left}"
    );
    let (_, body, _) = send(&app, req("GET", ME, Some(&arun), None)).await;
    assert_eq!(body["state"], "DO_NOT_DISTURB");
    for body in [
        None,
        Some(json!({ "ttl": "99999999s" })),
        Some(json!({ "ttl": "soon" })),
    ] {
        let (status, err, _) = send(
            &app,
            req(
                "POST",
                &format!("{ME}:markAsDoNotDisturb"),
                Some(&arun),
                body,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");
        assert_eq!(err["error"]["status"], "INVALID_ARGUMENT");
    }
    let (status, body, _) = send(
        &app,
        req(
            "POST",
            &format!("{ME}:markAsActive"),
            Some(&arun),
            Some(json!({ "ttl": "1800s" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "ACTIVE");
    assert!(body.get("doNotDisturbMetadata").is_none());

    // A custom status with a ttl, kept on get, then cleared two ways.
    const PATCH: &str = "/v1/users/me/availability?updateMask=customStatus";
    let (status, body, _) = send(
        &app,
        req(
            "PATCH",
            PATCH,
            Some(&arun),
            Some(json!({ "customStatus": {
                "text": "Heads down", "emoji": { "unicode": "🎧" }, "ttl": "3600s"
            } })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "ACTIVE");
    assert_eq!(body["customStatus"]["text"], "Heads down");
    assert_eq!(body["customStatus"]["emoji"]["unicode"], "🎧");
    let expires = instant(&body["customStatus"]["expireTime"]) - chrono::Utc::now();
    assert!(expires > chrono::Duration::seconds(3590), "{expires}");
    let (_, body, _) = send(&app, req("GET", ME, Some(&arun), None)).await;
    assert_eq!(body["customStatus"]["text"], "Heads down");
    let (status, err, _) = send(
        &app,
        req(
            "PATCH",
            PATCH,
            Some(&arun),
            Some(
                json!({ "customStatus": { "text": "x".repeat(65), "emoji": { "unicode": "🎧" } } }),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");
    assert_eq!(err["error"]["status"], "INVALID_ARGUMENT");
    let (status, _, _) = send(
        &app,
        req(
            "PATCH",
            PATCH,
            Some(&arun),
            Some(json!({ "customStatus": { "text": "No emoji" } })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = send(
        &app,
        req(
            "PATCH",
            ME,
            Some(&arun),
            Some(json!({ "customStatus": {} })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "the mask is required");
    let (status, body, _) = send(
        &app,
        req(
            "PATCH",
            PATCH,
            Some(&arun),
            Some(json!({ "customStatus": {} })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("customStatus").is_none(), "{body}");
    let (_, body, _) = send(
        &app,
        req(
            "PATCH",
            PATCH,
            Some(&arun),
            Some(json!({ "customStatus": { "text": "Lunch", "emoji": { "unicode": "🍜" } } })),
        ),
    )
    .await;
    assert_eq!(body["customStatus"]["text"], "Lunch");
    assert!(body["customStatus"].get("expireTime").is_none());
    let (status, body, _) = send(&app, req("PATCH", PATCH, Some(&arun), Some(json!({})))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.get("customStatus").is_none(),
        "an empty body clears too"
    );

    // Per caller, and only the caller's own.
    let priya = token(&state, "users/1002");
    let (_, body, _) = send(&app, req("GET", ME, Some(&priya), None)).await;
    assert_eq!(body["state"], "ACTIVE");
    assert!(body.get("customStatus").is_none());
    let (status, body, _) = send(
        &app,
        req("GET", "/v1/users/1001/availability", Some(&arun), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "users/1001/availability");
    let (status, err, _) = send(
        &app,
        req("GET", "/v1/users/1002/availability", Some(&arun), None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(err["error"]["status"], "PERMISSION_DENIED");
}
