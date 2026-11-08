//! Google Chat API v1 resources, the subset Prism reads and writes.
//! Field names follow the REST JSON (camelCase). Everything optional is `Option`
//! so a partial resource from a list call or an event round-trips unchanged.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct User {
    /// `users/{id}`; default because Google's events can carry a user without one, and that must not lose the event.
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain_id: Option<String>,
    /// `HUMAN` | `BOT`
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "type")]
    pub user_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_anonymous: Option<bool>,
    /// Not a Chat API field: the twin fills it, real deployments use the People API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpaceDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidelines: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MembershipCount {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joined_direct_human_user_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joined_group_count: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    /// `spaces/{id}`
    pub name: String,
    /// `SPACE` | `GROUP_CHAT` | `DIRECT_MESSAGE`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// `THREADED_MESSAGES` | `GROUPED_MESSAGES` | `UNTHREADED_MESSAGES`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_threading_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_details: Option<SpaceDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membership_count: Option<MembershipCount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_user_allowed: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Membership {
    /// `spaces/{space}/members/{member}`
    pub name: String,
    /// `JOINED` | `INVITED` | `NOT_A_MEMBER`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// `ROLE_MEMBER` | `ROLE_MANAGER`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member: Option<User>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_time: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    /// `spaces/{space}/threads/{thread}`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_key: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomEmoji {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporary_image_uri: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Emoji {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unicode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_emoji: Option<CustomEmoji>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmojiReactionSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<Emoji>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reaction_count: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Reaction {
    /// `spaces/{space}/messages/{message}/reactions/{reaction}`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<User>,
    #[serde(default)] // a deleted reaction's event carries only its name
    pub emoji: Emoji,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentDataRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_upload_token: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DriveDataRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive_file_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    /// `spaces/{space}/messages/{message}/attachments/{attachment}`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_data_ref: Option<AttachmentDataRef>,
    /// Drive attachments. A message list carries only `name` and `source`; the full
    /// resource, this field included, comes from `GET /v1/{attachment.name}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive_data_ref: Option<DriveDataRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_uri: Option<String>,
    /// `DRIVE_FILE` | `UPLOADED_CONTENT`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UserMentionMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<User>,
    /// `ADD` | `MENTION`
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "type")]
    pub mention_type: Option<String>,
}

/// `richLinkMetadata.meetSpaceLinkData`: the Meet space a `RICH_LINK` annotation
/// points at. Google posts a huddle as a message with no text and one of these.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeetSpaceLinkData {
    /// `abc-defg-hij`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting_code: Option<String>,
    /// `MEETING` | `HUDDLE`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    /// `STARTED` | `ENDED` | `MISSED`; Google updates the message when it changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub huddle_status: Option<String>,
}

/// `annotations[].richLinkMetadata` for a `RICH_LINK` annotation.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RichLinkMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// `DRIVE_FILE` | `CHAT_SPACE` | `MEET_SPACE` | `CALENDAR_EVENT`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rich_link_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meet_space_link_data: Option<MeetSpaceLinkData>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    /// `USER_MENTION` | `SLASH_COMMAND` | `RICH_LINK`
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "type")]
    pub annotation_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_index: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_mention: Option<UserMentionMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rich_link_metadata: Option<RichLinkMetadata>,
}

impl Annotation {
    pub const RICH_LINK: &'static str = "RICH_LINK";
    pub const MEET_SPACE: &'static str = "MEET_SPACE";
    pub const HUDDLE: &'static str = "HUDDLE";
    pub const HUDDLE_STARTED: &'static str = "STARTED";
    pub const HUDDLE_ENDED: &'static str = "ENDED";

    /// The annotation Google puts on a huddle message: a `RICH_LINK` at index 0 to
    /// the Meet space, `meetSpaceLinkData.type: HUDDLE`, `huddleStatus` as given.
    pub fn huddle(meeting_uri: &str, meeting_code: &str, huddle_status: &str) -> Self {
        Self {
            annotation_type: Some(Self::RICH_LINK.into()),
            start_index: Some(0),
            length: None,
            user_mention: None,
            rich_link_metadata: Some(RichLinkMetadata {
                uri: Some(meeting_uri.into()),
                rich_link_type: Some(Self::MEET_SPACE.into()),
                meet_space_link_data: Some(MeetSpaceLinkData {
                    meeting_code: Some(meeting_code.into()),
                    r#type: Some(Self::HUDDLE.into()),
                    huddle_status: Some(huddle_status.into()),
                }),
            }),
        }
    }

