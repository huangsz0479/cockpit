use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn default_query_page_size() -> usize {
    500
}

fn default_mysql_port() -> u16 {
    3306
}
fn default_connect_timeout() -> u64 {
    5
}
fn default_query_timeout() -> u64 {
    30
}
fn default_pool_size() -> usize {
    5
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProfile {
    pub id: Uuid,
    #[serde(default)]
    pub driver_kind: DatabaseKind,
    #[serde(default)]
    pub group: Option<String>,
    pub name: String,
    pub host: String,
    #[serde(default = "default_mysql_port")]
    pub port: u16,
    pub username: String,
    pub database: Option<String>,
    #[serde(default)]
    pub tls: TlsOptions,
    #[serde(default)]
    pub ssh: Option<SshOptions>,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_query_timeout")]
    pub query_timeout_secs: u64,
    #[serde(default = "default_pool_size")]
    pub pool_size: usize,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub production: bool,
    #[serde(default)]
    pub color: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseKind {
    #[default]
    MySql,
    MariaDb,
    PostgreSql,
    Sqlite,
    Redis,
}

impl ConnectionProfile {
    pub fn validate(&self) -> crate::Result<()> {
        if self.name.trim().is_empty() {
            return Err(crate::CockpitError::InvalidConfig(
                "连接名称不能为空".into(),
            ));
        }
        if self.host.trim().is_empty() {
            return Err(crate::CockpitError::InvalidConfig("主机不能为空".into()));
        }
        if !matches!(self.driver_kind, DatabaseKind::Sqlite | DatabaseKind::Redis)
            && self.username.trim().is_empty()
        {
            return Err(crate::CockpitError::InvalidConfig("用户名不能为空".into()));
        }
        if self.port == 0 {
            return Err(crate::CockpitError::InvalidConfig("端口必须大于 0".into()));
        }
        if !(1..=32).contains(&self.pool_size) {
            return Err(crate::CockpitError::InvalidConfig(
                "连接池大小必须在 1 到 32 之间".into(),
            ));
        }
        if let Some(ssh) = &self.ssh {
            if ssh.host.trim().is_empty() || ssh.username.trim().is_empty() || ssh.port == 0 {
                return Err(crate::CockpitError::InvalidConfig(
                    "SSH 主机、端口和用户名不能为空".into(),
                ));
            }
            if ssh.auth_method == SshAuthMethod::PrivateKey
                && ssh
                    .private_key_path
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
            {
                return Err(crate::CockpitError::InvalidConfig(
                    "SSH 私钥认证必须选择私钥文件".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TlsOptions {
    #[serde(default)]
    pub mode: TlsMode,
    #[serde(default)]
    pub ca_cert_path: Option<String>,
    #[serde(default)]
    pub client_cert_path: Option<String>,
    #[serde(default)]
    pub client_key_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TlsMode {
    #[default]
    Disabled,
    Preferred,
    Required,
    VerifyCa,
    VerifyIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshOptions {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub auth_method: SshAuthMethod,
    #[serde(default)]
    pub private_key_path: Option<String>,
    #[serde(default)]
    pub use_agent: bool,
    #[serde(default)]
    pub host_fingerprint: Option<String>,
}
fn default_ssh_port() -> u16 {
    22
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SshAuthMethod {
    #[default]
    Password,
    PrivateKey,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub server_version: String,
    pub server_comment: Option<String>,
    pub connection_id: u32,
    pub current_database: Option<String>,
    pub tls_cipher: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseInfo {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub database: String,
    pub name: String,
    pub table_type: String,
    pub comment: Option<String>,
    pub estimated_rows: Option<u64>,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    pub ordinal: u32,
    pub data_type: String,
    pub full_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub extra: Option<String>,
    pub comment: Option<String>,
    pub key: Option<String>,
    pub generation_expression: Option<String>,
    pub collation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
    pub index_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_database: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub on_update: Option<String>,
    pub on_delete: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TableDetail {
    pub table: TableInfo,
    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<IndexInfo>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
    pub ddl: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoutineInfo {
    pub database: String,
    pub name: String,
    pub routine_type: String,
    pub data_type: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerInfo {
    pub database: String,
    pub name: String,
    pub table_name: String,
    pub timing: String,
    pub event: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EventInfo {
    pub database: String,
    pub name: String,
    pub status: String,
    pub event_type: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseObjectKind {
    View,
    Procedure,
    Function,
    Trigger,
    Event,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseObjectDefinition {
    pub database: String,
    pub name: String,
    pub kind: DatabaseObjectKind,
    pub ddl: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoutineParameter {
    pub name: Option<String>,
    pub mode: Option<String>,
    pub data_type: String,
    pub ordinal: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerProcessInfo {
    pub id: u64,
    pub user: String,
    pub host: String,
    pub database: Option<String>,
    pub command: String,
    pub time_secs: u64,
    pub state: Option<String>,
    pub sql: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerMetric {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerVariable {
    pub name: String,
    pub value: String,
    pub dynamic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerLockInfo {
    pub waiting_thread_id: u64,
    pub blocking_thread_id: Option<u64>,
    pub object_name: Option<String>,
    pub lock_type: String,
    pub lock_mode: String,
    pub lock_status: String,
    pub waiting_sql: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserAccount {
    pub user: String,
    pub host: String,
    pub plugin: Option<String>,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteQueryRequest {
    pub execution_id: Uuid,
    pub sql: String,
    pub database: Option<String>,
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub allow_write: bool,
    #[serde(default = "default_query_page_size")]
    pub page_size: usize,
    #[serde(default)]
    pub row_offset: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CellValue {
    Null,
    Bool(bool),
    Signed(String),
    Unsigned(String),
    Decimal(String),
    Float(f64),
    Text(String),
    Bytes {
        base64: String,
        preview: Option<String>,
        length: usize,
    },
    Date(String),
    Time(String),
    DateTime(String),
    Json(String),
    Geometry {
        wkb_base64: String,
        srid: Option<u32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMeta {
    pub name: String,
    pub database_type: String,
    pub nullable: bool,
    pub unsigned: bool,
    pub binary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueryMessage {
    pub severity: String,
    pub code: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueryResultPage {
    pub execution_id: Uuid,
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<CellValue>>,
    pub affected_rows: u64,
    pub execution_time_ms: u128,
    pub truncated: bool,
    pub has_more: bool,
    pub result_set_index: usize,
    pub messages: Vec<QueryMessage>,
    #[serde(default)]
    pub row_offset: usize,
    #[serde(default = "default_query_page_size")]
    pub page_size: usize,
    #[serde(default)]
    pub additional_result_sets: Vec<QueryResultSet>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueryResultSet {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<CellValue>>,
    pub affected_rows: u64,
    pub truncated: bool,
    pub has_more: bool,
    pub result_set_index: usize,
    pub row_offset: usize,
    pub page_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Safe,
    Review,
    Destructive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SqlAssessment {
    pub statement_kind: String,
    pub risk: RiskLevel,
    pub requires_confirmation: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SchemaChangePlan {
    pub statements: Vec<String>,
    pub risk: RiskLevel,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RowMutationPlan {
    pub sql: String,
    pub parameter_count: usize,
    pub risk: RiskLevel,
    pub requires_unique_key: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RowMutationKind {
    Insert,
    Update,
    Delete,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImportConflictPolicy {
    #[default]
    Error,
    Ignore,
    Replace,
    Upsert,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RowMutationRequest {
    pub database: String,
    pub table: String,
    pub kind: RowMutationKind,
    #[serde(default)]
    pub values: Vec<(String, CellValue)>,
    #[serde(default)]
    pub key_values: Vec<(String, CellValue)>,
    #[serde(default)]
    pub original_values: Vec<(String, CellValue)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RowMutationResult {
    pub affected_rows: u64,
    pub concurrent_change: bool,
}
