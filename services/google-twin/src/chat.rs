//! Google Chat API v1 subset under `/v1/...` and `/upload/v1/...`.
//! Every handler: Bearer required, Google-shaped errors, confab_proto bodies.

use crate::auth::Caller;
use crate::error::{ApiError, ApiResult};
use crate::seed::attachment_for;
use crate::state::{AppState, Blob, Store};
use crate::util::{now_str, parse_time, random_id, user_id};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use confab_proto::chat::{
    Annotation, Attachment, ListMembershipsResponse, ListMessagesResponse, ListReactionsResponse,
    ListSpacesResponse, Membership, Message, QuotedMessageMetadata, Reaction,
    SearchMessagesResponse, Space, SpaceReadState, Thread, UploadAttachmentResponse, User,
};
use confab_proto::events::EventData;
use confab_proto::relay::event_types;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

type AppRouter = Router<Arc<AppState>>;
type Params = Query<HashMap<String, String>>;

pub fn routes() -> AppRouter {
    Router::new()
        .route("/v1/spaces", get(list_spaces))
        .route("/v1/spaces:setup", post(setup_space))
        .route("/v1/spaces/-/messages:search", get(search_messages))
        .route(
            "/v1/people:searchDirectoryPeople",
            get(search_directory_people),
        )
        .route("/v1/spaces/{s}", get(get_space))
        .route(
            "/v1/spaces/{s}/members",
            get(list_members).post(create_member),
        )
        .route("/v1/spaces/{s}/members/users/{id}", get(get_member_by_user))
        .route(
            "/v1/spaces/{s}/members/{m}",
            get(get_member).delete(delete_member),
        )
        .route(
            "/v1/spaces/{s}/messages",
            get(list_messages).post(create_message),
        )
        .route(
            "/v1/spaces/{s}/messages/{m}",
            get(get_message)
                .patch(update_message)
                .put(update_message)
                .delete(delete_message),
        )
        .route(
            "/v1/spaces/{s}/messages/{m}/attachments/{a}",
            get(get_attachment),
        )
        .route(
            "/v1/spaces/{s}/messages/{m}/reactions",
            get(list_reactions).post(create_reaction),
        )
        .route(
            "/v1/spaces/{s}/messages/{m}/reactions/{r}",
            delete(delete_reaction),
        )
        .route(
            "/v1/users/{u}/spaces/{s}/spaceReadState",
            get(get_read_state).patch(patch_read_state),
        )
        .route(
            "/upload/v1/spaces/{s}/attachments:upload",
            post(upload_attachment),
        )
        .route("/v1/media/{*resource}", get(download_media))
}