    /// `(meetingCode, huddleStatus)` when this is a Meet huddle link.
    pub fn huddle_link(&self) -> Option<(&str, &str)> {
        let data = self
            .rich_link_metadata
            .as_ref()?
            .meet_space_link_data
            .as_ref()?;
        if data.r#type.as_deref() != Some(Self::HUDDLE) {
            return None;
        }
        Some((
            data.meeting_code.as_deref().unwrap_or(""),
            data.huddle_status.as_deref().unwrap_or(""),
        ))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotedMessageMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update_time: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// `spaces/{space}/messages/{message}`; client-assigned ids look like
    /// `spaces/{space}/messages/client-{uuid}`.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<User>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_time: Option<String>,
    /// Set with `delete_time` on a tombstone (`showDeleted`, or the deleted event): who removed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletion_metadata: Option<DeletionMetadata>,
    /// Markdown as sent (`markupSyntax=MARKDOWN`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Output only: Google's rendered form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formatted_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<Thread>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_reply: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<Space>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachment: Vec<Attachment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emoji_reaction_summaries: Vec<EmojiReactionSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quoted_message_metadata: Option<QuotedMessageMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_assigned_message_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeletionMetadata {
    /// CREATOR | SPACE_OWNER | ADMIN | APP_MESSAGE_EXPIRY | CREATOR_VIA_APP | SPACE_OWNER_VIA_APP | SPACE_MEMBER
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletion_type: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpaceReadState {
    /// `users/{user}/spaces/{space}/spaceReadState`
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_read_time: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListSpacesResponse {
    #[serde(default)]
    pub spaces: Vec<Space>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListMessagesResponse {
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListMembershipsResponse {
    #[serde(default)]
    pub memberships: Vec<Membership>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListReactionsResponse {
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchMessagesResponse {
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UploadAttachmentResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_data_ref: Option<AttachmentDataRef>,
}

/// OpenID userinfo (`/oauth2/v3/userinfo`). `sub` is the Chat user id: `users/{sub}`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct UserInfo {
    pub sub: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hd: Option<String>,
}

/// OAuth token response (`/token`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default = "default_token_type")]
    pub token_type: String,
}
fn default_token_type() -> String {
    "Bearer".into()
}

/// Google-style error body `{ "error": { "code", "message", "status" } }`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ErrorDetail {
    pub code: u16,
    pub message: String,
    pub status: String,
}

impl Message {
    pub fn space_name(&self) -> &str {
        self.name.rsplitn(3, '/').nth(2).unwrap_or("")
    }
    pub fn message_id(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_round_trips_google_json() {
        let json = r#"{"name":"spaces/AAA/messages/BBB.CCC","sender":{"name":"users/1001","displayName":"Priya","type":"HUMAN"},"createTime":"2026-11-06T10:00:00.000Z","text":"hi **there**","thread":{"name":"spaces/AAA/threads/BBB"},"emojiReactionSummaries":[{"emoji":{"unicode":"👍"},"reactionCount":2}]}"#;
        let m: Message = serde_json::from_str(json).unwrap();
        assert_eq!(m.space_name(), "spaces/AAA");
        assert_eq!(m.message_id(), "BBB.CCC");
        assert_eq!(
            m.sender.as_ref().unwrap().user_type.as_deref(),
            Some("HUMAN")
        );
        let back = serde_json::to_value(&m).unwrap();
        assert_eq!(back["emojiReactionSummaries"][0]["reactionCount"], 2);
        assert!(back.get("deleteTime").is_none());
    }

    #[test]
    fn huddle_annotation_round_trips_google_json() {
        let json = r#"{"name":"spaces/AAA/messages/BBB.CCC","annotations":[{"type":"RICH_LINK","startIndex":0,
            "richLinkMetadata":{"uri":"https://meet.google.com/kyf-vogr-zmm","richLinkType":"MEET_SPACE",
            "meetSpaceLinkData":{"meetingCode":"kyf-vogr-zmm","type":"HUDDLE","huddleStatus":"STARTED"}}}]}"#;
        let m: Message = serde_json::from_str(json).unwrap();
        assert_eq!(m.annotations.len(), 1);
        assert_eq!(
            m.annotations[0].huddle_link(),
            Some(("kyf-vogr-zmm", "STARTED"))
        );
        assert_eq!(
            m.annotations[0],
            Annotation::huddle(
                "https://meet.google.com/kyf-vogr-zmm",
                "kyf-vogr-zmm",
                "STARTED"
            )
        );
        let back = serde_json::to_value(&m).unwrap();
        assert_eq!(
            back["annotations"][0],
            serde_json::json!({
                "type": "RICH_LINK",
                "startIndex": 0,
                "richLinkMetadata": {
                    "uri": "https://meet.google.com/kyf-vogr-zmm",
                    "richLinkType": "MEET_SPACE",
                    "meetSpaceLinkData": {
                        "meetingCode": "kyf-vogr-zmm",
                        "type": "HUDDLE",
                        "huddleStatus": "STARTED"
                    }
                }
            })
        );
        // A mention keeps its shape and carries no rich link.
        let mention: Annotation =
            serde_json::from_str(r#"{"type":"USER_MENTION","startIndex":6,"length":12}"#).unwrap();
        assert!(mention.rich_link_metadata.is_none());
        assert!(mention.huddle_link().is_none());
        assert!(serde_json::to_value(&mention)
            .unwrap()
            .get("richLinkMetadata")
            .is_none());
    }
}
