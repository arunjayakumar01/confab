//! End-to-end tests: the relay on a random port, a tiny Google stub in the same
//! process for userinfo / membership / message / subscriptions / JWKS / Pub/Sub
//! pull / token grant, and tokio-tungstenite clients. No twin, no network.

use std::collections::{HashMap, VecDeque};
use std::future::IntoFuture;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use futures::{SinkExt, StreamExt};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use prism_proto::chat::{Membership, Message, User, UserInfo};
use prism_proto::events::{EventData, PubSubPush};
use prism_proto::relay::{event_types, ClientMsg, PresenceState, ServerMsg, TYPING_TTL_MS};
use prism_proto::Endpoints;
use relay::pubsub_pull::{sign_assertion, Assertion, ServiceAccountKey};
use relay::{AppState, Config};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

const TOK_A: &str = "tok-alice"; // users/1001: member of S1 and S2
const TOK_B: &str = "tok-bob"; // users/1002: member of S1 only
const SECRET: &str = "test-secret";
const KID: &str = "k1";
const AUD: &str = "https://relay.test/events/push";
const SA: &str = "push@sa.test";
const PULL_SUB: &str = "projects/p/subscriptions/prism-relay-pull";
const PULL_SA: &str = "relay@p.iam.gserviceaccount.com";
const PULL_TOKEN: &str = "pull-access";
const WAIT: Duration = Duration::from_secs(3);
const QUIET: Duration = Duration::from_millis(300);

const TEST_KEY_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCeV/35JHqiUjUR
t/sqqoWuQt7lSCFDj8FNY/CPS43fh7LiVUGJHXf0fq2hXzwrWwxDldslR8a7Q6xA
Pz7+soRxHQaUqbzXP+cRGHuJTSfrmf3TED/BfR4Qegj3c917hMAxjV7PxnFO/24m
T7wB62gkw5rXirdzhqsADZ+GvqK27NRLyKJN4SiirWy7ZcFoREAozXEL2vXuPfup
CpOVN0Ikid0k722McRsIZLbTUWGhEqg1wJfxGRdVCKfB4ces5sUvc4WdLAmK4HPh
gdHEelsR2KOJOoMYNY8PEifEuql3tAJ5Icq0P5zicMmggZe/FL5GlIEpbUWMnkoi
EooxqeXFAgMBAAECggEAXskvqDd1DbwGNqAawzSmp2UyRrBB8U6ZmTktJKpGuDtk
EuaUP38bcoluaL44p39uPPr030EzLqBY+XCixhaY/5kvekiNWZDQQkSUluFP91IM
Ro9givPZzmUu/uol+C3EWyVUTy5sX6y6BEcQtVl00yUgiMxp+sbvkC2XhDKT9smn
SLbOZ7V95CDrv2+/PIZ5hTah0iaCkB3qosn8+PPuO9mKB+8+YlPfp4kmVSdFcLta
grf9D0U643vK2Y4rg371Z5q2yDoBWnDix+IViqNjqzr3FSp8Lh6t+NK8vOuwsai5
PZ8vjRQyoYClETYfI2n818Y9qg92hWGreo3eWFSOQQKBgQDNPnDKdFko49cdfUHt
ItOrMOyKnPOQrBnRaZxE0u2XGl7Fm5dQ8PADNIZ/G+ICaXJAgM5XEJx+Jjzz76Wk
1sITIliWaZW3o5u6KKjG7NFPwwbxWsmT7WjcpZNoJiBroQA06WvrIBKy3wsyzvce
jFwRbi8onPFHX7ICSBSW5nAmsQKBgQDFgGY5d5W7LbFX+l0G4yzE5KqltWwbA6Mq
PJTrmTryC2qKKNS56avFB65wa7WzCxDMQ3jSnwWAVfNUTlD1VjIbIHp/Qf2AzITh
3JNXWuuVCwdR9MxSKS6ksubdbhbpryYcJtL2cuQIgjeZviwRQ4YK4pg405q7AOT+
HnYXU3sdVQKBgQCaTNUS/4GAQjoZC3AizUoAq5lJZFGgdCI8038OJt1D+ilBuoV5
7okif7yVlurFEW05nRyDTBO6qxir84C+ek4RgILFIb/S/Yc33xVz59o8csynaq+d
LKDnMPuvEurQv4ueO9ZMQCKOoyySGBKkvrUAzatahxLFq+Xx3SrNf5TVIQKBgB4n
anQfs/auFUOPcCCbErzFBODkACtntMPAGCSYco19/Lg3X7M64ATnFVUIvOcnK1tk
ec/+g4KCnPIkOuwaSvAlV06Srre5ghhR6xUNRqas1mNG8uMgcoSm4+Su6ESpfzIR
I2iJRhOefgq1JFlGa/LBgeg7NoaSrmO2FuWc01E5AoGAd4LDSwR70RXLzbPt/Y4t
huH6+Re2YPZ1Pi7xhcg4lt5GZA4EdK4Yt+vRThAAysRWQIy+4HOJnp4MQT6qF9mN
VPaf66RbhxVqqbpjzg1uACXvWVDXS1MuyLecQYX5f8GHOfwRnTv5QUWy5r8378T5
1M+dlhxtNpcDAzb/e554Nj0=
-----END PRIVATE KEY-----"#;
const TEST_KEY_N: &str = "nlf9-SR6olI1Ebf7KqqFrkLe5UghQ4_BTWPwj0uN34ey4lVBiR139H6toV88K1sMQ5XbJUfGu0OsQD8-_rKEcR0GlKm81z_nERh7iU0n65n90xA_wX0eEHoI93Pde4TAMY1ez8ZxTv9uJk-8AetoJMOa14q3c4arAA2fhr6ituzUS8iiTeEooq1su2XBaERAKM1xC9r17j37qQqTlTdCJIndJO9tjHEbCGS201FhoRKoNcCX8RkXVQinweHHrObFL3OFnSwJiuBz4YHRxHpbEdijiTqDGDWPDxInxLqpd7QCeSHKtD-c4nDJoIGXvxS-RpSBKW1FjJ5KIhKKManlxQ";

