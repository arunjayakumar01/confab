//! The handful of Google (or twin) calls the relay makes with a user's token:
//! userinfo, "get my membership", fetch one message, and Workspace Events
//! subscription create / renew / list.

use anyhow::{anyhow, bail, Context};
use prism_proto::chat::{Message, UserInfo};
use prism_proto::relay::event_types;
use prism_proto::Endpoints;
use serde::Deserialize;

#[derive(Clone)]
pub struct Google {
    http: reqwest::Client,
    endpoints: Endpoints,
    events_api: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SubscriptionInfo {
    pub name: String,
    pub expire_time: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Operation {
    #[serde(default)]
    done: bool,
    #[serde(default)]
    response: Option<SubscriptionResource>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubscriptionResource {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    expire_time: Option<String>,
    #[serde(default)]
    target_resource: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListSubscriptions {
    #[serde(default)]
    subscriptions: Vec<SubscriptionResource>,
}

impl Google {
    pub fn new(http: reqwest::Client, endpoints: Endpoints, events_api: String) -> Self {
        Self {
            http,
            endpoints,
            events_api,
        }
    }

    pub async fn userinfo(&self, token: &str) -> anyhow::Result<UserInfo> {
        let resp = self
            .http
            .get(&self.endpoints.userinfo)
            .bearer_auth(token)
            .send()
            .await
            .context("userinfo request")?;
        if !resp.status().is_success() {
            bail!("userinfo returned {}", resp.status());
        }
        let info: UserInfo = resp.json().await.context("userinfo body")?;
        if info.sub.is_empty() {
            bail!("userinfo without sub");
        }
        Ok(info)
    }

    /// `GET /v1/spaces/{s}/members/users/{id}`: 200 = member, 403/404 = not.
    /// Anything else is an error, which the caller treats as deny-without-cache.
    // note: any 200 counts, INVITED included; check `state == JOINED` to be stricter
    pub async fn is_member(&self, token: &str, space: &str, user_id: &str) -> anyhow::Result<bool> {
        // Google's membership name is spaces/{space}/members/{numeric user id}; the
        // users/ prefix belongs to the User resource, not the membership (live 2026-11-06).
        let url = format!(
            "{}/v1/{}/members/{}",
            self.endpoints.chat_api,
            space,
            user_id.trim_start_matches("users/")
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("membership request")?;
        match resp.status().as_u16() {
            200..=299 => Ok(true),
            403 | 404 => Ok(false),
            other => bail!("membership check returned {other}"),
        }
    }

    pub async fn get_message(&self, token: &str, name: &str) -> anyhow::Result<Message> {
        let url = format!("{}/v1/{}", self.endpoints.chat_api, name);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("get message")?;
        if !resp.status().is_success() {
            bail!("get message returned {}", resp.status());
        }
        resp.json().await.context("message body")
    }

    fn subscription_from(
        op: Operation,
        fallback_name: Option<&str>,
    ) -> anyhow::Result<SubscriptionInfo> {
        let Some(r) = op.response else {
            // note: an operation that is not done yet is not polled; the next
            // hello of this user tries again and adopts the existing subscription.
            bail!("operation {} not done", op.name.unwrap_or_default());
        };
        let name = r
            .name
            .or_else(|| fallback_name.map(str::to_string))
            .ok_or_else(|| anyhow!("no name"))?;
        let expire_time = r.expire_time.ok_or_else(|| anyhow!("no expireTime"))?;
        let _ = op.done;
        Ok(SubscriptionInfo { name, expire_time })
    }

    /// `POST /v1/subscriptions` for all Chat spaces of the token's user.
    /// On 409 (already exists) the existing subscription is listed and adopted.
    pub async fn create_subscription(
        &self,
        token: &str,
        topic: &str,
    ) -> anyhow::Result<SubscriptionInfo> {
        let body = serde_json::json!({
            "targetResource": "//chat.googleapis.com/spaces/-",
            "eventTypes": [
                event_types::MESSAGE_CREATED, event_types::MESSAGE_UPDATED, event_types::MESSAGE_DELETED,
                event_types::REACTION_CREATED, event_types::REACTION_DELETED,
                event_types::MEMBERSHIP_CREATED, event_types::MEMBERSHIP_DELETED,
                event_types::SPACE_UPDATED,
            ],
            "notificationEndpoint": { "pubsubTopic": topic },
            "payloadOptions": { "includeResource": true },
        });
        let url = format!("{}/v1/subscriptions", self.events_api);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .context("create subscription")?;
        let status = resp.status();
        if status.as_u16() == 409 {
            return self.find_existing_subscription(token).await;
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("create subscription returned {status}: {text}");
        }
        let op: Operation = resp.json().await.context("operation body")?;
        Self::subscription_from(op, None)
    }

    async fn find_existing_subscription(&self, token: &str) -> anyhow::Result<SubscriptionInfo> {
        let filter = format!(
            "event_types:\"{}\" AND target_resource=\"//chat.googleapis.com/spaces/-\"",
            event_types::MESSAGE_CREATED
        );
        let url = format!("{}/v1/subscriptions", self.events_api);
        let resp = self
            .http
            .get(&url)
            .query(&[("filter", filter.as_str())])
            .bearer_auth(token)
            .send()
            .await
            .context("list subscriptions")?;
        if !resp.status().is_success() {
            bail!("list subscriptions returned {}", resp.status());
        }
        let list: ListSubscriptions = resp.json().await.context("list body")?;
        list.subscriptions
            .into_iter()
            .filter(|s| {
                s.target_resource
                    .as_deref()
                    .is_none_or(|t| t == "//chat.googleapis.com/spaces/-")
            })
            .find_map(|s| {
                Some(SubscriptionInfo {
                    name: s.name?,
                    expire_time: s.expire_time?,
                })
            })
            .ok_or_else(|| anyhow!("409 on create but no subscription listed"))
    }

    /// `PATCH /v1/{name}?updateMask=ttl` with `{"ttl":"0s"}` resets to the maximum lifetime.
    pub async fn renew_subscription(
        &self,
        token: &str,
        name: &str,
    ) -> anyhow::Result<SubscriptionInfo> {
        let url = format!("{}/v1/{}", self.events_api, name);
        let resp = self
            .http
            .patch(&url)
            .query(&[("updateMask", "ttl")])
            .bearer_auth(token)
            .json(&serde_json::json!({ "ttl": "0s" }))
            .send()
            .await
            .context("renew subscription")?;
        let status = resp.status();
        if status.as_u16() == 404 {
            bail!("gone");
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("renew returned {status}: {text}");
        }
        let op: Operation = resp.json().await.context("operation body")?;
        Self::subscription_from(op, Some(name))
    }
}
