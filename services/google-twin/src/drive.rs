//! Google Drive API v3 subset for the app-data folder, in memory and per caller:
//! `files.list` filtered by `name='...'`, `files.get` with `alt=media`, a
//! `multipart/related` create under `/upload/drive/v3` and a media update. Bearer
//! required, Google-shaped errors.

use crate::auth::Caller;
use crate::chat::boundary_of;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

type Params = Query<HashMap<String, String>>;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/drive/v3/files", get(list_files))
        .route("/drive/v3/files/{id}", get(get_file))
        .route("/upload/drive/v3/files", post(create_file))
        .route("/upload/drive/v3/files/{id}", patch(update_file))
}

/// The metadata part of a multipart create: `name` and, optionally, `mimeType`;
/// `parents` (`appDataFolder`) is accepted and ignored, every twin file lives there.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct FileMetadata {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
}

fn not_found(id: &str) -> ApiError {
    ApiError::not_found(format!("File not found: {id}."))
}

/// The `name = '...'` term of a Drive `q`, single or double quoted; `None` when the
/// query does not filter by name.
pub(crate) fn name_in_query(q: &str) -> Option<String> {
    let rest = q.trim().strip_prefix("name")?.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let mut chars = rest.chars();
    let quote = chars.next().filter(|c| matches!(c, '\'' | '"'))?;
    let mut out = String::new();
    let mut escaped = false;
    for c in chars {
        match c {
            _ if escaped => {
                out.push(c);
                escaped = false;
            }
            '\\' => escaped = true,
            c if c == quote => return Some(out),
            c => out.push(c),
        }
    }
    None
}

async fn list_files(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Query(q): Params,
) -> ApiResult<Json<serde_json::Value>> {
    let name = match q.get("q").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(query) => Some(name_in_query(query).ok_or_else(|| {
            ApiError::invalid_argument(format!(
                "Invalid query: {query} (only name = '...' is supported)"
            ))
        })?),
        None => None,
    };
    // note: spaces, fields and pageSize are accepted and ignored; one page of app data.
    let st = state.read();
    let files: Vec<serde_json::Value> = st
        .drive_files_of(&caller.user, name.as_deref())
        .iter()
        .map(|f| f.to_json())
        .collect();
    Ok(Json(serde_json::json!({
        "kind": "drive#fileList",
        "incompleteSearch": false,
        "files": files,
    })))
}

/// `alt=media` answers the bytes with their content type; otherwise the metadata.
async fn get_file(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(id): Path<String>,
    Query(q): Params,
) -> ApiResult<Response> {
    let st = state.read();
    let file = st
        .drive_file(&caller.user, &id)
        .ok_or_else(|| not_found(&id))?;
    if q.get("alt").map(String::as_str) == Some("media") {
        Ok((
            StatusCode::OK,
            [(header::CONTENT_TYPE, file.content_type.clone())],
            file.bytes.clone(),
        )
            .into_response())
    } else {
        Ok(Json(file.to_json()).into_response())
    }
}

fn content_type_of(headers: &HeaderMap) -> String {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

/// `POST /upload/drive/v3/files?uploadType=multipart`: a `multipart/related` body
/// whose first part is the metadata JSON and second the content.
async fn create_file(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Query(q): Params,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let (metadata, content, part_type) = match q.get("uploadType").map(String::as_str) {
        Some("multipart") => parse_related(content_type, body).await?,
        Some("media") => (
            FileMetadata::default(),
            body.to_vec(),
            Some(content_type_of(&headers)),
        ),
        other => {
            return Err(ApiError::invalid_argument(format!(
                "Invalid value for uploadType: {} (expected multipart or media)",
                other.unwrap_or("")
            )))
        }
    };
    let name = metadata
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Untitled".to_string());
    let mime = metadata
        .mime_type
        .filter(|m| !m.trim().is_empty())
        .or(part_type)
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let file = state
        .write()
        .new_drive_file(&caller.user, &name, &mime, content, Utc::now());
    Ok(Json(file.to_json()))
}

/// The two parts of Google's related multipart: `(metadata, content bytes, content type)`.
async fn parse_related(
    content_type: &str,
    body: Bytes,
) -> ApiResult<(FileMetadata, Vec<u8>, Option<String>)> {
    let boundary = boundary_of(content_type).ok_or_else(|| {
        ApiError::invalid_argument("Expected a multipart/related body with a boundary parameter.")
    })?;
    let stream = futures::stream::once(async move { Ok::<Bytes, std::io::Error>(body) });
    let mut multipart = multer::Multipart::new(stream, boundary);
    let mut metadata: Option<FileMetadata> = None;
    let mut content: Option<(Vec<u8>, Option<String>)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::invalid_argument(format!("Malformed multipart body: {e}")))?
    {
        let part_type = field.content_type().map(|m| m.essence_str().to_string());
        let bytes = field
            .bytes()
            .await
            .map_err(|e| ApiError::invalid_argument(format!("Malformed multipart body: {e}")))?;
        if metadata.is_none() {
            metadata =
                Some(serde_json::from_slice(&bytes).map_err(|e| {
                    ApiError::invalid_argument(format!("Invalid metadata JSON: {e}"))
                })?);
        } else if content.is_none() {
            content = Some((bytes.to_vec(), part_type));
        }
    }
    let metadata = metadata.ok_or_else(|| ApiError::invalid_argument("Missing metadata part."))?;
    let (bytes, part_type) =
        content.ok_or_else(|| ApiError::invalid_argument("Missing content part."))?;
    Ok((metadata, bytes, part_type))
}

/// `PATCH /upload/drive/v3/files/{id}?uploadType=media`: the body replaces the content.
async fn update_file(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(id): Path<String>,
    Query(q): Params,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    let (bytes, mime) = match q.get("uploadType").map(String::as_str) {
        Some("media") => (body.to_vec(), Some(content_type_of(&headers))),
        Some("multipart") => {
            let content_type = headers
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let (_, bytes, part_type) = parse_related(content_type, body).await?;
            (bytes, part_type)
        }
        other => {
            return Err(ApiError::invalid_argument(format!(
                "Invalid value for uploadType: {} (expected media or multipart)",
                other.unwrap_or("")
            )))
        }
    };
    let mut st = state.write();
    let file = st
        .drive_file_mut(&caller.user, &id)
        .ok_or_else(|| not_found(&id))?;
    file.bytes = bytes;
    if let Some(m) = mime {
        file.content_type = m;
    }
    file.modified_time = crate::state::Store::task_time(Utc::now());
    Ok(Json(file.to_json()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_queries_filter_by_name() {
        assert_eq!(
            name_in_query("name='prism-notes.json'").as_deref(),
            Some("prism-notes.json")
        );
        assert_eq!(
            name_in_query("name = \"notes\" and trashed = false").as_deref(),
            Some("notes")
        );
        assert_eq!(name_in_query("name='it\\'s'").as_deref(), Some("it's"));
        assert!(name_in_query("trashed = false").is_none());
        assert!(name_in_query("name = notes").is_none());
    }
}
