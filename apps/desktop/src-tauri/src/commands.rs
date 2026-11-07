//! Tauri commands, one per row of the table in docs/ARCHITECTURE.md. JS sends camelCase keys;
//! Tauri maps them onto the snake_case parameters here.

use crate::accounts::{Account, Accounts};
use crate::cache::{OutboxRow, OutboxState};
use crate::outbox;
use crate::shell::{
    media_origins, media_upstream_or_self, now_rfc3339, proxy_attachment, proxy_media, thread_name,
    upstream_allowed, user_from_info, AuthStatus, Page,
};
use crate::AppState;
use base64::Engine;
use prism_proto::availability::Availability;
use prism_proto::calendar::{CalEvent, CalendarEntry, EventInput, EventsPage};
use prism_proto::chat::{Attachment, Membership, Message, Space, Thread, User};
use prism_proto::meet::{MeetSpace, MeetStatus};
use prism_proto::relay::ClientMsg;
use prism_proto::tasks::Task;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

/// The accounts; every command acts on the active one (`state.active()`).
type S<'a> = State<'a, Arc<Accounts>>;

#[tauri::command]
pub async fn auth_status(state: S<'_>) -> Result<AuthStatus, String> {
    let st = state.active();
    st.auth.write_account_file();
    Ok(st.auth.status())
}

#[tauri::command]
pub async fn auth_start(app: AppHandle, state: S<'_>) -> Result<AuthStatus, String> {
    let st = state.active();
    let status = st.auth.start().await?;
    if let Some(u) = status.user.as_ref() {
        if st.cache.ensure_owner(&u.name)? {
            tracing::info!("cache reset for {}", u.name);
        }
    }
    st.relay.start(app.clone(), st.clone());
    outbox::resume_unsent(app, st.clone());
    Ok(status)
}

#[tauri::command]
pub async fn sign_out(state: S<'_>) -> Result<(), String> {
    let st = state.active();
    st.relay.stop();
    st.auth.sign_out()
}

#[tauri::command]
pub async fn list_spaces(state: S<'_>) -> Result<Vec<Space>, String> {
    let st = state.active();
    let spaces = st.google.list_spaces().await?;
    st.cache.upsert_spaces(&spaces)?;
    if st.relay.is_running() {
        let _ = st.relay.send(ClientMsg::Subscribe {
            spaces: spaces.iter().map(|s| s.name.clone()).collect(),
        });
    }
    Ok(spaces)
}

#[tauri::command]
pub async fn search_people(query: String, state: S<'_>) -> Result<Vec<User>, String> {
    let st = state.active();
    st.google.search_people(&query).await.map_err(|e| {
        tracing::warn!("search_people({query:?}): {e}");
        e.to_string()
    })
}

#[tauri::command]
pub async fn setup_dm(user: String, state: S<'_>) -> Result<Space, String> {
    let st = state.active();
    let space = st
        .google
        .setup_space("DIRECT_MESSAGE", None, std::slice::from_ref(&user))
        .await?;
    remember_space(&st, &space);
    Ok(space)
}

/// A group chat with the caller and `members` (`users/{id}`, two or more), no display name.
#[tauri::command]
pub async fn setup_group(members: Vec<String>, state: S<'_>) -> Result<Space, String> {
    let st = state.active();
    let space = st.google.setup_space("GROUP_CHAT", None, &members).await?;
    remember_space(&st, &space);
    Ok(space)
}

#[tauri::command]
pub async fn create_space(
    display_name: String,
    members: Vec<String>,
    state: S<'_>,
) -> Result<Space, String> {
    let st = state.active();
    let space = st
        .google
        .setup_space("SPACE", Some(&display_name), &members)
        .await?;
    remember_space(&st, &space);
    Ok(space)
}

/// Cache a space the user just created and re-subscribe the relay with the full set
/// (a subscribe frame replaces the session's space list).
fn remember_space(st: &AppState, space: &Space) {
    if let Err(e) = st.cache.upsert_spaces(std::slice::from_ref(space)) {
        tracing::warn!("{e}");
    }
    if st.relay.is_running() {
        let _ = st.relay.send(ClientMsg::Subscribe {
            spaces: st.cache.space_names().unwrap_or_default(),
        });
    }
}