// ---- request bodies -----------------------------------------------------------
// The client posts partial resources (`{text, thread?}`) without a `name`, while
// `confab_proto::chat::Message.name` is required, so inputs use these local shapes.
// Responses are always the confab_proto resources.

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MessageInput {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thread: Option<Thread>,
    #[serde(default)]
    attachment: Vec<Attachment>,
    /// Google computes mentions itself; a `RICH_LINK` (a huddle) arrives in the request and is kept.
    #[serde(default)]
    annotations: Vec<Annotation>,
    #[serde(default)]
    quoted_message_metadata: Option<QuotedMessageMetadata>,
    #[serde(default)]
    client_assigned_message_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ReadStateInput {
    #[serde(default)]
    last_read_time: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MembershipInput {
    #[serde(default)]
    member: Option<User>,
    #[serde(default)]
    role: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SpaceInput {
    #[serde(default)]
    space_type: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SetupInput {
    #[serde(default)]
    space: Option<SpaceInput>,
    #[serde(default)]
    memberships: Vec<MembershipInput>,
}

// ---- helpers -------------------------------------------------------------------

fn space_name(s: &str) -> String {
    format!("spaces/{s}")
}

/// 404 when the space does not exist, 403 when the caller is not a member.
fn space_for(st: &Store, caller: &str, space: &str) -> ApiResult<Space> {
    let s = st
        .spaces
        .get(space)
        .ok_or_else(|| ApiError::not_found(format!("Space {space} not found.")))?;
    if !st.is_member(space, caller) {
        return Err(ApiError::permission_denied(format!(
            "User {caller} does not have permission to access space {space}."
        )));
    }
    Ok(s.clone())
}

fn message_for(st: &Store, caller: &str, space: &str, id: &str) -> ApiResult<Message> {
    space_for(st, caller, space)?;
    let name = st
        .resolve_message(space, id)
        .ok_or_else(|| ApiError::not_found(format!("Message {space}/messages/{id} not found.")))?;
    Ok(st.messages[&name].clone())
}

/// (pageSize, offset) from `pageSize`/`pageToken`. Tokens are plain offsets.
// note: offset tokens instead of opaque cursors; fine for a few hundred rows.
fn paging(q: &HashMap<String, String>, default: usize, max: usize) -> ApiResult<(usize, usize)> {
    let size = match q
        .get("pageSize")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(s) => s
            .parse::<usize>()
            .map_err(|_| ApiError::invalid_argument("Invalid value for pageSize."))?,
        None => 0,
    };
    let size = if size == 0 { default } else { size.min(max) };
    let offset = match q
        .get("pageToken")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(t) => t
            .parse::<usize>()
            .map_err(|_| ApiError::invalid_argument("Invalid value for pageToken."))?,
        None => 0,
    };
    Ok((size, offset))
}

fn page<T: Clone>(items: &[T], size: usize, offset: usize) -> (Vec<T>, Option<String>) {
    let end = (offset + size).min(items.len());
    let slice = if offset >= items.len() {
        Vec::new()
    } else {
        items[offset..end].to_vec()
    };
    let next = (end < items.len()).then(|| end.to_string());
    (slice, next)
}

#[derive(Debug, PartialEq)]
struct Clause {
    field: String,
    op: String,
    value: String,
}

/// Google's list filter grammar, the subset Prism uses: `field op "value"`
/// clauses joined by `AND`. Values may be quoted with `"` or unquoted.
// note: no OR, no parentheses.
fn parse_filter(filter: &str) -> ApiResult<Vec<Clause>> {
    let mut out = Vec::new();
    let lower = filter.to_ascii_lowercase();
    let mut start = 0;
    let mut parts: Vec<&str> = Vec::new();
    let mut i = 0;
    while let Some(pos) = lower[i..].find(" and ") {
        let abs = i + pos;
        parts.push(&filter[start..abs]);
        start = abs + 5;
        i = start;
    }
    parts.push(&filter[start..]);
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let ops = [">=", "<=", "!=", "=", ">", "<", ":"];
        let Some((op_idx, op)) = ops
            .iter()
            .filter_map(|op| part.find(op).map(|idx| (idx, *op)))
            .min_by_key(|(idx, _)| *idx)
        else {
            return Err(ApiError::invalid_argument(format!(
                "Invalid filter clause: {part}"
            )));
        };
        let field = part[..op_idx].trim().to_string();
        let value = part[op_idx + op.len()..]
            .trim()
            .trim_matches('"')
            .to_string();
        if field.is_empty() {
            return Err(ApiError::invalid_argument(format!(
                "Invalid filter clause: {part}"
            )));
        }
        out.push(Clause {
            field,
            op: op.to_string(),
            value,
        });
    }
    Ok(out)
}

fn push_message(state: &AppState, event_type: &str, msg: &Message) {
    state.pusher.push(
        event_type,
        msg.space_name(),
        EventData {
            message: Some(msg.clone()),
            ..Default::default()
        },
    );
}

fn push_reaction(state: &AppState, event_type: &str, space: &str, reaction: &Reaction) {
    state.pusher.push(
        event_type,
        space,
        EventData {
            reaction: Some(reaction.clone()),
            ..Default::default()
        },
    );
}

fn push_membership(state: &AppState, event_type: &str, space: &str, membership: &Membership) {
    state.pusher.push(
        event_type,
        space,
        EventData {
            membership: Some(membership.clone()),
            ..Default::default()
        },
    );
}

fn resolve_attachments(
    st: &Store,
    space: &str,
    message: &str,
    inputs: &[Attachment],
) -> ApiResult<Vec<Attachment>> {
    let mut out = Vec::new();
    for a in inputs {
        let resource = a
            .attachment_data_ref
            .as_ref()
            .and_then(|r| r.resource_name.as_deref())
            .ok_or_else(|| {
                ApiError::invalid_argument("attachment.attachmentDataRef.resourceName is required.")
            })?;
        let blob = st
            .blobs
            .get(resource)
            .filter(|b| b.space == space)
            .ok_or_else(|| {
                ApiError::invalid_argument(format!("Unknown attachment resource {resource}."))
            })?;
        let id = resource.rsplit('/').next().unwrap_or("att");
        out.push(attachment_for(
            &st.public_base,
            message,
            resource,
            &blob.filename,
            &blob.content_type,
            id,
        ));
    }
    Ok(out)
}

fn empty_ok() -> Json<serde_json::Value> {
    Json(serde_json::json!({}))
}

// ---- spaces --------------------------------------------------------------------

async fn list_spaces(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Query(q): Params,
) -> ApiResult<Json<ListSpacesResponse>> {
    let (size, offset) = paging(&q, 100, 1000)?;
    let clauses = parse_filter(q.get("filter").map(String::as_str).unwrap_or(""))?;
    let st = state.read();
    let mut spaces = st.spaces_for(&caller.user);
    for c in clauses {
        match c.field.as_str() {
            "spaceType" | "space_type" => {
                spaces.retain(|s| s.space_type.as_deref() == Some(c.value.as_str()))
            }
            other => {
                return Err(ApiError::invalid_argument(format!(
                    "Unsupported filter field: {other}"
                )))
            }
        }
    }
    let (spaces, next_page_token) = page(&spaces, size, offset);
    Ok(Json(ListSpacesResponse {
        spaces,
        next_page_token,
    }))
}

async fn get_space(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(s): Path<String>,
) -> ApiResult<Json<Space>> {
    let st = state.read();
    Ok(Json(space_for(&st, &caller.user, &space_name(&s))?))
}

/// `spaces.setup`: find-or-create a DM, create a group chat with everyone (caller
/// included) as a member, or create a named space with the caller as manager. New
/// memberships push `membership.created` like `create_member`.
// note: the Google requestId is not handled; nobody asks for it yet.
async fn setup_space(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Json(body): Json<SetupInput>,
) -> ApiResult<Json<Space>> {
    let input = body.space.unwrap_or_default();
    let members = body
        .memberships
        .iter()
        .map(|m| {
            m.member
                .as_ref()
                .map(|u| u.name.clone())
                .filter(|n| !n.is_empty())
                .ok_or_else(|| ApiError::invalid_argument("memberships[].member.name is required."))
        })
        .collect::<ApiResult<Vec<String>>>()?;
    let now = now_str();
    let (space, created) = {
        let mut st = state.write();
        if let Some(missing) = members.iter().find(|m| !st.users.contains_key(*m)) {
            return Err(ApiError::not_found(format!("User {missing} not found.")));
        }
        let (name, roles): (String, Vec<(&str, &str)>) = match input.space_type.as_deref() {
            Some("DIRECT_MESSAGE") => {
                let [other] = members.as_slice() else {
                    return Err(ApiError::invalid_argument(
                        "A direct message requires exactly one membership.",
                    ));
                };
                let name = Store::dm_name(&caller.user, other);
                if let Some(existing) = st.spaces.get(&name) {
                    return Ok(Json(existing.clone()));
                }
                st.new_space(&name, "DIRECT_MESSAGE", None, "", &now);
                (
                    name,
                    vec![
                        (caller.user.as_str(), "ROLE_MEMBER"),
                        (other, "ROLE_MEMBER"),
                    ],
                )
            }
            Some("SPACE") => {
                let display = input
                    .display_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|d| !d.is_empty())
                    .ok_or_else(|| ApiError::invalid_argument("displayName is required"))?;
                if st.spaces.values().any(|s| {
                    s.display_name
                        .as_deref()
                        .is_some_and(|d| d.eq_ignore_ascii_case(display))
                }) {
                    return Err(ApiError::already_exists(format!(
                        "A space named {display} already exists."
                    )));
                }
                // note: Google ids are opaque; the seed's AAAA prefix plus seven random chars looks the part.
                let name = format!("spaces/AAAA{}", &random_id()[..7]);
                st.new_space(&name, "SPACE", Some(display), "", &now);
                let mut roles = vec![(caller.user.as_str(), "ROLE_MANAGER")];
                roles.extend(members.iter().map(|m| (m.as_str(), "ROLE_MEMBER")));
                (name, roles)
            }
            Some("GROUP_CHAT") => {
                if members.len() < 2 {
                    return Err(ApiError::invalid_argument(
                        "A group chat requires at least two memberships.",
                    ));
                }
                // note: same opaque-looking id as SPACE; the g marks a group chat in the seed's style.
                let name = format!("spaces/AAAAg{}", &random_id()[..6]);
                st.new_space(&name, "GROUP_CHAT", None, "", &now);
                let mut roles = vec![(caller.user.as_str(), "ROLE_MEMBER")];
                roles.extend(members.iter().map(|m| (m.as_str(), "ROLE_MEMBER")));
                (name, roles)
            }
            other => {
                return Err(ApiError::invalid_argument(format!(
                    "Unsupported spaceType: {}",
                    other.unwrap_or("(none)")
                )))
            }
        };
        let mut created = Vec::new();
        for (user, role) in roles {
            if !st.is_member(&name, user) {
                created.push(st.add_member(&name, user, role, &now));
            }
        }
        (st.spaces[&name].clone(), created)
    };
    for m in &created {
        push_membership(&state, event_types::MEMBERSHIP_CREATED, &space.name, m);
    }
    Ok(Json(space))
}

