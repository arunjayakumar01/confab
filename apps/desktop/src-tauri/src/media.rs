//! The `prism-media` URI scheme: an authenticated proxy for attachment bytes.
//! The web view loads `prism-media://localhost/?u=<base64url upstream>`
//! (`http://prism-media.localhost/?u=` on Windows); this handler fetches the
//! upstream through `Google` (Bearer, refresh, retries) and answers with its
//! bytes and content type. Only the chat API origin (and the twin's, in twin
//! mode) may be fetched, so the scheme is not an open proxy.

use crate::accounts::Accounts;
use crate::shell::{media_origins, media_upstream_of, upstream_allowed, MEDIA_SCHEME};
use crate::AppState;
use std::sync::Arc;
use tauri::http::{header, Method, Request, Response, StatusCode};
use tauri::{AppHandle, Manager, Runtime, UriSchemeResponder};

/// Registers the scheme on the builder; the handler runs on the async runtime.
pub fn register<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.register_asynchronous_uri_scheme_protocol(MEDIA_SCHEME, |ctx, req, responder| {
        let app = ctx.app_handle().clone();
        tauri::async_runtime::spawn(async move {
            respond(responder, serve(&app, &req).await, &req);
        });
    })
}

fn respond(
    responder: UriSchemeResponder,
    outcome: Result<(String, Vec<u8>), (StatusCode, String)>,
    req: &Request<Vec<u8>>,
) {
    let (status, content_type, body) = match outcome {
        Ok((ct, bytes)) => (StatusCode::OK, ct, bytes),
        Err((status, msg)) => (
            status,
            "text/plain; charset=utf-8".to_string(),
            msg.into_bytes(),
        ),
    };
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(
            header::CACHE_CONTROL,
            if status == StatusCode::OK {
                "private, max-age=3600"
            } else {
                "no-store"
            },
        );
    // The scheme is reachable only from inside this webview; echoing its origin lets `fetch()` read bytes too.
    if let Some(origin) = req.headers().get(header::ORIGIN) {
        builder = builder.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    match builder.body(body) {
        Ok(resp) => responder.respond(resp),
        Err(e) => {
            tracing::warn!("{MEDIA_SCHEME}: cannot build response: {e}");
            let fallback = Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Vec::new())
                .expect("empty response");
            responder.respond(fallback);
        }
    }
}

async fn serve<R: Runtime>(
    app: &AppHandle<R>,
    req: &Request<Vec<u8>>,
) -> Result<(String, Vec<u8>), (StatusCode, String)> {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return Err((StatusCode::METHOD_NOT_ALLOWED, "GET only".into()));
    }
    // The active account's token serves the bytes; the web view only shows that account.
    let state: Arc<AppState> = app
        .try_state::<Arc<Accounts>>()
        .map(|a| a.active())
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "shell not ready".into()))?;
    let request_url = req.uri().to_string();
    let upstream = media_upstream_of(&request_url).ok_or((
        StatusCode::BAD_REQUEST,
        "missing or malformed u parameter".into(),
    ))?;
    if !upstream_allowed(&upstream, &media_origins(&state.profile.endpoints)) {
        tracing::warn!("{MEDIA_SCHEME}: refused upstream {upstream}");
        return Err((StatusCode::FORBIDDEN, "upstream host not allowed".into()));
    }
    match state.google.get_bytes(&upstream).await {
        Ok((content_type, bytes)) => {
            tracing::debug!(
                "{MEDIA_SCHEME}: served {upstream} ({content_type}, {} bytes)",
                bytes.len()
            );
            Ok((content_type, bytes))
        }
        Err(e) => {
            tracing::warn!("{MEDIA_SCHEME}: {upstream}: {e}");
            Err((StatusCode::BAD_GATEWAY, format!("upstream: {e}")))
        }
    }
}
