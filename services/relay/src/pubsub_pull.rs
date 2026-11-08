//! Cloud Pub/Sub pull mode. A relay with no public HTTPS endpoint (a laptop
//! behind NAT) cannot take push deliveries, so when `PRISM_PUBSUB_SUBSCRIPTION`
//! is set it long-polls the subscription itself and hands every message to
//! [`crate::push::ingest`], the same pipeline the push endpoint uses after
//! authentication. Against the emulator (`PRISM_PUBSUB_EMULATOR_HOST`) there is
//! no auth; against Google, `GOOGLE_APPLICATION_CREDENTIALS` (default: the gcloud
//! ADC file) is either a service-account key, exchanged for an access token with
//! the RFC 7523 JWT bearer grant, or an `authorized_user` file written by
//! `gcloud auth application-default login`, refreshed with its refresh token.
//! User credentials also need a quota project, sent as `x-goog-user-project`.
//!
//! The loop never ends and never panics the process: every failure is logged
//! at warn and retried with a 1 s → 30 s exponential backoff.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use confab_proto::events::{PubSubMessage, PubSubPush};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::push;
use crate::state::AppState;

/// OAuth scope the access token is minted for.
pub const SCOPE: &str = "https://www.googleapis.com/auth/pubsub";
const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
/// Lifetime claimed in the assertion; Google caps it at one hour.
pub const ASSERTION_LIFETIME_SECS: i64 = 3600;
/// Refresh the cached access token this long before it expires.
const TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(60);
const MAX_MESSAGES: u32 = 100;
/// Google holds a `returnImmediately: false` pull open for up to ~90 s.
const PULL_TIMEOUT: Duration = Duration::from_secs(120);
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// Pause after an empty pull: Google has already waited ~90 s by then, but the
/// emulator answers at once and would otherwise be polled in a tight loop.
const EMPTY_PAUSE: Duration = Duration::from_millis(250);

/// Start the puller when a subscription is configured; otherwise log push-only.
pub fn spawn(state: Arc<AppState>) {
    let Some(subscription) = state.config.pubsub_subscription.clone() else {
        tracing::info!("events: push only (PRISM_PUBSUB_SUBSCRIPTION not set)");
        return;
    };
    let via = if state.config.pubsub_emulator_host.is_some() {
        "emulator"
    } else {
        "Google"
    };
    tracing::info!(
        subscription = %subscription,
        base = %state.config.pubsub_base(),
        "events: push and pull from {subscription} via {via}"
    );
    // note: single puller, one pull in flight; maxMessages=100 is the only flow control
    tokio::spawn(run(state, subscription));
}

async fn run(state: Arc<AppState>, subscription: String) {
    let http = match reqwest::Client::builder()
        .timeout(PULL_TIMEOUT)
        .user_agent("prism-relay/0.1")
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "pubsub pull: cannot build the http client; pull disabled");
            return;
        }
    };
    let mut puller = Puller {
        http,
        base: state.config.pubsub_base(),
        subscription,
        emulator: state.config.pubsub_emulator_host.is_some(),
        credentials: state.config.google_credentials.clone(),
        auth: None,
        quota_project: None,
    };
    let mut backoff = BACKOFF_MIN;
    loop {
        match puller.pull_once(&state).await {
            Ok(0) => {
                backoff = BACKOFF_MIN;
                tokio::time::sleep(EMPTY_PAUSE).await;
            }
            Ok(_) => backoff = BACKOFF_MIN,
            Err(e) => {
                tracing::warn!(
                    error = %format!("{e:#}"),
                    retry_in_ms = backoff.as_millis() as u64,
                    "pubsub pull failed"
                );
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
        }
    }
}

struct Puller {
    http: reqwest::Client,
    base: String,
    subscription: String,
    emulator: bool,
    credentials: Option<String>,
    /// Lazily loaded so a missing or bad key file is a retried error, not a crash.
    auth: Option<Auth>,
    /// `x-goog-user-project` for user credentials; `None` for a service account.
    quota_project: Option<String>,
}

/// One `receivedMessages[]` entry. Unknown fields (`deliveryAttempt`, the
/// emulator's extra `message_id`, ...) are ignored by serde's default.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedMessage {
    #[serde(alias = "ack_id")]
    pub ack_id: String,
    #[serde(default)]
    pub message: Option<PubSubMessage>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct PullResponse {
    #[serde(default, alias = "received_messages")]
    pub received_messages: Vec<ReceivedMessage>,
}

