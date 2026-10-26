use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use serde_json::{json, Value};
use tracing::error;

#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub message: String,
    pub detail: Option<Value>,
}

impl AppError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            detail: None,
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn upstream_unavailable(error: reqwest::Error) -> Self {
        error!(%error, "upstream request failed");
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: "hawkBit upstream unavailable".to_string(),
            detail: Some(json!({ "cause": error.to_string() })),
        }
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    status: u16,
    detail: Option<Value>,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
                status: self.status.as_u16(),
                detail: self.detail,
            }),
        )
            .into_response()
    }
}
