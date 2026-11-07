//! Process configuration, all from the environment with twin-friendly defaults.

#[derive(Clone, Debug)]
pub struct Config {
    /// Listen port (`PRISM_TWIN_PORT`, default 8081). Always binds 0.0.0.0.
    pub port: u16,
    /// Absolute base clients reach the twin at; used for avatar and download URLs.
    pub public_base: String,
    /// Where events go in direct mode (`PRISM_RELAY_PUSH_URL`).
    pub relay_push_url: String,
    /// Shared secret sent as `X-Prism-Push-Secret` in direct mode.
    pub push_secret: String,
    /// Pub/Sub emulator host (`PRISM_PUBSUB_EMULATOR_HOST`, e.g. `localhost:4588`, no scheme).
    pub pubsub_emulator_host: Option<String>,
    /// Pub/Sub topic (`PRISM_PUBSUB_TOPIC`, `projects/{p}/topics/{t}`).
    pub pubsub_topic: Option<String>,
    /// Access token lifetime in seconds.
    pub token_ttl_secs: i64,
}

impl Config {
    pub fn from_env() -> Self {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let port = get("PRISM_TWIN_PORT")
            .and_then(|p| p.parse().ok())
            .unwrap_or(8081);
        Self {
            port,
            public_base: get("PRISM_TWIN_BASE")
                .unwrap_or_else(|| format!("http://127.0.0.1:{port}"))
                .trim_end_matches('/')
                .to_string(),
            relay_push_url: get("PRISM_RELAY_PUSH_URL")
                .unwrap_or_else(|| "http://127.0.0.1:8080/events/push".into()),
            push_secret: get("PRISM_PUSH_SECRET").unwrap_or_else(|| "dev".into()),
            pubsub_emulator_host: get("PRISM_PUBSUB_EMULATOR_HOST"),
            pubsub_topic: get("PRISM_PUBSUB_TOPIC"),
            token_ttl_secs: 3600,
        }
    }

    /// Defaults for tests: nothing read from the environment.
    pub fn for_tests(push_url: &str) -> Self {
        Self {
            port: 0,
            public_base: "http://127.0.0.1:8081".into(),
            relay_push_url: push_url.into(),
            push_secret: "dev".into(),
            pubsub_emulator_host: None,
            pubsub_topic: None,
            token_ttl_secs: 3600,
        }
    }
}