// ---- people --------------------------------------------------------------------

/// People API `people.searchDirectoryPeople`, the subset the member picker needs.
// note: readMask and sources are accepted and ignored; one page, no nextPageToken.
async fn search_directory_people(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Query(q): Params,
) -> ApiResult<Json<serde_json::Value>> {
    let query = q
        .get("query")
        .map(|s| s.trim().to_lowercase())
        .ok_or_else(|| ApiError::invalid_argument("query is required."))?;
    let (size, _) = paging(&q, 100, 500)?;
    let st = state.read();
    let people: Vec<serde_json::Value> = if query.is_empty() {
        Vec::new()
    } else {
        st.users
            .values()
            .filter(|u| u.name != caller.user)
            .filter(|u| {
                [u.display_name.as_deref(), u.email.as_deref()]
                    .into_iter()
                    .flatten()
                    .any(|f| f.to_lowercase().contains(&query))
            })
            .take(size)
            .map(|u| {
                serde_json::json!({
                    "resourceName": format!("people/{}", user_id(&u.name)),
                    "names": [{"displayName": u.display_name}],
                    "emailAddresses": [{"value": u.email}],
                    "photos": [{"url": u.avatar_url}],
                })
            })
            .collect()
    };
    Ok(Json(serde_json::json!({ "people": people })))
}

// ---- members -------------------------------------------------------------------

async fn list_members(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(s): Path<String>,
    Query(q): Params,
) -> ApiResult<Json<ListMembershipsResponse>> {
    let (size, offset) = paging(&q, 100, 1000)?;
    let space = space_name(&s);
    let st = state.read();
    space_for(&st, &caller.user, &space)?;
    let members = st.members_of(&space);
    let (memberships, next_page_token) = page(&members, size, offset);
    Ok(Json(ListMembershipsResponse {
        memberships,
        next_page_token,
    }))
}

fn membership_of(st: &Store, caller: &str, space: &str, target: &str) -> ApiResult<Membership> {
    space_for(st, caller, space)?;
    st.memberships
        .get(&Store::membership_name(space, target))
        .cloned()
        .ok_or_else(|| {
            ApiError::not_found(format!("Membership for {target} in {space} not found."))
        })
}

