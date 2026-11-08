//! OAuth 2.0 authorization code + PKCE (S256) the way Google's endpoints behave,
//! plus the `Caller` extractor every Chat API handler uses.

use crate::error::{ApiError, OAuthError};
use crate::state::{AppState, AuthCode};
use axum::{
    extract::{FromRequestParts, Query, State},
    http::{header, request::Parts, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Form, Json, Router,
};
use base64::Engine;
use chrono::{Duration, Utc};
use confab_proto::chat::{TokenResponse, UserInfo};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;

/// Authenticated user (`users/{id}`), from `Authorization: Bearer` or, for
/// browser `<img>` loads, an `access_token` query parameter (Google accepts both).
#[derive(Clone, Debug)]
pub struct Caller {
    pub user: String,
    /// Granted scopes; `None` for a token minted without asking, which may do anything.
    pub scope: Option<String>,
}

impl Caller {
    /// 403 the way Google answers a token without `scope`.
    pub fn require(&self, scope: &str) -> Result<(), ApiError> {
        match &self.scope {
            Some(granted) if !granted.split_whitespace().any(|s| s == scope) => Err(ApiError::new(
                axum::http::StatusCode::FORBIDDEN,
                "PERMISSION_DENIED",
                "Request had insufficient authentication scopes.",
            )),
            _ => Ok(()),
        }
    }
}

impl FromRequestParts<Arc<AppState>> for Caller {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, ApiError> {
        let from_header = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                let (scheme, rest) = v.split_once(' ')?;
                scheme
                    .eq_ignore_ascii_case("bearer")
                    .then(|| rest.trim().to_string())
            });
        let from_query = || {
            parts.uri.query().and_then(|q| {
                url::form_urlencoded::parse(q.as_bytes())
                    .find(|(k, _)| k == "access_token")
                    .map(|(_, v)| v.into_owned())
            })
        };
        let token = from_header
            .or_else(from_query)
            .ok_or_else(ApiError::unauthenticated)?;
        let (user, scope) = state
            .read()
            .caller_for(&token)
            .ok_or_else(ApiError::unauthenticated)?;
        Ok(Caller { user, scope })
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/o/oauth2/v2/auth", get(authorize))
        .route("/token", post(token))
        .route("/oauth2/v3/userinfo", get(userinfo))
        .route("/v1/userinfo", get(userinfo))
        .route("/revoke", post(revoke))
}

fn html_error(code: StatusCode, title: &str, detail: &str) -> Response {
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Error {}: {title}</title></head>\
<body style=\"font-family:system-ui;padding:2rem\"><h1>Error {}: {title}</h1><p>{detail}</p></body></html>",
        code.as_u16(),
        code.as_u16()
    );
    (code, Html(body)).into_response()
}

async fn authorize(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let get = |k: &str| q.get(k).map(|s| s.trim()).filter(|s| !s.is_empty());
    let Some(redirect_uri) = get("redirect_uri") else {
        return html_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Missing required parameter: redirect_uri",
        );
    };
    if get("client_id").is_none() {
        return html_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Missing required parameter: client_id",
        );
    }
    if get("response_type") != Some("code") {
        return html_error(
            StatusCode::BAD_REQUEST,
            "unsupported_response_type",
            "Only response_type=code is supported",
        );
    }
    let Ok(mut redirect) = url::Url::parse(redirect_uri) else {
        return html_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Invalid parameter value for redirect_uri",
        );
    };
    let method = get("code_challenge_method").unwrap_or("plain");
    if get("code_challenge").is_some() && !matches!(method, "S256" | "plain") {
        return html_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Invalid code_challenge_method",
        );
    }

    if let Some(user) = get("twin_user") {
        let user = if user.starts_with("users/") {
            user.to_string()
        } else {
            format!("users/{user}")
        };
        let mut st = state.write();
        if !st.users.contains_key(&user) {
            return html_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "Unknown twin_user",
            );
        }
        // note: any client_id is accepted; Google would check it against the registered client.
        let code = st.issue_code(AuthCode {
            user,
            client_id: get("client_id").unwrap_or_default().to_string(),
            redirect_uri: redirect_uri.to_string(),
            code_challenge: get("code_challenge").map(str::to_string),
            challenge_method: method.to_string(),
            scope: get("scope").unwrap_or_default().to_string(),
            expires_at: Utc::now() + Duration::minutes(10),
        });
        drop(st);
        {
            let mut pairs = redirect.query_pairs_mut();
            pairs.append_pair("code", &code);
            if let Some(s) = get("state") {
                pairs.append_pair("state", s);
            }
            pairs.append_pair("scope", get("scope").unwrap_or_default());
        }
        return (
            StatusCode::FOUND,
            [(header::LOCATION, redirect.to_string())],
        )
            .into_response();
    }

    // Sign-in page: one button per seeded user, each a link back here with twin_user set.
    let mut base = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in &q {
        if k != "twin_user" {
            base.append_pair(k, v);
        }
    }
    let base = base.finish();
    let st = state.read();
    let mut buttons = String::new();
    for u in st.users.values() {
        let id = crate::util::user_id(&u.name);
        let href = format!("/o/oauth2/v2/auth?{base}&twin_user=users/{id}");
        buttons.push_str(&format!(
            "<a class=\"u\" href=\"{href}\"><img src=\"/twin/avatars/{id}.svg\" alt=\"\"><span><b>{}</b><small>{}</small></span></a>",
            escape(u.display_name.as_deref().unwrap_or(&u.name)),
            escape(u.email.as_deref().unwrap_or(""))
        ));
    }
    let page = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>Sign in to Prism (twin)</title>\
