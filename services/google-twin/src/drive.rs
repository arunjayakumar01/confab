//! Google Drive API v3 subset, in memory. Two halves:
//! the app-data folder (notes): `files.list` with `spaces=appDataFolder` filtered by
//! `name='...'`, `files.get` with `alt=media`, a `multipart/related` create and a media update;
//! and the user's Drive (needs the `drive` scope): `files.list` with a `q` of `and`-joined
//! terms, folder create, upload into a folder, metadata `PATCH` with `addParents` /
//! `removeParents`, `export` as PDF and `drives.list`. Bearer required, Google-shaped errors.

use crate::auth::Caller;
use crate::chat::boundary_of;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, DriveFile, Store, DRIVE_FOLDER};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use chrono::Utc;
use confab_proto::DRIVE_SCOPE;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

type Params = Query<HashMap<String, String>>;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/drive/v3/files", get(list_files).post(create_metadata))
        .route("/drive/v3/files/{id}", get(get_file).patch(patch_file))
        .route("/drive/v3/files/{id}/export", get(export_file))
        .route("/drive/v3/drives", get(list_drives))
        .route("/upload/drive/v3/files", post(create_file))
        .route("/upload/drive/v3/files/{id}", patch(update_file))
}

/// The metadata of a create: `name`, `mimeType` and `parents` (`appDataFolder` for app
/// data, otherwise one folder id or `root`, the default).
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct FileMetadata {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    parents: Vec<String>,
}

impl FileMetadata {
    fn is_app_data(&self) -> bool {
        self.parents.iter().any(|p| p == "appDataFolder")
    }
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
    if q.get("spaces").map(String::as_str) != Some("appDataFolder") {
        return list_drive(&state, &caller, &q).map(Json);
    }
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
    let (file, app_data) = match st.drive_file(&caller.user, &id) {
        Some(f) => (f, true),
        None => {
            caller.require(DRIVE_SCOPE)?;
            let f = st
                .drive_files
                .get(&id)
                .filter(|f| f.visible_to(&caller.user))
                .ok_or_else(|| not_found(&id))?;
            (f, false)
        }
    };
    if !app_data && q.get("alt").map(String::as_str) != Some("media") {
        return Ok(Json(file.to_full_json(&st, &caller.user)).into_response());
    }
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
    let app_data = metadata.is_app_data();
    let parent = metadata
        .parents
        .first()
        .cloned()
        .unwrap_or_else(|| "root".into());
    let name = metadata
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Untitled".to_string());
    let mime = metadata
        .mime_type
        .filter(|m| !m.trim().is_empty())
        .or(part_type)
        .unwrap_or_else(|| "application/octet-stream".to_string());
    if app_data {
        let file = state
            .write()
            .new_drive_file(&caller.user, &name, &mime, content, Utc::now());
        return Ok(Json(file.to_json()));
    }
    caller.require(DRIVE_SCOPE)?;
    let mut st = state.write();
    check_parent(&st, &caller.user, &parent)?;
    let file = st.new_drive_item(
        DriveFile {
            owner: caller.user.clone(),
            name,
            content_type: mime,
            bytes: content,
            ..Default::default()
        },
        &parent,
        Utc::now(),
    );
    Ok(Json(file.to_full_json(&st, &caller.user)))
}

/// A query term of `files.list`'s `q`.
#[derive(Debug, PartialEq)]
enum Term {
    InParents(String),
    Trashed(bool),
    Starred(bool),
    SharedWithMe,
    NameContains(String),
    NameEq(String),
    MimeEq(String),
    MimeNe(String),
}

/// A quoted literal at the start of `s`: `(value, rest)`.
fn quoted(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    let quote = s.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
    let mut out = String::new();
    let mut escaped = false;
    for (i, c) in s.char_indices().skip(1) {
        match c {
            _ if escaped => {
                out.push(c);
                escaped = false;
            }
            '\\' => escaped = true,
            c if c == quote => return Some((out, &s[i + 1..])),
            c => out.push(c),
        }
    }
    None
}

