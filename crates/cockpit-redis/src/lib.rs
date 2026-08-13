use std::{
    collections::HashMap,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use cockpit_core::{
    CockpitError, ConnectionInfo, ConnectionProfile, RedisDatabaseInfo, RedisDriverTrait,
    RedisHashField, RedisKeyInfo, RedisKeyType, RedisReply, RedisScanPage, RedisSession,
    RedisStreamEntry, RedisStringValue, RedisZSetMember, Result, ServerMetric,
    TlsMode,
};
use cockpit_core::RedisValue as RedisValueModel;
use redis::Value as RedisValue;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Default)]
pub struct RedisDriver;

pub struct RedisConnectionSession {
    profile: ConnectionProfile,
    client: redis::Client,
    connections: Mutex<HashMap<u32, redis::aio::MultiplexedConnection>>,
}

#[async_trait]
impl RedisDriverTrait for RedisDriver {
    fn kind(&self) -> &'static str {
        "redis"
    }

    async fn test(&self, profile: &ConnectionProfile, password: &str) -> Result<ConnectionInfo> {
        let session = open_session(profile.clone(), password.to_string()).await?;
        let info = session.connection_info().await;
        let _ = session.close().await;
        info
    }

    async fn open(
        &self,
        profile: ConnectionProfile,
        password: String,
    ) -> Result<Arc<dyn RedisSession>> {
        open_session(profile, password).await
    }
}

async fn open_session(
    profile: ConnectionProfile,
    password: String,
) -> Result<Arc<dyn RedisSession>> {
    profile.validate()?;
    if profile.tls.mode != TlsMode::Disabled {
        return Err(CockpitError::Unsupported(
            "Redis TLS 连接暂未启用，请先关闭 TLS 模式".into(),
        ));
    }
    let url = redis_url(&profile, &password)?;
    let client = redis::Client::open(url.as_str()).map_err(connection_error)?;
    let session = Arc::new(RedisConnectionSession {
        profile: profile.clone(),
        client,
        connections: Mutex::new(HashMap::new()),
    });
    tokio::time::timeout(
        Duration::from_secs(profile.connect_timeout_secs.max(1)),
        session.connection_info(),
    )
    .await
    .map_err(|_| {
        CockpitError::Connection(format!(
            "连接超时（{} 秒）",
            profile.connect_timeout_secs
        ))
    })??;
    Ok(session)
}

fn redis_url(profile: &ConnectionProfile, password: &str) -> Result<String> {
    let host = profile.host.trim();
    if host.is_empty() {
        return Err(CockpitError::InvalidConfig("主机不能为空".into()));
    }
    let username = profile.username.trim();
    let mut auth = String::new();
    if !username.is_empty() || !password.is_empty() {
        auth.push_str(&percent_encode_component(username));
        auth.push(':');
        auth.push_str(&percent_encode_component(password));
        auth.push('@');
    }
    let mut url = format!("redis://{auth}{host}:{}", profile.port);
    if let Some(database) = profile
        .database
        .as_deref()
        .and_then(|value| value.trim().parse::<u32>().ok())
    {
        url.push('/');
        url.push_str(&database.to_string());
    }
    Ok(url)
}

