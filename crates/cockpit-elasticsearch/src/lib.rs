use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use cockpit_core::{
    CellValue, CockpitError, ColumnInfo, ColumnMeta, ConnectionInfo, ConnectionProfile,
    DatabaseDriver, DatabaseInfo, DatabaseObjectDefinition, DatabaseObjectKind, DriverSession,
    EventInfo, ExecuteQueryRequest, QueryResultPage, Result, RiskLevel, RoutineInfo,
    RoutineParameter, RowMutationRequest, RowMutationResult, ServerMetric, ServerProcessInfo,
    TableDetail, TableInfo, TriggerInfo, UserAccount, safety::assess_sql,
};
use reqwest::{Certificate, Client, Identity, Method, RequestBuilder};
use serde_json::{Value, json};
use uuid::Uuid;

const CELL_EDIT_MESSAGE: &str = "Elasticsearch 暂不支持行级写入，请通过行 JSON 查看器编辑整份文档";

#[derive(Default)]
pub struct ElasticsearchDriver;

pub struct EsSession {
    profile: ConnectionProfile,
    http: Client,
    base_url: String,
    auth: Option<(String, String)>,
    cluster_name: Mutex<Option<String>>,
    running: Mutex<HashMap<Uuid, futures::future::AbortHandle>>,
    cursor_cache: Mutex<Option<CursorState>>,
    // 索引名 → 可被 ES SQL 安全查询的标量列（排除 nested/object），供 SELECT * 展开
    star_columns_cache: Mutex<HashMap<String, Vec<String>>>,
}

#[derive(Clone)]
struct CursorState {
    sql: String,
    next_offset: usize,
    cursor: String,
    // 游标续页响应不携带列信息，缓存首页的列元数据供快路径复用
    columns: Vec<ColumnMeta>,
}

