//! The Google Meet REST API v2 subset a huddle needs: a space (the room behind a
//! meeting code) and who is in its active conference. JSON camelCase on the wire,
//! Google's field names.

use serde::{Deserialize, Serialize};

/// `POST v2/spaces`, `GET v2/spaces/{code}`: the room. `config` and
/// `activeConference` are read by the client and not carried here.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeetSpace {
    /// `spaces/{id}`
    #[serde(default)]
    pub name: String,
    /// `https://meet.google.com/abc-defg-hij`
    #[serde(default)]
    pub meeting_uri: String,
    /// `abc-defg-hij`
    #[serde(default)]
    pub meeting_code: String,
}

/// One person in the active conference (Prism's flattening of Google's
/// `conferenceRecords/{id}/participants` entry).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeetParticipant {
    /// `users/{id}` for a signed-in Workspace user; absent for anonymous and phone callers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// RFC 3339: when they first joined (`earliestStartTime`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
}

/// Whether the space has a conference running and who is in it now.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeetStatus {
    #[serde(default)]
    pub active: bool,
    /// `conferenceRecords/{id}` while active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conference_record: Option<String>,
    /// People still in the call (no `latestEndTime`), joiners first.
    #[serde(default)]
    pub participants: Vec<MeetParticipant>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meet_json_uses_google_names_and_drops_none() {
        let space: MeetSpace = serde_json::from_str(
            r#"{"name":"spaces/abc","meetingUri":"https://meet.google.com/abc-defg-hij",
                "meetingCode":"abc-defg-hij","config":{"accessType":"TRUSTED"}}"#,
        )
        .unwrap();
        assert_eq!(space.name, "spaces/abc");
        assert_eq!(space.meeting_code, "abc-defg-hij");
        assert_eq!(
            serde_json::to_value(&space).unwrap(),
            serde_json::json!({
                "name": "spaces/abc",
                "meetingUri": "https://meet.google.com/abc-defg-hij",
                "meetingCode": "abc-defg-hij"
            })
        );

        let status = MeetStatus {
            active: true,
            conference_record: Some("conferenceRecords/xyz".into()),
            participants: vec![MeetParticipant {
                user: Some("users/1001".into()),
                display_name: Some("Arun Jayakumar".into()),
                since: None,
            }],
        };
        let v = serde_json::to_value(&status).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "active": true,
                "conferenceRecord": "conferenceRecords/xyz",
                "participants": [{ "user": "users/1001", "displayName": "Arun Jayakumar" }]
            })
        );
        let back: MeetStatus = serde_json::from_value(v).unwrap();
        assert_eq!(back, status);

        let idle: MeetStatus = serde_json::from_str(r#"{"active":false}"#).unwrap();
        assert!(!idle.active);
        assert!(idle.conference_record.is_none());
        assert!(idle.participants.is_empty());
    }
}