/// `q` split on ` and ` outside quotes. Twin grammar: `'id' in parents`, `trashed = bool`,
/// `starred [= bool]`, `sharedWithMe`, `name contains|= '...'`, `fullText contains '...'`
/// (matched against names), `mimeType =|!= '...'`.
fn parse_query(q: &str) -> Result<Vec<Term>, String> {
    let mut parts = Vec::new();
    let (mut start, mut quote) = (0, None::<char>);
    let bytes: Vec<(usize, char)> = q.char_indices().collect();
    let mut i = 0;
    while i < bytes.len() {
        let (at, c) = bytes[i];
        match quote {
            Some(_) if c == '\\' => i += 1,
            Some(qc) if c == qc => quote = None,
            Some(_) => {}
            None if c == '\'' || c == '"' => quote = Some(c),
            None if q[at..].len() >= 5 && q[at..at + 5].eq_ignore_ascii_case(" and ") => {
                parts.push(&q[start..at]);
                start = at + 5;
                i += 4;
            }
            None => {}
        }
        i += 1;
    }
    parts.push(&q[start..]);
    let bool_of = |v: &str| match v.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("expected true or false, not {other}")),
    };
    let mut out = Vec::new();
    for raw in parts.into_iter().map(str::trim).filter(|p| !p.is_empty()) {
        let term = if let Some((id, _)) = quoted(raw).filter(|(_, r)| r.trim() == "in parents") {
            Term::InParents(id)
        } else if raw == "sharedWithMe" || raw == "sharedWithMe = true" {
            Term::SharedWithMe
        } else if raw == "starred" {
            Term::Starred(true)
        } else if let Some(v) = raw
            .strip_prefix("starred")
            .and_then(|r| r.trim_start().strip_prefix('='))
        {
            Term::Starred(bool_of(v)?)
        } else if let Some(v) = raw
            .strip_prefix("trashed")
            .and_then(|r| r.trim_start().strip_prefix('='))
        {
            Term::Trashed(bool_of(v)?)
        } else if let Some(rest) = raw
            .strip_prefix("name contains")
            .or_else(|| raw.strip_prefix("fullText contains"))
        {
            Term::NameContains(quoted(rest).ok_or("expected a quoted value")?.0)
        } else if let Some(rest) = raw
            .strip_prefix("name")
            .and_then(|r| r.trim_start().strip_prefix('='))
        {
            Term::NameEq(quoted(rest).ok_or("expected a quoted value")?.0)
        } else if let Some(rest) = raw
            .strip_prefix("mimeType")
            .and_then(|r| r.trim_start().strip_prefix("!="))
        {
            Term::MimeNe(quoted(rest).ok_or("expected a quoted value")?.0)
        } else if let Some(rest) = raw
            .strip_prefix("mimeType")
            .and_then(|r| r.trim_start().strip_prefix('='))
        {
            Term::MimeEq(quoted(rest).ok_or("expected a quoted value")?.0)
        } else {
            return Err(format!("unsupported term: {raw}"));
        };
        out.push(term);
    }
    Ok(out)
}

fn matches(f: &DriveFile, caller: &str, terms: &[Term]) -> bool {
    terms.iter().all(|t| match t {
        Term::InParents(p) if p == "root" => {
            f.owner == caller && f.drive_id.is_none() && f.parents.iter().any(|x| x == "root")
        }
        Term::InParents(p) => f.parents.iter().any(|x| x == p),
        Term::Trashed(b) => f.trashed == *b,
        Term::Starred(b) => f.starred == *b,
        Term::SharedWithMe => f.owner != caller && f.shared_with.iter().any(|u| u == caller),
        Term::NameContains(n) => f.name.to_lowercase().contains(&n.to_lowercase()),
        Term::NameEq(n) => &f.name == n,
        Term::MimeEq(m) => &f.content_type == m,
        Term::MimeNe(m) => &f.content_type != m,
    })
}

