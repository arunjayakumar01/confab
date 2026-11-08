//! OAuth 2.0 authorization code + PKCE (S256) against `Endpoints::from_env()`,
//! the token store (OS keychain in real-Google mode, a 0600 file in twin mode)
//! and automatic refresh: 60 s before expiry, or when Google answers 401.
//! The web view never sees a token; everything here stays in the Rust core.

use crate::profile::{AccountInfo, Profile};
use crate::shell::{user_from_info, AuthStatus};
use base64::Engine;
use confab_proto::chat::{TokenResponse, UserInfo};
use confab_proto::{Endpoints, DRIVE_SCOPE, SCOPES};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

/// Refresh this many seconds before `expires_at`.
pub const REFRESH_SKEW_SECS: i64 = 60;
/// How long the loopback listener waits for the browser to come back.
const FLOW_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const KEYRING_USER: &str = "oauth";

/// What the token store holds: tokens plus the userinfo fetched at sign-in.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StoredAuth {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds.
    pub expires_at: i64,
    pub user: UserInfo,
    /// Space-separated scopes Google granted; a sign-in made before a scope was
    /// added to `SCOPES` is treated as signed out so consent is asked again.
    #[serde(default)]
    pub scopes: Option<String>,
}

/// True when every `https://` scope in `SCOPES` was granted (openid/email/profile
/// come back under other names, so they are not checked).
pub fn has_required_scopes(granted: Option<&str>) -> bool {
    let Some(g) = granted else { return false };
    let have: std::collections::HashSet<&str> = g.split_whitespace().collect();
    confab_proto::SCOPES
        .iter()
        .filter(|s| s.starts_with("https://"))
        .all(|s| have.contains(s))
}

/// True when `scope` is among the granted ones.
pub fn has_scope(granted: Option<&str>, scope: &str) -> bool {
    granted.is_some_and(|g| g.split_whitespace().any(|s| s == scope))
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// S256: `BASE64URL(SHA256(verifier))`, no padding (RFC 7636 §4.2).
pub fn pkce_challenge(verifier: &str) -> String {
    b64url(&Sha256::digest(verifier.as_bytes()))
}

/// A fresh (verifier, challenge) pair. 32 random bytes → 43 url-safe chars.
pub fn pkce_pair() -> (String, String) {
    let verifier = b64url(&rand::random::<[u8; 32]>());
    let challenge = pkce_challenge(&verifier);
    (verifier, challenge)
}

fn random_state() -> String {
    b64url(&rand::random::<[u8; 16]>())
}

/// The consent URL: `access_type=offline&prompt=consent` so a refresh token comes back.
/// `PRISM_TWIN_USER=users/1002` appends `twin_user` so the twin skips its pick-a-user click.
pub fn build_auth_url(
    endpoints: &Endpoints,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
    extra: Option<(&str, &str)>,
) -> Result<String, String> {
    let mut url = url::Url::parse(&endpoints.oauth_auth)
        .map_err(|e| format!("bad PRISM_OAUTH_AUTH {}: {e}", endpoints.oauth_auth))?;
    let mut scopes = SCOPES.join(" ");
    if let Some((scope, _)) = extra {
        scopes = format!("{scopes} {scope}");
    }
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &endpoints.client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", &scopes)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");
    if let Some((_, email)) = extra {
        // Incremental consent: keep what was granted and preselect the signed-in account.
        url.query_pairs_mut()
            .append_pair("include_granted_scopes", "true")
            .append_pair("login_hint", email);
    }
    if let Ok(user) = std::env::var("PRISM_TWIN_USER") {
        if !user.trim().is_empty() {
            url.query_pairs_mut().append_pair("twin_user", user.trim());
        }
    }
    Ok(url.to_string())
}

// ---------------------------------------------------------------------------
// Token store
// ---------------------------------------------------------------------------

pub enum TokenStore {
    /// OS keychain, service `prism-<profile>`.
    Keyring { service: String },
    /// `<profile dir>/auth.json`, mode 0600.
    File { path: PathBuf },
}

