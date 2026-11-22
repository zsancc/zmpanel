use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use zmpanel_core::{ApiResponse, AppError};

pub struct ServerError(pub AppError);

impl From<AppError> for ServerError {
    fn from(value: AppError) -> Self {
        Self(value)
    }
}

impl From<rusqlite::Error> for ServerError {
    fn from(value: rusqlite::Error) -> Self {
        Self(AppError::Internal(format!("数据库错误: {}", value)))
    }
}

impl From<std::io::Error> for ServerError {
    fn from(value: std::io::Error) -> Self {
        Self(AppError::Internal(format!("IO 错误: {}", value)))
    }
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let (status, msg) = match self.0 {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "未授权或登录已过期".to_string()),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, m),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m),
            AppError::TooManyRequests(m) => (StatusCode::TOO_MANY_REQUESTS, m),
            AppError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        let body = ApiResponse::<()>::fail(status.as_u16(), msg);
        (status, Json(body)).into_response()
    }
}

pub type ServerResult<T> = Result<T, ServerError>;