// ---------- Google stub ----------

#[derive(Default)]
struct StubState {
    subscription_bodies: Mutex<Vec<Value>>,
    /// Pub/Sub messages handed out by `:pull`, one per call.
    pull_queue: Mutex<VecDeque<Value>>,
    /// Every `:pull` request: (authorization header, body).
    pulls: Mutex<Vec<(Option<String>, Value)>>,
    /// Ack ids received by `:acknowledge`, in order.
    acks: Mutex<Vec<String>>,
    /// When set, `:pull` and `:acknowledge` require this bearer token.
    pull_bearer: Mutex<Option<String>>,
    /// Verified assertion claims of every `/token` grant.
    token_grants: Mutex<Vec<Value>>,
}

struct Stub {
    base: String,
    /// `host:port`, the form `PRISM_PUBSUB_EMULATOR_HOST` takes.
    host: String,
    state: Arc<StubState>,
}

fn user_for(headers: &HeaderMap) -> Option<(&'static str, &'static str, &'static str)> {
    let auth = headers.get("authorization")?.to_str().ok()?;
    match auth.strip_prefix("Bearer ")? {
        TOK_A => Some(("1001", "Alice", "alice@example.test")),
        TOK_B => Some(("1002", "Bob", "bob@example.test")),
        _ => None,
    }
}

fn google_error(code: StatusCode, status: &str) -> (StatusCode, Json<Value>) {
    (
        code,
        Json(json!({"error": {"code": code.as_u16(), "message": status, "status": status}})),
    )
}