impl TokenStore {
    /// Keychain when `chat_api` points at Google, else a file;
    /// `PRISM_TOKEN_STORE=file|keyring` overrides.
    pub fn for_profile(profile: &Profile) -> Self {
        // note: file store in twin mode avoids keychain prompts on every unsigned rebuild
        let use_keyring = match std::env::var("PRISM_TOKEN_STORE").ok().as_deref() {
            Some("keyring") => true,
            Some("file") => false,
            _ => profile.real_google(),
        };
        if use_keyring {
            Self::Keyring {
                service: format!("prism-{}", profile.name),
            }
        } else {
            Self::File {
                path: profile.token_file(),
            }
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Keyring { service } => format!("keyring service {service}"),
            Self::File { path } => format!("file {}", path.display()),
        }
    }

    pub fn load(&self) -> Option<StoredAuth> {
        let json = match self {
            Self::Keyring { service } => {
                match keyring::Entry::new(service, KEYRING_USER).and_then(|e| e.get_password()) {
                    Ok(j) => j,
                    Err(keyring::Error::NoEntry) => return None,
                    Err(e) => {
                        tracing::warn!("keyring read failed: {e}");
                        return None;
                    }
                }
            }
            Self::File { path } => std::fs::read_to_string(path).ok()?,
        };
        match serde_json::from_str(&json) {
            Ok(a) => Some(a),
            Err(e) => {
                tracing::warn!("stored auth unreadable, ignoring: {e}");
                None
            }
        }
    }

    pub fn save(&self, auth: &StoredAuth) -> Result<(), String> {
        let json = serde_json::to_string(auth).map_err(|e| e.to_string())?;
        match self {
            Self::Keyring { service } => keyring::Entry::new(service, KEYRING_USER)
                .and_then(|e| e.set_password(&json))
                .map_err(|e| format!("keyring write failed: {e}")),
            Self::File { path } => write_private(path, json.as_bytes()),
        }
    }

    pub fn clear(&self) -> Result<(), String> {
        match self {
            Self::Keyring { service } => {
                match keyring::Entry::new(service, KEYRING_USER).and_then(|e| e.delete_credential())
                {
                    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                    Err(e) => Err(format!("keyring delete failed: {e}")),
                }
            }
            Self::File { path } => match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
            },
        }
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    f.write_all(bytes)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        // A pre-existing file keeps its old mode; tighten it explicitly.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("cannot chmod {}: {e}", path.display()))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Auth state machine
// ---------------------------------------------------------------------------

pub struct Auth {
    endpoints: Endpoints,
    store: TokenStore,
    http: reqwest::Client,
    /// Never held across an await.
    current: Mutex<Option<StoredAuth>>,
    /// Serialises refreshes so a burst of 401s refreshes once.
    refresh_gate: tokio::sync::Mutex<()>,
    /// One interactive sign-in at a time.
    flow_gate: tokio::sync::Mutex<()>,
    /// Bumped on every new access token; the relay re-hellos when it changes.
    token_gen: watch::Sender<u64>,
    /// `account.json` of the profile, written whenever the signed-in user is known.
    account_file: Option<PathBuf>,
}

impl Auth {
    pub fn new(
        endpoints: Endpoints,
        store: TokenStore,
        http: reqwest::Client,
        account_file: Option<PathBuf>,
    ) -> Self {
        let current = store.load().filter(|a| {
            let ok = has_required_scopes(a.scopes.as_deref());
            if !ok {
                tracing::info!(
                    "stored sign-in lacks the current scopes; consent will be asked again"
                );
            }
            ok
        });
        tracing::info!(
            "token store: {} ({})",
            store.describe(),
            if current.is_some() {
                "signed in"
            } else {
                "signed out"
            }
        );
        let (token_gen, _) = watch::channel(0);
        let auth = Self {
            endpoints,
            store,
            http,
            current: Mutex::new(current),
            refresh_gate: tokio::sync::Mutex::new(()),
            flow_gate: tokio::sync::Mutex::new(()),
            token_gen,
            account_file,
        };
        auth.write_account_file();
        auth
    }

