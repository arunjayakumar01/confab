//! Types that cross the shell ↔ web view bridge and are not in `confab_proto`:
//! `AuthStatus`, `Page`, `ShellEvent`. JSON shapes mirror
//! `apps/desktop/src/bridge-types.ts` (camelCase, tagged by `type`).

use base64::Engine;
use confab_proto::chat::{Attachment, Message, User, UserInfo};
use confab_proto::Endpoints;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub signed_in: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<User>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RelayState {
    Connecting,
    Open,
    #[default]
    Closed,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutboxStatus {
    Sent,
    Failed,
}

/// Payload of the `shell` event. `relay_state` on socket transitions,
/// `outbox` when Google accepts or rejects an optimistic send, `account` when
/// the active account changes (the web view reboots its workspace).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // note: Outbox carries a full Message; boxing would ripple into the web view types
pub enum ShellEvent {
    RelayState {
        state: RelayState,
    },
    #[serde(rename_all = "camelCase")]
    Outbox {
        client_id: String,
        status: OutboxStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<Message>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Account {
        /// The profile directory name, as `accounts_list` reports it.
        id: String,
    },
    /// The user closed the Meet mini window.
    HuddleWindowClosed,
}

/// The Chat `User` for the signed-in account, built from OpenID userinfo.
pub fn user_from_info(info: &UserInfo) -> User {
    User {
        name: format!("users/{}", info.sub),
        display_name: info.name.clone(),
        domain_id: None,
        user_type: Some("HUMAN".into()),
        is_anonymous: None,
        // note: `picture` from userinfo stands in for People API avatars.
        avatar_url: info.picture.clone(),
        email: info.email.clone(),
    }
}

/// RFC 3339 with milliseconds and a `Z` suffix, the shape Google emits.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// `spaces/{id}` of any resource name under a space.
pub fn space_of(name: &str) -> String {
    name.splitn(3, '/').take(2).collect::<Vec<_>>().join("/")
}

/// Full thread name: accepts `spaces/{s}/threads/{t}` or a bare `{t}`.
pub fn thread_name(space: &str, thread: &str) -> String {
    if thread.starts_with("spaces/") {
        thread.to_string()
    } else {
        format!("{space}/threads/{thread}")
    }
}

// ---------------------------------------------------------------------------
// Attachment media proxy (`prism-media` scheme, served by `media.rs`)
// ---------------------------------------------------------------------------

/// An `<img>` cannot send a Bearer header, so attachment URLs handed to the web
/// view point at this scheme; the shell fetches upstream with the signed-in token.
pub const MEDIA_SCHEME: &str = "prism-media";

fn b64url() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
}

/// The scheme URL for an upstream media URL, in the shape Tauri documents per
/// platform: `prism-media://localhost/?u=` on macOS and Linux,
/// `http://prism-media.localhost/?u=` on Windows. `u` is base64url of the upstream.
pub fn media_scheme_url(upstream: &str) -> String {
    let u = b64url().encode(upstream.as_bytes());
    #[cfg(windows)]
    {
        format!("http://{MEDIA_SCHEME}.localhost/?u={u}")
    }
    #[cfg(not(windows))]
    {
        format!("{MEDIA_SCHEME}://localhost/?u={u}")
    }
}

fn is_media_scheme_url(url: &str) -> bool {
    url.starts_with(&format!("{MEDIA_SCHEME}://"))
        || url.starts_with(&format!("http://{MEDIA_SCHEME}."))
}

/// The upstream URL a `prism-media` request carries in `u`, when well-formed.
pub fn media_upstream_of(request_url: &str) -> Option<String> {
    let url = url::Url::parse(request_url).ok()?;
    let u = url
        .query_pairs()
        .find(|(k, _)| k == "u")
        .map(|(_, v)| v.into_owned())?;
    let bytes = b64url().decode(u.as_bytes()).ok()?;
    String::from_utf8(bytes).ok()
}

/// The upstream a web-view URL stands for: decoded from `u` when it is a
/// `prism-media` scheme URL, otherwise the URL itself. `None` for a malformed scheme URL.
pub fn media_upstream_or_self(url: &str) -> Option<String> {
    if is_media_scheme_url(url) {
        media_upstream_of(url)
    } else {
        Some(url.to_string())
    }
}

/// `{chat_api}/v1/media/{resourceName}?alt=media`, the only way to fetch attachment bytes from real Google.
pub fn media_api_url(chat_api: &str, resource_name: &str) -> String {
    format!(
        "{}/v1/media/{resource_name}?alt=media",
        chat_api.trim_end_matches('/')
    )
}

/// Origins the proxy may fetch from: the chat API and, in twin mode, the twin.
pub fn media_origins(endpoints: &Endpoints) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        if let Ok(u) = url::Url::parse(s) {
            let o = u.origin().ascii_serialization();
            if !out.contains(&o) {
                out.push(o);
            }
        }
    };
    push(&endpoints.chat_api);
    // Drive file bytes for the Drive view (www.googleapis.com on Google).
    push(&endpoints.drive_api);
    if !endpoints.chat_api.contains("googleapis.com") {
        // note: the twin builds media URLs on PRISM_TWIN_BASE, which Endpoints only exposes folded into
        // chat_api; re-read it here so a PRISM_CHAT_API override does not strand the twin's own URLs.
        push(&std::env::var("PRISM_TWIN_BASE").unwrap_or_else(|_| "http://127.0.0.1:8081".into()));
    }
    out
}