impl CursorState {
    fn usable_for(&self, sql: &str, row_offset: usize) -> Option<(usize, String)> {
        if self.sql == sql && self.next_offset <= row_offset {
            Some((row_offset - self.next_offset, self.cursor.clone()))
        } else {
            None
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct SqlColumn {
    name: String,
    #[serde(rename = "type")]
    es_type: String,
}

#[derive(Debug, serde::Deserialize)]
struct SqlQueryResponse {
    #[serde(default)]
    columns: Vec<SqlColumn>,
    #[serde(default)]
    rows: Vec<Vec<Value>>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct SearchHit {
    #[serde(default)]
    _id: String,
    // ES 7+ 的 hit 默认携带 _version，每次写操作自增，作为文档修改次数展示
    #[serde(default)]
    _version: Option<i64>,
    #[serde(default)]
    _source: Value,
}

#[derive(Debug, serde::Deserialize, Default)]
struct SearchHits {
    #[serde(default)]
    hits: Vec<SearchHit>,
}

#[derive(Debug, serde::Deserialize, Default)]
struct SearchResponse {
    #[serde(default)]
    hits: SearchHits,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn connection_error(error: reqwest::Error) -> CockpitError {
    CockpitError::Connection(error.to_string())
}

fn query_error(message: impl Into<String>) -> CockpitError {
    CockpitError::Query(message.into())
}

fn normalize_es_sql(sql: &str) -> &str {
    sql.trim().trim_end_matches(';').trim()
}

/// 识别 `SELECT * FROM <索引>` 形态的查询，返回索引名与星号的字节区间。
/// 仅匹配星号是唯一投影列的情况（COUNT(*) 等不会命中）。
fn star_projection(sql: &str) -> Option<(String, std::ops::Range<usize>)> {
    let trimmed = sql.trim();
    if !trimmed
        .get(..6)
        .is_some_and(|head| head.eq_ignore_ascii_case("select"))
    {
        return None;
    }
    let bytes = trimmed.as_bytes();
    let mut cursor = 6;
    while bytes
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'*') {
        return None;
    }
    let star_start = cursor;
    cursor += 1;
    while bytes
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        cursor += 1;
    }
    if !trimmed
        .get(cursor..cursor + 4)
        .is_some_and(|word| word.eq_ignore_ascii_case("from"))
    {
        return None;
    }
    cursor += 4;
    match bytes.get(cursor) {
        Some(byte) if byte.is_ascii_whitespace() || *byte == b'"' => {}
        _ => return None,
    }
    let target = trimmed[cursor..].trim_start();
    let (name, _) = if let Some(quoted) = target.strip_prefix('"') {
        let end = quoted.find('"')?;
        (quoted[..end].replace("\"\"", "\""), end + 2)
    } else {
        let end = target
            .find(|character: char| {
                !(character.is_alphanumeric() || matches!(character, '_' | '-' | '.'))
            })
            .unwrap_or(target.len());
        (target[..end].to_string(), end)
    };
    if name.is_empty() {
        return None;
    }
    let leading = sql.len() - sql.trim_start().len();
    Some((name, leading + star_start..leading + star_start + 1))
}

fn expand_star(sql: &str, star: std::ops::Range<usize>, columns: &[String]) -> Option<String> {
    if columns.is_empty() {
        return None;
    }
    let projection = columns
        .iter()
        .map(|column| format!("\"{}\"", column.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "{} {projection} {}",
        sql[..star.start].trim_end(),
        sql[star.end..].trim_start()
    ))
}

/// 从 ES SQL 的 "Arrays (returned by [field]) are not supported" 错误中提取字段名。
/// 该错误可能按 4xx（Query）也可能按 5xx（Connection）分类，两种都要识别。
fn unsupported_array_field(error: &CockpitError) -> Option<String> {
    let message = match error {
        CockpitError::Query(message) | CockpitError::Connection(message) => message,
        _ => return None,
    };
    let marker = "Arrays (returned by [";
    let start = message.find(marker)? + marker.len();
    let end = start + message[start..].find(']')?;
    Some(message[start..end].to_string())
}

fn scheme_for(tls_mode: cockpit_core::TlsMode) -> &'static str {
    match tls_mode {
        cockpit_core::TlsMode::Disabled => "http",
        _ => "https",
    }
}

fn base_url(profile: &ConnectionProfile) -> String {
    format!(
        "{}://{}:{}",
        scheme_for(profile.tls.mode),
        profile.host.trim(),
        profile.port
    )
}

fn build_client(profile: &ConnectionProfile) -> Result<Client> {
    let mut builder =
        Client::builder().connect_timeout(Duration::from_secs(profile.connect_timeout_secs.max(1)));
    match profile.tls.mode {
        cockpit_core::TlsMode::Disabled => {}
        cockpit_core::TlsMode::Preferred | cockpit_core::TlsMode::Required => {
            if profile.tls.ca_cert_path.is_none() {
                builder = builder.danger_accept_invalid_certs(true);
            }
        }
        cockpit_core::TlsMode::VerifyCa | cockpit_core::TlsMode::VerifyIdentity => {}
    }
    if let Some(ca_path) = profile.tls.ca_cert_path.as_deref() {
        let pem = std::fs::read(ca_path)
            .map_err(|error| CockpitError::InvalidConfig(format!("读取 CA 证书失败：{error}")))?;
        let certificate = Certificate::from_pem(&pem)
            .map_err(|error| CockpitError::InvalidConfig(format!("解析 CA 证书失败：{error}")))?;
        builder = builder.add_root_certificate(certificate);
    }
    if let (Some(cert), Some(key)) = (
        profile.tls.client_cert_path.as_deref(),
        profile.tls.client_key_path.as_deref(),
    ) {
        let mut pem = std::fs::read(cert)
            .map_err(|error| CockpitError::InvalidConfig(format!("读取客户端证书失败：{error}")))?;
        let key_pem = std::fs::read(key)
            .map_err(|error| CockpitError::InvalidConfig(format!("读取客户端私钥失败：{error}")))?;
        pem.extend_from_slice(&key_pem);
        let identity = Identity::from_pem(&pem)
            .map_err(|error| CockpitError::InvalidConfig(format!("解析客户端证书失败：{error}")))?;
        builder = builder.identity(identity);
    }
    builder.build().map_err(connection_error)
}

fn request_timeout(profile: &ConnectionProfile, override_secs: Option<u64>) -> Duration {
    Duration::from_secs(override_secs.unwrap_or(profile.query_timeout_secs).max(1))
}

fn parse_cluster_info(body: &Value) -> (String, String) {
    let version = body
        .pointer("/version/number")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let cluster = body
        .get("cluster_name")
        .and_then(Value::as_str)
        .unwrap_or("elasticsearch")
        .to_string();
    (version, cluster)
}

fn es_error_message(body: &Value) -> Option<String> {
    let error = body.get("error")?;
    if let Some(reason) = error.get("reason").and_then(Value::as_str) {
        let root_cause = error
            .pointer("/root_cause/0/reason")
            .and_then(Value::as_str)
            .unwrap_or(reason);
        return Some(root_cause.to_string());
    }
    Some(error.as_str()?.to_string())
}

/// 删除/新建索引这类管理请求由集群异步执行，acknowledged 才算提交成功。
fn ensure_acknowledged(body: &Value, action: &str) -> Result<()> {
    if body.get("acknowledged").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(query_error(format!("{action}未被集群确认")))
    }
}

/// `_delete_by_query` 的失败列表非空时，取第一条的原因用于报错。
fn delete_by_query_failure(body: &Value) -> Option<String> {
    let failure = body.get("failures")?.as_array()?.first()?;
    let reason = failure
        .pointer("/reason/reason")
        .and_then(Value::as_str)
        .or_else(|| failure.get("reason").and_then(Value::as_str))
        .unwrap_or("未知原因");
    Some(reason.to_string())
}

fn number_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

struct CatIndexEntry {
    name: String,
    docs: Option<u64>,
    bytes: Option<u64>,
    status: Option<String>,
}

fn parse_cat_indices(body: &Value) -> Vec<CatIndexEntry> {
    body.as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let name = entry.get("index")?.as_str()?.to_string();
                    let docs = entry
                        .get("docs.count")
                        .and_then(number_to_string)
                        .and_then(|text| text.parse::<u64>().ok());
                    let bytes = entry
                        .get("store.size")
                        .and_then(number_to_string)
                        .and_then(|text| text.parse::<u64>().ok());
                    let status = entry
                        .get("status")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    Some(CatIndexEntry {
                        name,
                        docs,
                        bytes,
                        status,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn flatten_properties(
    prefix: &str,
    properties: &serde_json::Map<String, Value>,
    out: &mut Vec<(String, String)>,
) {
    for (name, definition) in properties {
        let full = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        if let Some(es_type) = definition.get("type").and_then(Value::as_str) {
            if es_type != "object" {
                out.push((full.clone(), es_type.to_string()));
            }
        }
        if let Some(nested) = definition.get("properties").and_then(Value::as_object) {
            flatten_properties(&full, nested, out);
        }
    }
}

fn mapping_fields(mapping: &Value) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    if let Some(properties) = mapping
        .pointer("/mappings/properties")
        .and_then(Value::as_object)
    {
        flatten_properties("", properties, &mut fields);
    }
    fields.sort_by(|left, right| left.0.cmp(&right.0));
    fields
}

/// mapping 顶层字段及类型（大写对齐 ES SQL 的类型拼写）。
/// 与 `mapping_fields` 的展平叶子不同，object/nested 容器原样保留为整列，
/// 供 `_search` 路径把嵌套文档完整呈现为一个 JSON 单元格。
fn top_level_properties(mapping: &Value) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let Some(properties) = mapping
        .pointer("/mappings/properties")
        .and_then(Value::as_object)
    else {
        return fields;
    };
    for (name, definition) in properties {
        let es_type = match definition.get("type").and_then(Value::as_str) {
            Some(kind) => kind,
            None if definition.get("properties").is_some() => "object",
            None => continue,
        };
        fields.push((name.clone(), es_type.to_uppercase()));
    }
    fields.sort_by(|left, right| left.0.cmp(&right.0));
    fields
}

fn format_epoch_millis(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|moment| moment.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .unwrap_or_else(|| millis.to_string())
}

fn is_temporal_type(es_type: &str) -> bool {
    matches!(es_type, "DATE" | "DATETIME" | "TIME")
}

fn is_integer_type(es_type: &str) -> bool {
    matches!(es_type, "BYTE" | "SHORT" | "INTEGER" | "LONG")
}

fn is_float_type(es_type: &str) -> bool {
    matches!(
        es_type,
        "FLOAT" | "HALF_FLOAT" | "DOUBLE" | "SCALED_FLOAT" | "UNSIGNED_DOUBLE"
    )
}

fn json_value_to_cell(value: &Value, es_type: &str) -> CellValue {
    match value {
        Value::Null => CellValue::Null,
        Value::Bool(flag) => CellValue::Bool(*flag),
        Value::Number(number) => {
            if is_temporal_type(es_type) {
                return number.as_i64().map(format_epoch_millis).map_or_else(
                    || CellValue::Float(number.as_f64().unwrap_or_default()),
                    CellValue::DateTime,
                );
            }
            if is_float_type(es_type) {
                return CellValue::Float(number.as_f64().unwrap_or_default());
            }
            if es_type == "UNSIGNED_LONG" {
                return CellValue::Unsigned(number.to_string());
            }
            if is_integer_type(es_type) {
                return CellValue::Signed(number.to_string());
            }
            if let Some(int) = number.as_i64() {
                CellValue::Signed(int.to_string())
            } else if let Some(uint) = number.as_u64() {
                CellValue::Unsigned(uint.to_string())
            } else {
                CellValue::Float(number.as_f64().unwrap_or_default())
            }
        }
        Value::String(text) => {
            if is_temporal_type(es_type) {
                CellValue::DateTime(text.clone())
            } else if es_type == "UNSIGNED_LONG" {
                // UNSIGNED_LONG 到达 JSON 时是字符串，保持字符串承载避免精度丢失
                CellValue::Unsigned(text.clone())
            } else {
                CellValue::Text(text.clone())
            }
        }
        Value::Array(_) | Value::Object(_) => {
            CellValue::Json(serde_json::to_string(value).unwrap_or_default())
        }
    }
}

fn response_columns(columns: &[SqlColumn]) -> Vec<ColumnMeta> {
    columns
        .iter()
        .map(|column| ColumnMeta {
            name: column.name.clone(),
            database_type: column.es_type.clone(),
            nullable: true,
            unsigned: column.es_type == "UNSIGNED_LONG",
            binary: false,
        })
        .collect()
}

fn columns_from_mapping(mapping: &Value) -> Vec<ColumnInfo> {
    mapping_fields(mapping)
        .into_iter()
        .enumerate()
        .map(|(index, (name, es_type))| ColumnInfo {
            ordinal: index as u32 + 1,
            data_type: es_type.to_lowercase(),
            full_type: es_type,
            name,
            nullable: true,
            default_value: None,
            extra: None,
            comment: None,
            key: None,
            generation_expression: None,
            collation: None,
        })
        .collect()
}

/// `_search` 路径的列元数据：`_id`、`_version` 打头，其后为 mapping 顶层字段。
fn search_columns_from_mapping(mapping: &Value) -> Vec<ColumnMeta> {
    let mut columns = vec![
        ColumnMeta {
            name: "_id".into(),
            database_type: "KEYWORD".into(),
            nullable: false,
            unsigned: false,
            binary: false,
        },
        ColumnMeta {
            name: "_version".into(),
            database_type: "LONG".into(),
            nullable: true,
            unsigned: false,
            binary: false,
        },
    ];
    columns.extend(
        top_level_properties(mapping)
            .into_iter()
            .map(|(name, es_type)| ColumnMeta {
                unsigned: es_type == "UNSIGNED_LONG",
                database_type: es_type,
                name,
                nullable: true,
                binary: false,
            }),
    );
    columns
}

fn search_hit_to_row(hit: &SearchHit, columns: &[ColumnMeta]) -> Vec<CellValue> {
    let source = hit._source.as_object();
    columns
        .iter()
        .map(|meta| {
            if meta.name == "_id" {
                CellValue::Text(hit._id.clone())
            } else if meta.name == "_version" {
                hit._version
                    .map(|version| CellValue::Signed(version.to_string()))
                    .unwrap_or(CellValue::Null)
            } else {
                source
                    .and_then(|fields| fields.get(&meta.name))
                    .map(|value| json_value_to_cell(value, &meta.database_type))
                    .unwrap_or(CellValue::Null)
            }
        })
        .collect()
}

/// 解析 ES SQL 尾部的 `LIMIT <n>`（ES SQL 无 OFFSET 语法）。
/// translate 对不带 LIMIT 的查询也会输出默认 `size: 1000`，不能当作真实上限，
/// 因此 LIMIT 上限从 SQL 原文解析而不是透传 translate 的 size。
fn sql_limit(sql: &str) -> Option<u64> {
    let joined = sql
        .split_whitespace()
        .collect::<String>()
        .to_ascii_uppercase();
    let (_, digits) = joined.rsplit_once("LIMIT")?;
    if digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// 从 translate 响应中白名单抽取可透传给 `_search` 的字段。
/// `_source: false`、`fields` 等键被丢弃，`_search` 将默认返回完整 `_source`。
/// 聚合翻译结果（含 aggs）没有逐文档 hit，无法取回 `_id`，返回 None 走 SQL 回退。
fn translate_to_search_body(translated: &Value) -> Option<Value> {
    if translated.get("aggs").is_some() || translated.get("aggregations").is_some() {
        return None;
    }
    let mut body = serde_json::Map::new();
    if let Some(query) = translated.get("query") {
        body.insert("query".into(), query.clone());
    }
    if let Some(sort) = translated.get("sort") {
        body.insert("sort".into(), sort.clone());
    }
    Some(Value::Object(body))
}

/// 计算 `_search` 的取行数：多取 1 行用于判定 has_more；SQL 带 LIMIT 时以剩余额度封顶。
/// 已越过 LIMIT 时返回 None，调用方直接给出空页。
fn search_size(row_offset: usize, page_size: usize, limit: Option<u64>) -> Option<usize> {
    let remaining = match limit {
        Some(cap) => cap.checked_sub(row_offset as u64)?,
        None => u64::MAX,
    };
    if remaining == 0 {
        return None;
    }
    Some((page_size as u64 + 1).min(remaining) as usize)
}

/// 注入分页参数与稳定性排序：无排序时补 `_doc`，有排序时追加 `_doc` 消除同值行的翻页抖动。
fn finalize_search_body(mut body: Value, row_offset: usize, size: usize) -> Value {
    body["from"] = json!(row_offset);
    body["size"] = json!(size);
    match body.get_mut("sort") {
        Some(Value::Array(entries)) => entries.push(json!("_doc")),
        _ => body["sort"] = json!(["_doc"]),
    }
    body
}

/// WHERE 子句中一个被剥离的 `_id` 条件，等价于 `_search` 的 ids 查询。
#[derive(Debug, PartialEq)]
struct IdPredicate {
    values: Vec<String>,
    negated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SqlTokenKind {
    Word,
    String,
    QuotedIdent,
    Symbol,
}

/// SQL 词法单元。`depth` 记录 token 之前的括号深度，用于识别顶层 AND/OR。
#[derive(Debug)]
struct SqlToken {
    start: usize,
    end: usize,
    depth: usize,
    kind: SqlTokenKind,
}

impl SqlToken {
    fn text<'a>(&self, sql: &'a str) -> &'a str {
        &sql[self.start..self.end]
    }

    fn keyword(&self, sql: &str, word: &str) -> bool {
        self.kind == SqlTokenKind::Word && self.text(sql).eq_ignore_ascii_case(word)
    }

    fn symbol(&self, sql: &str, text: &str) -> bool {
        self.kind == SqlTokenKind::Symbol && self.text(sql) == text
    }

    /// 是否引用文档 `_id` 元字段（裸写或双引号标识符均可）。
    fn is_id(&self, sql: &str) -> bool {
        match self.kind {
            SqlTokenKind::Word => self.text(sql) == "_id",
            SqlTokenKind::QuotedIdent => self.text(sql) == "\"_id\"",
            _ => false,
        }
    }

    /// 提取数字或单引号字符串字面量作为 `_id` 取值。
    fn id_value(&self, sql: &str) -> Option<String> {
        let text = self.text(sql);
        match self.kind {
            SqlTokenKind::Word => (!text.is_empty()
                && text.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| text.to_string()),
            SqlTokenKind::String => Some(text[1..text.len() - 1].replace("''", "'")),
            _ => None,
        }
    }
}

fn tokenize_sql(sql: &str) -> Vec<SqlToken> {
    let bytes = sql.as_bytes();
    let mut tokens = Vec::new();
    let mut cursor = 0;
    let mut depth = 0usize;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if byte.is_ascii_whitespace() {
            cursor += 1;
            continue;
        }
        let start = cursor;
        let kind = if byte == b'\'' || byte == b'"' {
            // 引号内成对重复的引号是转义，成对消费避免提前断词
            cursor += 1;
            while cursor < bytes.len() {
                if bytes[cursor] == byte {
                    if bytes.get(cursor + 1) == Some(&byte) {
                        cursor += 2;
                        continue;
                    }
                    cursor += 1;
                    break;
                }
                cursor += 1;
            }
            if byte == b'\'' {
                SqlTokenKind::String
            } else {
                SqlTokenKind::QuotedIdent
            }
        } else if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$') {
            while cursor < bytes.len()
                && (bytes[cursor].is_ascii_alphanumeric() || matches!(bytes[cursor], b'_' | b'$'))
            {
                cursor += 1;
            }
            SqlTokenKind::Word
        } else {
            while cursor < bytes.len()
                && !bytes[cursor].is_ascii_whitespace()
                && !bytes[cursor].is_ascii_alphanumeric()
                && !matches!(bytes[cursor], b'_' | b'$' | b'\'' | b'"')
            {
                cursor += 1;
            }
            let text = &sql[start..cursor];
            depth = (depth + text.matches('(').count()).saturating_sub(text.matches(')').count());
            SqlTokenKind::Symbol
        };
        tokens.push(SqlToken {
            start,
            end: cursor,
            depth,
            kind,
        });
    }
    tokens
}

