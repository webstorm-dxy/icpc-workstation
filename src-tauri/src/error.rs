use serde::Serialize;

/// Structured error returned to the frontend. Tauri serializes this as the
/// rejection value of `invoke`, so the UI can branch on `code` and offer a
/// retry only when it is meaningful.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub detail: Option<String>,
    pub retryable: bool,
}

impl AppError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            detail: None,
            retryable: false,
        }
    }

    pub fn internal(err: impl std::fmt::Display) -> Self {
        Self {
            code: "E_INTERNAL".into(),
            message: "内部错误".into(),
            detail: Some(err.to_string()),
            retryable: false,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("E_INVALID", message)
    }

    pub fn not_found(what: &str) -> Self {
        Self::new("E_NOT_FOUND", format!("{what}不存在"))
    }

    pub fn network(err: impl std::fmt::Display) -> Self {
        Self {
            code: "E_NETWORK".into(),
            message: "网络请求失败".into(),
            detail: Some(err.to_string()),
            retryable: true,
        }
    }

    pub fn provider(code: &str, message: impl Into<String>) -> Self {
        Self::new(code, message)
    }

    pub fn needs_human(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            code: "E_AUTOMATION_HUMAN".into(),
            message: message.into(),
            detail: Some(hint.into()),
            retryable: true,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

macro_rules! from_error {
    ($ty:ty, $code:literal) => {
        impl From<$ty> for AppError {
            fn from(err: $ty) -> Self {
                AppError {
                    code: $code.into(),
                    message: err.to_string(),
                    detail: None,
                    retryable: false,
                }
            }
        }
    };
}

from_error!(sqlx::Error, "E_DB");
from_error!(sqlx::migrate::MigrateError, "E_DB");
from_error!(reqwest::Error, "E_NETWORK");
from_error!(serde_json::Error, "E_JSON");
from_error!(std::io::Error, "E_IO");
from_error!(keyring::Error, "E_KEYCHAIN");
from_error!(tauri::Error, "E_TAURI");
from_error!(url::ParseError, "E_URL");

pub type AppResult<T> = Result<T, AppError>;
