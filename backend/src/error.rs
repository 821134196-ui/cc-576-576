use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    /// 400 请求参数/状态不合法
    BadRequest(String),
    /// 401 未登录（缺少角色头）
    Unauthorized(String),
    /// 403 越权（前台尝试管理员区域 / 超时时长）
    Forbidden(String),
    /// 404
    NotFound(String),
    /// 409 状态冲突（如重复签到）
    Conflict(String),
    /// 500
    Internal(String),
}

impl AppError {
    pub fn status(&self) -> StatusCode {
        match self {
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            AppError::BadRequest(_) => "bad_request",
            AppError::Unauthorized(_) => "unauthorized",
            AppError::Forbidden(_) => "forbidden",
            AppError::NotFound(_) => "not_found",
            AppError::Conflict(_) => "conflict",
            AppError::Internal(_) => "internal",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            AppError::BadRequest(m)
            | AppError::Unauthorized(m)
            | AppError::Forbidden(m)
            | AppError::NotFound(m)
            | AppError::Conflict(m)
            | AppError::Internal(m) => m,
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for AppError {}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = Json(json!({ "error": self.code(), "message": self.message() }));
        (status, body).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::Internal(format!("database error: {e}"))
    }
}

pub type AppResult<T> = Result<T, AppError>;