impl Puller {
    /// `None` against the emulator; a cached or freshly minted token otherwise.
    async fn bearer(&mut self, state: &AppState) -> anyhow::Result<Option<String>> {
        if self.emulator {
            return Ok(None);
        }
        let auth = match &mut self.auth {
            Some(a) => a,
            None => {
                let auth = Auth::load(self.credentials.as_deref(), state.http.clone())?;
                self.quota_project = auth.quota_project(&self.subscription);
                self.auth.insert(auth)
            }
        };
        auth.token().await.map(Some)
    }

    /// Bearer plus the quota-project header user credentials need.
    fn authed(
        &self,
        req: reqwest::RequestBuilder,
        bearer: &Option<String>,
    ) -> reqwest::RequestBuilder {
        let mut req = req;
        if let Some(t) = bearer {
            req = req.bearer_auth(t);
        }
        if let Some(p) = &self.quota_project {
            req = req.header("x-goog-user-project", p);
        }
        req
    }

    /// One pull, fan-out of every message, one ack for the batch. Returns how
    /// many messages were received (acked), not how many were fanned out.
    async fn pull_once(&mut self, state: &AppState) -> anyhow::Result<usize> {
        let bearer = self.bearer(state).await?;
        let req = self
            .http
            .post(format!("{}/v1/{}:pull", self.base, self.subscription))
            .json(&serde_json::json!({
                "maxMessages": MAX_MESSAGES,
                "returnImmediately": false,
            }));
        let resp = self
            .authed(req, &bearer)
            .send()
            .await
            .context("pull request")?;
        let body: PullResponse = ok_json(resp, "pull").await?;
        if body.received_messages.is_empty() {
            return Ok(0);
        }
        let mut ack_ids = Vec::with_capacity(body.received_messages.len());
        let mut delivered = 0usize;
        for received in body.received_messages {
            ack_ids.push(received.ack_id);
            match received.message {
                Some(message) => {
                    let envelope = PubSubPush {
                        message,
                        subscription: self.subscription.clone(),
                    };
                    if push::ingest(state, envelope).await {
                        delivered += 1;
                    }
                }
                None => tracing::warn!("pulled a receivedMessage without a message; acking it"),
            }
        }
        let pulled = ack_ids.len();
        // note: ack after fan-out is at-least-once; a redelivery after the
        // 60 s dedupe window (ack lost, process restarted) is fanned out again
        self.acknowledge(state, ack_ids).await?;
        tracing::debug!(pulled, delivered, "pubsub pull batch");
        Ok(pulled)
    }

    async fn acknowledge(&mut self, state: &AppState, ack_ids: Vec<String>) -> anyhow::Result<()> {
        // A long poll can outlive the token's remaining life; re-check before acking.
        let bearer = self.bearer(state).await?;
        let req = self
            .http
            .post(format!(
                "{}/v1/{}:acknowledge",
                self.base, self.subscription
            ))
            .json(&serde_json::json!({ "ackIds": ack_ids }));
        let resp = self
            .authed(req, &bearer)
            .send()
            .await
            .context("acknowledge request")?;
        let _: serde_json::Value = ok_json(resp, "acknowledge").await?;
        Ok(())
    }
}

/// Status check with the body kept for the log, then JSON (an empty body reads as `{}`).
async fn ok_json<T: DeserializeOwned>(resp: reqwest::Response, what: &str) -> anyhow::Result<T> {
    let status = resp.status();
    let text = resp.text().await.with_context(|| format!("{what} body"))?;
    if !status.is_success() {
        let snippet: String = text.chars().take(200).collect();
        bail!("{what} returned {status}: {snippet}");
    }
    let text = if text.trim().is_empty() { "{}" } else { &text };
    serde_json::from_str(text).with_context(|| format!("{what} json"))
}

// ---------- Google auth: service-account key or gcloud ADC ----------

/// Google's OAuth token endpoint, used for the `authorized_user` refresh grant.
const USER_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";

/// Where `gcloud auth application-default login` writes its file.
// note: HOME-based path only; Windows (%APPDATA%\gcloud) sets GOOGLE_APPLICATION_CREDENTIALS explicitly
pub fn default_adc_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/.config/gcloud/application_default_credentials.json")
}

/// `projects/<p>/subscriptions/<s>` → `<p>`.
pub fn project_of(subscription: &str) -> Option<&str> {
    subscription.strip_prefix("projects/")?.split('/').next()
}

/// Both shapes a `GOOGLE_APPLICATION_CREDENTIALS` file can have.
#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum Credentials {
    #[serde(rename = "service_account")]
    ServiceAccount(ServiceAccountKey),
    #[serde(rename = "authorized_user")]
    AuthorizedUser(AuthorizedUser),
}