/// 解析单个 `_id` 合取项：`_id = v`、`_id != v`、`_id IN (...)`、`_id NOT IN (...)`。
fn parse_id_conjunct(conjunct: &[SqlToken], sql: &str) -> Option<IdPredicate> {
    let first = conjunct.first()?;
    if !first.is_id(sql) {
        return None;
    }
    let second = conjunct.get(1)?;
    let value = |token: &SqlToken| token.id_value(sql);
    if second.symbol(sql, "=") || second.symbol(sql, "!=") || second.symbol(sql, "<>") {
        if conjunct.len() != 3 {
            return None;
        }
        return Some(IdPredicate {
            values: vec![value(conjunct.get(2)?)?],
            negated: !second.symbol(sql, "="),
        });
    }
    let mut index = 1;
    let mut negated = false;
    if second.keyword(sql, "NOT") {
        negated = true;
        index += 1;
    }
    if !conjunct.get(index)?.keyword(sql, "IN") {
        return None;
    }
    index += 1;
    if !conjunct.get(index)?.symbol(sql, "(") {
        return None;
    }
    index += 1;
    let close = conjunct.len().checked_sub(1)?;
    if !conjunct[close].symbol(sql, ")") || close <= index {
        return None;
    }
    let mut values = Vec::new();
    while index < close {
        values.push(value(&conjunct[index])?);
        index += 1;
        if index < close {
            if !conjunct[index].symbol(sql, ",") {
                return None;
            }
            index += 1;
        }
    }
    Some(IdPredicate { values, negated })
}

/// 把 WHERE 子句里的 `_id` 条件从 SQL 中剥离，返回改写后的 SQL 与等价的 ids 过滤。
/// ES SQL 不认识 `_id` 元字段，而 `_search` 的 ids 查询原生支持按文档 id 过滤。
/// 返回 None 表示 `_id` 出现在无法安全改写的位置（OR、LIKE、排序等），
/// 调用方保留原 SQL，继续走 translate/ES SQL 原有的报错回退。
fn extract_id_predicates(sql: &str) -> Option<(String, Vec<IdPredicate>)> {
    let tokens = tokenize_sql(sql);
    if !tokens.iter().any(|token| token.is_id(sql)) {
        return Some((sql.to_string(), Vec::new()));
    }
    let where_index = tokens
        .iter()
        .position(|token| token.depth == 0 && token.keyword(sql, "WHERE"))?;
    // _id 出现在 WHERE 之外（如 ORDER BY _id）时无法等价改写
    if tokens[..where_index].iter().any(|token| token.is_id(sql)) {
        return None;
    }
    let clause_end = tokens
        .iter()
        .skip(where_index + 1)
        .position(|token| {
            token.depth == 0 && (token.keyword(sql, "ORDER") || token.keyword(sql, "LIMIT"))
        })
        .map(|offset| where_index + 1 + offset)
        .unwrap_or(tokens.len());
    if tokens[clause_end..].iter().any(|token| token.is_id(sql)) {
        return None;
    }

    // 仅拆分顶层 AND 连接的合取项；括号内的 AND 不受影响，OR 会留在合取项里导致解析失败
    let clause = &tokens[where_index + 1..clause_end];
    let mut conjunct_spans: Vec<(usize, usize)> = Vec::new();
    let mut span_start = 0;
    for (index, token) in clause.iter().enumerate() {
        if token.depth == 0 && token.keyword(sql, "AND") {
            conjunct_spans.push((span_start, index));
            span_start = index + 1;
        }
    }
    conjunct_spans.push((span_start, clause.len()));

    let mut predicates = Vec::new();
    let mut kept: Vec<(usize, usize)> = Vec::new();
    for (start, end) in conjunct_spans {
        let conjunct = &clause[start..end];
        if conjunct.is_empty() {
            continue;
        }
        if conjunct.iter().any(|token| token.is_id(sql)) {
            predicates.push(parse_id_conjunct(conjunct, sql)?);
        } else {
            kept.push((start, end));
        }
    }

    let mut rewritten = String::from(&sql[..tokens[where_index].start]);
    if !kept.is_empty() {
        rewritten.push_str("WHERE");
        for (index, (start, end)) in kept.iter().enumerate() {
            if index > 0 {
                rewritten.push_str(" AND");
            }
            rewritten.push(' ');
            rewritten.push_str(sql[clause[*start].start..clause[*end - 1].end].trim());
        }
    }
    let suffix_start = clause
        .last()
        .map(|token| token.end)
        .unwrap_or(tokens[where_index].end);
    let suffix = sql[suffix_start..].trim_start();
    rewritten = rewritten.trim_end().to_string();
    if !suffix.is_empty() {
        rewritten.push('\n');
    }
    rewritten.push_str(suffix);
    Some((rewritten, predicates))
}

/// 把剥离出的 `_id` 条件合并进 translate 得到的 `_search` 请求体：
/// 肯定条件并入 bool.filter，否定条件并入 bool.must_not。
fn apply_id_predicates(body: &mut Value, predicates: &[IdPredicate]) {
    if predicates.is_empty() {
        return;
    }
    let previous = body
        .get("query")
        .cloned()
        .unwrap_or_else(|| json!({ "match_all": {} }));
    let mut filter = vec![previous];
    let mut must_not = Vec::new();
    for predicate in predicates {
        let ids = json!({ "ids": { "values": predicate.values } });
        if predicate.negated {
            must_not.push(ids);
        } else {
            filter.push(ids);
        }
    }
    let mut boolean = serde_json::Map::new();
    boolean.insert("filter".into(), json!(filter));
    if !must_not.is_empty() {
        boolean.insert("must_not".into(), json!(must_not));
    }
    body["query"] = json!({ "bool": Value::Object(boolean) });
}