#[tauri::command]
pub async fn list_members(space: String, state: S<'_>) -> Result<Vec<Membership>, String> {
    let st = state.active();
    let members = st.google.list_members(&space).await?;
    st.cache.upsert_members(&space, &members)?;
    Ok(members)
}

#[tauri::command]
pub async fn list_messages(
    space: String,
    page_token: Option<String>,
    thread: Option<String>,
    page_size: Option<u32>,
    state: S<'_>,
) -> Result<Page<Message>, String> {
    let st = state.active();
    let filter = thread
        .filter(|t| !t.is_empty())
        .map(|t| format!("thread.name = \"{}\"", thread_name(&space, &t)));
    let size = page_size.unwrap_or(100).clamp(1, 1000);
    let r = st
        .google
        .list_messages(&space, page_token.as_deref(), filter.as_deref(), size)
        .await?;
    st.cache.upsert_messages(&r.messages)?;
    let mut items: Vec<Message> = r
        .messages
        .into_iter()
        .filter(|m| m.delete_time.is_none())
        .collect();
    items.sort_by(|a, b| b.create_time.cmp(&a.create_time));
    proxy_media(&mut items, &st.profile.endpoints);
    Ok(Page {
        items,
        next_page_token: r.next_page_token.filter(|t| !t.is_empty()),
    })
}

fn normalize_client_id(given: Option<String>) -> String {
    let id = given
        .map(|s| s.trim().trim_start_matches("client-").to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    // Google: lowercase letters, digits and hyphens only.
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(63)
        .collect()
}

/// Returns the optimistic message at once; a `shell` `outbox` event follows.
/// `attachments` (from `upload_attachment`) is an optional extension of the contract.
#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    space: String,
    text: String,
    thread: Option<String>,
    client_id: Option<String>,
    attachments: Option<Vec<Attachment>>,
    state: S<'_>,
) -> Result<Message, String> {
    let st = state.active();
    let me = st.auth.user().ok_or("not signed in")?;
    let id = normalize_client_id(client_id);
    let thread = thread
        .filter(|t| !t.is_empty())
        .map(|t| thread_name(&space, &t));
    let row = OutboxRow {
        client_id: id.clone(),
        space: space.clone(),
        thread: thread.clone(),
        text: text.clone(),
        attachments: attachments.unwrap_or_default(),
        attempts: 0,
        state: OutboxState::Pending,
        error: None,
    };
    st.cache.outbox_insert(&row)?;
    let optimistic = Message {
        name: format!("{space}/messages/client-{id}"),
        sender: Some(user_from_info(&me)),
        create_time: Some(now_rfc3339()),
        text: Some(text),
        thread_reply: thread.as_ref().map(|_| true),
        thread: thread.map(|name| Thread {
            name: Some(name),
            thread_key: None,
        }),
        attachment: row.attachments.clone(),
        client_assigned_message_id: Some(format!("client-{id}")),
        ..Default::default()
    };
    outbox::spawn_deliver(app, st.clone(), id);
    Ok(optimistic)
}

#[tauri::command]
pub async fn edit_message(name: String, text: String, state: S<'_>) -> Result<Message, String> {
    let st = state.active();
    let m = st.google.update_message(&name, &text).await?;
    st.cache.upsert_message(&m)?;
    Ok(m)
}

#[tauri::command]
/// `force` takes the thread's replies with a parent message (Google refuses the delete otherwise).
pub async fn delete_message(name: String, force: Option<bool>, state: S<'_>) -> Result<(), String> {
    let st = state.active();
    st.google
        .delete_message(&name, force.unwrap_or(false))
        .await?;
    st.cache.delete_message(&name)
}

