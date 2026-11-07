//! The huddle mini window: the real Google Meet in a small window, the way Google Chat runs a huddle.
//! First choice is Chrome's app window in the profile signed in as this account: no second sign-in, and all of
//! Chrome's Meet features. Without such a profile, Meet gets a top-level always-on-top webview of its own (it refuses
//! to load in a frame), which asks for a Google sign-in once per account. That webview is outside the `main`
//! capability, so the pages it loads have no access to Confab's commands.

use std::sync::Mutex;

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// The meeting code of the call open in a Chrome app window.
static CHROME_CALL: Mutex<Option<String>> = Mutex::new(None);

const PREFIX: &str = "huddle-";
/// Meet turns away a webview's default user agent; this one says what the engine is, the system's WebKit.
const SAFARI_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

pub fn is_huddle(label: &str) -> bool {
    label.starts_with(PREFIX)
}

fn is_meet_call(url: &url::Url) -> bool {
    url.scheme() == "https" && url.host_str() == Some("meet.google.com")
}

/// What the window may load, frames included: Meet, Google sign-in and their assets, nothing else.
/// note: a company single sign-on page outside Google is refused; allow its host if a workspace needs it.
pub fn nav_allowed(url: &url::Url) -> bool {
    match url.scheme() {
        "about" | "blob" | "data" => true,
        "https" => url.host_str().is_some_and(|h| {
            h.starts_with("accounts.google.")
                || [
                    "google.com",
                    "gstatic.com",
                    "googleusercontent.com",
                    "youtube.com",
                ]
                .iter()
                .any(|d| h == *d || h.ends_with(&format!(".{d}")))
        }),
        _ => false,
    }
}

/// The per-account browser profile: each Confab account stays signed in to Google on its own.
/// note: the id's first 16 bytes; profile ids are short, so they do not collide in practice.
fn store_id(account: &str) -> [u8; 16] {
    let mut id = [0u8; 16];
    for (slot, b) in id.iter_mut().zip(account.bytes()) {
        *slot = b;
    }
    id
}

/// The meeting code from a Meet link; only letters and dashes, so it is safe inside the script below.
fn meet_code(url: &url::Url) -> Option<String> {
    let code = url.path().trim_matches('/');
    let ok = !code.is_empty() && code.chars().all(|c| c.is_ascii_alphabetic() || c == '-');
    (is_meet_call(url) && ok).then(|| code.to_string())
}

/// AppleScript acting on the Chrome window holding this call; prints `found` when there is one.
fn chrome_script(code: &str, action: &str) -> String {
    format!(
        r#"if application "Google Chrome" is running then
tell application "Google Chrome"
repeat with w in windows
repeat with t in tabs of w
if URL of t contains "{code}" then
{action}
return "found"
end if
end repeat
end repeat
end tell
end if"#
    )
}

fn take_chrome_call() -> Option<String> {
    CHROME_CALL.lock().unwrap_or_else(|p| p.into_inner()).take()
}