/// `files.list` over the caller's Drive: visible files matching `q`, `driveId` narrowing
/// to one shared drive, ordered by `orderBy` (`folder,name` or `modifiedTime desc`, else name).
/// note: one page holds everything; the twin's Drive is small.
fn list_drive(
    state: &AppState,
    caller: &Caller,
    q: &HashMap<String, String>,
) -> ApiResult<serde_json::Value> {
    caller.require(DRIVE_SCOPE)?;
    let terms = parse_query(q.get("q").map(String::as_str).unwrap_or(""))
        .map_err(|e| ApiError::invalid_argument(format!("Invalid Value: {e}")))?;
    let drive = q.get("driveId").filter(|d| !d.is_empty());
    let st = state.read();
    let mut files: Vec<&DriveFile> = st
        .drive_files
        .values()
        .filter(|f| f.visible_to(&caller.user))
        .filter(|f| drive.is_none_or(|d| f.drive_id.as_ref() == Some(d)))
        .filter(|f| matches(f, &caller.user, &terms))
        .collect();
    let order = q.get("orderBy").map(String::as_str).unwrap_or("");
    let folder_first = |f: &DriveFile| f.content_type != DRIVE_FOLDER;
    if order.starts_with("modifiedTime") {
        files.sort_by(|a, b| b.modified_time.cmp(&a.modified_time));
    } else if order.starts_with("folder") {
        files.sort_by_key(|f| (folder_first(f), f.name.to_lowercase()));
    } else {
        files.sort_by_key(|f| f.name.to_lowercase());
    }
    Ok(serde_json::json!({
        "kind": "drive#fileList",
        "incompleteSearch": false,
        "files": files.iter().map(|f| f.to_full_json(&st, &caller.user)).collect::<Vec<_>>(),
    }))
}

/// `parent` is `root`, a shared drive, or a folder the caller can see and add to.
fn check_parent(st: &Store, caller: &str, parent: &str) -> ApiResult<()> {
    if parent == "root" || st.shared_drives.contains_key(parent) {
        return Ok(());
    }
    match st.drive_files.get(parent) {
        Some(p) if p.visible_to(caller) && p.content_type == DRIVE_FOLDER => Ok(()),
        _ => Err(not_found(parent)),
    }
}

/// `POST /drive/v3/files`: metadata only, a folder or an empty file.
async fn create_metadata(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Json(meta): Json<FileMetadata>,
) -> ApiResult<Json<serde_json::Value>> {
    caller.require(DRIVE_SCOPE)?;
    let parent = meta
        .parents
        .first()
        .cloned()
        .unwrap_or_else(|| "root".into());
    let name = meta
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Untitled".into());
    let mut st = state.write();
    check_parent(&st, &caller.user, &parent)?;
    let file = st.new_drive_item(
        DriveFile {
            owner: caller.user.clone(),
            name,
            content_type: meta
                .mime_type
                .unwrap_or_else(|| "application/octet-stream".into()),
            ..Default::default()
        },
        &parent,
        Utc::now(),
    );
    Ok(Json(file.to_full_json(&st, &caller.user)))
}

#[derive(Deserialize, Default)]
struct FilePatch {
    name: Option<String>,
    starred: Option<bool>,
    trashed: Option<bool>,
}

/// `PATCH /drive/v3/files/{id}`: rename, star, trash or restore, and move with
/// `addParents` / `removeParents`. Starring is open to anyone who sees the file
/// (Google keeps stars per user; the twin keeps one).
async fn patch_file(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(id): Path<String>,
    Query(q): Params,
    Json(patch): Json<FilePatch>,
) -> ApiResult<Json<serde_json::Value>> {
    caller.require(DRIVE_SCOPE)?;
    let mut st = state.write();
    let file = st
        .drive_files
        .get(&id)
        .filter(|f| f.visible_to(&caller.user))
        .cloned()
        .ok_or_else(|| not_found(&id))?;
    let editor = file.owner == caller.user || file.drive_id.is_some();
    let edits = patch.name.is_some() || patch.trashed.is_some() || q.contains_key("addParents");
    if edits && !editor {
        return Err(ApiError::permission_denied(
            "The user does not have sufficient permissions for this file.",
        ));
    }
    let add = q.get("addParents").filter(|p| !p.is_empty()).cloned();
    if let Some(p) = &add {
        if p == &id {
            return Err(ApiError::invalid_argument(
                "A file cannot be its own parent.",
            ));
        }
        check_parent(&st, &caller.user, p)?;
    }
    let drive_of_add = add.as_ref().map(|p| {
        if st.shared_drives.contains_key(p) {
            Some(p.clone())
        } else {
            st.drive_files.get(p).and_then(|f| f.drive_id.clone())
        }
    });
    let f = st.drive_files.get_mut(&id).expect("checked above");
    if let Some(n) = patch.name.filter(|n| !n.trim().is_empty()) {
        f.name = n;
    }
    if let Some(s) = patch.starred {
        f.starred = s;
    }
    if let Some(t) = patch.trashed {
        f.trashed = t;
    }
    if let Some(rm) = q.get("removeParents") {
        let rm: Vec<&str> = rm.split(',').collect();
        f.parents.retain(|p| !rm.contains(&p.as_str()));
    }
    if let Some(p) = add {
        if !f.parents.contains(&p) {
            f.parents.push(p);
        }
        f.drive_id = drive_of_add.flatten();
    }
    f.modified_time = Store::task_time(Utc::now());
    let f = f.clone();
    Ok(Json(f.to_full_json(&st, &caller.user)))
}