#[tauri::command]
pub async fn toggle_reaction(
    message_name: String,
    unicode: String,
    state: S<'_>,
) -> Result<(), String> {
    let st = state.active();
    let me = st.auth.me().ok_or("not signed in")?;
    let mine = st
        .google
        .list_reactions(&message_name, Some(&format!("user.name = \"{me}\"")))
        .await?;
    let existing = mine
        .iter()
        .find(|r| r.emoji.unicode.as_deref() == Some(unicode.as_str()))
        .and_then(|r| r.name.clone());
    match existing {
        Some(reaction) => st.google.delete_reaction(&reaction).await?,
        None => {
            st.google.create_reaction(&message_name, &unicode).await?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn mark_read(
    space: String,
    message: String,
    thread: Option<String>,
    state: S<'_>,
) -> Result<(), String> {
    let st = state.active();
    // lastReadTime = the message's createTime when we have it, else now.
    let last_read = st
        .cache
        .get_message(&message)?
        .and_then(|m| m.create_time)
        .unwrap_or_else(now_rfc3339);
    st.google.update_read_state(&space, &last_read).await?;
    let thread = thread
        .filter(|t| !t.is_empty())
        .map(|t| thread_name(&space, &t));
    // note: receipts are best-effort; a down relay must not fail the read.
    if let Err(e) = st.relay.send(ClientMsg::Read {
        space,
        thread,
        message,
    }) {
        tracing::debug!("mark_read: {e}");
    }
    Ok(())
}

#[tauri::command]
pub async fn search(query: String, state: S<'_>) -> Result<Vec<Message>, String> {
    let st = state.active();
    let mut hits = st.cache.search(&query)?;
    proxy_media(&mut hits, &st.profile.endpoints);
    Ok(hits)
}

#[tauri::command]
pub async fn upload_attachment(
    space: String,
    path: String,
    state: S<'_>,
) -> Result<Attachment, String> {
    let st = state.active();
    let p = std::path::Path::new(&path);
    let r = st.google.upload_attachment(&space, p).await?;
    Ok(Attachment {
        content_name: p.file_name().map(|n| n.to_string_lossy().to_string()),
        content_type: Some(mime_guess::from_path(p).first_or_octet_stream().to_string()),
        attachment_data_ref: r.attachment_data_ref,
        source: Some("UPLOADED_CONTENT".into()),
        ..Default::default()
    })
}

/// Upload straight from the web view. The raw IPC body is the file; the headers name it:
/// `x-prism-space` (space name), `x-prism-name` (filename, base64 standard of UTF-8),
/// `x-prism-mime` (optional, default `application/octet-stream`). Same `Attachment`
/// shape as `upload_attachment`.
#[tauri::command]
pub async fn upload_attachment_bytes(
    request: Request<'_>,
    state: S<'_>,
) -> Result<Attachment, String> {
    let st = state.active();
    let bytes = match request.body() {
        InvokeBody::Raw(b) => b.clone(),
        InvokeBody::Json(_) => {
            return Err("upload_attachment_bytes expects the file bytes as a raw body".into())
        }
    };
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let space = header("x-prism-space").ok_or("x-prism-space header is required")?;
    let encoded = header("x-prism-name").ok_or("x-prism-name header is required")?;
    let name = base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .ok_or("x-prism-name must be the base64 of a UTF-8 filename")?;
    let mime = header("x-prism-mime").unwrap_or_else(|| "application/octet-stream".into());
    let r = st.google.upload_bytes(&space, &name, &mime, bytes).await?;
    Ok(Attachment {
        content_name: Some(name),
        content_type: Some(mime),
        attachment_data_ref: r.attachment_data_ref,
        source: Some("UPLOADED_CONTENT".into()),
        ..Default::default()
    })
}

/// The full attachment resource. A message list carries a Drive attachment with only
/// `name` and `source`; this fills in `driveDataRef` and the rest.
#[tauri::command]
pub async fn get_attachment(name: String, state: S<'_>) -> Result<Attachment, String> {
    let st = state.active();
    let mut a = st.google.get_attachment(&name).await?;
    proxy_attachment(&mut a, &st.profile.endpoints);
    Ok(a)
}

/// Opens an http(s) link in the system browser; any other scheme is refused.
#[tauri::command]
pub async fn open_url(url: String, account: Option<String>, app: AppHandle) -> Result<(), String> {
    let allowed = url::Url::parse(&url)
        .map(|u| matches!(u.scheme(), "http" | "https"))
        .unwrap_or(false);
    if !allowed {
        return Err(format!("refusing to open {url}: only http and https links"));
    }
    // A Google link for the signed-in account: prefer the Chrome profile that is signed in as that account,
    // so a Meet does not land in another Google identity's window. Chrome's own profile list is read locally.
    if let Some(acct) = account.filter(|a| !a.is_empty()) {
        if let Some(dir) = chrome_profile_for(&acct) {
            if open_in_chrome(&dir, &[&url]) {
                tracing::debug!("open_url: {url} in Chrome profile {dir}");
                return Ok(());
            }
        }
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

/// Opens the real Meet in a small window: Chrome's app window in the profile already signed in as `account`, so
/// nobody signs in again, or Confab's own always-on-top webview where Chrome has no such profile.
#[tauri::command]
pub async fn huddle_window_open(
    url: String,
    title: String,
    account: Option<String>,
    app: AppHandle,
    accounts: S<'_>,
) -> Result<(), String> {
    if let Some(dir) = account.as_deref().and_then(chrome_profile_for) {
        if crate::huddle_window::open_chrome(&app, &dir, &url)? {
            return Ok(());
        }
    }
    crate::huddle_window::open(&app, &url, &title, &accounts.active_id())
}

#[tauri::command]
pub async fn huddle_window_close(app: AppHandle) -> Result<(), String> {
    crate::huddle_window::close(&app);
    Ok(())
}

/// The Chrome profile directory whose signed-in user is `email`, from Chrome's Local State; macOS only.
fn chrome_profile_for(email: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").ok()?;
        let base = std::path::PathBuf::from(home).join("Library/Application Support/Google/Chrome");
        let text = std::fs::read_to_string(base.join("Local State")).ok()?;
        chrome_profile_in(&text, email).or_else(|| {
            // Not any profile's main account: it may be added inside one, which is as good with `authuser`.
            let v: serde_json::Value = serde_json::from_str(&text).ok()?;
            let cache = v.get("profile")?.get("info_cache")?.as_object()?;
            cache
                .keys()
                .find(|dir| {
                    std::fs::read_to_string(base.join(dir).join("Preferences"))
                        .is_ok_and(|p| chrome_prefs_list(&p, email))
                })
                .cloned()
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = email;
        None
    }
}

/// Pure half of the lookup: `info_cache` maps profile directories to `{user_name, ...}`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // only macOS looks up Chrome profiles so far
fn chrome_profile_in(local_state: &str, email: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(local_state).ok()?;
    let cache = v.get("profile")?.get("info_cache")?.as_object()?;
    cache
        .iter()
        .find(|(_, info)| {
            info.get("user_name")
                .and_then(|u| u.as_str())
                .map(|u| u.eq_ignore_ascii_case(email))
                .unwrap_or(false)
        })
        .map(|(dir, _)| dir.clone())
}

/// A profile's `Preferences` list the Google accounts signed in to it under `account_info`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // only macOS looks up Chrome profiles so far
fn chrome_prefs_list(prefs: &str, email: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(prefs)
        .ok()
        .and_then(|v| {
            v.get("account_info")?.as_array().map(|a| {
                a.iter().any(|i| {
                    i.get("email")
                        .and_then(|e| e.as_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case(email))
                })
            })
        })
        .unwrap_or(false)
}

/// Hands the URL to Chrome in a given profile; Chrome forwards it to its running instance.
pub(crate) fn open_in_chrome(profile_dir: &str, args: &[&str]) -> bool {
    #[cfg(target_os = "macos")]
    {
        let bin = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
        if !std::path::Path::new(bin).exists() {
            return false;
        }
        std::process::Command::new(bin)
            .arg(format!("--profile-directory={profile_dir}"))
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (profile_dir, args);
        false
    }
}

/// A file name safe to join onto a directory: path separators and control characters
/// dropped, surrounding whitespace and dots trimmed, `file` when nothing is left.
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !matches!(c, '/' | '\\') && !c.is_control())
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.to_string()
    }
}

/// `name`, or `name (2).ext`, `name (3).ext`, ... for the first that `exists` denies.
/// The extension is the part after the last dot of a name that does not start with it.
fn dedupe_filename(name: &str, exists: impl Fn(&str) -> bool) -> String {
    if !exists(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|candidate| !exists(candidate))
        .expect("an unused name exists")
}

/// The directory downloads go to: the user's Downloads, else the app data dir.
fn download_dir(app: &AppHandle) -> Result<PathBuf, String> {
    match app.path().download_dir() {
        Ok(d) => Ok(d),
        Err(e) => {
            tracing::debug!("no download dir ({e}), using the app data dir");
            app.path().app_data_dir().map_err(|e| e.to_string())
        }
    }
}

/// Saves an attachment the web view cannot download itself (the `prism-media`
/// scheme has no download path): the upstream is resolved and allow-listed exactly
/// like `media.rs`, fetched with the signed-in token, written to the Downloads
/// folder under a sanitized, deduplicated `name`, then revealed in the file manager.
/// Returns the absolute path written.
#[tauri::command]
pub async fn save_attachment(
    url: String,
    name: String,
    app: AppHandle,
    state: S<'_>,
) -> Result<String, String> {
    let st = state.active();
    let upstream = media_upstream_or_self(&url).ok_or("malformed media URL")?;
    if !upstream_allowed(&upstream, &media_origins(&st.profile.endpoints)) {
        tracing::warn!("save_attachment: refused upstream {upstream}");
        return Err("upstream host not allowed".into());
    }
    let (_, bytes) = st.google.get_bytes(&upstream).await?;
    let dir = download_dir(&app)?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let file_name = dedupe_filename(&sanitize_filename(&name), |candidate| {
        Path::new(&dir).join(candidate).exists()
    });
    let path = dir.join(file_name);
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    tracing::debug!(
        "saved {upstream} to {} ({} bytes)",
        path.display(),
        bytes.len()
    );
    if let Err(e) = app.opener().reveal_item_in_dir(&path) {
        tracing::debug!("reveal {}: {e}", path.display());
    }
    Ok(path.to_string_lossy().into_owned())
}

/// Dock badge (macOS). `0` clears it; platforms without one log and return Ok.
#[tauri::command]
pub async fn set_badge(count: u32, window: tauri::WebviewWindow) -> Result<(), String> {
    let badge = if count == 0 { None } else { Some(count as i64) };
    if let Err(e) = window.set_badge_count(badge) {
        tracing::debug!("set_badge({count}): {e}");
    }
    Ok(())
}

/// Adds people (`users/{id}`) to a space one by one; the first failure stops and names the user.
#[tauri::command]
pub async fn add_members(
    space: String,
    users: Vec<String>,
    state: S<'_>,
) -> Result<Vec<Membership>, String> {
    let st = state.active();
    let mut out = Vec::with_capacity(users.len());
    for u in &users {
        let m = st
            .google
            .add_member(&space, u)
            .await
            .map_err(|e| format!("{u}: {e}"))?;
        out.push(m);
    }
    Ok(out)
}

/// Leaves a space: deletes the caller's own membership and forgets the space locally.
#[tauri::command]
pub async fn leave_space(space: String, state: S<'_>) -> Result<(), String> {
    let st = state.active();
    let me = st
        .auth
        .user()
        .and_then(|u| u.name)
        .ok_or_else(|| "not signed in".to_string())?;
    st.google.leave_space(&space, &me).await?;
    Ok(())
}

/// A link preview for a URL the user consented to; errors are short sentences for the chip.
#[tauri::command]
pub async fn unfurl(
    url: String,
    state: S<'_>,
) -> Result<Option<crate::unfurl::LinkPreview>, String> {
    let st = state.active();
    crate::unfurl::fetch_preview(&st.http, &url).await
}

/// Page zoom for the whole webview: `1.0` is 100%, clamped to what stays readable.
#[tauri::command]
pub async fn set_zoom(scale: f64, window: tauri::WebviewWindow) -> Result<(), String> {
    let scale = scale.clamp(0.5, 2.0);
    window.set_zoom(scale).map_err(|e| e.to_string())
}

// ----- Google Tasks (the signed-in user's default list) -----

#[tauri::command]
pub async fn list_tasks(state: S<'_>) -> Result<Vec<Task>, String> {
    let st = state.active();
    st.google.list_tasks().await.map_err(|e| {
        tracing::warn!("list_tasks: {e}");
        e.to_string()
    })
}

/// `due` is RFC 3339 (Google keeps the date part); blank `notes` and `due` are not sent.
#[tauri::command]
pub async fn add_task(
    title: String,
    notes: Option<String>,
    due: Option<String>,
    state: S<'_>,
) -> Result<Task, String> {
    let st = state.active();
    let title = title.trim();
    if title.is_empty() {
        return Err("a task needs a title".into());
    }
    st.google
        .add_task(title, notes.as_deref(), due.as_deref())
        .await
        .map_err(|e| {
            tracing::warn!("add_task({title:?}): {e}");
            e.to_string()
        })
}

#[tauri::command]
pub async fn set_task_done(id: String, done: bool, state: S<'_>) -> Result<Task, String> {
    let st = state.active();
    st.google.set_task_done(&id, done).await.map_err(|e| {
        tracing::warn!("set_task_done({id}, {done}): {e}");
        e.to_string()
    })
}

#[tauri::command]
pub async fn delete_task(id: String, state: S<'_>) -> Result<(), String> {
    let st = state.active();
    st.google.delete_task(&id).await.map_err(|e| {
        tracing::warn!("delete_task({id}): {e}");
        e.to_string()
    })
}

// ----- Google Meet (the room behind a huddle) -----

/// A fresh Meet space with `accessType: TRUSTED`: `{name, meetingUri, meetingCode}`.
#[tauri::command]
pub async fn create_meet(state: S<'_>) -> Result<MeetSpace, String> {
    let st = state.active();
    st.google.create_meet().await.map_err(|e| {
        tracing::warn!("create_meet: {e}");
        e.to_string()
    })
}

/// `code` is a meeting code (`abc-defg-hij`) or a `spaces/{id}` name. Active when the
/// space has a conference running; `participants` are the people still in it.
#[tauri::command]
pub async fn meet_status(code: String, state: S<'_>) -> Result<MeetStatus, String> {
    let st = state.active();
    st.google.meet_status(&code).await.map_err(|e| {
        tracing::warn!("meet_status({code}): {e}");
        e.to_string()
    })
}

/// Posts the huddle message Google itself would: no text, one `RICH_LINK` annotation
/// to the Meet space (`huddleStatus: STARTED`). Falls back to "Started a huddle" plus
/// the link when Google refuses the annotation. Returns the message as Google stored it.
#[tauri::command]
pub async fn post_huddle(
    space: String,
    meeting_uri: String,
    meeting_code: String,
    mention_all: Option<bool>,
    state: S<'_>,
) -> Result<Message, String> {
    let st = state.active();
    let meeting_uri = meeting_uri.trim();
    let meeting_code = meeting_code.trim();
    if meeting_uri.is_empty() || meeting_code.is_empty() {
        return Err("a huddle needs its meeting link and code".into());
    }
    let client_id = normalize_client_id(None);
    let mut message = st
        .google
        .create_huddle_message(
            &space,
            &client_id,
            meeting_uri,
            meeting_code,
            mention_all.unwrap_or(false),
        )
        .await
        .map_err(|e| {
            tracing::warn!("post_huddle({space}, {meeting_code}): {e}");
            e.to_string()
        })?;
    if let Err(e) = st.cache.upsert_message(&message) {
        tracing::warn!("{e}");
    }
    proxy_media(std::slice::from_mut(&mut message), &st.profile.endpoints);
    Ok(message)
}

// ----- Availability (Chat `users.availability`, the signed-in user) -----

/// `{name, state, customStatus?, doNotDisturbMetadata?}` for the signed-in user.
#[tauri::command]
pub async fn availability_get(state: S<'_>) -> Result<Availability, String> {
    let st = state.active();
    st.google.availability_get().await.map_err(|e| {
        tracing::warn!("availability_get: {e}");
        e.to_string()
    })
}

/// `state` is `active` (optional `ttlSeconds`, Google defaults to thirty minutes),
/// `away`, or `dnd` (`ttlSeconds`, an hour when omitted, at most a year).
#[tauri::command]
pub async fn availability_mark(
    state: String,
    ttl_seconds: Option<u64>,
    accounts: S<'_>,
) -> Result<Availability, String> {
    let st = accounts.active();
    st.google
        .availability_mark(&state, ttl_seconds)
        .await
        .map_err(|e| {
            tracing::warn!("availability_mark({state}, {ttl_seconds:?}): {e}");
            e.to_string()
        })
}

/// Sets the custom status (`text` up to 64 characters, `emoji` defaults to a speech
/// balloon, `ttlSeconds` optional); blank or missing `text` clears it.
#[tauri::command]
pub async fn availability_status(
    text: Option<String>,
    emoji: Option<String>,
    ttl_seconds: Option<u64>,
    state: S<'_>,
) -> Result<Availability, String> {
    let st = state.active();
    st.google
        .availability_status(text.as_deref(), emoji.as_deref(), ttl_seconds)
        .await
        .map_err(|e| {
            tracing::warn!("availability_status: {e}");
            e.to_string()
        })
}

// ----- Google Calendar (the signed-in user's calendars) -----

/// Every calendar the user can at least read: `{id, summary, primary, backgroundColor?, accessRole?}`.
#[tauri::command]
pub async fn calendar_list(state: S<'_>) -> Result<Vec<CalendarEntry>, String> {
    let st = state.active();
    st.google.calendar_list().await.map_err(|e| {
        tracing::warn!("calendar_list: {e}");
        e.to_string()
    })
}

/// Single instances of one calendar's events: a window (`timeMin`/`timeMax`, RFC 3339,
/// ordered by start) or, with `syncToken`, only what changed since (cancelled ones
/// carry `status: "cancelled"`). `nextSyncToken` feeds the next call. An expired token
/// is an error starting with `sync_expired`: refetch the window without a token.
#[tauri::command]
pub async fn calendar_events(
    calendar_id: String,
    time_min: Option<String>,
    time_max: Option<String>,
    sync_token: Option<String>,
    state: S<'_>,
) -> Result<EventsPage, String> {
    let st = state.active();
    st.google
        .calendar_events(
            &calendar_id,
            time_min.as_deref(),
            time_max.as_deref(),
            sync_token.as_deref(),
        )
        .await
        .map_err(|e| {
            if e.status != 410 {
                tracing::warn!("calendar_events({calendar_id}): {e}");
            }
            e.to_string()
        })
}

/// Creates an event (invitations sent); `event.addMeet` attaches a Meet conference.
#[tauri::command]
pub async fn calendar_create_event(
    calendar_id: String,
    event: EventInput,
    state: S<'_>,
) -> Result<CalEvent, String> {
    let st = state.active();
    if event.summary.trim().is_empty() {
        return Err("an event needs a title".into());
    }
    st.google
        .calendar_create_event(&calendar_id, &event)
        .await
        .map_err(|e| {
            tracing::warn!("calendar_create_event({calendar_id}): {e}");
            e.to_string()
        })
}

/// Patches an event with the input's fields (updates sent); `event.addMeet` on an
/// event without a conference attaches one.
#[tauri::command]
pub async fn calendar_update_event(
    calendar_id: String,
    event_id: String,
    event: EventInput,
    state: S<'_>,
) -> Result<CalEvent, String> {
    let st = state.active();
    if event.summary.trim().is_empty() {
        return Err("an event needs a title".into());
    }
    st.google
        .calendar_update_event(&calendar_id, &event_id, &event)
        .await
        .map_err(|e| {
            tracing::warn!("calendar_update_event({calendar_id}, {event_id}): {e}");
            e.to_string()
        })
}

/// RSVP as the signed-in attendee: `response` is `accepted`, `declined` or `tentative`.
#[tauri::command]
pub async fn calendar_respond(
    calendar_id: String,
    event_id: String,
    response: String,
    state: S<'_>,
) -> Result<CalEvent, String> {
    let st = state.active();
    st.google
        .calendar_respond(&calendar_id, &event_id, &response)
        .await
        .map_err(|e| {
            tracing::warn!("calendar_respond({calendar_id}, {event_id}, {response}): {e}");
            e.to_string()
        })
}

#[tauri::command]
pub async fn calendar_delete_event(
    calendar_id: String,
    event_id: String,
    state: S<'_>,
) -> Result<(), String> {
    let st = state.active();
    st.google
        .calendar_delete_event(&calendar_id, &event_id)
        .await
        .map_err(|e| {
            tracing::warn!("calendar_delete_event({calendar_id}, {event_id}): {e}");
            e.to_string()
        })
}

// ----- Notes (one JSON document in the Drive app-data folder) -----

/// The saved notes JSON as text, `null` before the first save.
#[tauri::command]
pub async fn notes_load(state: S<'_>) -> Result<Option<String>, String> {
    let st = state.active();
    st.google.notes_load().await.map_err(|e| {
        tracing::warn!("notes_load: {e}");
        e.to_string()
    })
}

/// Stores `json` as `prism-notes.json` in the app-data folder (created on first save).
#[tauri::command]
pub async fn notes_save(json: String, state: S<'_>) -> Result<(), String> {
    let st = state.active();
    st.google.notes_save(&json).await.map_err(|e| {
        tracing::warn!("notes_save: {e}");
        e.to_string()
    })
}

/// A native notification. A `cargo tauri dev` build is not an app bundle, so macOS
/// shows these as from Terminal and asks permission once.
#[tauri::command]
pub async fn notify(title: String, body: String, app: AppHandle) -> Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn relay_send(msg: ClientMsg, state: S<'_>) -> Result<(), String> {
    let st = state.active();
    st.relay.send(msg)
}

// ----- Accounts (Google accounts side by side) -----

/// Every account, the active one first, then by id.
#[tauri::command]
pub async fn accounts_list(accounts: S<'_>) -> Result<Vec<Account>, String> {
    Ok(accounts.list())
}

/// Makes `id` the active account and clears its unread count. The web view gets a
/// `shell` event `{type: "account", id}` to reboot its workspace, then the account's
/// current `relay_state`.
#[tauri::command]
pub async fn account_switch(
    id: String,
    app: AppHandle,
    accounts: S<'_>,
) -> Result<Account, String> {
    let account = accounts.switch(&id)?;
    announce_active(&app, &accounts);
    Ok(account)
}

/// A fresh signed-out profile (`acct-<n>`), made active like `account_switch`.
#[tauri::command]
pub async fn account_add(app: AppHandle, accounts: S<'_>) -> Result<Account, String> {
    let account = accounts.add()?;
    announce_active(&app, &accounts);
    Ok(account)
}

/// Signs the account out, drops it and deletes its profile directory; refused for the active one.
#[tauri::command]
pub async fn account_remove(id: String, accounts: S<'_>) -> Result<(), String> {
    accounts.remove(&id)
}

fn announce_active(app: &AppHandle, accounts: &Accounts) {
    let st = accounts.active();
    crate::accounts::emit_shell(
        app,
        &st,
        &crate::shell::ShellEvent::Account {
            id: accounts.active_id(),
        },
    );
    crate::accounts::emit_shell(
        app,
        &st,
        &crate::shell::ShellEvent::RelayState {
            state: st.relay.last_state(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_profile_lookup_matches_the_signed_in_email() {
        let state = r#"{"profile":{"info_cache":{"Default":{"user_name":"me@gmail.test","name":"Me"},"Profile 2":{"user_name":"arun@example.test","name":"Work"},"Profile 3":{"name":"Guest"}}}}"#;
        assert_eq!(
            chrome_profile_in(state, "arun@example.test").as_deref(),
            Some("Profile 2")
        );
        assert_eq!(chrome_profile_in(state, "nobody@example.test"), None);
        assert_eq!(chrome_profile_in("not json", "x"), None);
        let prefs = r#"{"account_info":[{"email":"a@x.test"},{"email":"arun@example.test"}]}"#;
        assert!(chrome_prefs_list(prefs, "arun@example.test"));
        assert!(!chrome_prefs_list(prefs, "nobody@example.test"));
        assert!(!chrome_prefs_list("{}", "a@x.test"));
    }

    #[test]
    fn client_ids_are_google_safe() {
        assert_eq!(normalize_client_id(Some("client-ABC_1".into())), "abc-1");
        assert_eq!(normalize_client_id(Some("  ".into())).len(), 36);
        assert_eq!(normalize_client_id(None).len(), 36);
    }

    #[test]
    fn download_names_are_sanitized() {
        assert_eq!(sanitize_filename("report.pdf"), "report.pdf");
        assert_eq!(sanitize_filename("../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_filename("a\\b/c\u{7}.txt"), "abc.txt");
        assert_eq!(sanitize_filename("  .hidden  "), "hidden");
        assert_eq!(sanitize_filename(""), "file");
        assert_eq!(sanitize_filename("///"), "file");
        assert_eq!(sanitize_filename("..."), "file");
    }

    #[test]
    fn download_names_are_deduplicated() {
        let taken: std::collections::HashSet<&str> =
            ["photo.png", "photo (2).png", "notes", "notes (2)", ".env"]
                .into_iter()
                .collect();
        let exists = |n: &str| taken.contains(n);
        assert_eq!(dedupe_filename("new.png", exists), "new.png");
        assert_eq!(dedupe_filename("photo.png", exists), "photo (3).png");
        assert_eq!(dedupe_filename("notes", exists), "notes (3)");
        assert_eq!(dedupe_filename(".env", exists), ".env (2)");
        assert_eq!(dedupe_filename("a.tar.gz", |_| false), "a.tar.gz");
        assert_eq!(
            dedupe_filename("a.tar.gz", |n| n == "a.tar.gz"),
            "a.tar (2).gz"
        );
    }
}