/// True when `upstream` is an http(s) URL on one of `origins` (scheme, host and port must all match).
pub fn upstream_allowed(upstream: &str, origins: &[String]) -> bool {
    match url::Url::parse(upstream) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => {
            origins.contains(&u.origin().ascii_serialization())
        }
        _ => false,
    }
}

/// Points every attachment's `downloadUri` and `thumbnailUri` at the `prism-media`
/// proxy. Real Google: both become the media API URL built from
/// `attachmentDataRef.resourceName` (the URIs Google returns are browser links).
/// Twin: its own URIs, as given.
pub fn proxy_media(msgs: &mut [Message], endpoints: &Endpoints) {
    for m in msgs {
        for a in &mut m.attachment {
            proxy_attachment(a, endpoints);
        }
    }
}

/// `proxy_media` for one attachment, also the result of `GET /v1/{attachment name}`.
pub fn proxy_attachment(a: &mut Attachment, endpoints: &Endpoints) {
    if endpoints.chat_api.contains("googleapis.com") {
        // note: attachments without a resourceName (Drive files) keep their browser links;
        // the web view opens those externally rather than inline.
        let Some(resource) = a
            .attachment_data_ref
            .as_ref()
            .and_then(|r| r.resource_name.as_deref())
        else {
            return;
        };
        let proxied = media_scheme_url(&media_api_url(&endpoints.chat_api, resource));
        a.download_uri = Some(proxied.clone());
        a.thumbnail_uri = Some(proxied);
    } else {
        for u in [&mut a.download_uri, &mut a.thumbnail_uri]
            .into_iter()
            .flatten()
        {
            if !is_media_scheme_url(u) {
                *u = media_scheme_url(u);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use confab_proto::chat::AttachmentDataRef;

    #[test]
    fn shell_event_json_matches_bridge_types() {
        let e = ShellEvent::Outbox {
            client_id: "abc".into(),
            status: OutboxStatus::Sent,
            message: Some(Message {
                name: "spaces/A/messages/B".into(),
                ..Default::default()
            }),
            error: None,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "outbox");
        assert_eq!(v["clientId"], "abc");
        assert_eq!(v["status"], "sent");
        assert_eq!(v["message"]["name"], "spaces/A/messages/B");
        assert!(v.get("error").is_none());

        let s = serde_json::to_string(&ShellEvent::RelayState {
            state: RelayState::Open,
        })
        .unwrap();
        assert_eq!(s, r#"{"type":"relay_state","state":"open"}"#);
        let s = serde_json::to_string(&ShellEvent::Account { id: "two".into() }).unwrap();
        assert_eq!(s, r#"{"type":"account","id":"two"}"#);
        assert_eq!(RelayState::default(), RelayState::Closed);

        let a = serde_json::to_value(AuthStatus {
            signed_in: false,
            user: None,
        })
        .unwrap();
        assert_eq!(a, serde_json::json!({ "signedIn": false }));
    }

    fn endpoints(chat_api: &str) -> Endpoints {
        Endpoints {
            chat_api: chat_api.into(),
            oauth_auth: String::new(),
            oauth_token: String::new(),
            userinfo: String::new(),
            relay_ws: String::new(),
            people_api: chat_api.into(),
            tasks_api: chat_api.into(),
            meet_api: chat_api.into(),
            calendar_api: format!("{chat_api}/calendar/v3"),
            drive_api: format!("{chat_api}/drive/v3"),
            client_id: String::new(),
            client_secret: None,
        }
    }

    fn with_attachment(resource: &str, uri: Option<&str>) -> Message {
        Message {
            name: "spaces/A/messages/B".into(),
            attachment: vec![Attachment {
                content_type: Some("image/png".into()),
                attachment_data_ref: Some(AttachmentDataRef {
                    resource_name: Some(resource.into()),
                    attachment_upload_token: None,
                }),
                download_uri: uri.map(str::to_string),
                thumbnail_uri: uri.map(str::to_string),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn assert_scheme_shape(url: &str) {
        #[cfg(windows)]
        assert!(url.starts_with("http://prism-media.localhost/?u="), "{url}");
        #[cfg(not(windows))]
        assert!(url.starts_with("prism-media://localhost/?u="), "{url}");
    }

    #[test]
    fn proxy_media_twin_wraps_the_twins_own_urls() {
        let upstream = "http://127.0.0.1:8081/v1/media/spaces/A/attachments/x?alt=media";
        let mut msgs = vec![with_attachment("spaces/A/attachments/x", Some(upstream))];
        proxy_media(&mut msgs, &endpoints("http://127.0.0.1:8081"));
        let a = &msgs[0].attachment[0];
        let url = a.download_uri.clone().unwrap();
        assert_scheme_shape(&url);
        assert_eq!(Some(url.as_str()), a.thumbnail_uri.as_deref());
        assert_eq!(media_upstream_of(&url).as_deref(), Some(upstream));
        assert!(!url.contains("access_token"));
        // Idempotent: a second pass does not wrap the scheme URL again.
        proxy_media(&mut msgs, &endpoints("http://127.0.0.1:8081"));
        assert_eq!(
            msgs[0].attachment[0].download_uri.as_deref(),
            Some(url.as_str())
        );
    }

    #[test]
    fn proxy_media_real_google_uses_the_media_api_from_resource_name() {
        let browser_link = "https://chat.google.com/api/get_attachment_url?x=1";
        let mut msgs = vec![with_attachment(
            "spaces/A/messages/B/attachments/C",
            Some(browser_link),
        )];
        proxy_media(&mut msgs, &endpoints("https://chat.googleapis.com"));
        let a = &msgs[0].attachment[0];
        let url = a.download_uri.as_deref().unwrap();
        assert_scheme_shape(url);
        assert_eq!(url, a.thumbnail_uri.as_deref().unwrap());
        assert_eq!(
            media_upstream_of(url).as_deref(),
            Some(
                "https://chat.googleapis.com/v1/media/spaces/A/messages/B/attachments/C?alt=media"
            )
        );
        // No resourceName (a Drive file): the browser links stay.
        let mut drive = vec![with_attachment("", Some(browser_link))];
        drive[0].attachment[0].attachment_data_ref = None;
        proxy_media(&mut drive, &endpoints("https://chat.googleapis.com"));
        assert_eq!(
            drive[0].attachment[0].download_uri.as_deref(),
            Some(browser_link)
        );
    }

    #[test]
    fn media_host_allow_list() {
        let twin = media_origins(&endpoints("http://127.0.0.1:8081"));
        assert!(upstream_allowed(
            "http://127.0.0.1:8081/v1/media/x?alt=media",
            &twin
        ));
        assert!(!upstream_allowed("https://evil.example/v1/media/x", &twin));
        assert!(!upstream_allowed("http://127.0.0.1:9999/v1/media/x", &twin));
        assert!(!upstream_allowed("file:///etc/passwd", &twin));
        let real = media_origins(&endpoints("https://chat.googleapis.com"));
        assert_eq!(real, vec!["https://chat.googleapis.com".to_string()]);
        assert!(upstream_allowed(
            "https://chat.googleapis.com/v1/media/y?alt=media",
            &real
        ));
        assert!(!upstream_allowed(
            "http://chat.googleapis.com/v1/media/y",
            &real
        ));
        assert!(!upstream_allowed("https://evil.example", &real));
        assert!(!upstream_allowed(
            "https://chat.googleapis.com.evil.example/x",
            &real
        ));
    }

    #[test]
    fn media_upstream_or_self_decodes_only_the_scheme() {
        let upstream = "http://127.0.0.1:8081/v1/media/spaces/A/attachments/x?alt=media";
        let wrapped = media_scheme_url(upstream);
        assert_eq!(media_upstream_or_self(&wrapped).as_deref(), Some(upstream));
        assert_eq!(media_upstream_or_self(upstream).as_deref(), Some(upstream));
        assert_eq!(
            media_upstream_or_self("prism-media://localhost/?u=%%%"),
            None
        );
    }

    #[test]
    fn media_request_decoding() {
        assert_eq!(
            media_upstream_of("prism-media://localhost/?u=aHR0cDovL3gvYQ").as_deref(),
            Some("http://x/a")
        );
        assert_eq!(
            media_upstream_of("http://prism-media.localhost/?u=aHR0cDovL3gvYQ").as_deref(),
            Some("http://x/a")
        );
        assert_eq!(media_upstream_of("prism-media://localhost/"), None);
        assert_eq!(media_upstream_of("prism-media://localhost/?u=%%%"), None);
    }

    #[test]
    fn name_helpers() {
        assert_eq!(space_of("spaces/A/messages/B/reactions/C"), "spaces/A");
        assert_eq!(space_of("spaces/A"), "spaces/A");
        assert_eq!(thread_name("spaces/A", "T"), "spaces/A/threads/T");
        assert_eq!(
            thread_name("spaces/A", "spaces/A/threads/T"),
            "spaces/A/threads/T"
        );
    }
}
