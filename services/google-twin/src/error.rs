//! Google-shaped errors: `{"error": {"code", "message", "status"}}` for the Chat API
//! and userinfo, `{"error", "error_description"}` for the OAuth token endpoint.

use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use prism_proto::chat::{ErrorBody, ErrorDetail};

#[derive(Debug, Clone)]
pub struct ApiError {
    pub code: StatusCode,
    pub status: &'static str,
    pub message: String,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(code: StatusCode, status: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            status,
            message: message.into(),
        }
    }
    pub fn unauthenticated() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "UNAUTHENTICATED",
            "Request had invalid authentication credentials. Expected OAuth 2 access token, login cookie or other valid authentication credential. See https://developers.google.com/identity/sign-in/web/devconsole-project.",
        )
    }
    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "PERMISSION_DENIED", message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "NOT_FOUND", message)
    }
    pub fn already_exists(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "ALREADY_EXISTS", message)
    }
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "INVALID_ARGUMENT", message)
    }
    pub fn failed_precondition(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "FAILED_PRECONDITION", message)
    }
    /// 410 with Google's `reason` as the status: `fullSyncRequired` for a stale
    /// calendar sync token, `deleted` for a resource that is already gone.
    pub fn gone(reason: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::GONE, reason, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code.as_u16(),
                message: self.message,
                status: self.status.to_string(),
            },
        };
        let mut resp = (self.code, Json(body)).into_response();
        if self.code == StatusCode::UNAUTHORIZED {
            resp.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                header::HeaderValue::from_static(
                    "Bearer realm=\"https://accounts.google.com/\", error=\"invalid_token\"",
                ),
            );
        }
        resp
    }
}

/// OAuth 2.0 token endpoint error (RFC 6749 §5.2), as Google returns it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OAuthError {
    pub error: &'static str,
    pub error_description: String,
    #[serde(skip)]
    pub code: StatusCode,
}

impl OAuthError {
    pub fn invalid_grant(description: impl Into<String>) -> Self {
        Self {
            error: "invalid_grant",
            error_description: description.into(),
            code: StatusCode::BAD_REQUEST,
        }
    }
    pub fn invalid_request(description: impl Into<String>) -> Self {
        Self {
            error: "invalid_request",
            error_description: description.into(),
            code: StatusCode::BAD_REQUEST,
        }
    }
    pub fn unsupported_grant_type(description: impl Into<String>) -> Self {
        Self {
            error: "unsupported_grant_type",
            error_description: description.into(),
            code: StatusCode::BAD_REQUEST,
        }
    }
}

impl IntoResponse for OAuthError {
    fn into_response(self) -> Response {
        (self.code, Json(self)).into_response()
    }
}
