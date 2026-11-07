//! Shared contracts for Prism: the Google Chat API v1 subset the client uses,
//! the relay WebSocket protocol, and the Workspace Events push envelope.
//! Every other crate builds against these types; change them here first.

pub mod availability;
pub mod calendar;
pub mod chat;
pub mod events;
pub mod meet;
pub mod relay;
pub mod tasks;

/// Runtime endpoints. The Google twin serves all of them from one base;
/// real Google splits them across hosts. Read from env with `Endpoints::from_env`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Endpoints {
    /// Chat API base, e.g. `https://chat.googleapis.com` or `http://127.0.0.1:8081`
    pub chat_api: String,
    /// OAuth authorization page, e.g. `https://accounts.google.com/o/oauth2/v2/auth`
    pub oauth_auth: String,
    /// Token exchange, e.g. `https://oauth2.googleapis.com/token`
    pub oauth_token: String,
    /// OpenID userinfo, e.g. `https://openidconnect.googleapis.com/v1/userinfo`
    pub userinfo: String,
    /// Relay WebSocket, e.g. `ws://127.0.0.1:8080/ws`
    pub relay_ws: String,
    /// People API base for directory search, e.g. `https://people.googleapis.com`;
    /// the twin serves `people:searchDirectoryPeople` on its own base.
    #[serde(default)]
    pub people_api: String,
    /// Google Tasks API base, e.g. `https://tasks.googleapis.com`; the twin serves
    /// `tasks/v1/lists/@default/tasks` on its own base. Profiles saved before this
    /// field existed deserialize with the Google default.
    #[serde(default = "default_tasks_api")]
    pub tasks_api: String,
    /// Google Meet REST API base, e.g. `https://meet.googleapis.com`; the twin serves
    /// `v2/spaces` and `v2/conferenceRecords` on its own base. Profiles saved before
    /// this field existed deserialize with the Google default.
    #[serde(default = "default_meet_api")]
    pub meet_api: String,
    /// Google Calendar API v3 base including the version path, e.g.
    /// `https://www.googleapis.com/calendar/v3`; the twin serves the same paths under
    /// `{twin}/calendar/v3`. Profiles saved before this field existed deserialize with
    /// the Google default.
    #[serde(default = "default_calendar_api")]
    pub calendar_api: String,
    /// Google Drive API v3 base including the version path, e.g.
    /// `https://www.googleapis.com/drive/v3`; the twin serves the same paths under
    /// `{twin}/drive/v3`. Uploads go to the same host with `/upload/drive/v3` in place of
    /// `/drive/v3`. Profiles saved before this field existed deserialize with the Google default.
    #[serde(default = "default_drive_api")]
    pub drive_api: String,
    pub client_id: String,
    /// Google issues a client secret even for Desktop (installed) clients and the token
    /// endpoint requires it beside PKCE; it is not treated as a secret. The twin ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}

/// Real Google's Tasks API base; also the serde default for `Endpoints::tasks_api`.
pub const GOOGLE_TASKS_API: &str = "https://tasks.googleapis.com";

fn default_tasks_api() -> String {
    GOOGLE_TASKS_API.to_string()
}

/// Real Google's Meet REST API base; also the serde default for `Endpoints::meet_api`.
pub const GOOGLE_MEET_API: &str = "https://meet.googleapis.com";

fn default_meet_api() -> String {
    GOOGLE_MEET_API.to_string()
}

/// Real Google's Calendar API v3 base; also the serde default for `Endpoints::calendar_api`.
pub const GOOGLE_CALENDAR_API: &str = "https://www.googleapis.com/calendar/v3";

fn default_calendar_api() -> String {
    GOOGLE_CALENDAR_API.to_string()
}

/// Real Google's Drive API v3 base; also the serde default for `Endpoints::drive_api`.
pub const GOOGLE_DRIVE_API: &str = "https://www.googleapis.com/drive/v3";

fn default_drive_api() -> String {
    GOOGLE_DRIVE_API.to_string()
}

/// The Drive upload base for a Drive API base: `/drive/v3` becomes `/upload/drive/v3`
/// (`https://www.googleapis.com/upload/drive/v3` for Google); a base without that
/// suffix, such as a bare twin base, gets `/upload/drive/v3` appended.
pub fn drive_upload_api(drive_api: &str) -> String {
    let base = drive_api.trim_end_matches('/');
    match base.strip_suffix("/drive/v3") {
        Some(host) => format!("{host}/upload/drive/v3"),
        None => format!("{base}/upload/drive/v3"),
    }
}

