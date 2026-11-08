//! Google Tasks API v1 subset under `/tasks/v1/lists/@default/tasks`: one list,
//! in memory, shared by every twin user. Bearer required, Google-shaped errors.

use crate::auth::Caller;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::Utc;
use confab_proto::tasks::Task;
use serde::Deserialize;
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/tasks/v1/lists/@default/tasks",
            get(list_tasks).post(create_task),
        )
        .route(
            "/tasks/v1/lists/@default/tasks/{id}",
            get(get_task)
                .patch(patch_task)
                .put(patch_task)
                .delete(delete_task),
        )
}

/// The client posts and patches partial tasks; a missing field is left alone.
/// An explicit `completed: null` is accepted and ignored: the status decides it.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TaskInput {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    due: Option<String>,
}

fn not_found(id: &str) -> ApiError {
    ApiError::not_found(format!("Task {id} not found."))
}

fn check_status(status: &str) -> ApiResult<()> {
    if status == Task::NEEDS_ACTION || status == Task::COMPLETED {
        Ok(())
    } else {
        Err(ApiError::invalid_argument(format!(
            "Invalid value for status: {status}. Expected needsAction or completed."
        )))
    }
}

// note: showCompleted, showHidden and maxResults are accepted and ignored; one page.
async fn list_tasks(
    State(state): State<Arc<AppState>>,
    _caller: Caller,
) -> Json<serde_json::Value> {
    let st = state.read();
    let items: Vec<serde_json::Value> = st.tasks_sorted().iter().map(|r| r.to_json()).collect();
    Json(serde_json::json!({ "kind": "tasks#tasks", "items": items }))
}

async fn get_task(
    State(state): State<Arc<AppState>>,
    _caller: Caller,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let st = state.read();
    st.tasks
        .get(&id)
        .map(|r| Json(r.to_json()))
        .ok_or_else(|| not_found(&id))
}

async fn create_task(
    State(state): State<Arc<AppState>>,
    _caller: Caller,
    Json(input): Json<TaskInput>,
) -> ApiResult<Response> {
    let title = input.title.unwrap_or_default();
    if let Some(s) = input.status.as_deref() {
        check_status(s)?;
    }
    let now = Utc::now();
    let json = {
        let mut st = state.write();
        let task = st.new_task(&title, input.notes.as_deref(), input.due.as_deref(), now);
        if input.status.as_deref() == Some(Task::COMPLETED) {
            if let Some(rec) = st.tasks.get_mut(&task.id) {
                rec.task.status = Task::COMPLETED.into();
                rec.completed = Some(crate::state::Store::task_time(now));
            }
        }
        st.tasks[&task.id].to_json()
    };
    Ok((StatusCode::CREATED, Json(json)).into_response())
}

async fn patch_task(
    State(state): State<Arc<AppState>>,
    _caller: Caller,
    Path(id): Path<String>,
    Json(input): Json<TaskInput>,
) -> ApiResult<Json<serde_json::Value>> {
    if let Some(s) = input.status.as_deref() {
        check_status(s)?;
    }
    let now = crate::state::Store::task_time(Utc::now());
    let mut st = state.write();
    let rec = st.tasks.get_mut(&id).ok_or_else(|| not_found(&id))?;
    if let Some(t) = input.title {
        rec.task.title = t;
    }
    if let Some(n) = input.notes {
        rec.task.notes = Some(n);
    }
    if let Some(d) = input.due {
        rec.task.due = Some(d);
    }
    match input.status.as_deref() {
        Some(Task::COMPLETED) if !rec.task.is_done() => {
            rec.task.status = Task::COMPLETED.into();
            rec.completed = Some(now.clone());
        }
        Some(Task::NEEDS_ACTION) => {
            rec.task.status = Task::NEEDS_ACTION.into();
            rec.completed = None;
        }
        _ => {}
    }
    rec.task.updated = Some(now);
    Ok(Json(rec.to_json()))
}

async fn delete_task(
    State(state): State<Arc<AppState>>,
    _caller: Caller,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let mut st = state.write();
    st.tasks
        .remove(&id)
        .map(|_| StatusCode::NO_CONTENT)
        .ok_or_else(|| not_found(&id))
}