async fn userinfo(headers: HeaderMap) -> Result<Json<UserInfo>, (StatusCode, Json<Value>)> {
    let (sub, name, email) =
        user_for(&headers).ok_or(google_error(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED"))?;
    Ok(Json(UserInfo {
        sub: sub.into(),
        email: Some(email.into()),
        name: Some(name.into()),
        picture: Some(format!("https://avatars.test/{sub}.png")),
        hd: None,
    }))
}

async fn membership(
    Path((space, user)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Membership>, (StatusCode, Json<Value>)> {
    let (sub, _, _) =
        user_for(&headers).ok_or(google_error(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED"))?;
    if sub != user {
        return Err(google_error(StatusCode::FORBIDDEN, "PERMISSION_DENIED"));
    }
    let member = match space.as_str() {
        "S1" => true,
        "S2" => sub == "1001",
        _ => false,
    };
    if !member {
        return Err(google_error(StatusCode::NOT_FOUND, "NOT_FOUND"));
    }
    Ok(Json(Membership {
        name: format!("spaces/{space}/members/{user}"),
        state: Some("JOINED".into()),
        role: Some("ROLE_MEMBER".into()),
        member: Some(User {
            name: format!("users/{user}"),
            ..Default::default()
        }),
        create_time: None,
    }))
}

async fn get_message(
    Path((space, msg)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Message>, (StatusCode, Json<Value>)> {
    user_for(&headers).ok_or(google_error(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED"))?;
    Ok(Json(Message {
        name: format!("spaces/{space}/messages/{msg}"),
        text: Some("hydrated".into()),
        create_time: Some("2026-11-06T10:00:00Z".into()),
        ..Default::default()
    }))
}

async fn create_subscription(
    State(stub): State<Arc<StubState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (sub, _, _) =
        user_for(&headers).ok_or(google_error(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED"))?;
    stub.subscription_bodies.lock().unwrap().push(body);
    let expire = (chrono::Utc::now() + chrono::Duration::hours(4)).to_rfc3339();
    Ok(Json(json!({
        "name": "operations/op-1",
        "done": true,
        "response": {"name": format!("subscriptions/sub-{sub}"), "expireTime": expire}
    })))
}

async fn certs() -> Json<Value> {
    Json(
        json!({"keys": [{"kty": "RSA", "alg": "RS256", "use": "sig", "kid": KID, "n": TEST_KEY_N, "e": "AQAB"}]}),
    )
}

/// `POST /v1/projects/{p}/subscriptions/{s}:pull` and `:acknowledge` in the
/// Pub/Sub REST shape. The colon verb rides in the last path segment.
async fn pubsub_call(
    State(stub): State<Arc<StubState>>,
    Path((project, call)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let expected = stub.pull_bearer.lock().unwrap().clone();
    if let Some(expected) = expected {
        if auth.as_deref() != Some(format!("Bearer {expected}").as_str()) {
            return Err(google_error(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED"));
        }
    }
    let (sub, verb) = call.rsplit_once(':').unwrap_or((call.as_str(), ""));
    if format!("projects/{project}/subscriptions/{sub}") != PULL_SUB {
        return Err(google_error(StatusCode::NOT_FOUND, "NOT_FOUND"));
    }
    match verb {
        "pull" => {
            stub.pulls.lock().unwrap().push((auth, body));
            let next = stub.pull_queue.lock().unwrap().pop_front();
            match next {
                Some(m) => Ok(Json(json!({ "receivedMessages": [m] }))),
                None => {
                    // A short long-poll so an idle puller does not hammer the stub.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Ok(Json(json!({})))
                }
            }
        }
        "acknowledge" => {
            let ids: Vec<String> = body["ackIds"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            stub.acks.lock().unwrap().extend(ids);
            Ok(Json(json!({})))
        }
        _ => Err(google_error(StatusCode::NOT_FOUND, "NOT_FOUND")),
    }
}

/// `POST /token`: the JWT bearer grant. Verifies the assertion against the
/// test key, keeps its claims, hands out `PULL_TOKEN`.
async fn token(
    State(stub): State<Arc<StubState>>,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if form.get("grant_type").map(String::as_str)
        != Some("urn:ietf:params:oauth:grant-type:jwt-bearer")
    {
        return Err(google_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
        ));
    }
    let assertion = form
        .get("assertion")
        .ok_or(google_error(StatusCode::BAD_REQUEST, "invalid_request"))?;
    let key = DecodingKey::from_rsa_components(TEST_KEY_N, "AQAB").unwrap();
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_aud = false; // the test asserts aud itself
    let data = decode::<Value>(assertion, &key, &validation)
        .map_err(|_| google_error(StatusCode::UNAUTHORIZED, "invalid_grant"))?;
    stub.token_grants.lock().unwrap().push(data.claims);
    Ok(Json(json!({
        "access_token": PULL_TOKEN,
        "expires_in": 3600,
        "token_type": "Bearer",
    })))
}

async fn stub() -> Stub {
    let state = Arc::new(StubState::default());
    let app = Router::new()
        .route("/oauth2/v3/userinfo", get(userinfo))
        .route("/oauth2/v3/certs", get(certs))
        .route("/token", post(token))
        .route("/v1/spaces/{space}/members/{user}", get(membership))
        .route("/v1/spaces/{space}/messages/{msg}", get(get_message))
        .route("/v1/subscriptions", post(create_subscription))
        .route(
            "/v1/projects/{project}/subscriptions/{call}",
            post(pubsub_call),
        )
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = listener.local_addr().unwrap().to_string();
    let base = format!("http://{host}");
    tokio::spawn(axum::serve(listener, app).into_future());
    Stub { base, host, state }
}

// ---------- relay under test ----------

struct Relay {
    state: Arc<AppState>,
    http: String,
    ws: String,
}

fn twin_config(stub: &Stub) -> Config {
    let base = &stub.base;
    let mut c = Config::new(Endpoints {
        chat_api: base.clone(),
        oauth_auth: format!("{base}/o/oauth2/v2/auth"),
        oauth_token: format!("{base}/token"),
        userinfo: format!("{base}/oauth2/v3/userinfo"),
        relay_ws: String::new(),
        people_api: base.clone(),
        tasks_api: base.clone(),
        meet_api: base.clone(),
        calendar_api: format!("{base}/calendar/v3"),
        drive_api: format!("{base}/drive/v3"),
        client_id: "prism-dev".into(),
        client_secret: None,
    });
    c.push_secret = SECRET.into();
    c.db_path = ":memory:".into();
    c.hello_timeout = Duration::from_millis(500);
    c.jwks_url = format!("{base}/oauth2/v3/certs");
    c.events_api = base.clone();
    c
}

async fn start_relay(config: Config) -> Relay {
    let state = AppState::new(config).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, relay::app(state.clone())).into_future());
    relay::pubsub_pull::spawn(state.clone()); // a no-op unless a subscription is configured
    Relay {
        state,
        http: format!("http://{addr}"),
        ws: format!("ws://{addr}/ws"),
    }
}

async fn twin_relay() -> (Stub, Relay) {
    let stub = stub().await;
    let relay = start_relay(twin_config(&stub)).await;
    (stub, relay)
}

// ---------- ws client ----------

struct Client {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    pending: VecDeque<ServerMsg>,
    pings: u64,
}

impl Client {
    async fn connect(url: &str) -> Self {
        let (ws, _) = connect_async(url).await.unwrap();
        Self {
            ws,
            pending: VecDeque::new(),
            pings: 0,
        }
    }

    async fn hello(url: &str, token: &str, device: &str) -> (Self, ServerMsg) {
        let mut c = Self::connect(url).await;
        c.send(&ClientMsg::Hello {
            token: token.into(),
            device_id: device.into(),
            client_version: "test".into(),
        })
        .await;
        let first = c.next(WAIT).await.expect("a reply to hello");
        (c, first)
    }

    async fn send(&mut self, m: &ClientMsg) {
        self.ws
            .send(WsMessage::Text(serde_json::to_string(m).unwrap().into()))
            .await
            .unwrap();
    }

    /// Next text frame, or `None` on timeout / close.
    async fn next(&mut self, timeout: Duration) -> Option<ServerMsg> {
        if let Some(m) = self.pending.pop_front() {
            return Some(m);
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let frame = tokio::time::timeout_at(deadline, self.ws.next())
                .await
                .ok()??;
            match frame.ok()? {
                WsMessage::Text(t) => return Some(serde_json::from_str(t.as_str()).unwrap()),
                WsMessage::Close(_) => return None,
                _ => continue,
            }
        }
    }

    /// Drain frames until one matches; the rest stay queued for later asserts.
    async fn expect(
        &mut self,
        timeout: Duration,
        pred: impl Fn(&ServerMsg) -> bool,
    ) -> Option<ServerMsg> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut kept = VecDeque::new();
        let found = loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match self.next(remaining).await {
                Some(m) if pred(&m) => break Some(m),
                Some(m) => kept.push_back(m),
                None => break None,
            }
        };
        kept.append(&mut self.pending);
        self.pending = kept;
        found
    }

    async fn assert_none(
        &mut self,
        quiet: Duration,
        pred: impl Fn(&ServerMsg) -> bool,
        what: &str,
    ) {
        if let Some(m) = self.expect(quiet, pred).await {
            panic!("unexpected {what}: {m:?}");
        }
    }

    /// Subscribe and wait for the relay to finish processing it (ping/pong is ordered).
    async fn subscribe(&mut self, spaces: &[&str]) {
        self.send(&ClientMsg::Subscribe {
            spaces: spaces.iter().map(|s| s.to_string()).collect(),
        })
        .await;
        self.sync().await;
    }

    async fn sync(&mut self) {
        self.pings += 1;
        let t = self.pings;
        self.send(&ClientMsg::Ping { t }).await;
        self.expect(
            WAIT,
            |m| matches!(m, ServerMsg::Pong { t: got } if *got == t),
        )
        .await
        .expect("pong");
    }
}

fn is_typing(m: &ServerMsg) -> bool {
    matches!(m, ServerMsg::Typing { .. })
}
fn is_event(m: &ServerMsg) -> bool {
    matches!(m, ServerMsg::Event { .. })
}
fn is_draft(m: &ServerMsg) -> bool {
    matches!(m, ServerMsg::Draft { .. })
}

fn message_push(space: &str, id: &str, last_update: &str) -> PubSubPush {
    let data = EventData {
        message: Some(Message {
            name: format!("{space}/messages/{id}"),
            text: Some("hello".into()),
            create_time: Some("2026-11-06T10:00:00Z".into()),
            last_update_time: Some(last_update.into()),
            sender: Some(User {
                name: "users/1001".into(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    PubSubPush::wrap(
        event_types::MESSAGE_CREATED,
        space,
        "2026-11-06T10:00:00Z",
        &data,
    )
}

/// A `receivedMessages[]` entry the way the emulator sends it: camelCase and
/// snake_case ids side by side, plus `deliveryAttempt`.
fn pulled(push: &PubSubPush, ack_id: &str) -> Value {
    let mut m = serde_json::to_value(&push.message).unwrap();
    m["messageId"] = json!("42");
    m["message_id"] = json!("42");
    m["publishTime"] = json!("2026-11-06T10:00:00.123Z");
    m["publish_time"] = json!("2026-11-06T10:00:00.123Z");
    m["orderingKey"] = json!("");
    json!({ "ackId": ack_id, "message": m, "deliveryAttempt": 1 })
}

async fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..60 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn post_push(relay: &Relay, push: &PubSubPush, secret: Option<&str>) -> StatusCode {
    let mut req = reqwest::Client::new()
        .post(format!("{}/events/push", relay.http))
        .json(push);
    if let Some(s) = secret {
        req = req.header("X-Prism-Push-Secret", s);
    }
    req.send().await.unwrap().status()
}

// ---------- tests ----------

#[tokio::test]
async fn hello_resolves_identity_and_lists_online() {
    let (_stub, relay) = twin_relay().await;
    let (mut a, welcome) = Client::hello(&relay.ws, TOK_A, "d1").await;
    match &welcome {
        ServerMsg::Welcome {
            user,
            session_id,
            online,
        } => {
            assert_eq!(user.name, "users/1001");
            assert_eq!(user.display_name.as_deref(), Some("Alice"));
            assert_eq!(user.email.as_deref(), Some("alice@example.test"));
            assert_eq!(
                user.avatar_url.as_deref(),
                Some("https://avatars.test/1001.png")
            );
            assert!(!session_id.is_empty());
            assert_eq!(online, &["users/1001"]);
        }
        other => panic!("expected welcome, got {other:?}"),
    }
    let (_b, welcome_b) = Client::hello(&relay.ws, TOK_B, "d1").await;
    match welcome_b {
        ServerMsg::Welcome { online, .. } => assert_eq!(online, ["users/1001", "users/1002"]),
        other => panic!("expected welcome, got {other:?}"),
    }
    let health: Value = reqwest::get(format!("{}/health", relay.http))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["sessions"], 2);
    assert_eq!(health["users_online"], 2);
    assert_eq!(health["mode"], "twin");
    a.sync().await;
}

#[tokio::test]
async fn bad_token_gets_error_then_close() {
    let (_stub, relay) = twin_relay().await;
    let (mut c, first) = Client::hello(&relay.ws, "tok-nobody", "d1").await;
    assert!(
        matches!(first, ServerMsg::Error { ref code, .. } if code == "unauthorized"),
        "{first:?}"
    );
    assert!(c.next(WAIT).await.is_none(), "socket should be closed");
}

#[tokio::test]
async fn silent_socket_is_closed_after_hello_timeout() {
    let (_stub, relay) = twin_relay().await;
    let mut c = Client::connect(&relay.ws).await;
    let first = c.next(WAIT).await;
    assert!(
        matches!(first, Some(ServerMsg::Error { ref code, .. }) if code == "hello_timeout"),
        "{first:?}"
    );
    assert!(c.next(WAIT).await.is_none());
}

fn is_huddle(m: &ServerMsg) -> bool {
    matches!(m, ServerMsg::Huddle { .. })
}

#[tokio::test]
async fn huddle_membership_fans_out_and_signals_reach_one_user() {
    let (_stub, relay) = twin_relay().await;
    let (mut a, _) = Client::hello(&relay.ws, TOK_A, "d1").await;
    let (mut b, _) = Client::hello(&relay.ws, TOK_B, "d1").await;
    a.subscribe(&["spaces/S1"]).await;
    b.subscribe(&["spaces/S1"]).await;

    a.send(&ClientMsg::HuddleJoin {
        space: "spaces/S1".into(),
    })
    .await;
    let got = b.expect(WAIT, is_huddle).await.expect("B sees A's huddle");
    assert!(
        matches!(got, ServerMsg::Huddle { ref users, .. } if users == &["users/1001".to_string()])
    );
    a.expect(WAIT, is_huddle).await.expect("A sees it too");

    b.send(&ClientMsg::HuddleJoin {
        space: "spaces/S1".into(),
    })
    .await;
    let got = a.expect(WAIT, is_huddle).await.expect("A sees B join");
    assert!(
        matches!(got, ServerMsg::Huddle { ref users, .. } if users == &["users/1001".to_string(), "users/1002".to_string()])
    );

    // A signal is for one user only.
    a.send(&ClientMsg::HuddleSignal {
        space: "spaces/S1".into(),
        to: "users/1002".into(),
        payload: serde_json::json!({ "kind": "offer", "sdp": "v=0" }),
    })
    .await;
    let sig = b
        .expect(WAIT, |m| matches!(m, ServerMsg::HuddleSignal { .. }))
        .await
        .expect("B gets the offer");
    assert!(
        matches!(sig, ServerMsg::HuddleSignal { ref from, ref payload, .. } if from == "users/1001" && payload["kind"] == "offer")
    );
    a.assert_none(
        QUIET,
        |m| matches!(m, ServerMsg::HuddleSignal { .. }),
        "signal echoed to the sender",
    )
    .await;

    // Leaving, and a dropped socket, both shrink the huddle; the last one out ends it.
    b.send(&ClientMsg::HuddleLeave {
        space: "spaces/S1".into(),
    })
    .await;
    let got = a.expect(WAIT, is_huddle).await.expect("A sees B leave");
    assert!(
        matches!(got, ServerMsg::Huddle { ref users, .. } if users == &["users/1001".to_string()])
    );
    drop(a);
    let (mut c, _) = Client::hello(&relay.ws, TOK_B, "d2").await;
    c.subscribe(&["spaces/S1"]).await;
    c.assert_none(
        QUIET,
        |m| matches!(m, ServerMsg::Huddle { ref users, .. } if !users.is_empty()),
        "a huddle survived its last member",
    )
    .await;
}

#[tokio::test]
async fn typing_reaches_other_session_not_sender() {
    let (_stub, relay) = twin_relay().await;
    let (mut a, _) = Client::hello(&relay.ws, TOK_A, "d1").await;
    let (mut b, _) = Client::hello(&relay.ws, TOK_B, "d1").await;
    a.subscribe(&["spaces/S1"]).await;
    b.subscribe(&["spaces/S1"]).await;

    a.send(&ClientMsg::Typing {
        space: "spaces/S1".into(),
        thread: None,
        active: true,
    })
    .await;
    let got = b.expect(WAIT, is_typing).await.expect("B sees A typing");
    assert_eq!(
        got,
        ServerMsg::Typing {
            space: "spaces/S1".into(),
            thread: None,
            user: "users/1001".into(),
            active: true,
            ttl_ms: TYPING_TTL_MS
        }
    );
    a.assert_none(QUIET, is_typing, "typing echoed to the sender")
        .await;

    // Not subscribed to S2 (never asked): typing there is refused, B sees nothing.
    a.send(&ClientMsg::Typing {
        space: "spaces/S2".into(),
        thread: None,
        active: true,
    })
    .await;
    let err = a
        .expect(WAIT, |m| matches!(m, ServerMsg::Error { .. }))
        .await
        .expect("error for unsubscribed space");
    assert!(matches!(err, ServerMsg::Error { ref code, .. } if code == "not_subscribed"));
    b.assert_none(QUIET, is_typing, "typing in an unsubscribed space")
        .await;
}

#[tokio::test]
async fn push_with_secret_reaches_subscriber_and_duplicate_is_dropped() {
    let (_stub, relay) = twin_relay().await;
    let (mut b, _) = Client::hello(&relay.ws, TOK_B, "d1").await;
    b.subscribe(&["spaces/S1"]).await;

    let push = message_push("spaces/S1", "M1", "2026-11-06T10:00:00Z");
    assert_eq!(
        post_push(&relay, &push, Some(SECRET)).await,
        StatusCode::NO_CONTENT
    );
    let got = b.expect(WAIT, is_event).await.expect("B gets the event");
    match got {
        ServerMsg::Event { event } => {
            assert_eq!(event.event_type, event_types::MESSAGE_CREATED);
            assert_eq!(event.space, "spaces/S1");
            assert_eq!(event.message.unwrap().name, "spaces/S1/messages/M1");
        }
        other => panic!("{other:?}"),
    }

    assert_eq!(
        post_push(&relay, &push, Some(SECRET)).await,
        StatusCode::NO_CONTENT
    );
    b.assert_none(QUIET, is_event, "duplicate event").await;

    // Same message, new lastUpdateTime: a different event, delivered.
    let edited = message_push("spaces/S1", "M1", "2026-11-06T10:01:00Z");
    assert_eq!(
        post_push(&relay, &edited, Some(SECRET)).await,
        StatusCode::NO_CONTENT
    );
    b.expect(WAIT, is_event)
        .await
        .expect("edited message is a new event");
}

#[tokio::test]
async fn push_with_wrong_or_missing_secret_is_rejected() {
    let (_stub, relay) = twin_relay().await;
    let (mut b, _) = Client::hello(&relay.ws, TOK_B, "d1").await;
    b.subscribe(&["spaces/S1"]).await;

    let push = message_push("spaces/S1", "M2", "t");
    assert_eq!(
        post_push(&relay, &push, Some("nope")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post_push(&relay, &push, None).await,
        StatusCode::UNAUTHORIZED
    );
    b.assert_none(QUIET, is_event, "event from an unauthenticated push")
        .await;

    // Garbage with the right secret: 400, nothing delivered.
    let status = reqwest::Client::new()
        .post(format!("{}/events/push", relay.http))
        .header("X-Prism-Push-Secret", SECRET)
        .body("not json")
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::BAD_REQUEST);
    b.assert_none(QUIET, is_event, "event from a bad body")
        .await;
}

#[tokio::test]
async fn push_secret_as_query_param_with_extra_envelope_fields() {
    let (_stub, relay) = twin_relay().await;
    let (mut b, _) = Client::hello(&relay.ws, TOK_B, "d1").await;
    b.subscribe(&["spaces/S1"]).await;

    // The shape a Pub/Sub push subscription (or the emulator) actually sends.
    let mut body = serde_json::to_value(message_push("spaces/S1", "M3", "t")).unwrap();
    body["message"]["messageId"] = json!("1234567890");
    body["message"]["publishTime"] = json!("2026-11-06T10:00:00.123Z");
    body["message"]["orderingKey"] = json!("");
    body["subscription"] = json!("projects/p/subscriptions/prism-events-push");
    body["deliveryAttempt"] = json!(1);

    let client = reqwest::Client::new();
    let status = client
        .post(format!("{}/events/push?secret={SECRET}", relay.http))
        .json(&body)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::NO_CONTENT);
    let got = b
        .expect(WAIT, is_event)
        .await
        .expect("event via query-param secret");
    assert!(
        matches!(got, ServerMsg::Event { ref event } if event.message.as_ref().unwrap().name == "spaces/S1/messages/M3")
    );

    let status = client
        .post(format!("{}/events/push?secret=wrong", relay.http))
        .json(&message_push("spaces/S1", "M4", "t"))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    b.assert_none(QUIET, is_event, "event from a wrong query secret")
        .await;
}

#[tokio::test]
async fn draft_syncs_to_same_user_other_devices_only() {
    let (_stub, relay) = twin_relay().await;
    let (mut a1, _) = Client::hello(&relay.ws, TOK_A, "d1").await;
    let (mut a2, _) = Client::hello(&relay.ws, TOK_A, "d2").await;
    let (mut b, _) = Client::hello(&relay.ws, TOK_B, "d1").await;
    a1.subscribe(&["spaces/S1"]).await;
    a2.subscribe(&["spaces/S1"]).await;
    b.subscribe(&["spaces/S1"]).await;

    a1.send(&ClientMsg::Draft {
        space: "spaces/S1".into(),
        thread: None,
        text: "half a thought".into(),
    })
    .await;
    let got = a2
        .expect(WAIT, is_draft)
        .await
        .expect("A's other device gets the draft");
    match got {
        ServerMsg::Draft {
            space,
            text,
            device_id,
            ..
        } => {
            assert_eq!(space, "spaces/S1");
            assert_eq!(text, "half a thought");
            assert_eq!(device_id, "d1");
        }
        other => panic!("{other:?}"),
    }
    b.assert_none(QUIET, is_draft, "draft leaked to another user")
        .await;
    a1.assert_none(QUIET, is_draft, "draft echoed to its origin")
        .await;

    // A third device of A subscribing later is caught up from storage.
    let (mut a3, _) = Client::hello(&relay.ws, TOK_A, "d3").await;
    a3.subscribe(&["spaces/S1"]).await;
    let stored = a3
        .expect(WAIT, is_draft)
        .await
        .expect("stored draft on subscribe");
    assert!(
        matches!(stored, ServerMsg::Draft { ref device_id, ref text, .. } if device_id == "d1" && text == "half a thought")
    );
}

#[tokio::test]
async fn denied_membership_means_no_events_for_that_space() {
    let (_stub, relay) = twin_relay().await;
    let (mut a, _) = Client::hello(&relay.ws, TOK_A, "d1").await;
    let (mut b, _) = Client::hello(&relay.ws, TOK_B, "d1").await;
    a.subscribe(&["spaces/S1", "spaces/S2"]).await;
    b.subscribe(&["spaces/S1", "spaces/S2", "spaces/../v1/x"])
        .await; // S2 denied, last one malformed

    let s2 = message_push("spaces/S2", "M5", "t");
    assert_eq!(
        post_push(&relay, &s2, Some(SECRET)).await,
        StatusCode::NO_CONTENT
    );
    a.expect(WAIT, is_event).await.expect("A is a member of S2");
    b.assert_none(QUIET, is_event, "S2 event for a non-member")
        .await;

    let s1 = message_push("spaces/S1", "M6", "t");
    assert_eq!(
        post_push(&relay, &s1, Some(SECRET)).await,
        StatusCode::NO_CONTENT
    );
    b.expect(WAIT, is_event).await.expect("B still gets S1");
    a.expect(WAIT, is_event).await.expect("A gets S1 too");

    // Typing into the denied space is refused as well.
    b.send(&ClientMsg::Typing {
        space: "spaces/S2".into(),
        thread: None,
        active: true,
    })
    .await;
    a.assert_none(QUIET, is_typing, "typing from a non-member")
        .await;
}

#[tokio::test]
async fn receipts_fan_out_and_presence_goes_offline_on_last_close() {
    let (_stub, relay) = twin_relay().await;
    let (mut a, _) = Client::hello(&relay.ws, TOK_A, "d1").await;
    let (mut b, _) = Client::hello(&relay.ws, TOK_B, "d1").await;
    a.subscribe(&["spaces/S1"]).await;
    b.subscribe(&["spaces/S1"]).await;

    // B learned A is active when it subscribed to the space A was already in.
    let p = b
        .expect(
            WAIT,
            |m| matches!(m, ServerMsg::Presence { user, .. } if user == "users/1001"),
        )
        .await
        .expect("A's presence on subscribe");
    assert!(matches!(
        p,
        ServerMsg::Presence {
            state: PresenceState::Active,
            ..
        }
    ));

    a.send(&ClientMsg::Read {
        space: "spaces/S1".into(),
        thread: Some("spaces/S1/threads/T".into()),
        message: "spaces/S1/messages/M7".into(),
    })
    .await;
    let r = b
        .expect(WAIT, |m| matches!(m, ServerMsg::Receipt { .. }))
        .await
        .expect("receipt");
    match r {
        ServerMsg::Receipt {
            space,
            thread,
            message,
            user,
            ..
        } => {
            assert_eq!(space, "spaces/S1");
            assert_eq!(thread.as_deref(), Some("spaces/S1/threads/T"));
            assert_eq!(message, "spaces/S1/messages/M7");
            assert_eq!(user, "users/1001");
        }
        other => panic!("{other:?}"),
    }
    let stored = relay
        .state
        .db
        .receipt("users/1001", "spaces/S1", Some("spaces/S1/threads/T"))
        .unwrap();
    assert_eq!(stored.unwrap().0, "spaces/S1/messages/M7");

    a.send(&ClientMsg::Presence {
        state: PresenceState::Dnd,
        custom_status: None,
    })
    .await;
    b.expect(WAIT, |m| matches!(m, ServerMsg::Presence { user, state: PresenceState::Dnd, .. } if user == "users/1001"))
        .await
        .expect("dnd");

    drop(a);
    let off = b
        .expect(WAIT, |m| matches!(m, ServerMsg::Presence { user, state: PresenceState::Offline, .. } if user == "users/1001"))
        .await;
    assert!(
        off.is_some(),
        "offline broadcast after A's last socket closed"
    );
}

#[tokio::test]
async fn push_without_resource_is_hydrated_with_a_subscriber_token() {
    let (_stub, relay) = twin_relay().await;
    let (mut b, _) = Client::hello(&relay.ws, TOK_B, "d1").await;
    b.subscribe(&["spaces/S1"]).await;

    let data = EventData {
        message: Some(Message {
            name: "spaces/S1/messages/M9".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let push = PubSubPush::wrap(
        event_types::MESSAGE_CREATED,
        "spaces/S1",
        "2026-11-06T10:00:00Z",
        &data,
    );
    assert_eq!(
        post_push(&relay, &push, Some(SECRET)).await,
        StatusCode::NO_CONTENT
    );
    let got = b.expect(WAIT, is_event).await.expect("hydrated event");
    match got {
        ServerMsg::Event { event } => {
            assert_eq!(event.message.unwrap().text.as_deref(), Some("hydrated"))
        }
        other => panic!("{other:?}"),
    }
}

// ---------- real mode: OIDC push auth and the subscription manager ----------

#[derive(serde::Serialize)]
struct Claims<'a> {
    iss: &'a str,
    aud: &'a str,
    email: &'a str,
    email_verified: bool,
    iat: i64,
    exp: i64,
    sub: &'a str,
}

fn oidc_token(aud: &str, email: &str, iss: &str) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = Claims {
        iss,
        aud,
        email,
        email_verified: true,
        iat: now,
        exp: now + 300,
        sub: "1234",
    };
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(KID.into());
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(TEST_KEY_PEM.as_bytes()).unwrap();
    jsonwebtoken::encode(&header, &claims, &key).unwrap()
}

async fn post_oidc(relay: &Relay, push: &PubSubPush, token: &str) -> StatusCode {
    reqwest::Client::new()
        .post(format!("{}/events/push", relay.http))
        .bearer_auth(token)
        .json(push)
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn real_mode_push_requires_a_valid_google_oidc_token() {
    let stub = stub().await;
    let mut config = twin_config(&stub);
    config.real_mode = true;
    config.push_audience = Some(AUD.into());
    config.push_service_account = Some(SA.into());
    let relay = start_relay(config).await;
    let (mut b, _) = Client::hello(&relay.ws, TOK_B, "d1").await;
    b.subscribe(&["spaces/S1"]).await;

    let push = message_push("spaces/S1", "M10", "t");
    assert_eq!(
        post_push(&relay, &push, Some(SECRET)).await,
        StatusCode::UNAUTHORIZED,
        "shared secret is twin-only"
    );
    assert_eq!(
        post_oidc(
            &relay,
            &push,
            &oidc_token("https://other.test/", SA, "https://accounts.google.com")
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post_oidc(
            &relay,
            &push,
            &oidc_token(AUD, "someone@else.test", "https://accounts.google.com")
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post_oidc(&relay, &push, &oidc_token(AUD, SA, "https://evil.test")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post_oidc(&relay, &push, "not.a.jwt").await,
        StatusCode::UNAUTHORIZED
    );
    b.assert_none(QUIET, is_event, "event from a rejected push")
        .await;

    assert_eq!(
        post_oidc(
            &relay,
            &push,
            &oidc_token(AUD, SA, "https://accounts.google.com")
        )
        .await,
        StatusCode::NO_CONTENT
    );
    b.expect(WAIT, is_event)
        .await
        .expect("event after a valid OIDC push");
    assert_eq!(
        post_oidc(
            &relay,
            &message_push("spaces/S1", "M11", "t"),
            &oidc_token(AUD, SA, "accounts.google.com")
        )
        .await,
        StatusCode::NO_CONTENT
    );
    b.expect(WAIT, is_event)
        .await
        .expect("bare issuer form is accepted too");
}

#[tokio::test]
async fn real_mode_creates_a_workspace_events_subscription_on_first_hello() {
    let stub = stub().await;
    let mut config = twin_config(&stub);
    config.real_mode = true;
    config.pubsub_topic = Some("projects/p/topics/prism-events".into());
    let relay = start_relay(config).await;
    let (mut a, _) = Client::hello(&relay.ws, TOK_A, "d1").await;

    let mut row = None;
    for _ in 0..40 {
        row = relay.state.db.subscription("users/1001").unwrap();
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let row = row.expect("subscription stored after hello");
    assert_eq!(row.name, "subscriptions/sub-1001");
    assert!(chrono::DateTime::parse_from_rfc3339(&row.expire_time).is_ok());

    let bodies = stub.state.subscription_bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1);
    let body = &bodies[0];
    assert_eq!(body["targetResource"], "//chat.googleapis.com/spaces/-");
    assert_eq!(
        body["notificationEndpoint"]["pubsubTopic"],
        "projects/p/topics/prism-events"
    );
    assert_eq!(body["payloadOptions"]["includeResource"], true);
    assert_eq!(body["eventTypes"].as_array().unwrap().len(), 8);

    // A second device of the same user does not create another one.
    let (mut a2, _) = Client::hello(&relay.ws, TOK_A, "d2").await;
    a2.sync().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(stub.state.subscription_bodies.lock().unwrap().len(), 1);
    a.sync().await;
}

#[tokio::test]
async fn twin_mode_never_touches_the_events_api() {
    let (stub, relay) = twin_relay().await;
    let (mut a, _) = Client::hello(&relay.ws, TOK_A, "d1").await;
    a.sync().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(stub.state.subscription_bodies.lock().unwrap().is_empty());
    assert!(relay.state.db.subscription("users/1001").unwrap().is_none());
}

// ---------- Pub/Sub pull mode ----------

#[tokio::test]
async fn pull_from_the_emulator_feeds_the_push_pipeline_and_acks() {
    let stub = stub().await;
    let mut config = twin_config(&stub);
    config.pubsub_subscription = Some(PULL_SUB.into());
    config.pubsub_emulator_host = Some(stub.host.clone());
    let relay = start_relay(config).await;
    let (mut b, _) = Client::hello(&relay.ws, TOK_B, "d1").await;
    b.subscribe(&["spaces/S1"]).await;

    let health: Value = reqwest::get(format!("{}/health", relay.http))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["pull"], PULL_SUB);

    stub.state
        .pull_queue
        .lock()
        .unwrap()
        .push_back(pulled(&message_push("spaces/S1", "P1", "t"), "ack-1"));
    let got = b
        .expect(WAIT, is_event)
        .await
        .expect("event pulled from the emulator");
    match got {
        ServerMsg::Event { event } => {
            assert_eq!(event.event_type, event_types::MESSAGE_CREATED);
            assert_eq!(event.space, "spaces/S1");
            assert_eq!(event.message.unwrap().name, "spaces/S1/messages/P1");
        }
        other => panic!("{other:?}"),
    }
    wait_for("the ack", || stub.state.acks.lock().unwrap().len() == 1).await;
    assert_eq!(*stub.state.acks.lock().unwrap(), ["ack-1"]);

    // Google's long-poll request shape, and no auth header against the emulator.
    let pulls = stub.state.pulls.lock().unwrap().clone();
    assert!(!pulls.is_empty());
    for (auth, body) in &pulls {
        assert!(auth.is_none(), "emulator pull carried {auth:?}");
        assert_eq!(body["maxMessages"], 100);
        assert_eq!(body["returnImmediately"], false);
    }

    // A redelivery of the same message is acked again but deduped, not fanned out.
    stub.state
        .pull_queue
        .lock()
        .unwrap()
        .push_back(pulled(&message_push("spaces/S1", "P1", "t"), "ack-2"));
    b.assert_none(QUIET, is_event, "duplicate pulled event")
        .await;
    wait_for("the second ack", || {
        stub.state.acks.lock().unwrap().len() == 2
    })
    .await;
    assert_eq!(*stub.state.acks.lock().unwrap(), ["ack-1", "ack-2"]);

    // Push still works alongside pull.
    assert_eq!(
        post_push(&relay, &message_push("spaces/S1", "P1b", "t"), Some(SECRET)).await,
        StatusCode::NO_CONTENT
    );
    b.expect(WAIT, is_event).await.expect("pushed event");
}

#[tokio::test]
async fn pull_from_google_mints_a_token_with_the_jwt_bearer_grant() {
    let stub = stub().await;
    *stub.state.pull_bearer.lock().unwrap() = Some(PULL_TOKEN.into());
    let token_uri = format!("{}/token", stub.base);
    let key_path =
        std::env::temp_dir().join(format!("prism-relay-test-sa-{}.json", uuid::Uuid::new_v4()));
    std::fs::write(
        &key_path,
        serde_json::to_vec(&json!({
            "type": "service_account",
            "project_id": "p",
            "client_email": PULL_SA,
            "private_key": TEST_KEY_PEM,
            "token_uri": token_uri,
        }))
        .unwrap(),
    )
    .unwrap();

    let mut config = twin_config(&stub);
    config.pubsub_subscription = Some(PULL_SUB.into());
    config.pubsub_api = stub.base.clone();
    config.google_credentials = Some(key_path.to_string_lossy().into_owned());
    let relay = start_relay(config).await;
    let (mut b, _) = Client::hello(&relay.ws, TOK_B, "d1").await;
    b.subscribe(&["spaces/S1"]).await;

    stub.state
        .pull_queue
        .lock()
        .unwrap()
        .push_back(pulled(&message_push("spaces/S1", "P2", "t"), "ack-g1"));
    let got = b
        .expect(WAIT, is_event)
        .await
        .expect("event pulled from Google with a minted token");
    assert!(
        matches!(got, ServerMsg::Event { ref event } if event.message.as_ref().unwrap().name == "spaces/S1/messages/P2")
    );
    wait_for("the ack", || stub.state.acks.lock().unwrap().len() == 1).await;
    assert_eq!(*stub.state.acks.lock().unwrap(), ["ack-g1"]);

    // Several pulls and one ack have gone by; one grant served them all.
    let pulls = stub.state.pulls.lock().unwrap().clone();
    assert!(pulls.len() >= 2, "{} pulls", pulls.len());
    assert!(pulls
        .iter()
        .all(|(auth, _)| auth.as_deref() == Some("Bearer pull-access")));
    let grants = stub.state.token_grants.lock().unwrap().clone();
    assert_eq!(grants.len(), 1, "token cached across pulls");
    let claims = &grants[0];
    assert_eq!(claims["iss"], PULL_SA);
    assert_eq!(claims["aud"], token_uri);
    assert_eq!(claims["scope"], "https://www.googleapis.com/auth/pubsub");
    let iat = claims["iat"].as_i64().unwrap();
    let exp = claims["exp"].as_i64().unwrap();
    assert_eq!(exp - iat, 3600);
    assert!((chrono::Utc::now().timestamp() - iat).abs() < 60);
    let _ = std::fs::remove_file(&key_path);
}

#[test]
fn signed_assertion_verifies_with_the_public_key() {
    let key = ServiceAccountKey {
        client_email: PULL_SA.into(),
        private_key: TEST_KEY_PEM.into(),
        token_uri: "https://oauth2.googleapis.com/token".into(),
    };
    let now = chrono::Utc::now().timestamp();
    let jwt = sign_assertion(&key, now).unwrap();
    assert_eq!(decode_header(&jwt).unwrap().alg, Algorithm::RS256);
    let public = DecodingKey::from_rsa_components(TEST_KEY_N, "AQAB").unwrap();
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&["https://oauth2.googleapis.com/token"]);
    let data = decode::<Assertion>(&jwt, &public, &validation).unwrap();
    assert_eq!(data.claims, Assertion::new(&key, now));
    // A different public key must not verify it.
    let mut wrong = Validation::new(Algorithm::RS256);
    wrong.set_audience(&["https://other.test/token"]);
    assert!(decode::<Assertion>(&jwt, &public, &wrong).is_err());
}