async fn get_member_by_user(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((s, id)): Path<(String, String)>,
) -> ApiResult<Json<Membership>> {
    let target = if id == "me" {
        caller.user.clone()
    } else {
        format!("users/{id}")
    };
    let st = state.read();
    Ok(Json(membership_of(
        &st,
        &caller.user,
        &space_name(&s),
        &target,
    )?))
}

async fn get_member(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((s, m)): Path<(String, String)>,
) -> ApiResult<Json<Membership>> {
    let st = state.read();
    Ok(Json(membership_of(
        &st,
        &caller.user,
        &space_name(&s),
        &format!("users/{m}"),
    )?))
}

async fn create_member(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(s): Path<String>,
    Json(body): Json<MembershipInput>,
) -> ApiResult<Json<Membership>> {
    let space = space_name(&s);
    let membership = {
        let mut st = state.write();
        let sp = space_for(&st, &caller.user, &space)?;
        if sp.space_type.as_deref() == Some("DIRECT_MESSAGE") {
            return Err(ApiError::invalid_argument(
                "Cannot add members to a direct message.",
            ));
        }
        let target = body
            .member
            .as_ref()
            .map(|u| u.name.clone())
            .filter(|n| !n.is_empty())
            .ok_or_else(|| ApiError::invalid_argument("member.name is required."))?;
        if !st.users.contains_key(&target) {
            return Err(ApiError::not_found(format!("User {target} not found.")));
        }
        if st.is_member(&space, &target) {
            return Err(ApiError::already_exists(format!(
                "{target} is already a member of {space}."
            )));
        }
        let role = body.role.as_deref().unwrap_or("ROLE_MEMBER");
        let m = st.add_member(&space, &target, role, &now_str());
        st.touch_space(&space, &now_str());
        m
    };
    push_membership(&state, event_types::MEMBERSHIP_CREATED, &space, &membership);
    Ok(Json(membership))
}

async fn delete_member(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((s, m)): Path<(String, String)>,
) -> ApiResult<Json<Membership>> {
    let space = space_name(&s);
    let target = if m == "me" {
        caller.user.clone()
    } else {
        format!("users/{}", user_id(&m))
    };
    let removed = {
        let mut st = state.write();
        let sp = space_for(&st, &caller.user, &space)?;
        if sp.space_type.as_deref() == Some("DIRECT_MESSAGE") {
            return Err(ApiError::invalid_argument(
                "Cannot remove members from a direct message.",
            ));
        }
        // note: any member may remove any member; Google restricts this to managers and self.
        let mut removed = st.remove_member(&space, &target).ok_or_else(|| {
            ApiError::not_found(format!("Membership for {target} in {space} not found."))
        })?;
        removed.state = Some("NOT_A_MEMBER".into());
        removed
    };
    push_membership(&state, event_types::MEMBERSHIP_DELETED, &space, &removed);
    Ok(Json(removed))
}

// ---- messages ------------------------------------------------------------------

async fn list_messages(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(s): Path<String>,
    Query(q): Params,
) -> ApiResult<Json<ListMessagesResponse>> {
    let (size, offset) = paging(&q, 25, 1000)?;
    let clauses = parse_filter(q.get("filter").map(String::as_str).unwrap_or(""))?;
    let desc = q
        .get("orderBy")
        .map(|o| o.to_ascii_lowercase().contains("desc"))
        .unwrap_or(false);
    let space = space_name(&s);
    let st = state.read();
    space_for(&st, &caller.user, &space)?;
    let mut msgs: Vec<Message> = st.messages_in(&space).into_iter().cloned().collect();
    for c in &clauses {
        match (c.field.as_str(), c.op.as_str()) {
            ("thread.name", "=") => msgs.retain(|m| {
                m.thread.as_ref().and_then(|t| t.name.as_deref()) == Some(c.value.as_str())
            }),
            ("createTime" | "create_time", op) => {
                let bound = parse_time(&c.value).ok_or_else(|| {
                    ApiError::invalid_argument("createTime filter must be RFC 3339.")
                })?;
                msgs.retain(|m| {
                    let t = m.create_time.as_deref().and_then(parse_time);
                    match (t, op) {
                        (Some(t), ">") => t > bound,
                        (Some(t), ">=") => t >= bound,
                        (Some(t), "<") => t < bound,
                        (Some(t), "<=") => t <= bound,
                        (Some(t), "=") => t == bound,
                        _ => false,
                    }
                })
            }
            (field, _) => {
                return Err(ApiError::invalid_argument(format!(
                    "Unsupported filter: {field} {}",
                    c.op
                )))
            }
        }
    }
    if desc {
        msgs.reverse();
    }
    let (messages, next_page_token) = page(&msgs, size, offset);
    Ok(Json(ListMessagesResponse {
        messages,
        next_page_token,
    }))
}

async fn get_message(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((s, m)): Path<(String, String)>,
) -> ApiResult<Json<Message>> {
    let st = state.read();
    Ok(Json(message_for(&st, &caller.user, &space_name(&s), &m)?))
}

