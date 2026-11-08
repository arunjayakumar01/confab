//! The Google Chat `users.availability` resource: the signed-in user's presence
//! state, custom status and do-not-disturb window. JSON camelCase on the wire,
//! Google's field names.

use serde::{Deserialize, Serialize};

/// `GET|PATCH v1/users/me/availability` and the `markAs*` answers.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Availability {
    /// `users/me/availability`
    #[serde(default)]
    pub name: String,
    /// `ACTIVE` | `IDLE` | `AWAY` | `DO_NOT_DISTURB` | `STATE_UNSPECIFIED`
    #[serde(default)]
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_status: Option<CustomStatusWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub do_not_disturb_metadata: Option<DndMeta>,
}

impl Availability {
    pub const ACTIVE: &'static str = "ACTIVE";
    pub const IDLE: &'static str = "IDLE";
    pub const AWAY: &'static str = "AWAY";
    pub const DO_NOT_DISTURB: &'static str = "DO_NOT_DISTURB";
    pub const STATE_UNSPECIFIED: &'static str = "STATE_UNSPECIFIED";
    /// The longest custom status text Google accepts.
    pub const STATUS_TEXT_MAX: usize = 64;
}

/// What the person set as their status: text with an emoji, optionally expiring.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomStatusWire {
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<EmojiWire>,
    /// RFC 3339; absent when the status does not expire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expire_time: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmojiWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unicode: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DndMeta {
    /// RFC 3339: when do-not-disturb ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiration_time: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_json_uses_google_names_and_drops_none() {
        let a: Availability = serde_json::from_str(
            r#"{"name":"users/me/availability","state":"DO_NOT_DISTURB",
                "customStatus":{"text":"Heads down","emoji":{"unicode":"🎧"},"expireTime":"2026-11-07T12:00:00Z"},
                "doNotDisturbMetadata":{"expirationTime":"2026-11-07T11:00:00Z"}}"#,
        )
        .unwrap();
        assert_eq!(a.state, Availability::DO_NOT_DISTURB);
        assert_eq!(a.custom_status.as_ref().unwrap().text, "Heads down");
        assert_eq!(
            a.custom_status
                .as_ref()
                .unwrap()
                .emoji
                .as_ref()
                .unwrap()
                .unicode
                .as_deref(),
            Some("🎧")
        );
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(
            v["doNotDisturbMetadata"]["expirationTime"],
            "2026-11-07T11:00:00Z"
        );
        let back: Availability = serde_json::from_value(v).unwrap();
        assert_eq!(back, a);

        let plain: Availability = serde_json::from_str(r#"{"state":"ACTIVE"}"#).unwrap();
        assert!(plain.custom_status.is_none());
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            serde_json::json!({ "name": "", "state": "ACTIVE" })
        );
    }
}
