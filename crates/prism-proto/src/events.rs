//! Workspace Events as they arrive from Cloud Pub/Sub push (and from the twin,
//! which posts the same envelope). Pub/Sub wraps the CloudEvent: attributes carry
//! `ce-type`, `ce-subject` (`//chat.googleapis.com/spaces/{id}`), `ce-time`;
//! `data` is base64 JSON holding the resource (`message`, `reaction`, ...).

use crate::relay::ChatEvent;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PubSubPush {
    pub message: PubSubMessage,
    #[serde(default)]
    pub subscription: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PubSubMessage {
    /// base64 of the event JSON
    #[serde(default)]
    pub data: String,
    #[serde(default)]
    pub attributes: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_time: Option<String>,
}

/// Decoded `data` payload. Google sends the resource under its own key.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<crate::chat::Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reaction: Option<crate::chat::Reaction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membership: Option<crate::chat::Membership>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<crate::chat::Space>,
}

impl PubSubPush {
    /// Build a push envelope the way Pub/Sub would (used by the twin and tests).
    pub fn wrap(event_type: &str, space: &str, event_time: &str, data: &EventData) -> Self {
        let json = serde_json::to_vec(data).unwrap_or_default();
        let mut attributes = HashMap::new();
        attributes.insert("ce-type".into(), event_type.to_string());
        attributes.insert(
            "ce-subject".into(),
            format!("//chat.googleapis.com/{space}"),
        );
        attributes.insert("ce-time".into(), event_time.to_string());
        attributes.insert("ce-specversion".into(), "1.0".into());
        Self {
            message: PubSubMessage {
                data: base64::engine::general_purpose::STANDARD.encode(json),
                attributes,
                message_id: None,
                publish_time: Some(event_time.to_string()),
            },
            subscription: String::new(),
        }
    }

    /// Decode into the normalised event the relay fans out.
    pub fn into_chat_event(self) -> Result<ChatEvent, String> {
        let a = &self.message.attributes;
        let event_type = a.get("ce-type").cloned().ok_or("missing ce-type")?;
        let subject = a.get("ce-subject").cloned().unwrap_or_default();
        let space = subject
            .rsplit("googleapis.com/")
            .next()
            .unwrap_or("")
            .to_string();
        let event_time = a.get("ce-time").cloned().unwrap_or_default();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(self.message.data.as_bytes())
            .or_else(|_| {
                base64::engine::general_purpose::URL_SAFE.decode(self.message.data.as_bytes())
            })
            .map_err(|e| format!("bad base64: {e}"))?;
        let data: EventData = if bytes.is_empty() {
            EventData::default()
        } else {
            serde_json::from_slice(&bytes)
                .map_err(|e| format!("bad event json in {event_type}: {e}"))?
        };
        Ok(ChatEvent {
            event_type,
            space,
            event_time,
            message: data.message,
            reaction: data.reaction,
            membership: data.membership,
            space_resource: data.space,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::Message;

    #[test]
    fn wrap_then_decode() {
        let data = EventData {
            message: Some(Message {
                name: "spaces/A/messages/B".into(),
                text: Some("hi".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let push = PubSubPush::wrap(
            crate::relay::event_types::MESSAGE_CREATED,
            "spaces/A",
            "2026-11-06T10:00:00Z",
            &data,
        );
        let ev = push.into_chat_event().unwrap();
        assert_eq!(ev.space, "spaces/A");
        assert_eq!(ev.message.unwrap().text.as_deref(), Some("hi"));
    }
}