<style>body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#0f1220;color:#e6e8ef;font-family:Inter,system-ui,sans-serif}}\
main{{width:min(420px,92vw);background:#181c2f;border:1px solid #2d3350;border-radius:16px;padding:28px}}\
h1{{font-size:20px;margin:0 0 4px}}p{{color:#9aa0b5;margin:0 0 20px;font-size:14px}}\
.u{{display:flex;align-items:center;gap:14px;padding:12px;border-radius:12px;border:1px solid #2d3350;margin-bottom:10px;color:inherit;text-decoration:none;background:#1c2037}}\
.u:hover{{border-color:#5b6cff;background:#20254a}}.u img{{width:40px;height:40px;border-radius:50%}}.u span{{display:flex;flex-direction:column}}\
.u b{{font-weight:600}}.u small{{color:#9aa0b5;font-size:12px}}</style></head>\
<body><main><h1>Sign in to Prism (twin)</h1><p>Choose a prism.test account to continue to <code>{}</code>.</p>{buttons}</main></body></html>",
        escape(get("client_id").unwrap_or("prism"))
    );
    Html(page).into_response()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn pkce_ok(challenge: &str, method: &str, verifier: &str) -> bool {
    match method {
        "S256" => {
            let digest = Sha256::digest(verifier.as_bytes());
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest) == challenge
        }
        _ => challenge == verifier,
    }
}

async fn token(
    State(state): State<Arc<AppState>>,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Json<TokenResponse>, OAuthError> {
    let get = |k: &str| form.get(k).map(|s| s.trim()).filter(|s| !s.is_empty());
    match get("grant_type") {
        Some("authorization_code") => {
            let code = get("code")
                .ok_or_else(|| OAuthError::invalid_request("Missing required parameter: code"))?;
            let mut st = state.write();
            let rec = st
                .codes
                .remove(code)
                .ok_or_else(|| OAuthError::invalid_grant("Malformed auth code."))?;
            if rec.expires_at < Utc::now() {
                return Err(OAuthError::invalid_grant("Bad Request: code expired"));
            }
            if let Some(c) = get("client_id") {
                if !rec.client_id.is_empty() && c != rec.client_id {
                    return Err(OAuthError::invalid_grant("Bad Request: client_id mismatch"));
                }
            }
            if let Some(r) = get("redirect_uri") {
                if r != rec.redirect_uri {
                    return Err(OAuthError {
                        error: "redirect_uri_mismatch",
                        error_description: "Bad Request".into(),
                        code: StatusCode::BAD_REQUEST,
                    });
                }
            }
            if let Some(challenge) = &rec.code_challenge {
                let verifier = get("code_verifier")
                    .ok_or_else(|| OAuthError::invalid_grant("Missing code verifier."))?;
                if !pkce_ok(challenge, &rec.challenge_method, verifier) {
                    return Err(OAuthError::invalid_grant("Invalid code verifier."));
                }
            }
            let scope = (!rec.scope.is_empty()).then(|| rec.scope.clone());
            Ok(Json(st.mint_token(&rec.user, scope)))
        }
        Some("refresh_token") => {
            let refresh = get("refresh_token").ok_or_else(|| {
                OAuthError::invalid_request("Missing required parameter: refresh_token")
            })?;
            let mut st = state.write();
            let user =
                st.refresh_tokens.get(refresh).cloned().ok_or_else(|| {
                    OAuthError::invalid_grant("Token has been expired or revoked.")
                })?;
            let scope = st.refresh_scopes.get(refresh).cloned();
            let (access_token, expires_in) = st.issue_access(&user, scope.clone());
            Ok(Json(TokenResponse {
                access_token,
                refresh_token: None,
                expires_in,
                id_token: None,
                scope: scope.or_else(|| Some(confab_proto::SCOPES.join(" "))),
                token_type: "Bearer".into(),
            }))
        }
        Some(other) => Err(OAuthError::unsupported_grant_type(format!(
            "Invalid grant_type: {other}"
        ))),
        None => Err(OAuthError::invalid_request(
            "Missing required parameter: grant_type",
        )),
    }
}

async fn userinfo(
    State(state): State<Arc<AppState>>,
    caller: Caller,
) -> Result<Json<UserInfo>, ApiError> {
    let st = state.read();
    let user = st
        .user(&caller.user)
        .ok_or_else(ApiError::unauthenticated)?;
    Ok(Json(UserInfo {
        sub: crate::util::user_id(&user.name).to_string(),
        email: user.email.clone(),
        name: user.display_name.clone(),
        picture: user.avatar_url.clone(),
        hd: Some(crate::seed::DOMAIN.into()),
    }))
}

async fn revoke(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
    body: String,
) -> Response {
    let token = q.get("token").cloned().or_else(|| {
        url::form_urlencoded::parse(body.as_bytes())
            .find(|(k, _)| k == "token")
            .map(|(_, v)| v.into_owned())
    });
    match token {
        Some(t) if state.write().revoke(&t) => {
            (StatusCode::OK, Json(serde_json::json!({}))).into_response()
        }
        _ => OAuthError {
            error: "invalid_token",
            error_description: "Token expired or revoked".into(),
            code: StatusCode::BAD_REQUEST,
        }
        .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s256_matches_rfc7636_vector() {
        // RFC 7636 appendix B
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(pkce_ok(challenge, "S256", verifier));
        assert!(!pkce_ok(challenge, "S256", "nope"));
        assert!(pkce_ok("abc", "plain", "abc"));
    }
}
