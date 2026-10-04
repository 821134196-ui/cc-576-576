use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Unauthorized(String),
    Forbidden(String),
    NotFound(String),
    Conflict(String),
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (code, msg) = match self {
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, m),
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m),
            AppError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (code, Json(json!({ "error": msg }))).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::Internal(format!("数据库错误: {e}"))
    }
}

pub type AppResult<T> = Result<T, AppError>;

pub fn bad_request(msg: impl Into<String>) -> AppError {
    AppError::BadRequest(msg.into())
}
pub fn forbidden(msg: impl Into<String>) -> AppError {
    AppError::Forbidden(msg.into())
}
pub fn not_found(msg: impl Into<String>) -> AppError {
    AppError::NotFound(msg.into())
}
pub fn conflict(msg: impl Into<String>) -> AppError {
    AppError::Conflict(msg.into())
}