fn percent_encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(*byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[async_trait]
impl RedisSession for RedisConnectionSession {
    fn connection_id(&self) -> Uuid {
        self.profile.id
    }

    async fn connection_info(&self) -> Result<ConnectionInfo> {
        let mut conn = self.connection(0).await?;
        let info: String = redis::cmd("INFO")
            .arg("server")
            .query_async(&mut conn)
            .await
            .map_err(query_error)?;
        let fields = parse_info(&info);
        let client_id = redis::cmd("CLIENT")
            .arg("ID")
            .query_async::<i64>(&mut conn)
            .await
            .map_err(query_error)?;
        Ok(ConnectionInfo {
            server_version: fields
                .get("redis_version")
                .cloned()
                .unwrap_or_default(),
            server_comment: fields.get("redis_mode").cloned(),
            connection_id: client_id.max(0) as u32,
            current_database: None,
            tls_cipher: None,
        })
    }

    async fn list_databases(&self) -> Result<Vec<RedisDatabaseInfo>> {
        let mut conn = self.connection(0).await?;
        let database_count = redis::cmd("CONFIG")
            .arg("GET")
            .arg("databases")
            .query_async::<RedisValue>(&mut conn)
            .await
            .ok()
            .and_then(|reply| match reply {
                RedisValue::Array(mut parts) if parts.len() >= 2 => {
                    value_to_string(parts.remove(1))
                }
                _ => None,
            })
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(16);
        let key_counts = key_space_counts(
            redis::cmd("INFO")
                .arg("keyspace")
                .query_async::<String>(&mut conn)
                .await
                .ok(),
        );
        Ok((0..database_count)
            .map(|index| RedisDatabaseInfo {
                index,
                key_count: key_counts.get(&index).copied().unwrap_or_default(),
            })
            .collect())
    }

    async fn scan_keys(
        &self,
        database: u32,
        cursor: u64,
        pattern: Option<&str>,
        count: usize,
    ) -> Result<RedisScanPage> {
        let mut conn = self.connection(database).await?;
        let count = count.clamp(1, 1_000);
        let mut command = redis::cmd("SCAN");
        command.arg(cursor);
        if let Some(pattern) = pattern.filter(|value| !value.trim().is_empty()) {
            command.arg("MATCH").arg(pattern);
        }
        command.arg("COUNT").arg(count);
        let reply = command.query_async::<RedisValue>(&mut conn).await.map_err(query_error)?;
        let (next_cursor, keys) = parse_scan_reply(reply)?;
        let mut key_infos = Vec::with_capacity(keys.len());
        for key in keys {
            let kind = read_key_type(&mut conn, &key).await?;
            let ttl_secs = read_ttl(&mut conn, &key).await?;
            key_infos.push(RedisKeyInfo {
                key,
                kind,
                ttl_secs,
                size_bytes: None,
            });
        }
        Ok(RedisScanPage {
            cursor: next_cursor,
            complete: next_cursor == 0,
            keys: key_infos,
        })
    }

    async fn key_info(&self, database: u32, key: &str) -> Result<RedisKeyInfo> {
        let mut conn = self.connection(database).await?;
        let kind = read_key_type(&mut conn, key).await?;
        let ttl_secs = read_ttl(&mut conn, key).await?;
        let size_bytes = redis::cmd("MEMORY")
            .arg("USAGE")
            .arg(key)
            .query_async::<Option<u64>>(&mut conn)
            .await
            .ok()
            .flatten();
        Ok(RedisKeyInfo {
            key: key.to_string(),
            kind,
            ttl_secs,
            size_bytes,
        })
    }

    async fn get_value(&self, database: u32, key: &str, limit: usize) -> Result<RedisValueModel> {
        let mut conn = self.connection(database).await?;
        let kind = read_key_type(&mut conn, key).await?;
        let ttl_secs = read_ttl(&mut conn, key).await?;
        let limit = limit.clamp(1, 2_000);
        match kind {
            RedisKeyType::None => Ok(RedisValueModel::None { ttl_secs }),
            RedisKeyType::String => {
                let reply = redis::cmd("GET")
                    .arg(key)
                    .query_async::<RedisValue>(&mut conn)
                    .await
                    .map_err(query_error)?;
                let value = match reply {
                    RedisValue::Nil => RedisValueModel::None { ttl_secs },
                    other => RedisValueModel::String {
                        value: redis_string_value(value_to_bytes(&other).unwrap_or_default()),
                        ttl_secs,
                    },
                };
                Ok(value)
            }
            RedisKeyType::Hash => {
                let length = redis::cmd("HLEN")
                    .arg(key)
                    .query_async::<u64>(&mut conn)
                    .await
                    .map_err(query_error)?;
                let reply = redis::cmd("HSCAN")
                    .arg(key)
                    .arg(0)
                    .arg("MATCH")
                    .arg("*")
                    .arg("COUNT")
                    .arg(limit)
                    .query_async::<RedisValue>(&mut conn)
                    .await
                    .map_err(query_error)?;
                let items = scan_items(reply);
                let mut fields = Vec::new();
                for pair in items.chunks_exact(2) {
                    fields.push(RedisHashField {
                        field: redis_string_value(value_to_bytes(&pair[0]).unwrap_or_default()),
                        value: redis_string_value(value_to_bytes(&pair[1]).unwrap_or_default()),
                    });
                }
                let truncated = length > fields.len() as u64;
                fields.truncate(limit);
                Ok(RedisValueModel::Hash {
                    fields,
                    length,
                    truncated,
                    ttl_secs,
                })
            }
            RedisKeyType::List => {
                let length = redis::cmd("LLEN")
                    .arg(key)
                    .query_async::<u64>(&mut conn)
                    .await
                    .map_err(query_error)?;
                let reply = redis::cmd("LRANGE")
                    .arg(key)
                    .arg(0)
                    .arg(limit.saturating_sub(1) as i64)
                    .query_async::<RedisValue>(&mut conn)
                    .await
                    .map_err(query_error)?;
                let values = bulk_items(reply)
                    .into_iter()
                    .map(|item| redis_string_value(value_to_bytes(&item).unwrap_or_default()))
                    .collect::<Vec<_>>();
                let truncated = length > values.len() as u64;
                Ok(RedisValueModel::List {
                    values,
                    length,
                    truncated,
                    ttl_secs,
                })
            }
            RedisKeyType::Set => {
                let length = redis::cmd("SCARD")
                    .arg(key)
                    .query_async::<u64>(&mut conn)
                    .await
                    .map_err(query_error)?;
                let reply = redis::cmd("SSCAN")
                    .arg(key)
                    .arg(0)
                    .arg("MATCH")
                    .arg("*")
                    .arg("COUNT")
                    .arg(limit)
                    .query_async::<RedisValue>(&mut conn)
                    .await
                    .map_err(query_error)?;
                let mut values = scan_items(reply)
                    .into_iter()
                    .map(|item| redis_string_value(value_to_bytes(&item).unwrap_or_default()))
                    .collect::<Vec<_>>();
                let truncated = length > values.len() as u64;
                values.truncate(limit);
                Ok(RedisValueModel::Set {
                    values,
                    length,
                    truncated,
                    ttl_secs,
                })
            }
            RedisKeyType::ZSet => {
                let length = redis::cmd("ZCARD")
                    .arg(key)
                    .query_async::<u64>(&mut conn)
                    .await
                    .map_err(query_error)?;
                let reply = redis::cmd("ZSCAN")
                    .arg(key)
                    .arg(0)
                    .arg("MATCH")
                    .arg("*")
                    .arg("COUNT")
                    .arg(limit)
                    .query_async::<RedisValue>(&mut conn)
                    .await
                    .map_err(query_error)?;
                let items = scan_items(reply);
                let mut members = Vec::new();
                for pair in items.chunks_exact(2) {
                    members.push(RedisZSetMember {
                        value: redis_string_value(value_to_bytes(&pair[0]).unwrap_or_default()),
                        score: value_to_string(pair[1].clone()).unwrap_or_default(),
                    });
                }
                let truncated = length > members.len() as u64;
                members.truncate(limit);
                Ok(RedisValueModel::ZSet {
                    members,
                    length,
                    truncated,
                    ttl_secs,
                })
            }
            RedisKeyType::Stream => {
                let length = redis::cmd("XLEN")
                    .arg(key)
                    .query_async::<u64>(&mut conn)
                    .await
                    .map_err(query_error)?;
                let reply = redis::cmd("XRANGE")
                    .arg(key)
                    .arg("-")
                    .arg("+")
                    .arg("COUNT")
                    .arg(limit)
                    .query_async::<RedisValue>(&mut conn)
                    .await
                    .map_err(query_error)?;
                let mut entries = parse_stream_entries(reply);
                let truncated = length > entries.len() as u64;
                entries.truncate(limit);
                Ok(RedisValueModel::Stream {
                    entries,
                    length,
                    truncated,
                    ttl_secs,
                })
            }
        }
    }

    async fn set_string(
        &self,
        database: u32,
        key: &str,
        value: &[u8],
        ttl_secs: Option<i64>,
    ) -> Result<()> {
        self.ensure_writable()?;
        let mut conn = self.connection(database).await?;
        redis::cmd("SET")
            .arg(key)
            .arg(value)
            .query_async::<()>(&mut conn)
            .await
            .map_err(query_error)?;
        if let Some(ttl) = ttl_secs {
            if ttl > 0 {
                redis::cmd("EXPIRE")
                    .arg(key)
                    .arg(ttl)
                    .query_async::<()>(&mut conn)
                    .await
                    .map_err(query_error)?;
            } else if ttl == 0 {
                redis::cmd("PERSIST")
                    .arg(key)
                    .query_async::<()>(&mut conn)
                    .await
                    .map_err(query_error)?;
            }
        }
        Ok(())
    }

    async fn delete_keys(&self, database: u32, keys: &[String]) -> Result<u64> {
        self.ensure_writable()?;
        if keys.is_empty() {
            return Ok(0);
        }
        let mut conn = self.connection(database).await?;
        let mut command = redis::cmd("DEL");
        for key in keys {
            command.arg(key);
        }
        command.query_async::<u64>(&mut conn).await.map_err(query_error)
    }

    async fn expire(&self, database: u32, key: &str, seconds: i64) -> Result<bool> {
        self.ensure_writable()?;
        let mut conn = self.connection(database).await?;
        if seconds < 0 {
            return Err(CockpitError::InvalidConfig("过期时间不能小于 0".into()));
        }
        if seconds == 0 {
            return redis::cmd("PERSIST")
                .arg(key)
                .query_async::<bool>(&mut conn)
                .await
                .map_err(query_error);
        }
        redis::cmd("EXPIRE")
            .arg(key)
            .arg(seconds)
            .query_async::<bool>(&mut conn)
            .await
            .map_err(query_error)
    }

    async fn rename(&self, database: u32, from: &str, to: &str) -> Result<()> {
        self.ensure_writable()?;
        let mut conn = self.connection(database).await?;
        redis::cmd("RENAME")
            .arg(from)
            .arg(to)
            .query_async::<()>(&mut conn)
            .await
            .map_err(query_error)
    }

    async fn run_command(
        &self,
        database: u32,
        args: &[String],
        allow_write: bool,
    ) -> Result<RedisReply> {
        let command = args
            .first()
            .map(|value| value.trim().to_ascii_uppercase())
            .unwrap_or_default();
        if command.is_empty() {
            return Err(CockpitError::InvalidConfig("命令不能为空".into()));
        }
        let safety = redis_command_safety(&command, args.get(1).map(String::as_str));
        if self.profile.read_only && safety != RedisCommandSafety::ReadOnly {
            return Err(CockpitError::Query("该连接处于只读模式".into()));
        }
        if safety == RedisCommandSafety::Destructive && !allow_write {
            return Err(CockpitError::Query(format!(
                "该命令属于高风险操作：{command}"
            )));
        }
        let mut conn = self.connection(database).await?;
        let mut command_builder = redis::cmd(&command);
        for argument in &args[1..] {
            command_builder.arg(argument);
        }
        let reply = command_builder
            .query_async::<RedisValue>(&mut conn)
            .await
            .map_err(query_error)?;
        Ok(redis_reply(reply))
    }

    async fn server_info(&self) -> Result<Vec<ServerMetric>> {
        let mut conn = self.connection(0).await?;
        let info: String = redis::cmd("INFO")
            .query_async(&mut conn)
            .await
            .map_err(query_error)?;
        Ok(parse_info_sections(&info))
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }
}

impl RedisConnectionSession {
    async fn connection(
        &self,
        database: u32,
    ) -> Result<redis::aio::MultiplexedConnection> {
        let mut connections = self.connections.lock().await;
        if let Some(connection) = connections.get(&database) {
            return Ok(connection.clone());
        }
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(connection_error)?;
        if database != 0 {
            let _ = redis::cmd("SELECT")
                .arg(database)
                .query_async::<RedisValue>(&mut connection)
                .await
                .map_err(query_error)?;
        }
        connections.insert(database, connection.clone());
        Ok(connection)
    }

    fn ensure_writable(&self) -> Result<()> {
        if self.profile.read_only {
            Err(CockpitError::Query("该连接处于只读模式".into()))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RedisCommandSafety {
    ReadOnly,
    Write,
    Destructive,
}

fn redis_command_safety(command: &str, second: Option<&str>) -> RedisCommandSafety {
    const DESTRUCTIVE: &[&str] = &[
        "FLUSHALL",
        "FLUSHDB",
        "SHUTDOWN",
        "DEBUG",
        "SWAPDB",
        "REPLICAOF",
        "SLAVEOF",
        "CLUSTER",
        "FAILOVER",
        "LATENCY",
        "MIGRATE",
        "RESTORE",
    ];
    const READ_ONLY: &[&str] = &[
        "PING",
        "ECHO",
        "SELECT",
        "DBSIZE",
        "INFO",
        "CLIENT",
        "SLOWLOG",
        "SCAN",
        "KEYS",
        "RANDOMKEY",
        "TYPE",
        "EXISTS",
        "TTL",
        "PTTL",
        "EXPIRETIME",
        "PEXPIRETIME",
        "STRLEN",
        "GET",
        "GETRANGE",
        "MGET",
        "GETDEL",
        "GETEX",
        "HGET",
        "HGETALL",
        "HKEYS",
        "HVALS",
        "HMGET",
        "HLEN",
        "HEXISTS",
        "HSTRLEN",
        "HRANDFIELD",
        "HSCAN",
        "LLEN",
        "LINDEX",
        "LRANGE",
        "LPOS",
        "SCARD",
        "SISMEMBER",
        "SMISMEMBER",
        "SMEMBERS",
        "SRANDMEMBER",
        "SSCAN",
        "ZCARD",
        "ZCOUNT",
        "ZLEXCOUNT",
        "ZSCORE",
        "ZMSCORE",
        "ZRANK",
        "ZREVRANK",
        "ZRANGE",
        "ZRANGEBYSCORE",
        "ZRANGEBYLEX",
        "ZREVRANGE",
        "ZREVRANGEBYSCORE",
        "ZREVRANGEBYLEX",
        "ZRANDMEMBER",
        "ZSCAN",
        "XLEN",
        "XRANGE",
        "XREVRANGE",
        "XINFO",
        "GEOHASH",
        "GEOPOS",
        "GEODIST",
        "GEOSEARCH",
        "OBJECT",
        "MEMORY",
        "PFCOUNT",
        "BITCOUNT",
        "BITPOS",
        "BITFIELD_RO",
        "COMMAND",
    ];
    if DESTRUCTIVE.contains(&command) {
        return RedisCommandSafety::Destructive;
    }
    if command == "CONFIG" {
        return match second.map(str::to_ascii_uppercase).as_deref() {
            Some("GET") | Some("HELP") => RedisCommandSafety::ReadOnly,
            _ => RedisCommandSafety::Destructive,
        };
    }
    if command == "SCRIPT" {
        return match second.map(str::to_ascii_uppercase).as_deref() {
            Some("FLUSH") => RedisCommandSafety::Destructive,
            _ => RedisCommandSafety::Write,
        };
    }
    if READ_ONLY.contains(&command) {
        RedisCommandSafety::ReadOnly
    } else {
        RedisCommandSafety::Write
    }
}

async fn read_key_type(
    conn: &mut redis::aio::MultiplexedConnection,
    key: &str,
) -> Result<RedisKeyType> {
    let reply = redis::cmd("TYPE")
        .arg(key)
        .query_async::<RedisValue>(conn)
        .await
        .map_err(query_error)?;
    Ok(RedisKeyType::from_reply(
        &value_to_string(reply).unwrap_or_default(),
    ))
}

async fn read_ttl(conn: &mut redis::aio::MultiplexedConnection, key: &str) -> Result<i64> {
    redis::cmd("TTL")
        .arg(key)
        .query_async::<i64>(conn)
        .await
        .map_err(query_error)
}

fn parse_scan_reply(reply: RedisValue) -> Result<(u64, Vec<String>)> {
    let RedisValue::Array(mut parts) = reply else {
        return Err(CockpitError::Query("Redis SCAN 返回格式无效".into()));
    };
    if parts.len() != 2 {
        return Err(CockpitError::Query("Redis SCAN 返回元素数量无效".into()));
    }
    let cursor = value_to_string(parts.remove(0))
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| CockpitError::Query("Redis SCAN 游标无效".into()))?;
    let keys = match parts.remove(0) {
        RedisValue::Array(items) => items
            .into_iter()
            .filter_map(value_to_string)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    Ok((cursor, keys))
}

fn scan_items(reply: RedisValue) -> Vec<RedisValue> {
    match reply {
        RedisValue::Array(mut parts) if parts.len() >= 2 => match parts.remove(1) {
            RedisValue::Array(items) => items,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn bulk_items(reply: RedisValue) -> Vec<RedisValue> {
    match reply {
        RedisValue::Array(items) => items,
        _ => Vec::new(),
    }
}

fn parse_stream_entries(reply: RedisValue) -> Vec<RedisStreamEntry> {
    let items = bulk_items(reply);
    items
        .into_iter()
        .filter_map(|entry| match entry {
            RedisValue::Array(mut parts) if parts.len() == 2 => {
                let id = value_to_string(parts.remove(0)).unwrap_or_default();
                let fields = match parts.remove(0) {
                    RedisValue::Array(fields) => fields
                        .chunks_exact(2)
                        .map(|pair| RedisHashField {
                            field: redis_string_value(
                                value_to_bytes(&pair[0]).unwrap_or_default(),
                            ),
                            value: redis_string_value(
                                value_to_bytes(&pair[1]).unwrap_or_default(),
                            ),
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                Some(RedisStreamEntry { id, fields })
            }
            _ => None,
        })
        .collect()
}

fn redis_string_value(bytes: Vec<u8>) -> RedisStringValue {
    RedisStringValue {
        value_base64: BASE64_STANDARD.encode(&bytes),
        preview: printable_preview(&bytes),
        length: bytes.len(),
    }
}

fn redis_reply(value: RedisValue) -> RedisReply {
    match value {
        RedisValue::Nil => RedisReply::Nil,
        RedisValue::SimpleString(text) => RedisReply::Status { text },
        RedisValue::Int(value) => RedisReply::Integer { value },
        RedisValue::BulkString(bytes) => RedisReply::BulkString {
            base64: BASE64_STANDARD.encode(&bytes),
            preview: printable_preview(&bytes),
            length: bytes.len(),
        },
        RedisValue::Array(items) => RedisReply::Array {
            items: items.into_iter().map(redis_reply).collect(),
        },
        RedisValue::Okay => RedisReply::Status {
            text: "OK".into(),
        },
        RedisValue::ServerError(error) => RedisReply::Error {
            message: match error.details() {
                Some(details) => format!("{} {details}", error.code()),
                None => error.code().to_string(),
            },
        },
        other => RedisReply::Error {
            message: format!("无法解析 Redis 返回类型：{other:?}"),
        },
    }
}

fn value_to_string(value: RedisValue) -> Option<String> {
    match value {
        RedisValue::BulkString(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
        RedisValue::SimpleString(text) => Some(text),
        RedisValue::Int(value) => Some(value.to_string()),
        _ => None,
    }
}

fn value_to_bytes(value: &RedisValue) -> Option<Vec<u8>> {
    match value {
        RedisValue::BulkString(bytes) => Some(bytes.clone()),
        RedisValue::SimpleString(text) => Some(text.as_bytes().to_vec()),
        RedisValue::Int(value) => Some(value.to_string().into_bytes()),
        RedisValue::Okay => Some(Vec::new()),
        _ => None,
    }
}

fn printable_preview(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    text.chars()
        .all(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .then(|| text.chars().take(200).collect())
}

fn parse_info(value: &str) -> HashMap<String, String> {
    value
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once(':')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn parse_info_sections(value: &str) -> Vec<ServerMetric> {
    let mut metrics = Vec::new();
    let mut section = String::new();
    for line in value.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('#') {
            section = name.trim().to_string();
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let name = if section.is_empty() {
                key.to_string()
            } else {
                format!("{section}.{key}")
            };
            metrics.push(ServerMetric {
                name,
                value: value.to_string(),
            });
        }
    }
    metrics
}

fn key_space_counts(info: Option<String>) -> HashMap<u32, u64> {
    let mut counts = HashMap::new();
    let Some(info) = info else {
        return counts;
    };
    for line in info.lines() {
        let Some((database, fields)) = line.split_once(':') else {
            continue;
        };
        let Some(index) = database.strip_prefix("db").and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        for field in fields.split(',') {
            if let Some(keys) = field.strip_prefix("keys=") {
                if let Ok(count) = keys.parse::<u64>() {
                    counts.insert(index, count);
                }
            }
        }
    }
    counts
}

fn connection_error(error: redis::RedisError) -> CockpitError {
    CockpitError::Connection(error.to_string())
}

fn query_error(error: redis::RedisError) -> CockpitError {
    CockpitError::Query(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_includes_credentials_only_when_needed() {
        let now = chrono::Utc::now();
        let profile = ConnectionProfile {
            id: Uuid::new_v4(),
            driver_kind: cockpit_core::DatabaseKind::Redis,
            group: None,
            name: "redis".into(),
            host: "127.0.0.1".into(),
            port: 6379,
            username: String::new(),
            database: Some("2".into()),
            tls: cockpit_core::TlsOptions::default(),
            ssh: None,
            connect_timeout_secs: 5,
            query_timeout_secs: 30,
            pool_size: 5,
            read_only: false,
            production: false,
            color: None,
            created_at: now,
            updated_at: now,
        };
        assert_eq!(redis_url(&profile, "").unwrap(), "redis://127.0.0.1:6379/2");
        assert_eq!(
            redis_url(&profile, "secret").unwrap(),
            "redis://:secret@127.0.0.1:6379/2"
        );
    }

    #[test]
    fn percent_encoding_escapes_credentials() {
        assert_eq!(
            percent_encode_component("user@name:p@ss/word"),
            "user%40name%3Ap%40ss%2Fword"
        );
    }

    #[test]
    fn info_parser_ignores_sections_and_comments() {
        let parsed = parse_info("# Server\nredis_version:7.2.4\nredis_mode:standalone\n");
        assert_eq!(parsed.get("redis_version").map(String::as_str), Some("7.2.4"));
        assert_eq!(parsed.get("redis_mode").map(String::as_str), Some("standalone"));
    }

    #[test]
    fn info_sections_are_prefixed_in_metrics() {
        let metrics = parse_info_sections("# Server\nredis_version:7.2.4\n# Memory\nused_memory:123\n");
        assert_eq!(metrics[0].name, "Server.redis_version");
        assert_eq!(metrics[1].name, "Memory.used_memory");
    }

    #[test]
    fn key_space_counts_parse_database_indices() {
        let counts = key_space_counts(Some(
            "db0:keys=1,expires=0,avg_ttl=0\ndb3:keys=12,expires=2,avg_ttl=123\n".into(),
        ));
        assert_eq!(counts.get(&0), Some(&1));
        assert_eq!(counts.get(&3), Some(&12));
    }

    #[test]
    fn safety_classifies_read_write_and_destructive_commands() {
        assert_eq!(
            redis_command_safety("GET", None),
            RedisCommandSafety::ReadOnly
        );
        assert_eq!(
            redis_command_safety("SET", None),
            RedisCommandSafety::Write
        );
        assert_eq!(
            redis_command_safety("FLUSHALL", None),
            RedisCommandSafety::Destructive
        );
        assert_eq!(
            redis_command_safety("CONFIG", Some("GET")),
            RedisCommandSafety::ReadOnly
        );
        assert_eq!(
            redis_command_safety("CONFIG", Some("SET")),
            RedisCommandSafety::Destructive
        );
    }

    #[test]
    fn reply_converts_binary_to_base64_preview() {
        let reply = redis_reply(RedisValue::BulkString(vec![0, 1, 2]));
        assert_eq!(reply, RedisReply::BulkString {
            base64: "AAEC".into(),
            preview: None,
            length: 3,
        });
    }
}