    /// Writes `account.json` for the signed-in user so the account list can show
    /// them without a network call; a no-op when signed out or without a profile dir.
    pub fn write_account_file(&self) {
        let Some(path) = self.account_file.as_deref() else {
            return;
        };
        let Some(user) = self.user() else {
            return;
        };
        let info = AccountInfo::from_user(&user);
        if AccountInfo::read(path).as_ref() == Some(&info) {
            return;
        }
        if let Err(e) = info.write(path) {
            tracing::warn!("{e}");
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<StoredAuth>> {
        self.current.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn is_signed_in(&self) -> bool {
        self.lock().is_some()
    }

    pub fn user(&self) -> Option<UserInfo> {
        self.lock().as_ref().map(|a| a.user.clone())
    }

    /// `users/{sub}` of the signed-in account.
    pub fn me(&self) -> Option<String> {
        self.user().map(|u| format!("users/{}", u.sub))
    }

    pub fn status(&self) -> AuthStatus {
        match self.lock().as_ref() {
            Some(a) => AuthStatus {
                signed_in: true,
                user: Some(user_from_info(&a.user)),
            },
            None => AuthStatus {
                signed_in: false,
                user: None,
            },
        }
    }

    /// Receives a new value whenever the access token changes.
    pub fn subscribe_token_changes(&self) -> watch::Receiver<u64> {
        self.token_gen.subscribe()
    }

    /// A valid access token, refreshed when within 60 s of expiry.
    pub async fn access_token(&self) -> Result<String, String> {
        let (token, fresh) = {
            let g = self.lock();
            let a = g.as_ref().ok_or("not signed in")?;
            (
                a.access_token.clone(),
                a.expires_at - now_secs() > REFRESH_SKEW_SECS,
            )
        };
        if fresh {
            return Ok(token);
        }
        self.refresh(&token).await
    }

    /// Called on a 401: refresh unless someone already replaced `seen`.
    pub async fn force_refresh(&self, seen: &str) -> Result<String, String> {
        self.refresh(seen).await
    }

    async fn refresh(&self, seen: &str) -> Result<String, String> {
        let _gate = self.refresh_gate.lock().await;
        let (refresh_token, old) = {
            let g = self.lock();
            let a = g.as_ref().ok_or("not signed in")?;
            if a.access_token != seen && a.expires_at - now_secs() > REFRESH_SKEW_SECS {
                // Another task refreshed while we waited on the gate.
                return Ok(a.access_token.clone());
            }
            (a.refresh_token.clone(), a.clone())
        };
        let refresh_token = refresh_token.ok_or("no refresh token stored; sign in again")?;
        let mut form = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
            ("client_id", self.endpoints.client_id.as_str()),
        ];
        if let Some(secret) = self.endpoints.client_secret.as_deref() {
            form.push(("client_secret", secret));
        }
        let resp = self
            .http
            .post(&self.endpoints.oauth_token)
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("token refresh: {e}"))?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            if status.as_u16() == 400 && body.contains("invalid_grant") {
                // Revoked or expired grant: the session is dead, drop it so the UI asks to sign in.
                tracing::warn!("refresh token rejected (invalid_grant); clearing stored auth");
                self.clear_local();
            }
            return Err(format!(
                "token refresh failed: HTTP {status} {}",
                body.chars().take(300).collect::<String>()
            ));
        }
        let tr: TokenResponse =
            serde_json::from_str(&body).map_err(|e| format!("token refresh: bad response: {e}"))?;
        let updated = StoredAuth {
            access_token: tr.access_token.clone(),
            refresh_token: tr.refresh_token.or(old.refresh_token),
            expires_at: now_secs() + tr.expires_in.max(0),
            user: old.user,
            scopes: tr.scope.or(old.scopes),
        };
        self.store.save(&updated)?;
        *self.lock() = Some(updated);
        self.token_gen.send_modify(|g| *g += 1);
        tracing::info!("access token refreshed");
        Ok(tr.access_token)
    }

    /// The whole interactive flow: loopback listener, system browser, code
    /// exchange, userinfo, persist. Resolves when the user is signed in.
    pub async fn start(&self) -> Result<AuthStatus, String> {
        self.start_with(None).await
    }

    /// True when the stored sign-in includes Drive access.
    pub fn has_drive(&self) -> bool {
        self.lock()
            .as_ref()
            .is_some_and(|a| has_scope(a.scopes.as_deref(), DRIVE_SCOPE))
    }

