use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("未授权或登录已过期")]
    Unauthorized,
    #[error("禁止访问: {0}")]
    Forbidden(String),
    #[error("请求参数错误: {0}")]
    BadRequest(String),
    #[error("资源不存在: {0}")]
    NotFound(String),
    #[error("操作冲突: {0}")]
    Conflict(String),
    #[error("请求过于频繁: {0}")]
    TooManyRequests(String),
    #[error("内部服务错误: {0}")]
    Internal(String),
}

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: u16,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<ApiError>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn fail(code: u16, message: impl Into<String>) -> ApiResponse<()> {
        ApiResponse {
            ok: false,
            data: None,
            error: Some(ApiError {
                code,
                message: message.into(),
            }),
        }
    }
}