/// The fields of a Google service-account key JSON the grant needs.
#[derive(Deserialize, Clone, Debug)]
pub struct ServiceAccountKey {
    pub client_email: String,
    pub private_key: String,
    pub token_uri: String,
}

/// The fields of a gcloud ADC (`authorized_user`) file the refresh grant needs.
#[derive(Deserialize, Clone, Debug)]
pub struct AuthorizedUser {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    #[serde(default)]
    pub quota_project_id: Option<String>,
}

/// Claims of the JWT bearer grant assertion (RFC 7523 as Google applies it).
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Assertion {
    pub iss: String,
    pub scope: String,
    pub aud: String,
    pub iat: i64,
    pub exp: i64,
}

impl Assertion {
    pub fn new(key: &ServiceAccountKey, now: i64) -> Self {
        Self {
            iss: key.client_email.clone(),
            scope: SCOPE.into(),
            aud: key.token_uri.clone(),
            iat: now,
            exp: now + ASSERTION_LIFETIME_SECS,
        }
    }
}

/// RS256-signed assertion for `now` (unix seconds).
pub fn sign_assertion(key: &ServiceAccountKey, now: i64) -> anyhow::Result<String> {
    let signer = EncodingKey::from_rsa_pem(key.private_key.as_bytes())
        .context("private_key is not an RSA PEM")?;
    encode(
        &Header::new(Algorithm::RS256),
        &Assertion::new(key, now),
        &signer,
    )
    .context("sign assertion")
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
}

fn default_expires_in() -> u64 {
    ASSERTION_LIFETIME_SECS as u64
}

// note: the token lives in this task only; a second puller would mint its own
struct Auth {
    creds: Credentials,
    http: reqwest::Client,
    /// (access token, expires at)
    cached: Option<(String, Instant)>,
}

impl Auth {
    /// `path` unset falls back to the gcloud ADC file, like Google's own clients.
    fn load(path: Option<&str>, http: reqwest::Client) -> anyhow::Result<Self> {
        let path = path.map(str::to_string).unwrap_or_else(default_adc_path);
        let raw = std::fs::read(&path).with_context(|| {
            format!("read {path} (set GOOGLE_APPLICATION_CREDENTIALS or run `gcloud auth application-default login`)")
        })?;
        let creds: Credentials = serde_json::from_slice(&raw).context(
            "credentials json: expected a service_account key or an authorized_user ADC file",
        )?;
        match &creds {
            Credentials::ServiceAccount(key) => {
                EncodingKey::from_rsa_pem(key.private_key.as_bytes())
                    .context("private_key is not an RSA PEM")?;
                tracing::info!(client_email = %key.client_email, token_uri = %key.token_uri, "pubsub pull: service account key loaded");
            }
            Credentials::AuthorizedUser(u) => {
                tracing::info!(path = %path, quota_project = ?u.quota_project_id, "pubsub pull: gcloud user credentials loaded");
            }
        }
        Ok(Self {
            creds,
            http,
            cached: None,
        })
    }

    /// User credentials bill the ADC file's quota project, else the subscription's.
    fn quota_project(&self, subscription: &str) -> Option<String> {
        match &self.creds {
            Credentials::ServiceAccount(_) => None,
            Credentials::AuthorizedUser(u) => u
                .quota_project_id
                .clone()
                .or_else(|| project_of(subscription).map(str::to_string)),
        }
    }

