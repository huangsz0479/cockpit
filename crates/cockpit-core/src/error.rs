use serde::Serialize;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, CockpitError>;

#[derive(Debug, Error)]
pub enum CockpitError {
    #[error("配置无效：{0}")]
    InvalidConfig(String),
    #[error("连接失败：{0}")]
    Connection(String),
    #[error("查询失败：{0}")]
    Query(String),
    #[error("查询已取消")]
    Canceled,
    #[error("查询超时")]
    Timeout,
    #[error("本地存储失败：{0}")]
    Storage(String),
    #[error("系统凭据存储失败：{0}")]
    SecretStore(String),
    #[error("数据交换失败：{0}")]
    Exchange(String),
    #[error("当前版本暂不支持：{0}")]
    Unsupported(String),
    #[error("未找到：{0}")]
    NotFound(String),
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub code: &'static str,
    pub message: String,
}

impl CockpitError {
    pub fn payload(&self) -> ErrorPayload {
        let code = match self {
            Self::InvalidConfig(_) => "INVALID_CONFIG",
            Self::Connection(_) => "CONNECTION_ERROR",
            Self::Query(_) => "QUERY_ERROR",
            Self::Canceled => "CANCELED",
            Self::Timeout => "TIMEOUT",
            Self::Storage(_) => "STORAGE_ERROR",
            Self::SecretStore(_) => "SECRET_STORE_ERROR",
            Self::Exchange(_) => "EXCHANGE_ERROR",
            Self::Unsupported(_) => "UNSUPPORTED",
            Self::NotFound(_) => "NOT_FOUND",
            Self::Other(_) => "INTERNAL_ERROR",
        };
        ErrorPayload {
            code,
            message: self.to_string(),
        }
    }
}

impl From<rusqlite::Error> for CockpitError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Storage(value.to_string())
    }
}

impl From<serde_json::Error> for CockpitError {
    fn from(value: serde_json::Error) -> Self {
        Self::Storage(value.to_string())
    }
}