impl Endpoints {
    /// Local twin defaults; every field overridable by `PRISM_<FIELD>` env vars.
    pub fn from_env() -> Self {
        let get = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let twin = get("PRISM_TWIN_BASE", "http://127.0.0.1:8081");
        let chat_api = get("PRISM_CHAT_API", &twin);
        let real_google = chat_api.contains("googleapis.com");
        Self {
            chat_api: chat_api.clone(),
            oauth_auth: get("PRISM_OAUTH_AUTH", &format!("{twin}/o/oauth2/v2/auth")),
            oauth_token: get("PRISM_OAUTH_TOKEN", &format!("{twin}/token")),
            userinfo: get("PRISM_USERINFO", &format!("{twin}/oauth2/v3/userinfo")),
            relay_ws: get("PRISM_RELAY_WS", "ws://127.0.0.1:8080/ws"),
            people_api: get(
                "PRISM_PEOPLE_API",
                if real_google {
                    "https://people.googleapis.com"
                } else {
                    &twin
                },
            ),
            tasks_api: get(
                "PRISM_TASKS_API",
                if real_google { GOOGLE_TASKS_API } else { &twin },
            ),
            meet_api: get(
                "PRISM_MEET_API",
                if real_google { GOOGLE_MEET_API } else { &twin },
            ),
            calendar_api: get(
                "PRISM_CALENDAR_API",
                &if real_google {
                    GOOGLE_CALENDAR_API.to_string()
                } else {
                    format!("{twin}/calendar/v3")
                },
            ),
            drive_api: get(
                "PRISM_DRIVE_API",
                &if real_google {
                    GOOGLE_DRIVE_API.to_string()
                } else {
                    format!("{twin}/drive/v3")
                },
            ),
            client_id: get("PRISM_CLIENT_ID", "prism-dev"),
            client_secret: std::env::var("PRISM_CLIENT_SECRET")
                .ok()
                .filter(|s| !s.trim().is_empty()),
        }
    }
}

/// OAuth scopes Prism requests (internal consent screen, one Workspace org).
pub const SCOPES: &[&str] = &[
    "openid",
    "email",
    "profile",
    "https://www.googleapis.com/auth/chat.spaces.readonly",
    "https://www.googleapis.com/auth/chat.memberships",
    "https://www.googleapis.com/auth/chat.messages",
    "https://www.googleapis.com/auth/chat.messages.reactions",
    "https://www.googleapis.com/auth/chat.users.readstate",
    "https://www.googleapis.com/auth/chat.spaces.create",
    "https://www.googleapis.com/auth/directory.readonly",
    "https://www.googleapis.com/auth/tasks",
    "https://www.googleapis.com/auth/meetings.space.created",
    "https://www.googleapis.com/auth/meetings.space.readonly",
    "https://www.googleapis.com/auth/calendar.events",
    "https://www.googleapis.com/auth/calendar.readonly",
    "https://www.googleapis.com/auth/drive.appdata",
    "https://www.googleapis.com/auth/chat.users.availability",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_saved_before_tasks_api_still_deserialize() {
        let raw = r#"{"chat_api":"http://127.0.0.1:8081","oauth_auth":"","oauth_token":"",
            "userinfo":"","relay_ws":"","people_api":"http://127.0.0.1:8081","client_id":"prism-dev"}"#;
        let ep: Endpoints = serde_json::from_str(raw).unwrap();
        assert_eq!(ep.tasks_api, GOOGLE_TASKS_API);
        assert_eq!(ep.meet_api, GOOGLE_MEET_API);
        assert_eq!(ep.people_api, "http://127.0.0.1:8081");
    }

    #[test]
    fn endpoints_saved_before_meet_api_still_deserialize() {
        let raw = r#"{"chat_api":"http://127.0.0.1:8081","oauth_auth":"","oauth_token":"",
            "userinfo":"","relay_ws":"","people_api":"http://127.0.0.1:8081",
            "tasks_api":"http://127.0.0.1:8081","client_id":"prism-dev"}"#;
        let ep: Endpoints = serde_json::from_str(raw).unwrap();
        assert_eq!(ep.tasks_api, "http://127.0.0.1:8081");
        assert_eq!(ep.meet_api, GOOGLE_MEET_API);
        assert!(SCOPES.contains(&"https://www.googleapis.com/auth/meetings.space.created"));
        assert!(SCOPES.contains(&"https://www.googleapis.com/auth/meetings.space.readonly"));
    }

    #[test]
    fn endpoints_saved_before_calendar_and_drive_still_deserialize() {
        let raw = r#"{"chat_api":"http://127.0.0.1:8081","oauth_auth":"","oauth_token":"",
            "userinfo":"","relay_ws":"","people_api":"http://127.0.0.1:8081",
            "tasks_api":"http://127.0.0.1:8081","meet_api":"http://127.0.0.1:8081","client_id":"prism-dev"}"#;
        let ep: Endpoints = serde_json::from_str(raw).unwrap();
        assert_eq!(ep.calendar_api, GOOGLE_CALENDAR_API);
        assert_eq!(ep.drive_api, GOOGLE_DRIVE_API);
        assert!(SCOPES.contains(&"https://www.googleapis.com/auth/calendar.events"));
        assert!(SCOPES.contains(&"https://www.googleapis.com/auth/calendar.readonly"));
        assert!(SCOPES.contains(&"https://www.googleapis.com/auth/drive.appdata"));
        assert!(SCOPES.contains(&"https://www.googleapis.com/auth/chat.users.availability"));
    }

    #[test]
    fn drive_upload_base_is_derived_from_the_api_base() {
        assert_eq!(
            drive_upload_api(GOOGLE_DRIVE_API),
            "https://www.googleapis.com/upload/drive/v3"
        );
        assert_eq!(
            drive_upload_api("http://127.0.0.1:8081/drive/v3/"),
            "http://127.0.0.1:8081/upload/drive/v3"
        );
        assert_eq!(
            drive_upload_api("http://127.0.0.1:8081"),
            "http://127.0.0.1:8081/upload/drive/v3"
        );
    }
}