    /// Asks Google for Drive access on top of the current sign-in. The chat sign-in stays
    /// valid throughout; a different account picked in the browser is refused.
    pub async fn grant_drive(&self) -> Result<bool, String> {
        let email = self
            .user()
            .ok_or("not signed in")?
            .email
            .ok_or("the signed-in account has no email")?;
        self.start_with(Some((DRIVE_SCOPE, email.as_str()))).await?;
        Ok(self.has_drive())
    }

    async fn start_with(&self, extra: Option<(&str, &str)>) -> Result<AuthStatus, String> {
        let _flow = self
            .flow_gate
            .try_lock()
            .map_err(|_| "a sign-in is already in progress")?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("cannot bind loopback listener: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}/callback");
        let (verifier, challenge) = pkce_pair();
        let state = random_state();
        let url = build_auth_url(&self.endpoints, &redirect_uri, &state, &challenge, extra)?;
        let headless_twin = std::env::var("PRISM_TWIN_USER")
            .map(|u| !u.trim().is_empty())
            .unwrap_or(false)
            && !self.endpoints.oauth_auth.contains("google.com");
        if headless_twin {
            // note: scripted twin sign-in follows the redirect chain itself, no browser tab
            tracing::info!(
                "twin sign-in as PRISM_TWIN_USER without a browser (loopback port {port})"
            );
            let http = self.http.clone();
            let auth_url = url.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = http.get(&auth_url).send().await {
                    tracing::warn!("twin sign-in request failed: {e}");
                }
            });
        } else {
            tracing::info!("opening consent page in the system browser (loopback port {port})");
            if let Err(e) = tauri_plugin_opener::open_url(&url, None::<&str>) {
                return Err(format!(
                    "could not open the browser ({e}); open this URL yourself: {url}"
                ));
            }
        }
        let code = tokio::time::timeout(FLOW_TIMEOUT, wait_for_callback(listener, &state))
            .await
            .map_err(|_| "sign-in timed out waiting for the browser".to_string())??;
        let tr = self.exchange_code(&code, &verifier, &redirect_uri).await?;
        let user = fetch_userinfo(&self.http, &self.endpoints.userinfo, &tr.access_token).await?;
        if let Some((_, email)) = extra {
            let got = user.email.as_deref().unwrap_or("another account");
            if !got.eq_ignore_ascii_case(email) {
                return Err(format!(
                    "Drive access was given for {got}, not {email}. Try again and pick {email}."
                ));
            }
        }
        let stored = StoredAuth {
            access_token: tr.access_token,
            refresh_token: tr.refresh_token,
            expires_at: now_secs() + tr.expires_in.max(0),
            user,
            scopes: tr.scope,
        };
        self.store.save(&stored)?;
        *self.lock() = Some(stored);
        self.write_account_file();
        self.token_gen.send_modify(|g| *g += 1);
        tracing::info!("signed in");
        Ok(self.status())
    }

    async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<TokenResponse, String> {
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
            ("client_id", self.endpoints.client_id.as_str()),
        ];
        if let Some(secret) = self.endpoints.client_secret.as_deref() {
            form.push(("client_secret", secret));
        }
        let resp = self
            .http
            .post(&self.endpoints.oauth_token)
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("token exchange: {e}"))?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!(
                "token exchange failed: HTTP {status} {}",
                body.chars().take(300).collect::<String>()
            ));
        }
        serde_json::from_str(&body).map_err(|e| format!("token exchange: bad response: {e}"))
    }

    fn clear_local(&self) {
        if let Err(e) = self.store.clear() {
            tracing::warn!("{e}");
        }
        *self.lock() = None;
    }

    pub fn sign_out(&self) -> Result<(), String> {
        self.store.clear()?;
        *self.lock() = None;
        tracing::info!("signed out");
        Ok(())
    }
}

async fn fetch_userinfo(
    http: &reqwest::Client,
    url: &str,
    token: &str,
) -> Result<UserInfo, String> {
    let resp = http
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("userinfo: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "userinfo failed: HTTP {status} {}",
            body.chars().take(300).collect::<String>()
        ));
    }
    serde_json::from_str(&body).map_err(|e| format!("userinfo: bad response: {e}"))
}

