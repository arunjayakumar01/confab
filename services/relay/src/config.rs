//! Runtime configuration. `Endpoints::from_env()` is the shared source of truth;
//! the relay adds its own `PRISM_*` knobs on top.

use std::time::Duration;

use confab_proto::Endpoints;

#[derive(Clone, Debug)]
pub struct Config {
    pub endpoints: Endpoints,
    /// `PRISM_CHAT_API` contains `googleapis.com`: OIDC push auth and the subscription manager.
    pub real_mode: bool,
    /// Twin-mode shared secret for `POST /events/push`.
    pub push_secret: String,
    /// Real mode: expected `aud` of the Pub/Sub OIDC token.
    pub push_audience: Option<String>,
    /// Real mode: expected `email` of the Pub/Sub OIDC token.
    pub push_service_account: Option<String>,
    /// Real mode: `projects/<p>/topics/<t>`; absent disables the subscription manager.
    pub pubsub_topic: Option<String>,
    /// `PRISM_PUBSUB_SUBSCRIPTION`: `projects/<p>/subscriptions/<s>`. Set, the relay
    /// pulls events from that subscription itself (no inbound HTTPS needed); unset,
    /// events arrive only through `POST /events/push`.
    pub pubsub_subscription: Option<String>,
    /// `PRISM_PUBSUB_EMULATOR_HOST` (`host:port`, no scheme): pull from a local
    /// Pub/Sub emulator over plain HTTP with no auth.
    pub pubsub_emulator_host: Option<String>,
    /// Pub/Sub REST base used when no emulator host is set (`PRISM_PUBSUB_API`).
    pub pubsub_api: String,
    /// `GOOGLE_APPLICATION_CREDENTIALS`: service-account key or gcloud ADC file for the
    /// Google pull; unset reads `~/.config/gcloud/application_default_credentials.json`.
    pub google_credentials: Option<String>,
    pub db_path: String,
    pub port: u16,
    pub jwks_url: String,
    /// Workspace Events API base.
    pub events_api: String,
    /// How long a fresh socket may stay silent before its `hello`.
    pub hello_timeout: Duration,
}

impl Config {
    /// Defaults around a given endpoint set (twin mode unless `chat_api` is Google's).
    pub fn new(endpoints: Endpoints) -> Self {
        let real_mode = endpoints.chat_api.contains("googleapis.com");
        Self {
            endpoints,
            real_mode,
            push_secret: "dev".into(),
            push_audience: None,
            push_service_account: None,
            pubsub_topic: None,
            pubsub_subscription: None,
            pubsub_emulator_host: None,
            pubsub_api: "https://pubsub.googleapis.com".into(),
            google_credentials: None,
            db_path: "relay.db".into(),
            port: 8080,
            jwks_url: "https://www.googleapis.com/oauth2/v3/certs".into(),
            events_api: "https://workspaceevents.googleapis.com".into(),
            hello_timeout: Duration::from_secs(10),
        }
    }

    pub fn from_env() -> Self {
        let mut c = Self::new(Endpoints::from_env());
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        if let Some(v) = get("PRISM_PUSH_SECRET") {
            c.push_secret = v;
        }
        c.push_audience = get("PRISM_PUSH_AUDIENCE");
        c.push_service_account = get("PRISM_PUSH_SERVICE_ACCOUNT");
        c.pubsub_topic = get("PRISM_PUBSUB_TOPIC");
        c.pubsub_subscription = get("PRISM_PUBSUB_SUBSCRIPTION");
        c.pubsub_emulator_host = get("PRISM_PUBSUB_EMULATOR_HOST");
        if let Some(v) = get("PRISM_PUBSUB_API") {
            c.pubsub_api = v;
        }
        c.google_credentials = get("GOOGLE_APPLICATION_CREDENTIALS");
        if let Some(v) = get("PRISM_DB") {
            c.db_path = v;
        }
        if let Some(p) = get("PRISM_RELAY_PORT").and_then(|v| v.parse().ok()) {
            c.port = p;
        }
        if let Some(v) = get("PRISM_JWKS_URL") {
            c.jwks_url = v;
        }
        if let Some(v) = get("PRISM_EVENTS_API") {
            c.events_api = v;
        }
        c
    }

    pub fn subscriptions_enabled(&self) -> bool {
        self.real_mode && self.pubsub_topic.is_some()
    }

    /// Pull mode is independent of twin/real mode: the twin can publish to the
    /// emulator and the relay pulls from it, the same shape as real GCP.
    pub fn pull_enabled(&self) -> bool {
        self.pubsub_subscription.is_some()
    }

    /// Pub/Sub REST base for the pull loop: the emulator when configured, else Google.
    pub fn pubsub_base(&self) -> String {
        match &self.pubsub_emulator_host {
            Some(host) => format!("http://{}", host.trim_end_matches('/')),
            None => self.pubsub_api.trim_end_matches('/').to_string(),
        }
    }
}
