use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ConnectionInfo, ConnectionProfile, Result, ServerMetric};

#[async_trait]
pub trait RedisDriverTrait: Send + Sync {
    fn kind(&self) -> &'static str;
    async fn test(&self, profile: &ConnectionProfile, password: &str) -> Result<ConnectionInfo>;
    async fn open(
        &self,
        profile: ConnectionProfile,
        password: String,
    ) -> Result<Arc<dyn RedisSession>>;
}

#[async_trait]
pub trait RedisSession: Send + Sync {
    fn connection_id(&self) -> Uuid;
    async fn connection_info(&self) -> Result<ConnectionInfo>;
    async fn list_databases(&self) -> Result<Vec<RedisDatabaseInfo>>;
    async fn scan_keys(
        &self,
        database: u32,
        cursor: u64,
        pattern: Option<&str>,
        count: usize,
    ) -> Result<RedisScanPage>;
    async fn key_info(&self, database: u32, key: &str) -> Result<RedisKeyInfo>;
    async fn get_value(&self, database: u32, key: &str, limit: usize) -> Result<RedisValue>;
    async fn set_string(
        &self,
        database: u32,
        key: &str,
        value: &[u8],
        ttl_secs: Option<i64>,
    ) -> Result<()>;
    async fn delete_keys(&self, database: u32, keys: &[String]) -> Result<u64>;
    async fn expire(&self, database: u32, key: &str, seconds: i64) -> Result<bool>;
    async fn rename(&self, database: u32, from: &str, to: &str) -> Result<()>;
    async fn run_command(
        &self,
        database: u32,
        args: &[String],
        allow_write: bool,
    ) -> Result<RedisReply>;
    async fn server_info(&self) -> Result<Vec<ServerMetric>>;
    async fn close(&self) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedisDatabaseInfo {
    pub index: u32,
    pub key_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedisScanPage {
    pub cursor: u64,
    pub complete: bool,
    pub keys: Vec<RedisKeyInfo>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RedisKeyType {
    String,
    List,
    Set,
    ZSet,
    Hash,
    Stream,
    None,
}

impl RedisKeyType {
    pub fn from_reply(value: &str) -> Self {
        match value {
            "string" => Self::String,
            "list" => Self::List,
            "set" => Self::Set,
            "zset" => Self::ZSet,
            "hash" => Self::Hash,
            "stream" => Self::Stream,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedisKeyInfo {
    pub key: String,
    pub kind: RedisKeyType,
    pub ttl_secs: i64,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedisStringValue {
    pub value_base64: String,
    pub preview: Option<String>,
    pub length: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedisHashField {
    pub field: RedisStringValue,
    pub value: RedisStringValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedisZSetMember {
    pub value: RedisStringValue,
    pub score: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedisStreamEntry {
    pub id: String,
    pub fields: Vec<RedisHashField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RedisValue {
    None {
        #[serde(rename = "ttlSecs")]
        ttl_secs: i64,
    },
    String {
        value: RedisStringValue,
        #[serde(rename = "ttlSecs")]
        ttl_secs: i64,
    },
    Hash {
        fields: Vec<RedisHashField>,
        length: u64,
        truncated: bool,
        #[serde(rename = "ttlSecs")]
        ttl_secs: i64,
    },
    List {
        values: Vec<RedisStringValue>,
        length: u64,
        truncated: bool,
        #[serde(rename = "ttlSecs")]
        ttl_secs: i64,
    },
    Set {
        values: Vec<RedisStringValue>,
        length: u64,
        truncated: bool,
        #[serde(rename = "ttlSecs")]
        ttl_secs: i64,
    },
    ZSet {
        members: Vec<RedisZSetMember>,
        length: u64,
        truncated: bool,
        #[serde(rename = "ttlSecs")]
        ttl_secs: i64,
    },
    Stream {
        entries: Vec<RedisStreamEntry>,
        length: u64,
        truncated: bool,
        #[serde(rename = "ttlSecs")]
        ttl_secs: i64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RedisReply {
    Nil,
    Status { text: String },
    Integer { value: i64 },
    BulkString {
        base64: String,
        preview: Option<String>,
        length: usize,
    },
    Array { items: Vec<RedisReply> },
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_type_parses_reply_names() {
        assert_eq!(RedisKeyType::from_reply("zset"), RedisKeyType::ZSet);
        assert_eq!(RedisKeyType::from_reply("string"), RedisKeyType::String);
        assert_eq!(RedisKeyType::from_reply("unknown"), RedisKeyType::None);
    }

    #[test]
    fn redis_value_serializes_as_tagged_json() {
        let value = RedisValue::String {
            value: RedisStringValue {
                value_base64: "aGVsbG8=".into(),
                preview: Some("hello".into()),
                length: 5,
            },
            ttl_secs: -1,
        };
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["kind"], "string");
        assert_eq!(json["value"]["preview"], "hello");
        assert_eq!(json["ttlSecs"], -1);
    }

    #[test]
    fn redis_reply_serializes_bulk_string_without_exposing_base64_in_tag() {
        let reply = RedisReply::BulkString {
            base64: "AQI=".into(),
            preview: None,
            length: 2,
        };
        let json = serde_json::to_value(reply).unwrap();
        assert_eq!(json["kind"], "bulk_string");
        assert_eq!(json["base64"], "AQI=");
        assert_eq!(json["length"], 2);
    }
}