/// ES 索引名会拼进 URL 路径，按 ES 命名规则限制字符集，防路径注入。
fn is_valid_index_name(index: &str) -> bool {
    !index.is_empty()
        && !index.starts_with(['-', '_', '+'])
        && index != "."
        && index != ".."
        && index.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

/// 文档 `_id` 允许任意字符串，进 URL 前除字母数字与 `-._~` 外全部百分号编码。
fn percent_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

impl EsSession {
    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        let mut request = self
            .http
            .request(method, format!("{}{}", self.base_url, path));
        if let Some((user, password)) = &self.auth {
            request = request.basic_auth(user, Some(password));
        }
        request
    }

    async fn send(
        &self,
        execution_id: Uuid,
        timeout: Duration,
        request: RequestBuilder,
    ) -> Result<Value> {
        let future = async move {
            let response = request.send().await?;
            let status = response.status();
            let body: Value = response.json().await.unwrap_or(Value::Null);
            Ok::<_, reqwest::Error>((status, body))
        };
        let (abortable, abort_handle) = futures::future::abortable(future);
        lock(&self.running).insert(execution_id, abort_handle);
        let outcome = tokio::time::timeout(timeout, abortable).await;
        lock(&self.running).remove(&execution_id);
        match outcome {
            Ok(Ok(Ok((status, body)))) => {
                if status.is_success() {
                    return Ok(body);
                }
                let message = es_error_message(&body);
                // SQL 能力类错误（如数组字段不支持）即使被 ES 归为 5xx，
                // 本质也是查询层面的限制，按查询失败处理以便驱动重试。
                if message
                    .as_deref()
                    .is_some_and(|text| text.contains("are not supported"))
                {
                    return Err(query_error(
                        message.unwrap_or_else(|| format!("请求失败：{status}")),
                    ));
                }
                if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    Err(CockpitError::Connection(
                        message.unwrap_or_else(|| "认证失败或权限不足".into()),
                    ))
                } else if status.is_server_error() {
                    Err(CockpitError::Connection(
                        message.unwrap_or_else(|| format!("服务端错误：{status}")),
                    ))
                } else {
                    Err(query_error(
                        message.unwrap_or_else(|| format!("请求失败：{status}")),
                    ))
                }
            }
            Ok(Ok(Err(error))) => Err(connection_error(error)),
            Ok(Err(_aborted)) => Err(CockpitError::Canceled),
            Err(_elapsed) => Err(CockpitError::Timeout),
        }
    }

    async fn fetch_root(&self, timeout: Duration) -> Result<(String, String)> {
        let body = self
            .send(Uuid::new_v4(), timeout, self.request(Method::GET, "/"))
            .await?;
        Ok(parse_cluster_info(&body))
    }

    async fn fetch_mapping(&self, table: &str, timeout: Duration) -> Result<Value> {
        let body = self
            .send(
                Uuid::new_v4(),
                timeout,
                self.request(Method::GET, &format!("/{table}/_mapping")),
            )
            .await?;
        body.as_object()
            .and_then(|entries| entries.values().next())
            .cloned()
            .ok_or_else(|| CockpitError::NotFound(format!("索引不存在：{table}")))
    }

    async fn close_cursor(&self, cursor: &str) {
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            self.request(Method::POST, "/_sql/close")
                .json(&json!({ "cursor": cursor }))
                .send(),
        )
        .await;
    }

    /// 返回索引可被 ES SQL 安全查询的标量列（排除 nested/object）。
    async fn safe_columns(&self, table: &str, timeout: Duration) -> Option<Vec<String>> {
        if let Some(columns) = lock(&self.star_columns_cache).get(table) {
            return Some(columns.clone());
        }
        let mapping = self.fetch_mapping(table, timeout).await.ok()?;
        let columns: Vec<String> = mapping_fields(&mapping)
            .into_iter()
            .filter(|(_, es_type)| es_type != "nested" && es_type != "object")
            .map(|(name, _)| name)
            .collect();
        if columns.is_empty() {
            return None;
        }
        lock(&self.star_columns_cache).insert(table.to_string(), columns.clone());
        Some(columns)
    }

    /// `SELECT * FROM <索引>` 专用执行路径：`_sql/translate` 把 SQL 翻译成 Query DSL
    /// 后走 `_search` 执行。ES SQL 无法返回文档 `_id` 与嵌套结构，而 `_search` 的每个
    /// hit 天然携带 `_id`，`_source` 也能完整带回数组/对象字段。
    /// 返回 `Ok(None)` 表示该查询不适合此路径（语法不支持、聚合或索引不存在），
    /// 调用方回退 ES SQL；连接/认证/超时/取消类错误不回退，直接上抛。
    async fn execute_search(
        &self,
        table: &str,
        sql: &str,
        request: &ExecuteQueryRequest,
        timeout: Duration,
        page_size: usize,
    ) -> Result<Option<QueryResultPage>> {
        let started = Instant::now();
        // ES SQL 不认识 _id 元字段：先剥离 WHERE 中的 _id 条件，等价改写为 ids 过滤；
        // 无法安全改写时保留原 SQL，交给 translate/ES SQL 走原有报错路径。
        let (search_sql, id_predicates) =
            extract_id_predicates(sql).unwrap_or_else(|| (sql.to_string(), Vec::new()));
        let translated = match self
            .send(
                request.execution_id,
                timeout,
                self.request(Method::POST, "/_sql/translate")
                    .json(&json!({ "query": search_sql })),
            )
            .await
        {
            Ok(value) => value,
            Err(CockpitError::Query(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        let Some(mut base_body) = translate_to_search_body(&translated) else {
            return Ok(None);
        };
        apply_id_predicates(&mut base_body, &id_predicates);
        let limit = sql_limit(sql);
        let mapping = match self.fetch_mapping(table, timeout).await {
            Ok(mapping) => mapping,
            Err(CockpitError::Query(_)) | Err(CockpitError::NotFound(_)) => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let columns = search_columns_from_mapping(&mapping);
        let page = |rows: Vec<Vec<CellValue>>, has_more: bool| QueryResultPage {
            execution_id: request.execution_id,
            columns: columns.clone(),
            rows,
            affected_rows: 0,
            execution_time_ms: started.elapsed().as_millis(),
            truncated: has_more,
            has_more,
            result_set_index: 0,
            messages: Vec::new(),
            row_offset: request.row_offset,
            page_size,
            additional_result_sets: Vec::new(),
            // 记录来源索引，前端据此支持整文档编辑后写回
            source_table: Some(table.to_string()),
        };
        let Some(size) = search_size(request.row_offset, page_size, limit) else {
            return Ok(Some(page(Vec::new(), false)));
        };
        let body = finalize_search_body(base_body, request.row_offset, size);
        let raw = match self
            .send(
                request.execution_id,
                timeout,
                // hit 默认不携带 _version，需显式请求（7.x/8.x 行为一致）
                self.request(Method::POST, &format!("/{table}/_search?version=true"))
                    .json(&body),
            )
            .await
        {
            Ok(value) => value,
            Err(CockpitError::Query(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        let response: SearchResponse = serde_json::from_value(raw)
            .map_err(|error| query_error(format!("解析 Elasticsearch 搜索响应失败：{error}")))?;
        let has_more = response.hits.hits.len() > page_size;
        let rows = response
            .hits
            .hits
            .iter()
            .take(page_size)
            .map(|hit| search_hit_to_row(hit, &columns))
            .collect();
        // from+size 翻页无游标可复用，关闭可能残留的 SQL 游标避免悬挂
        let stale = lock(&self.cursor_cache).take();
        if let Some(state) = stale {
            self.close_cursor(&state.cursor).await;
        }
        Ok(Some(page(rows, has_more)))
    }

    async fn execute_sql(
        &self,
        sql: &str,
        request: &ExecuteQueryRequest,
        timeout: Duration,
        page_size: usize,
    ) -> Result<QueryResultPage> {
        let started = Instant::now();
        let mut skip = request.row_offset;
        let mut cursor: Option<String> = None;
        let mut columns: Vec<ColumnMeta> = Vec::new();
        let cached = {
            let guard = lock(&self.cursor_cache);
            guard.as_ref().and_then(|state| {
                state
                    .usable_for(sql, request.row_offset)
                    .map(|(skip, cursor)| (skip, cursor, state.columns.clone()))
            })
        };
        if let Some((offset_delta, cached_cursor, cached_columns)) = cached {
            skip = offset_delta;
            cursor = Some(cached_cursor);
            columns = cached_columns;
        } else {
            let stale = lock(&self.cursor_cache).take();
            if let Some(state) = stale {
                self.close_cursor(&state.cursor).await;
            }
        }

        let mut rows: Vec<Vec<CellValue>> = Vec::with_capacity(page_size);
        let mut current_cursor = cursor;
        let latest_cursor;
        let has_more;
        loop {
            let body = match &current_cursor {
                Some(value) => json!({ "cursor": value }),
                None => json!({ "query": sql, "fetch_size": page_size }),
            };
            let raw = self
                .send(
                    request.execution_id,
                    timeout,
                    self.request(Method::POST, "/_sql").json(&body),
                )
                .await?;
            let response: SqlQueryResponse = serde_json::from_value(raw).map_err(|error| {
                query_error(format!("解析 Elasticsearch SQL 响应失败：{error}"))
            })?;
            if columns.is_empty() {
                columns = response_columns(&response.columns);
            }
            let mut overflow = false;
            for row in &response.rows {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                if rows.len() < page_size {
                    // 游标续页响应不携带 columns，复用首次响应缓存的列元数据
                    rows.push(
                        columns
                            .iter()
                            .zip(row)
                            .map(|(meta, value)| json_value_to_cell(value, &meta.database_type))
                            .collect(),
                    );
                } else {
                    overflow = true;
                    break;
                }
            }
            if rows.len() == page_size {
                // 页已取满：本响应还有溢出行或服务端仍有游标，都说明后面还有数据
                has_more = overflow || response.cursor.is_some();
                latest_cursor = response.cursor;
                break;
            }
            match response.cursor {
                Some(value) => current_cursor = Some(value),
                None => {
                    latest_cursor = None;
                    has_more = false;
                    break;
                }
            }
        }

        {
            let mut guard = lock(&self.cursor_cache);
            match latest_cursor {
                Some(value) => {
                    *guard = Some(CursorState {
                        sql: sql.to_string(),
                        next_offset: request.row_offset + rows.len(),
                        cursor: value,
                        columns: columns.clone(),
                    });
                }
                None => *guard = None,
            }
        }

        Ok(QueryResultPage {
            execution_id: request.execution_id,
            columns,
            rows,
            affected_rows: 0,
            execution_time_ms: started.elapsed().as_millis(),
            truncated: has_more,
            has_more,
            result_set_index: 0,
            messages: Vec::new(),
            row_offset: request.row_offset,
            page_size,
            additional_result_sets: Vec::new(),
            source_table: None,
        })
    }
}

#[async_trait]
impl DatabaseDriver for ElasticsearchDriver {
    fn kind(&self) -> &'static str {
        "elasticsearch"
    }

    async fn test(&self, profile: &ConnectionProfile, password: &str) -> Result<ConnectionInfo> {
        let session = self.open(profile.clone(), password.to_string()).await?;
        session.connection_info().await
    }

    async fn open(
        &self,
        profile: ConnectionProfile,
        password: String,
    ) -> Result<Arc<dyn DriverSession>> {
        let http = build_client(&profile)?;
        let auth = if profile.username.trim().is_empty() {
            None
        } else {
            Some((profile.username.trim().to_string(), password))
        };
        Ok(Arc::new(EsSession {
            cluster_name: Mutex::new(None),
            base_url: base_url(&profile),
            auth,
            http,
            profile,
            running: Mutex::new(HashMap::new()),
            cursor_cache: Mutex::new(None),
            star_columns_cache: Mutex::new(HashMap::new()),
        }))
    }
}

#[async_trait]
impl DriverSession for EsSession {
    fn connection_id(&self) -> Uuid {
        self.profile.id
    }

    async fn connection_info(&self) -> Result<ConnectionInfo> {
        let timeout = request_timeout(&self.profile, None);
        let (version, cluster) = self.fetch_root(timeout).await?;
        *lock(&self.cluster_name) = Some(cluster.clone());
        Ok(ConnectionInfo {
            server_version: version,
            server_comment: Some(format!("Elasticsearch 集群 {cluster}")),
            connection_id: 0,
            current_database: Some(cluster),
            tls_cipher: if scheme_for(self.profile.tls.mode) == "https" {
                Some("TLS".into())
            } else {
                None
            },
        })
    }

    async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        let timeout = request_timeout(&self.profile, None);
        let (_, cluster) = self.fetch_root(timeout).await?;
        *lock(&self.cluster_name) = Some(cluster.clone());
        Ok(vec![DatabaseInfo { name: cluster }])
    }

    async fn list_tables(
        &self,
        database: &str,
        filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TableInfo>> {
        let timeout = request_timeout(&self.profile, None);
        let body = self
            .send(
                Uuid::new_v4(),
                timeout,
                self.request(
                    Method::GET,
                    "/_cat/indices?format=json&bytes=b&h=index,docs.count,store.size,status",
                ),
            )
            .await?;
        let needle = filter.map(|value| value.to_lowercase());
        let mut entries = parse_cat_indices(&body);
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries
            .into_iter()
            .filter(|entry| {
                needle
                    .as_deref()
                    .is_none_or(|needle| entry.name.to_lowercase().contains(needle))
            })
            .skip(offset)
            .take(limit)
            .map(|entry| TableInfo {
                // 与其他驱动一致回填所属库名（集群伪库），否则前端会误判
                // “表不在当前库”而折叠左侧导航树
                database: database.to_string(),
                name: entry.name,
                table_type: "BASE TABLE".into(),
                comment: entry.status,
                estimated_rows: entry.docs,
                total_bytes: entry.bytes,
            })
            .collect())
    }

    async fn list_columns(&self, _database: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let timeout = request_timeout(&self.profile, None);
        let mapping = self.fetch_mapping(table, timeout).await?;
        Ok(columns_from_mapping(&mapping))
    }

    async fn table_detail(&self, database: &str, table: &str) -> Result<TableDetail> {
        let timeout = request_timeout(&self.profile, None);
        let (mapping, settings) = tokio::try_join!(self.fetch_mapping(table, timeout), async {
            self.send(
                Uuid::new_v4(),
                timeout,
                self.request(Method::GET, &format!("/{table}/_settings")),
            )
            .await
        })?;
        let columns = columns_from_mapping(&mapping);
        let ddl = serde_json::to_string_pretty(&json!({
            "settings": settings,
            "mappings": mapping,
        }))
        .map_err(|error| query_error(error.to_string()))?;
        Ok(TableDetail {
            table: TableInfo {
                database: database.to_string(),
                name: table.to_string(),
                table_type: "BASE TABLE".into(),
                comment: None,
                estimated_rows: None,
                total_bytes: None,
            },
            columns,
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
            ddl,
        })
    }

    async fn list_routines(&self, _database: &str) -> Result<Vec<RoutineInfo>> {
        Ok(Vec::new())
    }

    async fn list_triggers(&self, _database: &str) -> Result<Vec<TriggerInfo>> {
        Ok(Vec::new())
    }

    async fn list_events(&self, _database: &str) -> Result<Vec<EventInfo>> {
        Ok(Vec::new())
    }

    async fn object_definition(
        &self,
        _database: &str,
        _kind: DatabaseObjectKind,
        _name: &str,
    ) -> Result<DatabaseObjectDefinition> {
        Err(CockpitError::Unsupported(
            "Elasticsearch 没有视图、存储过程等数据库对象".into(),
        ))
    }

    async fn routine_parameters(
        &self,
        _database: &str,
        _name: &str,
    ) -> Result<Vec<RoutineParameter>> {
        Ok(Vec::new())
    }

    async fn list_processes(&self) -> Result<Vec<ServerProcessInfo>> {
        Ok(Vec::new())
    }

    async fn kill_process(&self, _process_id: u64) -> Result<()> {
        Err(CockpitError::Unsupported(
            "Elasticsearch 驱动暂不支持终止任务".into(),
        ))
    }

    async fn server_status(&self) -> Result<Vec<ServerMetric>> {
        let timeout = request_timeout(&self.profile, None);
        let health = self
            .send(
                Uuid::new_v4(),
                timeout,
                self.request(Method::GET, "/_cluster/health"),
            )
            .await?;
        let stats = self
            .send(
                Uuid::new_v4(),
                timeout,
                self.request(
                    Method::GET,
                    "/_nodes/stats?filter_path=_nodes.total,nodes.*.jvm.mem.heap_used_in_bytes,nodes.*.indices.docs.count",
                ),
            )
            .await?;
        let metric = |name: &str, value: &Value| ServerMetric {
            name: name.into(),
            value: value.as_str().map_or_else(
                || value.to_string().trim_matches('"').to_string(),
                str::to_string,
            ),
        };
        let mut metrics = vec![
            metric(
                "cluster_status",
                health.get("status").unwrap_or(&Value::Null),
            ),
            metric(
                "number_of_nodes",
                health.get("number_of_nodes").unwrap_or(&Value::Null),
            ),
            metric(
                "active_shards",
                health.get("active_shards").unwrap_or(&Value::Null),
            ),
            metric(
                "active_primary_shards",
                health.get("active_primary_shards").unwrap_or(&Value::Null),
            ),
            metric(
                "unassigned_shards",
                health.get("unassigned_shards").unwrap_or(&Value::Null),
            ),
        ];
        let nodes = stats.get("nodes").and_then(Value::as_object);
        let docs: u64 = nodes
            .map(|entries| {
                entries
                    .values()
                    .filter_map(|node| node.pointer("/indices/docs/count").and_then(Value::as_u64))
                    .sum()
            })
            .unwrap_or_default();
        let heap: u64 = nodes
            .map(|entries| {
                entries
                    .values()
                    .filter_map(|node| {
                        node.pointer("/jvm/mem/heap_used_in_bytes")
                            .and_then(Value::as_u64)
                    })
                    .sum()
            })
            .unwrap_or_default();
        metrics.push(ServerMetric {
            name: "docs_count".into(),
            value: docs.to_string(),
        });
        metrics.push(ServerMetric {
            name: "heap_used_bytes".into(),
            value: heap.to_string(),
        });
        Ok(metrics)
    }

    async fn list_users(&self) -> Result<Vec<UserAccount>> {
        Ok(Vec::new())
    }

    async fn user_grants(&self, _user: &str, _host: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn execute(&self, request: ExecuteQueryRequest) -> Result<QueryResultPage> {
        let assessment = assess_sql(&request.sql);
        if self.profile.read_only && assessment.risk != RiskLevel::Safe {
            return Err(CockpitError::Query("该连接处于只读模式".into()));
        }
        if assessment.requires_confirmation && !request.allow_write {
            return Err(CockpitError::Query(
                assessment
                    .reason
                    .unwrap_or_else(|| "该语句需要确认后执行".into()),
            ));
        }
        let timeout = request_timeout(&self.profile, request.timeout_secs);
        let page_size = request.page_size.clamp(1, 10_000);
        // ES SQL 解析器不接受尾部分号，统一剥掉
        let sql = normalize_es_sql(&request.sql);
        if let Some((table, star)) = star_projection(sql) {
            // 星号查询优先走 _search 路径取回 _id 与完整嵌套结构，不适用的查询自动回退
            if let Some(page) = self
                .execute_search(&table, sql, &request, timeout, page_size)
                .await?
            {
                return Ok(page);
            }
            if let Some(mut columns) = self.safe_columns(&table, timeout).await {
                // ES SQL 无法返回数组/对象字段。把 * 展开为显式列；若仍有字段在
                // 文档层面存了数组值（mapping 上不可见），按报错逐个剔除后重试。
                loop {
                    let Some(expanded) = expand_star(sql, star.clone(), &columns) else {
                        break;
                    };
                    match self
                        .execute_sql(&expanded, &request, timeout, page_size)
                        .await
                    {
                        Ok(page) => return Ok(page),
                        Err(error) => match unsupported_array_field(&error) {
                            Some(field) => {
                                let prefix = format!("{field}.");
                                let before = columns.len();
                                columns.retain(|column| {
                                    column != &field && !column.starts_with(&prefix)
                                });
                                if columns.len() == before {
                                    return Err(error);
                                }
                            }
                            None => return Err(error),
                        },
                    }
                }
            }
        }
        self.execute_sql(sql, &request, timeout, page_size).await
    }

    async fn mutate_row(&self, _request: RowMutationRequest) -> Result<RowMutationResult> {
        Err(CockpitError::Unsupported(CELL_EDIT_MESSAGE.into()))
    }

    /// 按文档 `_id` 全量替换整份文档（PUT /{index}/_doc/{id}）。
    /// 供行 JSON 查看器编辑后保存。
    async fn update_document(&self, index: &str, document_id: &str, source: &Value) -> Result<()> {
        if self.profile.read_only {
            return Err(CockpitError::Query("该连接处于只读模式".into()));
        }
        if !is_valid_index_name(index) {
            return Err(CockpitError::Query(format!("非法索引名：{index}")));
        }
        if document_id.is_empty() {
            return Err(CockpitError::Query("文档 _id 不能为空".into()));
        }
        let timeout = request_timeout(&self.profile, None);
        let body = self
            .send(
                Uuid::new_v4(),
                timeout,
                self.request(
                    Method::PUT,
                    &format!("/{index}/_doc/{}", percent_encode(document_id)),
                )
                .json(source),
            )
            .await?;
        let result = body.get("result").and_then(Value::as_str).unwrap_or("");
        if !matches!(result, "created" | "updated" | "noop") {
            return Err(query_error(format!("更新文档失败：{result}")));
        }
        // 管理工具场景：保存后立即刷新索引，让重新查询马上反映新值
        let _ = self
            .send(
                Uuid::new_v4(),
                Duration::from_secs(5),
                self.request(Method::POST, &format!("/{index}/_refresh")),
            )
            .await;
        Ok(())
    }

    /// 删除整个索引（DELETE /{index}），mapping、设置与全部文档一并移除。
    async fn delete_index(&self, index: &str) -> Result<()> {
        if self.profile.read_only {
            return Err(CockpitError::Query("该连接处于只读模式".into()));
        }
        if !is_valid_index_name(index) {
            return Err(CockpitError::Query(format!("非法索引名：{index}")));
        }
        let timeout = request_timeout(&self.profile, None);
        let body = self
            .send(
                Uuid::new_v4(),
                timeout,
                self.request(Method::DELETE, &format!("/{index}")),
            )
            .await?;
        ensure_acknowledged(&body, "删除索引")?;
        // 索引已不存在：相关列缓存与可能还挂着该索引的 SQL 游标一并作废
        lock(&self.star_columns_cache).remove(index);
        let stale = lock(&self.cursor_cache).take();
        if let Some(state) = stale {
            self.close_cursor(&state.cursor).await;
        }
        Ok(())
    }

    /// 清空索引中的全部文档但保留索引与 mapping（POST /{index}/_delete_by_query）。
    /// 返回删除的文档数。
    async fn clear_index(&self, index: &str) -> Result<u64> {
        if self.profile.read_only {
            return Err(CockpitError::Query("该连接处于只读模式".into()));
        }
        if !is_valid_index_name(index) {
            return Err(CockpitError::Query(format!("非法索引名：{index}")));
        }
        let timeout = request_timeout(&self.profile, None);
        let body = self
            .send(
                Uuid::new_v4(),
                timeout,
                // conflicts=proceed：清空语义下版本冲突的文档也继续删除；
                // slices=auto 并行删大批量索引；refresh=true 完成后立即可查
                self.request(
                    Method::POST,
                    &format!(
                        "/{index}/_delete_by_query?conflicts=proceed&slices=auto&refresh=true"
                    ),
                )
                .json(&json!({ "query": { "match_all": {} } })),
            )
            .await?;
        if let Some(reason) = delete_by_query_failure(&body) {
            return Err(query_error(format!("清空索引失败：{reason}")));
        }
        Ok(body.get("deleted").and_then(Value::as_u64).unwrap_or(0))
    }

    /// 创建索引（PUT /{index}）。body 可携带 settings/mappings，
    /// 为空时按动态 mapping 创建，字段随首份文档自动推断。
    async fn create_index(&self, name: &str, body: Option<&Value>) -> Result<()> {
        if self.profile.read_only {
            return Err(CockpitError::Query("该连接处于只读模式".into()));
        }
        if !is_valid_index_name(name) {
            return Err(CockpitError::Query(format!("非法索引名：{name}")));
        }
        let timeout = request_timeout(&self.profile, None);
        let request = match body {
            Some(definition) => self
                .request(Method::PUT, &format!("/{name}"))
                .json(definition),
            None => self
                .request(Method::PUT, &format!("/{name}"))
                .json(&json!({})),
        };
        let response = self.send(Uuid::new_v4(), timeout, request).await?;
        ensure_acknowledged(&response, "创建索引")?;
        Ok(())
    }

    async fn insert_rows(
        &self,
        _database: &str,
        _table: &str,
        _columns: &[String],
        _rows: &[Vec<CellValue>],
    ) -> Result<u64> {
        Err(CockpitError::Unsupported(CELL_EDIT_MESSAGE.into()))
    }

    async fn begin_transaction(&self) -> Result<()> {
        Ok(())
    }
    async fn commit_transaction(&self) -> Result<()> {
        Ok(())
    }
    async fn rollback_transaction(&self) -> Result<()> {
        Ok(())
    }
    async fn transaction_active(&self) -> bool {
        false
    }

    async fn cancel(&self, execution_id: Uuid) -> Result<bool> {
        let handle = lock(&self.running).remove(&execution_id);
        match handle {
            Some(handle) => {
                handle.abort();
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn close(&self) -> Result<()> {
        let stale = lock(&self.cursor_cache).take();
        if let Some(state) = stale {
            self.close_cursor(&state.cursor).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use cockpit_core::{DatabaseKind, TlsMode, TlsOptions};

    use super::*;

    fn profile(tls_mode: TlsMode) -> ConnectionProfile {
        let now = Utc::now();
        ConnectionProfile {
            id: Uuid::new_v4(),
            driver_kind: DatabaseKind::Elasticsearch,
            group: None,
            name: "local".into(),
            host: "127.0.0.1".into(),
            port: 9200,
            username: String::new(),
            database: None,
            tls: TlsOptions {
                mode: tls_mode,
                ..TlsOptions::default()
            },
            ssh: None,
            connect_timeout_secs: 5,
            query_timeout_secs: 30,
            pool_size: 5,
            read_only: false,
            production: false,
            color: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn base_url_follows_tls_mode() {
        assert_eq!(
            base_url(&profile(TlsMode::Disabled)),
            "http://127.0.0.1:9200"
        );
        assert_eq!(
            base_url(&profile(TlsMode::Required)),
            "https://127.0.0.1:9200"
        );
    }

    #[test]
    fn cluster_info_parses_version_and_name() {
        let body = json!({
            "name": "node-1",
            "cluster_name": "docker-cluster",
            "version": { "number": "8.15.0" }
        });
        assert_eq!(
            parse_cluster_info(&body),
            ("8.15.0".into(), "docker-cluster".into())
        );
    }

    #[test]
    fn error_reason_prefers_root_cause() {
        let body = json!({
            "error": {
                "type": "parsing_exception",
                "reason": "outer reason",
                "root_cause": [{ "type": "parsing_exception", "reason": "root reason" }]
            },
            "status": "400"
        });
        assert_eq!(es_error_message(&body).as_deref(), Some("root reason"));
    }

    #[test]
    fn acknowledged_responses_pass_management_check() {
        assert!(ensure_acknowledged(&json!({ "acknowledged": true }), "删除索引").is_ok());
        let denied = ensure_acknowledged(&json!({ "acknowledged": false }), "删除索引");
        assert!(
            matches!(denied, Err(CockpitError::Query(message)) if message.contains("删除索引未被集群确认"))
        );
        // 响应缺失 acknowledged 字段（异常网关/代理）同样视为未确认
        assert!(ensure_acknowledged(&json!({}), "创建索引").is_err());
    }

    #[test]
    fn delete_by_query_failures_surface_first_reason() {
        let body = json!({
            "took": 12,
            "deleted": 2,
            "failures": [
                {
                    "index": "orders",
                    "id": "a1",
                    "cause": { "type": "version_conflict_engine_exception" },
                    "reason": { "type": "bulkre", "reason": "version conflict" }
                }
            ]
        });
        assert_eq!(
            delete_by_query_failure(&body).as_deref(),
            Some("version conflict")
        );
        // 无失败时返回 None；失败条目缺 reason 结构时也不 panic
        assert!(delete_by_query_failure(&json!({ "deleted": 3, "failures": [] })).is_none());
        assert_eq!(
            delete_by_query_failure(&json!({ "failures": [{ "index": "orders" }] })).as_deref(),
            Some("未知原因")
        );
    }

    #[test]
    fn cat_indices_parses_lenient_values() {
        let body = json!([
            { "index": "orders", "docs.count": "42", "store.size": "1024", "status": "open" },
            { "index": ".internal", "docs.count": null, "store.size": null, "status": "close" }
        ]);
        let entries = parse_cat_indices(&body);
        assert_eq!(entries[0].name, "orders");
        assert_eq!(entries[0].docs, Some(42));
        assert_eq!(entries[0].bytes, Some(1024));
        assert_eq!(entries[0].status.as_deref(), Some("open"));
        assert_eq!(entries[1].docs, None);
    }

    #[test]
    fn mapping_fields_flatten_nested_objects() {
        let mapping = json!({
            "mappings": {
                "properties": {
                    "title": { "type": "text" },
                    "user": {
                        "properties": {
                            "name": { "type": "keyword" },
                            "address": {
                                "properties": { "city": { "type": "text" } }
                            }
                        }
                    },
                    "meta": { "type": "object", "properties": {} }
                }
            }
        });
        let fields = mapping_fields(&mapping);
        let names: Vec<&str> = fields.iter().map(|(name, _)| name.as_str()).collect();
        // object 父字段不可被 ES SQL 直接查询，只保留叶子字段
        assert_eq!(names, vec!["title", "user.address.city", "user.name"]);
        assert_eq!(fields[2].1, "keyword");
    }

    #[test]
    fn normalize_strips_trailing_semicolons_and_whitespace() {
        assert_eq!(normalize_es_sql("SELECT 1"), "SELECT 1");
        assert_eq!(normalize_es_sql("SELECT 1;"), "SELECT 1");
        assert_eq!(normalize_es_sql("  SELECT 1;;\n\t"), "SELECT 1");
        assert_eq!(normalize_es_sql("SELECT ';'"), "SELECT ';'");
    }

    #[test]
    fn star_projection_detects_select_star_queries() {
        let (table, star) =
            star_projection("SELECT *\nFROM \"card-info\"\nORDER BY title").unwrap();
        assert_eq!(table, "card-info");
        assert_eq!(&"SELECT *\nFROM \"card-info\""[star], "*");
        let (table, _) = star_projection("select * from plain_index LIMIT 10").unwrap();
        assert_eq!(table, "plain_index");
        assert!(star_projection("SELECT title FROM \"idx\"").is_none());
        assert!(star_projection("SELECT COUNT(*) FROM \"idx\"").is_none());
        assert!(star_projection("SELECT *, title FROM \"idx\"").is_none());
        assert!(star_projection("DELETE FROM \"idx\"").is_none());
    }

    #[test]
    fn expand_star_replaces_only_the_star_token() {
        let sql = "SELECT *\nFROM \"idx\" WHERE a = 1 LIMIT 5";
        let (_, star) = star_projection(sql).unwrap();
        assert_eq!(
            expand_star(sql, star.clone(), &["title".into(), "price".into()]).unwrap(),
            "SELECT \"title\", \"price\" FROM \"idx\" WHERE a = 1 LIMIT 5"
        );
        assert!(expand_star(sql, star, &[]).is_none());
    }

    #[test]
    fn array_error_extracts_the_offending_field() {
        let error = query_error(
            "Found 1 problem\nline 1:8: Arrays (returned by [tenantList]) are not supported",
        );
        assert_eq!(
            unsupported_array_field(&error).as_deref(),
            Some("tenantList")
        );
        // ES 把取值阶段的数组错误归为 5xx 时会走 Connection 分类，同样要识别
        let connection_error =
            CockpitError::Connection("Arrays (returned by [tenantList]) are not supported".into());
        assert_eq!(
            unsupported_array_field(&connection_error).as_deref(),
            Some("tenantList")
        );
        assert_eq!(
            unsupported_array_field(&query_error("mismatched input ';'")),
            None
        );
        assert_eq!(unsupported_array_field(&CockpitError::Timeout), None);
    }

    #[test]
    fn cursor_cache_only_matches_same_sql_and_forward_offsets() {
        let state = CursorState {
            sql: "SELECT 1".into(),
            next_offset: 500,
            cursor: "c".into(),
            columns: Vec::new(),
        };
        assert_eq!(state.usable_for("SELECT 1", 1000), Some((500, "c".into())));
        assert_eq!(state.usable_for("SELECT 1", 300), None);
        assert_eq!(state.usable_for("SELECT 2", 1000), None);
    }

    #[test]
    fn values_convert_by_es_type() {
        assert_eq!(
            json_value_to_cell(&json!(1_700_000_000_000i64), "DATETIME"),
            CellValue::DateTime(format_epoch_millis(1_700_000_000_000))
        );
        assert_eq!(
            json_value_to_cell(&json!(42), "INTEGER"),
            CellValue::Signed("42".into())
        );
        assert_eq!(
            json_value_to_cell(&json!(1.5), "SCALED_FLOAT"),
            CellValue::Float(1.5)
        );
        assert_eq!(
            json_value_to_cell(&json!("18446744073709551615"), "UNSIGNED_LONG"),
            CellValue::Unsigned("18446744073709551615".into())
        );
        assert_eq!(
            json_value_to_cell(&json!(true), "BOOLEAN"),
            CellValue::Bool(true)
        );
        assert_eq!(
            json_value_to_cell(&json!({"a": 1}), "OBJECT"),
            CellValue::Json("{\"a\":1}".into())
        );
        assert_eq!(json_value_to_cell(&Value::Null, "KEYWORD"), CellValue::Null);
    }

    #[test]
    fn top_level_properties_keep_containers_and_uppercase_types() {
        let mapping = json!({
            "mappings": {
                "properties": {
                    "title": { "type": "text" },
                    "user": { "properties": { "name": { "type": "keyword" } } },
                    "tags": { "type": "nested", "properties": { "label": { "type": "keyword" } } },
                    "created": { "type": "date" },
                    "meta": { "type": "object" }
                }
            }
        });
        let fields = top_level_properties(&mapping);
        assert_eq!(
            fields,
            vec![
                ("created".into(), "DATE".into()),
                ("meta".into(), "OBJECT".into()),
                ("tags".into(), "NESTED".into()),
                ("title".into(), "TEXT".into()),
                ("user".into(), "OBJECT".into()),
            ]
        );
    }

    #[test]
    fn search_columns_lead_with_id() {
        let mapping = json!({
            "mappings": { "properties": { "title": { "type": "keyword" } } }
        });
        let columns = search_columns_from_mapping(&mapping);
        assert_eq!(columns.len(), 3);
        assert_eq!(columns[0].name, "_id");
        assert_eq!(columns[0].database_type, "KEYWORD");
        assert!(!columns[0].nullable);
        assert_eq!(columns[1].name, "_version");
        assert_eq!(columns[1].database_type, "LONG");
        assert_eq!(columns[2].name, "title");
        assert_eq!(columns[2].database_type, "KEYWORD");
    }

    #[test]
    fn translate_body_whitelists_fields_and_rejects_aggregations() {
        let translated = json!({
            "size": 10,
            "_source": false,
            "fields": [{ "field": "title" }],
            "query": { "match_all": {} },
            "sort": [{ "title": "asc" }],
            "columns": [{ "name": "a", "type": "KEYWORD" }]
        });
        let body = translate_to_search_body(&translated).unwrap();
        assert_eq!(body["query"], json!({ "match_all": {} }));
        assert_eq!(body["sort"], json!([{ "title": "asc" }]));
        // size/_source/fields 等键一律丢弃，避免压制完整 _source 返回
        assert!(body.get("size").is_none());
        assert!(body.get("_source").is_none());
        assert!(body.get("fields").is_none());
        assert!(body.get("columns").is_none());
        // 聚合翻译结果没有逐文档 hit，必须回退 SQL 路径
        assert!(translate_to_search_body(&json!({ "aggs": {} })).is_none());
        assert!(
            translate_to_search_body(&json!({
                "aggregations": { "by_title": {} }
            }))
            .is_none()
        );
    }

    #[test]
    fn sql_limit_parses_trailing_literal_only() {
        assert_eq!(sql_limit("SELECT * FROM \"idx\" LIMIT 2"), Some(2));
        assert_eq!(sql_limit("select * from idx limit 100"), Some(100));
        assert_eq!(sql_limit("SELECT *\nFROM idx\nLIMIT 25 "), Some(25));
        assert_eq!(sql_limit("SELECT * FROM idx"), None);
        // 字符串字面量里的 LIMIT 不是分页上限
        assert_eq!(sql_limit("SELECT * FROM idx WHERE t = 'LIMIT'"), None);
        assert_eq!(
            sql_limit("SELECT * FROM idx WHERE t = 'x' LIMIT 3"),
            Some(3)
        );
    }

    #[test]
    fn search_size_fetches_one_extra_row_until_limit() {
        assert_eq!(search_size(0, 100, None), Some(101));
        assert_eq!(search_size(100, 100, None), Some(101));
        assert_eq!(search_size(0, 100, Some(150)), Some(101));
        assert_eq!(search_size(100, 100, Some(150)), Some(50));
        assert_eq!(search_size(149, 100, Some(150)), Some(1));
        assert_eq!(search_size(150, 100, Some(150)), None);
        assert_eq!(search_size(160, 100, Some(150)), None);
    }

    #[test]
    fn finalize_search_body_adds_paging_and_doc_tiebreaker() {
        let with_sort = finalize_search_body(
            json!({ "query": { "match_all": {} }, "sort": [{ "title": "asc" }] }),
            40,
            101,
        );
        assert_eq!(with_sort["from"], json!(40));
        assert_eq!(with_sort["size"], json!(101));
        assert_eq!(with_sort["sort"], json!([{ "title": "asc" }, "_doc"]));
        let without_sort = finalize_search_body(json!({ "query": { "match_all": {} } }), 0, 101);
        assert_eq!(without_sort["sort"], json!(["_doc"]));
    }

    #[test]
    fn extract_id_predicates_strips_id_and_keeps_other_conditions() {
        let (sql, predicates) = extract_id_predicates(
            "SELECT *\nFROM \"card-info\"\nWHERE _id = 13177 AND status = 1 AND \"_id\" = 'ab'",
        )
        .unwrap();
        assert_eq!(sql, "SELECT *\nFROM \"card-info\"\nWHERE status = 1");
        assert_eq!(
            predicates,
            vec![
                IdPredicate {
                    values: vec!["13177".into()],
                    negated: false
                },
                IdPredicate {
                    values: vec!["ab".into()],
                    negated: false
                },
            ]
        );
    }

    #[test]
    fn extract_id_predicates_drops_where_when_id_is_the_only_condition() {
        let (sql, predicates) =
            extract_id_predicates("SELECT *\nFROM \"card-info\"\nWHERE _id=13177\nORDER BY title")
                .unwrap();
        assert_eq!(sql, "SELECT *\nFROM \"card-info\"\nORDER BY title");
        assert_eq!(
            predicates,
            vec![IdPredicate {
                values: vec!["13177".into()],
                negated: false
            }]
        );
    }

    #[test]
    fn extract_id_predicates_handles_in_not_in_and_escaped_strings() {
        let (_, predicates) =
            extract_id_predicates("SELECT * FROM idx WHERE _id IN (1, 2, 'a''b')").unwrap();
        assert_eq!(
            predicates,
            vec![IdPredicate {
                values: vec!["1".into(), "2".into(), "a'b".into()],
                negated: false
            }]
        );
        let (sql, predicates) =
            extract_id_predicates("SELECT * FROM idx WHERE _id NOT IN (1,2) AND _id <> 9").unwrap();
        assert_eq!(sql, "SELECT * FROM idx");
        assert_eq!(
            predicates,
            vec![
                IdPredicate {
                    values: vec!["1".into(), "2".into()],
                    negated: true
                },
                IdPredicate {
                    values: vec!["9".into()],
                    negated: true
                },
            ]
        );
    }

    #[test]
    fn extract_id_predicates_leaves_sql_without_id_untouched() {
        let (sql, predicates) =
            extract_id_predicates("SELECT * FROM idx WHERE user_id = 1 AND title = 'x'").unwrap();
        assert_eq!(sql, "SELECT * FROM idx WHERE user_id = 1 AND title = 'x'");
        assert!(predicates.is_empty());
    }

    #[test]
    fn extract_id_predicates_bails_on_unrewritable_id_usage() {
        // OR、LIKE、排序等无法等价改写成 ids 过滤，保留原 SQL 走报错回退
        assert!(extract_id_predicates("SELECT * FROM idx WHERE status = 1 OR _id = 2").is_none());
        assert!(extract_id_predicates("SELECT * FROM idx WHERE _id LIKE 'a%'").is_none());
        assert!(extract_id_predicates("SELECT * FROM idx ORDER BY _id").is_none());
        assert!(extract_id_predicates("SELECT * FROM idx WHERE _id > 5").is_none());
    }

    #[test]
    fn apply_id_predicates_merges_into_bool_filter_and_must_not() {
        let mut body = json!({ "query": { "range": { "price": { "gte": 10 } } } });
        apply_id_predicates(
            &mut body,
            &[
                IdPredicate {
                    values: vec!["1".into()],
                    negated: false,
                },
                IdPredicate {
                    values: vec!["9".into()],
                    negated: true,
                },
            ],
        );
        assert_eq!(
            body["query"],
            json!({
                "bool": {
                    "filter": [
                        { "range": { "price": { "gte": 10 } } },
                        { "ids": { "values": ["1"] } }
                    ],
                    "must_not": [{ "ids": { "values": ["9"] } }]
                }
            })
        );

        let mut empty_query = json!({});
        apply_id_predicates(
            &mut empty_query,
            &[IdPredicate {
                values: vec!["7".into()],
                negated: false,
            }],
        );
        assert_eq!(
            empty_query["query"],
            json!({
                "bool": {
                    "filter": [{ "match_all": {} }, { "ids": { "values": ["7"] } }]
                }
            })
        );
    }

    #[test]
    fn search_hit_rows_carry_id_and_nested_json() {
        let mapping = json!({
            "mappings": {
                "properties": {
                    "title": { "type": "keyword" },
                    "user": { "properties": { "name": { "type": "keyword" } } },
                    "created": { "type": "date" }
                }
            }
        });
        let columns = search_columns_from_mapping(&mapping);
        let index_of = |name: &str| {
            columns
                .iter()
                .position(|column| column.name == name)
                .unwrap()
        };
        let hit = SearchHit {
            _id: "doc-1".into(),
            _version: Some(3),
            _source: json!({
                "title": "hello",
                "user": { "name": "ann" },
                "created": 1_700_000_000_000i64
            }),
        };
        let row = search_hit_to_row(&hit, &columns);
        assert_eq!(row[0], CellValue::Text("doc-1".into()));
        assert_eq!(row[index_of("_version")], CellValue::Signed("3".into()));
        assert_eq!(row[index_of("title")], CellValue::Text("hello".into()));
        assert_eq!(
            row[index_of("user")],
            CellValue::Json("{\"name\":\"ann\"}".into())
        );
        assert_eq!(
            row[index_of("created")],
            CellValue::DateTime(format_epoch_millis(1_700_000_000_000))
        );
        // 文档缺失的字段按 NULL 呈现；旧版本 ES 的 hit 不带 _version 时同样置 NULL
        let missing = SearchHit {
            _id: "doc-2".into(),
            _version: None,
            _source: json!({ "title": "world" }),
        };
        let row = search_hit_to_row(&missing, &columns);
        assert_eq!(row[index_of("_version")], CellValue::Null);
        assert_eq!(row[index_of("user")], CellValue::Null);
    }

    #[test]
    fn index_names_are_validated_for_url_safety() {
        assert!(is_valid_index_name("orders"));
        assert!(is_valid_index_name("cockpit_it_abc123"));
        assert!(is_valid_index_name("logs-2024.01"));
        assert!(!is_valid_index_name(""));
        assert!(!is_valid_index_name("-hidden"));
        assert!(!is_valid_index_name("_internal"));
        assert!(!is_valid_index_name("+plus"));
        assert!(!is_valid_index_name("."));
        assert!(!is_valid_index_name(".."));
        // 大写、路径与查询串字符都不允许，防止拼 URL 时被注入
        assert!(!is_valid_index_name("Orders"));
        assert!(!is_valid_index_name("a/b"));
        assert!(!is_valid_index_name("a?refresh"));
        assert!(!is_valid_index_name("a b"));
    }

    #[test]
    fn document_ids_are_percent_encoded() {
        assert_eq!(percent_encode("alpha-1.2_3~4"), "alpha-1.2_3~4");
        assert_eq!(percent_encode("a b/c"), "a%20b%2Fc");
        assert_eq!(percent_encode("中文"), "%E4%B8%AD%E6%96%87");
        assert_eq!(percent_encode(""), "");
    }
}