// ---------------------------------------------------------------------------
// Loopback redirect
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum Callback {
    /// `GET /callback?code=..&state=<expected>`
    Code(String),
    /// `GET /callback?error=..` (the user declined)
    Denied(String),
    /// `/callback` without a matching state: answer 400, keep listening.
    BadState,
    /// Anything else (favicon, probes): answer with this status, keep listening.
    NotCallback(&'static str),
}

/// Classify the request line of a loopback hit. Pure, so it is unit-tested.
pub fn parse_callback(request_line: &str, expected_state: &str) -> Callback {
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    if method != "GET" {
        return Callback::NotCallback("405 Method Not Allowed");
    }
    let absolute = if target.starts_with("http://") || target.starts_with("https://") {
        target.to_string()
    } else {
        format!("http://127.0.0.1{target}")
    };
    let url = match url::Url::parse(&absolute) {
        Ok(u) => u,
        Err(_) => return Callback::NotCallback("400 Bad Request"),
    };
    if url.path() != "/callback" {
        return Callback::NotCallback("404 Not Found");
    }
    let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
    if let Some(err) = q.get("error") {
        return Callback::Denied(err.clone());
    }
    match (q.get("code"), q.get("state")) {
        (Some(code), Some(state)) if state == expected_state && !code.is_empty() => {
            Callback::Code(code.clone())
        }
        _ => Callback::BadState,
    }
}

const DONE_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Confab</title>\
<style>body{font:16px -apple-system,system-ui,sans-serif;background:#111;color:#eee;display:grid;place-items:center;height:100vh;margin:0}</style>\
</head><body><div><h2>Signed in to Confab</h2><p>You can close this tab.</p></div></body></html>";
const DENIED_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Confab</title></head>\
<body><h2>Sign-in was not completed</h2><p>You can close this tab and try again from Confab.</p></body></html>";

/// Accept exactly one `GET /callback` with the expected state; everything else
/// gets a short answer and the listener keeps waiting.
async fn wait_for_callback(listener: TcpListener, expected_state: &str) -> Result<String, String> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| format!("loopback accept: {e}"))?;
        let request_line = match read_request_line(&mut stream).await {
            Some(l) => l,
            None => continue,
        };
        match parse_callback(&request_line, expected_state) {
            Callback::Code(code) => {
                respond(&mut stream, "200 OK", DONE_PAGE).await;
                return Ok(code);
            }
            Callback::Denied(err) => {
                respond(&mut stream, "200 OK", DENIED_PAGE).await;
                return Err(format!("sign-in denied: {err}"));
            }
            Callback::BadState => {
                tracing::warn!("loopback callback with a mismatched state ignored");
                respond(
                    &mut stream,
                    "400 Bad Request",
                    "<p>State mismatch. Start the sign-in again from Confab.</p>",
                )
                .await;
            }
            Callback::NotCallback(status) => {
                respond(&mut stream, status, "").await;
            }
        }
    }
}