/// Closes the Chrome window of a call. note: macOS asks once for permission to control Chrome; if refused,
/// the call's window stays open and the person leaves from Meet itself.
fn close_chrome(code: &str) {
    let _ = std::process::Command::new("osascript")
        .args(["-e", &chrome_script(code, "close w")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Opens the call in Chrome's app window for this profile, or brings it forward when this call is already open.
/// False when Chrome could not start, so the caller can use the webview.
pub fn open_chrome<R: Runtime>(
    app: &AppHandle<R>,
    profile_dir: &str,
    url: &str,
) -> Result<bool, String> {
    let target = url::Url::parse(url).map_err(|e| e.to_string())?;
    let code = meet_code(&target).ok_or_else(|| format!("{url} is not a Google Meet link"))?;
    close_webviews(app);
    match take_chrome_call() {
        Some(open) if open == code => {
            let found = std::process::Command::new("osascript")
                .args([
                    "-e",
                    &chrome_script(
                        &code,
                        "set index of w to 1\nactivate"
                            .replace("\\n", "\n")
                            .as_str(),
                    ),
                ])
                .output()
                .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("found"));
            if found {
                *CHROME_CALL.lock().unwrap_or_else(|p| p.into_inner()) = Some(code);
                return Ok(true);
            }
        }
        Some(open) => close_chrome(&open),
        None => {}
    }
    let opened = crate::commands::open_in_chrome(profile_dir, &[&format!("--app={url}")]);
    if opened {
        // A running Chrome ignores --window-size, so the window is sized once it exists.
        let script = format!(
            "delay 2\n{}",
            chrome_script(&code, "set bounds of w to {80, 80, 560, 760}")
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        *CHROME_CALL.lock().unwrap_or_else(|p| p.into_inner()) = Some(code);
    }
    Ok(opened)
}

/// Opens the call, or brings it forward when this call is already open. A different call replaces the old one.
pub fn open<R: Runtime>(
    app: &AppHandle<R>,
    url: &str,
    title: &str,
    account: &str,
) -> Result<(), String> {
    let target = url::Url::parse(url).map_err(|e| e.to_string())?;
    if !is_meet_call(&target) {
        return Err(format!("{url} is not a Google Meet link"));
    }
    if let Some(code) = take_chrome_call() {
        close_chrome(&code);
    }
    for (label, w) in app.webview_windows() {
        if !is_huddle(&label) {
            continue;
        }
        if w.url().is_ok_and(|u| u.path() == target.path()) {
            w.show().map_err(|e| e.to_string())?;
            return w.set_focus().map_err(|e| e.to_string());
        }
        w.destroy().map_err(|e| e.to_string())?;
    }
    // A fresh label each time: a destroyed window keeps its label until the event loop has run.
    let label = format!("{PREFIX}{}", uuid::Uuid::new_v4().simple());
    let builder = WebviewWindowBuilder::new(app, label, WebviewUrl::External(target))
        .title(title)
        .inner_size(440.0, 640.0)
        .min_inner_size(340.0, 420.0)
        .always_on_top(true)
        .user_agent(SAFARI_UA)
        .on_navigation(|u| {
            let ok = nav_allowed(u);
            if !ok {
                tracing::debug!("huddle window: refused {u}");
            }
            ok
        });
    #[cfg(target_os = "macos")]
    let builder = builder.data_store_identifier(store_id(account));
    #[cfg(not(target_os = "macos"))]
    let _ = store_id(account);
    builder.build().map_err(|e| e.to_string())?;
    Ok(())
}

/// Closes the call without reporting it back: the web view asked for this itself.
pub fn close<R: Runtime>(app: &AppHandle<R>) {
    if let Some(code) = take_chrome_call() {
        close_chrome(&code);
    }
    close_webviews(app);
}

fn close_webviews<R: Runtime>(app: &AppHandle<R>) {
    for (label, w) in app.webview_windows() {
        if is_huddle(&label) {
            let _ = w.destroy();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> url::Url {
        url::Url::parse(s).unwrap()
    }

    #[test]
    fn only_meet_opens_and_only_google_loads() {
        assert!(is_meet_call(&u(
            "https://meet.google.com/abc-defg-hij?authuser=a"
        )));
        assert!(!is_meet_call(&u("http://meet.google.com/abc")));
        assert!(!is_meet_call(&u("https://meet.google.com.evil.test/abc")));
        for ok in [
            "https://accounts.google.com/x",
            "https://accounts.google.co.in/x",
            "https://www.gstatic.com/x",
            "about:blank",
        ] {
            assert!(nav_allowed(&u(ok)), "{ok}");
        }
        for no in [
            "https://evil.test/",
            "https://notgoogle.com/",
            "https://google.com.evil.test/",
            "http://meet.google.com/",
            "file:///etc/hosts",
        ] {
            assert!(!nav_allowed(&u(no)), "{no}");
        }
    }

    #[test]
    fn only_a_plain_meeting_code_reaches_the_script() {
        assert_eq!(
            meet_code(&u("https://meet.google.com/abc-defg-hij?authuser=a")).as_deref(),
            Some("abc-defg-hij")
        );
        assert_eq!(meet_code(&u("https://meet.google.com/abc%22def")), None);
        assert_eq!(meet_code(&u("https://evil.test/abc-defg-hij")), None);
        assert_eq!(meet_code(&u("https://meet.google.com/")), None);
    }

    #[test]
    fn accounts_get_their_own_store() {
        assert_ne!(store_id("default"), store_id("acct-2"));
        assert_eq!(store_id("a-profile-name-longer-than-sixteen").len(), 16);
    }
}