/// `GET /drive/v3/files/{id}/export?mimeType=application/pdf` for Google Docs, Sheets and
/// Slides: a one-page PDF with the file's name, which is all the twin knows of it.
async fn export_file(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(id): Path<String>,
    Query(q): Params,
) -> ApiResult<Response> {
    caller.require(DRIVE_SCOPE)?;
    let st = state.read();
    let file = st
        .drive_files
        .get(&id)
        .filter(|f| f.visible_to(&caller.user))
        .ok_or_else(|| not_found(&id))?;
    if !file
        .content_type
        .starts_with("application/vnd.google-apps.")
        || file.content_type == DRIVE_FOLDER
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "PERMISSION_DENIED",
            "Export only supports Docs Editors files.",
        ));
    }
    if q.get("mimeType").map(String::as_str) != Some("application/pdf") {
        return Err(ApiError::invalid_argument(
            "The twin exports application/pdf only.",
        ));
    }
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/pdf".to_string())],
        tiny_pdf(&file.name),
    )
        .into_response())
}

/// A valid single-page PDF that prints `title`.
pub(crate) fn tiny_pdf(title: &str) -> Vec<u8> {
    let safe: String = title
        .chars()
        .filter(|c| c.is_ascii() && !matches!(c, '(' | ')' | '\\'))
        .collect();
    let stream = format!("BT /F1 24 Tf 72 720 Td ({safe}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// `drives.list`: every shared drive (each twin user is a member of all of them).
async fn list_drives(
    State(state): State<Arc<AppState>>,
    caller: Caller,
) -> ApiResult<Json<serde_json::Value>> {
    caller.require(DRIVE_SCOPE)?;
    let st = state.read();
    let drives: Vec<_> = st
        .shared_drives
        .iter()
        .map(|(id, name)| serde_json::json!({ "kind": "drive#drive", "id": id, "name": name }))
        .collect();
    Ok(Json(
        serde_json::json!({ "kind": "drive#driveList", "drives": drives }),
    ))
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

    #[test]
    fn drive_query_terms() {
        assert_eq!(
            parse_query("'f1' in parents and trashed = false").unwrap(),
            vec![Term::InParents("f1".into()), Term::Trashed(false)]
        );
        assert_eq!(
            parse_query("fullText contains 'tom and jerry' and starred").unwrap(),
            vec![
                Term::NameContains("tom and jerry".into()),
                Term::Starred(true)
            ]
        );
        assert_eq!(
            parse_query("sharedWithMe and mimeType != 'application/vnd.google-apps.folder'")
                .unwrap(),
            vec![Term::SharedWithMe, Term::MimeNe(DRIVE_FOLDER.into())]
        );
        assert_eq!(
            parse_query("name contains 'it\\'s'").unwrap(),
            vec![Term::NameContains("it's".into())]
        );
        assert!(parse_query("").unwrap().is_empty());
        assert!(parse_query("owners in 'x'").is_err());
        let pdf = tiny_pdf("Q4 (plan)");
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF\n"));
    }
}
