//! Shared process state: config, Google client, SQLite, the session hub, and
//! the small caches (membership decisions, push dedupe, JWKS).

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::RwLock;

use crate::config::Config;
use crate::db::Db;
use crate::google::Google;
use crate::hub::Hub;

pub const MEMBERSHIP_TTL: Duration = Duration::from_secs(600);
pub const DEDUPE_TTL: Duration = Duration::from_secs(60);

pub struct JwksCache {
    pub fetched_at: Instant,
    pub keys: jsonwebtoken::jwk::JwkSet,
}

pub struct AppState {
    pub config: Config,
    pub http: reqwest::Client,
    pub google: Google,
    pub db: Db,
    pub hub: Hub,
    // note: both caches are per process; a second instance re-checks and may re-deliver
    /// (user, space) -> (allowed, decided_at)
    pub membership: DashMap<(String, String), (bool, Instant)>,
    /// push dedupe key -> first seen
    pub dedupe: DashMap<String, Instant>,
    pub jwks: RwLock<Option<JwksCache>>,
}

impl AppState {
    pub fn new(config: Config) -> anyhow::Result<Arc<Self>> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("prism-relay/0.1")
            .build()?;
        let google = Google::new(
            http.clone(),
            config.endpoints.clone(),
            config.events_api.clone(),
        );
        let db = Db::open(&config.db_path)?;
        Ok(Arc::new(Self {
            config,
            http,
            google,
            db,
            hub: Hub::default(),
            membership: DashMap::new(),
            dedupe: DashMap::new(),
            jwks: RwLock::new(None),
        }))
    }

    pub fn now_rfc3339() -> String {
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    /// Cached "is this user a member of this space" with the session's own token.
    /// Transport or 5xx errors deny without caching so a retry can succeed.
    pub async fn authorize_space(
        &self,
        user: &str,
        user_id: &str,
        token: &str,
        space: &str,
    ) -> bool {
        let key = (user.to_string(), space.to_string());
        if let Some(entry) = self.membership.get(&key) {
            let (allowed, at) = *entry;
            if at.elapsed() < MEMBERSHIP_TTL {
                return allowed;
            }
        }
        match self.google.is_member(token, space, user_id).await {
            Ok(allowed) => {
                self.membership.insert(key, (allowed, Instant::now()));
                if !allowed {
                    tracing::info!(user, space, "membership denied");
                }
                allowed
            }
            Err(e) => {
                tracing::warn!(user, space, error = %e, "membership check failed; denying");
                false
            }
        }
    }

    /// True the first time a key is seen within `DEDUPE_TTL`.
    pub fn first_seen(&self, key: String) -> bool {
        let now = Instant::now();
        if let Some(at) = self.dedupe.get(&key) {
            if now.duration_since(*at) < DEDUPE_TTL {
                return false;
            }
        }
        self.dedupe.insert(key, now);
        if self.dedupe.len() > 4096 {
            self.dedupe
                .retain(|_, at| now.duration_since(*at) < DEDUPE_TTL);
        }
        true
    }

    pub fn sweep_caches(&self) {
        let now = Instant::now();
        self.dedupe
            .retain(|_, at| now.duration_since(*at) < DEDUPE_TTL);
        self.membership
            .retain(|_, (_, at)| now.duration_since(*at) < MEMBERSHIP_TTL);
    }
}
