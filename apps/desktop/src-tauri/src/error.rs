use serde::ser::{Serialize, SerializeStruct, Serializer};

/// Unified error type returned by Tauri commands.
/// Serializes to `{ "code": u16, "message": String }` so the frontend
/// can surface a clean message instead of an opaque string.
#[derive(Debug)]
pub struct AppError {
    pub code: u16,
    pub message: String,
}

impl AppError {
    pub fn new(code: u16, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Convenience for generic 500-style errors.
    pub fn msg(message: impl Into<String>) -> Self {
        Self::new(500, message)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("code", &self.code)?;
        state.serialize_field("message", &self.message)?;
        state.end()
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::new(502, format!("网络请求失败: {e}"))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::new(500, format!("响应解析失败: {e}"))
    }
}