    /// The cached access token while it has more than the margin left, else a new one.
    async fn token(&mut self) -> anyhow::Result<String> {
        if let Some((token, expires_at)) = &self.cached {
            if expires_at.saturating_duration_since(Instant::now()) > TOKEN_REFRESH_MARGIN {
                return Ok(token.clone());
            }
        }
        let (token_uri, form): (String, Vec<(&str, String)>) = match &self.creds {
            Credentials::ServiceAccount(key) => (
                key.token_uri.clone(),
                vec![
                    ("grant_type", GRANT_TYPE.into()),
                    (
                        "assertion",
                        sign_assertion(key, chrono::Utc::now().timestamp())?,
                    ),
                ],
            ),
            Credentials::AuthorizedUser(u) => (
                USER_TOKEN_URI.into(),
                vec![
                    ("grant_type", "refresh_token".into()),
                    ("client_id", u.client_id.clone()),
                    ("client_secret", u.client_secret.clone()),
                    ("refresh_token", u.refresh_token.clone()),
                ],
            ),
        };
        let resp = self
            .http
            .post(&token_uri)
            .form(&form)
            .send()
            .await
            .context("token request")?;
        let tr: TokenResponse = match ok_json(resp, "token endpoint").await {
            Ok(t) => t,
            // Workspace session-control policies expire user refresh tokens (invalid_rapt).
            Err(e) if matches!(self.creds, Credentials::AuthorizedUser(_)) => {
                return Err(e.context(
                    "Google refused the ADC refresh token; run `gcloud auth application-default login` again",
                ));
            }
            Err(e) => return Err(e),
        };
        if tr.access_token.is_empty() {
            bail!("token endpoint returned an empty access_token");
        }
        self.cached = Some((
            tr.access_token.clone(),
            Instant::now() + Duration::from_secs(tr.expires_in),
        ));
        tracing::info!(
            expires_in = tr.expires_in,
            "pubsub pull: access token minted"
        );
        Ok(tr.access_token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> ServiceAccountKey {
        ServiceAccountKey {
            client_email: "relay@p.iam.gserviceaccount.com".into(),
            private_key: "not a pem".into(),
            token_uri: "https://oauth2.googleapis.com/token".into(),
        }
    }

    #[test]
    fn assertion_claims_follow_the_jwt_bearer_grant() {
        let k = key();
        let a = Assertion::new(&k, 1_762_000_000);
        assert_eq!(a.iss, k.client_email);
        assert_eq!(a.aud, k.token_uri);
        assert_eq!(a.scope, "https://www.googleapis.com/auth/pubsub");
        assert_eq!(a.iat, 1_762_000_000);
        assert_eq!(a.exp - a.iat, 3600);
        // Exactly these five claims and nothing else on the wire.
        let json = serde_json::to_value(&a).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["aud", "exp", "iat", "iss", "scope"]);
    }

    #[test]
    fn signing_with_a_bad_key_is_an_error_not_a_panic() {
        let err = sign_assertion(&key(), 0).unwrap_err();
        assert!(err.to_string().contains("RSA PEM"), "{err:#}");
    }

    #[test]
    fn credentials_file_can_be_a_key_or_a_gcloud_adc_file() {
        let sa = r#"{"type":"service_account","client_email":"r@p.iam.gserviceaccount.com",
            "private_key":"pem","token_uri":"https://oauth2.googleapis.com/token","project_id":"p"}"#;
        let adc = r#"{"type":"authorized_user","client_id":"c.apps.googleusercontent.com",
            "client_secret":"s","refresh_token":"r","quota_project_id":"q"}"#;
        let adc_no_quota =
            r#"{"type":"authorized_user","client_id":"c","client_secret":"s","refresh_token":"r"}"#;
        let http = reqwest::Client::new();
        let auth = |raw: &str| Auth {
            creds: serde_json::from_str(raw).unwrap(),
            http: http.clone(),
            cached: None,
        };
        let sub = "projects/my-project/subscriptions/prism-relay-pull";
        assert_eq!(auth(sa).quota_project(sub), None);
        assert_eq!(auth(adc).quota_project(sub).as_deref(), Some("q"));
        assert_eq!(
            auth(adc_no_quota).quota_project(sub).as_deref(),
            Some("my-project")
        );
        assert_eq!(project_of("subscriptions/x"), None);
        assert!(serde_json::from_str::<Credentials>(r#"{"type":"external_account"}"#).is_err());
    }

    #[test]
    fn pull_response_tolerates_the_emulator_shape() {
        // Both messageId and message_id, ack_id in snake case, deliveryAttempt, and an
        // unknown top-level field.
        let raw = r#"{
          "receivedMessages": [{
            "ack_id": "a-1",
            "deliveryAttempt": 1,
            "message": {
              "data": "e30=",
              "attributes": {"ce-type": "t"},
              "messageId": "7",
              "message_id": "7",
              "publishTime": "2026-11-06T10:00:00Z",
              "publish_time": "2026-11-06T10:00:00Z",
              "orderingKey": ""
            }
          }],
          "extra": true
        }"#;
        let r: PullResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(r.received_messages.len(), 1);
        let m = &r.received_messages[0];
        assert_eq!(m.ack_id, "a-1");
        let msg = m.message.as_ref().unwrap();
        assert_eq!(msg.message_id.as_deref(), Some("7"));
        assert_eq!(msg.attributes["ce-type"], "t");

        let empty: PullResponse = serde_json::from_str("{}").unwrap();
        assert!(empty.received_messages.is_empty());
    }
}
