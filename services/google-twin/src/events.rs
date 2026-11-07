//! Workspace Events delivery. Two modes:
//!
//! * direct: `POST PRISM_RELAY_PUSH_URL` with the Pub/Sub push envelope and
//!   `X-Prism-Push-Secret` (what the relay's `/events/push` expects in twin mode);
//! * publish: when `PRISM_PUBSUB_EMULATOR_HOST` and `PRISM_PUBSUB_TOPIC` are both set,
//!   `POST http://{host}/v1/{topic}:publish` with `{"messages":[{data, attributes}]}`
//!   and no auth header, letting the emulator push to the relay.
//!
//! Delivery is fire-and-forget on a tokio task; failures are logged at warn.

use crate::config::Config;
use crate::util::now_str;
use prism_proto::events::{EventData, PubSubMessage, PubSubPush};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug)]
enum Mode {
    Direct { url: String, secret: String },
    Publish { url: String },
}

#[derive(Clone)]
pub struct Pusher {
    client: reqwest::Client,
    mode: Mode,
    seq: Arc<AtomicU64>,
}

#[derive(serde::Serialize)]
struct PublishBody<'a> {
    messages: [&'a PubSubMessage; 1],
}

impl Pusher {
    pub fn from_config(config: &Config) -> Self {
        let mode = match (&config.pubsub_emulator_host, &config.pubsub_topic) {
            (Some(host), Some(topic)) => {
                let host = host.trim_start_matches("http://").trim_end_matches('/');
                Mode::Publish {
                    url: format!("http://{host}/v1/{topic}:publish"),
                }
            }
            _ => Mode::Direct {
                url: config.relay_push_url.clone(),
                secret: config.push_secret.clone(),
            },
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("reqwest client");
        Self {
            client,
            mode,
            seq: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Human-readable description for the startup banner.
    pub fn describe(&self) -> String {
        match &self.mode {
            Mode::Direct { url, .. } => format!("direct push to {url}"),
            Mode::Publish { url } => format!("Pub/Sub publish to {url}"),
        }
    }

    /// Wrap and send one event. Returns immediately; the HTTP call runs on its own task.
    pub fn push(&self, event_type: &str, space: &str, data: EventData) {
        let event_time = now_str();
        let mut push = PubSubPush::wrap(event_type, space, &event_time, &data);
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        push.message.message_id = Some(format!("{}", 1_000_000_000 + n));
        let client = self.client.clone();
        let mode = self.mode.clone();
        let event_type = event_type.to_string();
        let space = space.to_string();
        let (log_type, log_space) = (event_type.clone(), space.clone());
        let fut = async move {
            let result = match &mode {
                Mode::Direct { url, secret } => {
                    client
                        .post(url)
                        .header("X-Prism-Push-Secret", secret)
                        .json(&push)
                        .send()
                        .await
                }
                Mode::Publish { url } => {
                    let mut msg = push.message.clone();
                    msg.message_id = None;
                    msg.publish_time = None;
                    client
                        .post(url)
                        .json(&PublishBody { messages: [&msg] })
                        .send()
                        .await
                }
            };
            match result {
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!(event_type, space, "event delivered");
                }
                Ok(resp) => {
                    tracing::warn!(event_type, space, status = %resp.status(), "event delivery rejected");
                }
                Err(err) => {
                    tracing::warn!(event_type, space, error = %err, "event delivery failed");
                }
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(fut);
            }
            Err(_) => tracing::warn!(
                event_type = log_type,
                space = log_space,
                "no tokio runtime; event dropped"
            ),
        }
    }
}