fn valid_client_id(id: &str) -> bool {
    id.len() <= 63
        && id.strip_prefix("client-").is_some_and(|rest| {
            !rest.is_empty()
                && rest
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
}

async fn create_message(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(s): Path<String>,
    Query(q): Params,
    Json(body): Json<MessageInput>,
) -> ApiResult<Json<Message>> {
    let space = space_name(&s);
    let client_id = q
        .get("messageId")
        .map(String::as_str)
        .or(body.client_assigned_message_id.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let reply_option = q
        .get("messageReplyOption")
        .map(String::as_str)
        .unwrap_or("MESSAGE_REPLY_OPTION_UNSPECIFIED");
    let text = body.text.clone().unwrap_or_default();
    let rich_links: Vec<Annotation> = body
        .annotations
        .iter()
        .filter(|a| a.annotation_type.as_deref() == Some(Annotation::RICH_LINK))
        .cloned()
        .collect();
    if text.trim().is_empty() && body.attachment.is_empty() && rich_links.is_empty() {
        return Err(ApiError::invalid_argument("Message cannot be empty."));
    }
    if text.chars().count() > 4096 {
        return Err(ApiError::invalid_argument(
            "Message text exceeds 4096 characters.",
        ));
    }

    let message = {
        let mut st = state.write();
        space_for(&st, &caller.user, &space)?;
        if let Some(cid) = &client_id {
            if !valid_client_id(cid) {
                return Err(ApiError::invalid_argument(
                    "messageId must start with client- and contain only lowercase letters, numbers and hyphens, up to 63 characters.",
                ));
            }
            if st.client_ids.contains_key(&format!("{space}|{cid}")) {
                return Err(ApiError::already_exists(format!(
                    "Message with id {cid} already exists in {space}."
                )));
            }
        }

        let requested_thread = body.thread.as_ref().and_then(|t| {
            t.name.clone().filter(|n| !n.is_empty()).or_else(|| {
                t.thread_key
                    .as_ref()
                    .map(|k| format!("{space}/threads/{k}"))
            })
        });
        let existing_thread = requested_thread.clone().filter(|name| {
            st.messages_in(&space)
                .iter()
                .any(|m| m.thread.as_ref().and_then(|t| t.name.as_deref()) == Some(name.as_str()))
        });
        // Google refuses a reply option without a thread to reply to (seen live 2026-11-06).
        if reply_option.starts_with("REPLY_MESSAGE") && requested_thread.is_none() {
            return Err(ApiError::invalid_argument(
                "The request does not specify which message to reply to.",
            ));
        }
        let reply_to: Option<String> = match reply_option {
            "REPLY_MESSAGE_FALLBACK_TO_NEW_THREAD" => existing_thread,
            "REPLY_MESSAGE_OR_FAIL" => Some(existing_thread.ok_or_else(|| {
                ApiError::not_found(format!(
                    "Thread {} not found.",
                    requested_thread.as_deref().unwrap_or("(none)")
                ))
            })?),
            "MESSAGE_REPLY_OPTION_UNSPECIFIED" => None,
            other => {
                return Err(ApiError::invalid_argument(format!(
                    "Invalid messageReplyOption: {other}"
                )))
            }
        };
        let thread_id = reply_to
            .as_deref()
            .and_then(|t| t.rsplit('/').next())
            .map(str::to_string);
        let (name, thread_id) = st.new_message_name(&space, thread_id.as_deref());
        let attachment = resolve_attachments(&st, &space, &name, &body.attachment)?;
        let now = now_str();
        let msg = Message {
            name: name.clone(),
            sender: st.users.get(&caller.user).cloned(),
            create_time: Some(now.clone()),
            last_update_time: None,
            delete_time: None,
            deletion_metadata: None,
            text: Some(text.clone()),
            formatted_text: None,
            thread: Some(Thread {
                name: Some(format!("{space}/threads/{thread_id}")),
                thread_key: None,
            }),
            thread_reply: Some(reply_to.is_some()),
            space: Some(Space {
                name: space.clone(),
                ..Default::default()
            }),
            attachment,
            emoji_reaction_summaries: Vec::new(),
            annotations: rich_links.into_iter().chain(st.annotate(&text)).collect(),
            quoted_message_metadata: body.quoted_message_metadata.clone(),
            client_assigned_message_id: client_id.clone(),
        };
        st.messages.insert(name.clone(), msg.clone());
        if let Some(cid) = &client_id {
            st.client_ids.insert(format!("{space}|{cid}"), name);
        }
        st.touch_space(&space, &now);
        msg
    };
    push_message(&state, event_types::MESSAGE_CREATED, &message);
    Ok(Json(message))
}

async fn update_message(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((s, m)): Path<(String, String)>,
    Query(q): Params,
    Json(body): Json<MessageInput>,
) -> ApiResult<Json<Message>> {
    let space = space_name(&s);
    let mask: Vec<String> = q
        .get("updateMask")
        .map(|m| {
            m.split(',')
                .map(|f| f.trim().to_string())
                .filter(|f| !f.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if mask.is_empty() {
        return Err(ApiError::invalid_argument("updateMask is required."));
    }
    let all = mask.iter().any(|f| f == "*");
    for f in &mask {
        if !matches!(f.as_str(), "*" | "text" | "attachment") {
            return Err(ApiError::invalid_argument(format!(
                "Unsupported updateMask field: {f}"
            )));
        }
    }
    let message = {
        let mut st = state.write();
        let current = message_for(&st, &caller.user, &space, &m)?;
        if current.sender.as_ref().map(|u| u.name.as_str()) != Some(caller.user.as_str()) {
            return Err(ApiError::permission_denied(
                "Only the sender can update a message.",
            ));
        }
        let mut updated = current.clone();
        if all || mask.iter().any(|f| f == "text") {
            let text = body.text.clone().unwrap_or_default();
            if text.trim().is_empty() && updated.attachment.is_empty() {
                return Err(ApiError::invalid_argument("Message cannot be empty."));
            }
            updated.annotations = updated
                .annotations
                .iter()
                .filter(|a| a.annotation_type.as_deref() == Some(Annotation::RICH_LINK))
                .cloned()
                .chain(st.annotate(&text))
                .collect();
            updated.text = Some(text);
        }
        if all || mask.iter().any(|f| f == "attachment") {
            updated.attachment = resolve_attachments(&st, &space, &updated.name, &body.attachment)?;
        }
        updated.last_update_time = Some(now_str());
        st.messages.insert(updated.name.clone(), updated.clone());
        updated
    };
    push_message(&state, event_types::MESSAGE_UPDATED, &message);
    Ok(Json(message))
}

async fn delete_message(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((s, m)): Path<(String, String)>,
    Query(q): Params,
) -> ApiResult<Json<serde_json::Value>> {
    let space = space_name(&s);
    let force = q.get("force").is_some_and(|v| v == "true" || v == "1");
    let deleted: Vec<Message> = {
        let mut st = state.write();
        let current = message_for(&st, &caller.user, &space, &m)?;
        let is_sender =
            current.sender.as_ref().map(|u| u.name.as_str()) == Some(caller.user.as_str());
        let is_manager = st
            .memberships
            .get(&Store::membership_name(&space, &caller.user))
            .and_then(|mm| mm.role.clone())
            .is_some_and(|r| r == "ROLE_MANAGER");
        if !is_sender && !is_manager {
            return Err(ApiError::permission_denied(
                "Only the sender or a space manager can delete a message.",
            ));
        }
        let thread_name = current
            .thread
            .as_ref()
            .and_then(|t| t.name.clone())
            .unwrap_or_default();
        let replies = if current.thread_reply == Some(false) {
            st.thread_replies(&space, &thread_name, &current.name)
        } else {
            Vec::new()
        };
        if !replies.is_empty() && !force {
            return Err(ApiError::failed_precondition(
                "Message has threaded replies. Set force=true to delete the message and its replies.",
            ));
        }
        let mut names = replies;
        names.push(current.name.clone());
        let now = now_str();
        let mut out = Vec::new();
        for name in names {
            let Some(mut msg) = st.messages.remove(&name) else {
                continue;
            };
            st.reactions.retain(|r| {
                !r.name
                    .as_deref()
                    .is_some_and(|n| n.starts_with(&format!("{name}/reactions/")))
            });
            st.client_ids.retain(|_, v| v != &name);
            msg.text = None;
            msg.formatted_text = None;
            msg.annotations.clear();
            msg.attachment.clear();
            msg.emoji_reaction_summaries.clear();
            msg.delete_time = Some(now.clone());
            out.push(msg);
        }
        out
    };
    for msg in &deleted {
        push_message(&state, event_types::MESSAGE_DELETED, msg);
    }
    Ok(empty_ok())
}

async fn get_attachment(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((s, m, a)): Path<(String, String, String)>,
) -> ApiResult<Json<Attachment>> {
    let st = state.read();
    let msg = message_for(&st, &caller.user, &space_name(&s), &m)?;
    msg.attachment
        .iter()
        .find(|att| {
            att.name
                .as_deref()
                .is_some_and(|n| n.ends_with(&format!("/attachments/{a}")))
        })
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("Attachment {a} not found.")))
}

// ---- reactions -----------------------------------------------------------------

async fn list_reactions(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((s, m)): Path<(String, String)>,
    Query(q): Params,
) -> ApiResult<Json<ListReactionsResponse>> {
    let (size, offset) = paging(&q, 25, 200)?;
    let clauses = parse_filter(q.get("filter").map(String::as_str).unwrap_or(""))?;
    let st = state.read();
    let msg = message_for(&st, &caller.user, &space_name(&s), &m)?;
    let mut reactions = st.reactions_of(&msg.name);
    for c in clauses {
        match c.field.as_str() {
            "user.name" => {
                let who = if c.value == "users/me" {
                    caller.user.clone()
                } else {
                    c.value.clone()
                };
                reactions
                    .retain(|r| r.user.as_ref().map(|u| u.name.as_str()) == Some(who.as_str()));
            }
            "emoji.unicode" => {
                reactions.retain(|r| r.emoji.unicode.as_deref() == Some(c.value.as_str()))
            }
            other => {
                return Err(ApiError::invalid_argument(format!(
                    "Unsupported filter field: {other}"
                )))
            }
        }
    }
    let (reactions, next_page_token) = page(&reactions, size, offset);
    Ok(Json(ListReactionsResponse {
        reactions,
        next_page_token,
    }))
}

async fn create_reaction(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((s, m)): Path<(String, String)>,
    Json(body): Json<Reaction>,
) -> ApiResult<Json<Reaction>> {
    let space = space_name(&s);
    let unicode = body
        .emoji
        .unicode
        .clone()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| ApiError::invalid_argument("emoji.unicode is required."))?;
    let reaction = {
        let mut st = state.write();
        let msg = message_for(&st, &caller.user, &space, &m)?;
        let dup = st.reactions_of(&msg.name).iter().any(|r| {
            r.user.as_ref().map(|u| u.name.as_str()) == Some(caller.user.as_str())
                && r.emoji.unicode.as_deref() == Some(unicode.as_str())
        });
        if dup {
            return Err(ApiError::already_exists(
                "The user has already reacted with this emoji.",
            ));
        }
        let reaction = Reaction {
            name: Some(format!("{}/reactions/{}", msg.name, random_id())),
            user: st.users.get(&caller.user).cloned(),
            emoji: confab_proto::chat::Emoji {
                unicode: Some(unicode),
                custom_emoji: None,
            },
        };
        st.reactions.push(reaction.clone());
        st.recompute_summaries(&msg.name);
        reaction
    };
    push_reaction(&state, event_types::REACTION_CREATED, &space, &reaction);
    Ok(Json(reaction))
}

async fn delete_reaction(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((s, m, r)): Path<(String, String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let space = space_name(&s);
    let reaction = {
        let mut st = state.write();
        let msg = message_for(&st, &caller.user, &space, &m)?;
        let name = format!("{}/reactions/{r}", msg.name);
        let idx = st
            .reactions
            .iter()
            .position(|x| x.name.as_deref() == Some(name.as_str()))
            .ok_or_else(|| ApiError::not_found(format!("Reaction {name} not found.")))?;
        if st.reactions[idx].user.as_ref().map(|u| u.name.as_str()) != Some(caller.user.as_str()) {
            return Err(ApiError::permission_denied(
                "Only the reacting user can delete a reaction.",
            ));
        }
        let reaction = st.reactions.remove(idx);
        st.recompute_summaries(&msg.name);
        reaction
    };
    push_reaction(&state, event_types::REACTION_DELETED, &space, &reaction);
    Ok(empty_ok())
}

// ---- read state ----------------------------------------------------------------

fn read_state_user(caller: &str, u: &str) -> ApiResult<String> {
    if u == "me" || u == user_id(caller) {
        Ok(caller.to_string())
    } else {
        Err(ApiError::permission_denied(
            "Read state is only available for the authenticated user.",
        ))
    }
}

async fn get_read_state(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((u, s)): Path<(String, String)>,
) -> ApiResult<Json<SpaceReadState>> {
    let user = read_state_user(&caller.user, &u)?;
    let space = space_name(&s);
    let st = state.read();
    space_for(&st, &user, &space)?;
    Ok(Json(SpaceReadState {
        name: format!("{user}/spaces/{s}/spaceReadState"),
        last_read_time: st.read_states.get(&(user.clone(), space)).cloned(),
    }))
}

async fn patch_read_state(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path((u, s)): Path<(String, String)>,
    Query(q): Params,
    Json(body): Json<ReadStateInput>,
) -> ApiResult<Json<SpaceReadState>> {
    let user = read_state_user(&caller.user, &u)?;
    let space = space_name(&s);
    let mask_ok = q
        .get("updateMask")
        .map(|m| {
            m.split(',')
                .any(|f| matches!(f.trim(), "lastReadTime" | "last_read_time" | "*"))
        })
        .unwrap_or(false);
    if !mask_ok {
        return Err(ApiError::invalid_argument(
            "updateMask must include lastReadTime.",
        ));
    }
    let requested = body
        .last_read_time
        .as_deref()
        .ok_or_else(|| ApiError::invalid_argument("lastReadTime is required."))?;
    let mut t = parse_time(requested)
        .ok_or_else(|| ApiError::invalid_argument("lastReadTime must be RFC 3339."))?;
    let mut st = state.write();
    space_for(&st, &user, &space)?;
    // Google coerces a future lastReadTime to the latest message's createTime.
    if let Some(latest) = st
        .latest_message_time(&space)
        .as_deref()
        .and_then(parse_time)
    {
        if t > latest {
            t = latest;
        }
    }
    let value = st.fmt(t);
    st.read_states.insert((user.clone(), space), value.clone());
    Ok(Json(SpaceReadState {
        name: format!("{user}/spaces/{s}/spaceReadState"),
        last_read_time: Some(value),
    }))
}

// ---- search --------------------------------------------------------------------

async fn search_messages(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Query(q): Params,
) -> ApiResult<Json<SearchMessagesResponse>> {
    let (size, offset) = paging(&q, 25, 100)?;
    let query = q
        .get("query")
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::invalid_argument("query is required."))?;
    let st = state.read();
    let mut hits: Vec<Message> = st
        .spaces_for(&caller.user)
        .iter()
        .flat_map(|sp| st.messages_in(&sp.name))
        .filter(|m| {
            m.text
                .as_deref()
                .is_some_and(|t| t.to_lowercase().contains(&query))
        })
        .cloned()
        .collect();
    hits.sort_by(|a, b| b.create_time.cmp(&a.create_time));
    let (messages, next_page_token) = page(&hits, size, offset);
    Ok(Json(SearchMessagesResponse {
        messages,
        next_page_token,
    }))
}

// ---- attachments ---------------------------------------------------------------

fn guess_content_type(filename: &str) -> &'static str {
    match filename
        .rsplit('.')
        .next()
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("txt" | "md" | "log") => "text/plain",
        Some("json") => "application/json",
        Some("csv") => "text/csv",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

#[derive(Deserialize, Default)]
struct UploadMetadata {
    #[serde(default)]
    filename: Option<String>,
}

/// Boundary from any `multipart/*` content type: the browser and reqwest send
/// `multipart/form-data`, Google's own upload protocol is `multipart/related`.
pub(crate) fn boundary_of(content_type: &str) -> Option<String> {
    let ct = content_type.trim();
    if !ct.to_ascii_lowercase().starts_with("multipart/") {
        return None;
    }
    ct.split(';')
        .map(str::trim)
        .find_map(|p| {
            let (k, v) = p.split_once('=')?;
            k.trim()
                .eq_ignore_ascii_case("boundary")
                .then(|| v.trim().trim_matches('"').to_string())
        })
        .filter(|b| !b.is_empty())
}

async fn upload_attachment(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(s): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<UploadAttachmentResponse>> {
    let space = space_name(&s);
    {
        let st = state.read();
        space_for(&st, &caller.user, &space)?;
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let boundary = boundary_of(content_type).ok_or_else(|| {
        ApiError::invalid_argument("Expected a multipart body with a boundary parameter.")
    })?;
    let stream = futures::stream::once(async move { Ok::<Bytes, std::io::Error>(body) });
    let mut multipart = multer::Multipart::new(stream, boundary);
    let mut metadata: Option<UploadMetadata> = None;
    let mut file: Option<(Option<String>, Option<String>, Vec<u8>)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::invalid_argument(format!("Malformed multipart body: {e}")))?
    {
        let field_name = field.name().unwrap_or("").to_string();
        let part_filename = field.file_name().map(str::to_string);
        let part_type = field.content_type().map(|m| m.essence_str().to_string());
        let bytes = field
            .bytes()
            .await
            .map_err(|e| ApiError::invalid_argument(format!("Malformed multipart body: {e}")))?;
        let is_json = part_type.as_deref() == Some("application/json");
        let looks_like_metadata = field_name == "metadata"
            || (metadata.is_none() && field_name.is_empty() && part_filename.is_none() && is_json);
        if looks_like_metadata {
            metadata =
                Some(serde_json::from_slice(&bytes).map_err(|e| {
                    ApiError::invalid_argument(format!("Invalid metadata JSON: {e}"))
                })?);
        } else if file.is_none() {
            file = Some((part_filename, part_type, bytes.to_vec()));
        }
    }
    let (part_filename, part_type, bytes) =
        file.ok_or_else(|| ApiError::invalid_argument("Missing file part."))?;
    let filename = metadata
        .and_then(|m| m.filename)
        .or(part_filename)
        .filter(|f| !f.trim().is_empty())
        .unwrap_or_else(|| "upload.bin".into());
    let content_type = part_type
        .filter(|t| !t.is_empty() && t != "application/octet-stream")
        .unwrap_or_else(|| guess_content_type(&filename).to_string());
    let resource = format!("{space}/attachments/{}", uuid::Uuid::new_v4().simple());
    state.write().blobs.insert(
        resource.clone(),
        Blob {
            space,
            filename,
            content_type,
            bytes,
        },
    );
    Ok(Json(UploadAttachmentResponse {
        attachment_data_ref: Some(confab_proto::chat::AttachmentDataRef {
            resource_name: Some(resource),
            attachment_upload_token: Some(crate::util::random_token()),
        }),
    }))
}

async fn download_media(
    State(state): State<Arc<AppState>>,
    caller: Caller,
    Path(resource): Path<String>,
) -> Result<Response, ApiError> {
    let st = state.read();
    let blob = st
        .blobs
        .get(&resource)
        .ok_or_else(|| ApiError::not_found(format!("Media {resource} not found.")))?;
    if !st.is_member(&blob.space, &caller.user) {
        return Err(ApiError::permission_denied(
            "The caller does not have permission to download this media.",
        ));
    }
    let disposition = format!("inline; filename=\"{}\"", blob.filename.replace('"', ""));
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, blob.content_type.clone()),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CACHE_CONTROL, "private, max-age=3600".to_string()),
        ],
        blob.bytes.clone(),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_grammar() {
        let c = parse_filter(
            r#"thread.name = "spaces/A/threads/T" AND createTime > "2026-11-01T00:00:00Z""#,
        )
        .unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(
            c[0],
            Clause {
                field: "thread.name".into(),
                op: "=".into(),
                value: "spaces/A/threads/T".into()
            }
        );
        assert_eq!(c[1].op, ">");
        assert!(parse_filter("garbage").is_err());
        assert!(parse_filter("").unwrap().is_empty());
        let u = parse_filter(r#"user.name = "users/me""#).unwrap();
        assert_eq!(u[0].value, "users/me");
    }

    #[test]
    fn paging_tokens() {
        let items: Vec<u32> = (0..7).collect();
        let (p, next) = page(&items, 3, 0);
        assert_eq!(p, vec![0, 1, 2]);
        assert_eq!(next.as_deref(), Some("3"));
        let (p, next) = page(&items, 3, 6);
        assert_eq!(p, vec![6]);
        assert!(next.is_none());
        let (p, _) = page(&items, 3, 40);
        assert!(p.is_empty());
    }

    #[test]
    fn boundaries() {
        assert_eq!(
            boundary_of("multipart/form-data; boundary=abc").as_deref(),
            Some("abc")
        );
        assert_eq!(
            boundary_of("multipart/related; Boundary=\"x y\"").as_deref(),
            Some("x y")
        );
        assert!(boundary_of("application/json").is_none());
    }

    #[test]
    fn client_ids() {
        assert!(valid_client_id("client-abc-123"));
        assert!(!valid_client_id("abc"));
        assert!(!valid_client_id("client-ABC"));
        assert!(!valid_client_id("client-"));
    }
}