async fn read_request_line(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];
    loop {
        let n = match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk)).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(n)) => n,
            Ok(Err(_)) => return None,
        };
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 16 * 1024 {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buf);
    text.lines().next().map(|l| l.to_string())
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc7636_vector() {
        // RFC 7636 Appendix B
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            pkce_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let (v, c) = pkce_pair();
        assert_eq!(v.len(), 43);
        assert!(v
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
        assert_eq!(pkce_challenge(&v), c);
    }

    #[test]
    fn auth_url_carries_pkce_and_offline_access() {
        let ep = Endpoints {
            chat_api: "http://127.0.0.1:8081".into(),
            oauth_auth: "http://127.0.0.1:8081/o/oauth2/v2/auth".into(),
            oauth_token: "http://127.0.0.1:8081/token".into(),
            userinfo: "http://127.0.0.1:8081/oauth2/v3/userinfo".into(),
            relay_ws: "ws://127.0.0.1:8080/ws".into(),
            people_api: "http://127.0.0.1:8081".into(),
            tasks_api: "http://127.0.0.1:8081".into(),
            meet_api: "http://127.0.0.1:8081".into(),
            calendar_api: "http://127.0.0.1:8081/calendar/v3".into(),
            drive_api: "http://127.0.0.1:8081/drive/v3".into(),
            client_id: "prism-dev".into(),
            client_secret: None,
        };
        let url = build_auth_url(&ep, "http://127.0.0.1:4321/callback", "st", "ch", None).unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        let q: HashMap<String, String> = parsed.query_pairs().into_owned().collect();
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["client_id"], "prism-dev");
        assert_eq!(q["redirect_uri"], "http://127.0.0.1:4321/callback");
        assert_eq!(q["code_challenge"], "ch");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["access_type"], "offline");
        assert_eq!(q["prompt"], "consent");
        assert_eq!(q["state"], "st");
        assert_eq!(q["scope"], SCOPES.join(" "));
        assert!(!q.contains_key("include_granted_scopes"));

        let url = build_auth_url(
            &ep,
            "http://127.0.0.1:4321/callback",
            "st",
            "ch",
            Some((DRIVE_SCOPE, "ana@example.test")),
        )
        .unwrap();
        let q: HashMap<String, String> = url::Url::parse(&url)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
        assert_eq!(q["scope"], format!("{} {DRIVE_SCOPE}", SCOPES.join(" ")));
        assert_eq!(q["include_granted_scopes"], "true");
        assert_eq!(q["login_hint"], "ana@example.test");
        assert!(has_scope(Some(&q["scope"]), DRIVE_SCOPE));
        assert!(!has_scope(Some(&SCOPES.join(" ")), DRIVE_SCOPE));
    }

    #[test]
    fn callback_classification() {
        assert_eq!(
            parse_callback("GET /callback?code=abc&state=xyz HTTP/1.1", "xyz"),
            Callback::Code("abc".into())
        );
        assert_eq!(
            parse_callback("GET /callback?code=abc&state=nope HTTP/1.1", "xyz"),
            Callback::BadState
        );
        assert_eq!(
            parse_callback("GET /callback?state=xyz HTTP/1.1", "xyz"),
            Callback::BadState
        );
        assert_eq!(
            parse_callback(
                "GET /callback?error=access_denied&state=xyz HTTP/1.1",
                "xyz"
            ),
            Callback::Denied("access_denied".into())
        );
        assert_eq!(
            parse_callback("GET /favicon.ico HTTP/1.1", "xyz"),
            Callback::NotCallback("404 Not Found")
        );
        assert_eq!(
            parse_callback("POST /callback HTTP/1.1", "xyz"),
            Callback::NotCallback("405 Method Not Allowed")
        );
        assert_eq!(
            parse_callback("GET /callback?code=a%2Fb&state=xyz HTTP/1.1", "xyz"),
            Callback::Code("a/b".into())
        );
    }

    #[tokio::test]
    async fn loopback_accepts_one_matching_callback() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiter = tokio::spawn(async move { wait_for_callback(listener, "s1").await });

        // A stray probe first: answered, ignored.
        let mut probe = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        probe
            .write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut out = String::new();
        probe.read_to_string(&mut out).await.unwrap();
        assert!(out.starts_with("HTTP/1.1 404"));

        let mut real = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        real.write_all(b"GET /callback?state=s1&code=the-code HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut page = String::new();
        real.read_to_string(&mut page).await.unwrap();
        assert!(page.starts_with("HTTP/1.1 200"));
        assert!(page.contains("close this tab"));
        assert_eq!(waiter.await.unwrap().unwrap(), "the-code");
    }

    #[test]
    fn scope_gate_requires_every_https_scope() {
        let all = confab_proto::SCOPES.join(" ");
        assert!(has_required_scopes(Some(&all)));
        assert!(!has_required_scopes(None));
        let old = all.replace("https://www.googleapis.com/auth/directory.readonly", "");
        assert!(
            !has_required_scopes(Some(&old)),
            "a sign-in from before a scope was added must re-consent"
        );
    }

    #[test]
    fn file_store_round_trips_and_is_private() {
        let dir = std::env::temp_dir().join(format!("prism-auth-test-{}", uuid::Uuid::new_v4()));
        let store = TokenStore::File {
            path: dir.join("auth.json"),
        };
        assert!(store.load().is_none());
        let auth = StoredAuth {
            access_token: "at".into(),
            refresh_token: Some("rt".into()),
            expires_at: 123,
            user: UserInfo {
                sub: "1001".into(),
                ..Default::default()
            },
            scopes: None,
        };
        store.save(&auth).unwrap();
        assert_eq!(store.load(), Some(auth));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("auth.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        store.clear().unwrap();
        assert!(store.load().is_none());
        store.clear().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
