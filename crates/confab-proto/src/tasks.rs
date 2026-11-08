//! The Google Tasks API v1 subset Prism uses: one resource, the task in the
//! signed-in user's default list. JSON camelCase on the wire, Google's field names.

use serde::{Deserialize, Serialize};

/// `GET|POST|PATCH tasks/v1/lists/@default/tasks[/{id}]`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// `needsAction` | `completed`
    #[serde(default)]
    pub status: String,
    /// RFC 3339; Google keeps only the date part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_view_link: Option<String>,
}

impl Task {
    pub const NEEDS_ACTION: &'static str = "needsAction";
    pub const COMPLETED: &'static str = "completed";

    pub fn is_done(&self) -> bool {
        self.status == Self::COMPLETED
    }
}

/// `GET tasks/v1/lists/@default/tasks`; `items` is absent when the list is empty.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Task>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_json_uses_google_names_and_drops_none() {
        let t = Task {
            id: "task-1".into(),
            title: "Ship it".into(),
            notes: None,
            status: Task::NEEDS_ACTION.into(),
            due: None,
            updated: Some("2026-11-06T00:00:00.000Z".into()),
            web_view_link: Some("https://tasks.google.com/".into()),
        };
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "id": "task-1",
                "title": "Ship it",
                "status": "needsAction",
                "updated": "2026-11-06T00:00:00.000Z",
                "webViewLink": "https://tasks.google.com/"
            })
        );
        let back: Task = serde_json::from_value(v).unwrap();
        assert_eq!(back, t);
        assert!(!back.is_done());

        let empty: ListTasksResponse = serde_json::from_str(r#"{"kind":"tasks#tasks"}"#).unwrap();
        assert!(empty.items.is_empty());
    }
}
