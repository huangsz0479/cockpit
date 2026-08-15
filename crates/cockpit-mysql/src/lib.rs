use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use cockpit_core::{
    CellValue, CockpitError, ColumnInfo, ColumnMeta, ConnectionInfo, ConnectionProfile,
    DatabaseDriver, DatabaseInfo, DatabaseObjectDefinition, DatabaseObjectKind, DriverSession,
    EventInfo, ExecuteQueryRequest, ForeignKeyInfo, ImportConflictPolicy, IndexInfo, QueryMessage,
    QueryResultPage, QueryResultSet, Result, RiskLevel, RoutineInfo, RoutineParameter,
    RowMutationKind, RowMutationRequest, RowMutationResult, ServerLockInfo, ServerMetric,
    ServerProcessInfo, ServerVariable, SshAuthMethod, TableDetail, TableInfo, TlsMode, TriggerInfo,
    UserAccount, safety::assess_sql,
};
use futures::StreamExt;
use mysql_async::{
    Conn, Opts, OptsBuilder, Params, Pool, PoolConstraints, PoolOpts, Row, SslOpts, Value,
    consts::{ColumnFlags, ColumnType},
    prelude::Queryable,
};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, RwLock},
};
use url::Url;
use uuid::Uuid;

#[derive(Default)]
pub struct MySqlDriver;

pub struct MySqlSession {
    profile: ConnectionProfile,
    pool: Pool,
    control_opts: Opts,
    running: RwLock<HashMap<Uuid, u32>>,
    killed_executions: RwLock<HashSet<Uuid>>,
    transaction: Mutex<Option<Conn>>,
    read_transaction_time_zone: Mutex<Option<String>>,
    ssh_tunnel: Option<Arc<SshTunnel>>,
}

struct SshTunnel {
    child: Mutex<Option<tokio::process::Child>>,
    known_hosts_path: PathBuf,
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.known_hosts_path);
    }
}

#[async_trait]
impl DatabaseDriver for MySqlDriver {
    fn kind(&self) -> &'static str {
        "mysql"
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
    ) -> Result<Arc<dyn DriverSession>> {
        Ok(open_session(profile, password).await?)
    }
}

async fn open_session(profile: ConnectionProfile, password: String) -> Result<Arc<MySqlSession>> {
    profile.validate()?;
    let (connect_profile, ssh_tunnel) = if profile.ssh.is_some() {
        let (local_port, tunnel) = start_ssh_tunnel(&profile).await?;
        let mut connect_profile = profile.clone();
        connect_profile.host = "127.0.0.1".into();
        connect_profile.port = local_port;
        connect_profile.ssh = None;
        (connect_profile, Some(tunnel))
    } else {
        (profile.clone(), None)
    };
    let tls_hostname = profile
        .ssh
        .as_ref()
        .map(|_| profile.host.trim().to_string());
    let (pool, opts) = create_pool(
        &connect_profile,
        &password,
        profile.tls.mode,
        tls_hostname.as_deref(),
    )?;
    let session = Arc::new(MySqlSession {
        profile: profile.clone(),
        pool,
        control_opts: opts,
        running: RwLock::new(HashMap::new()),
        killed_executions: RwLock::new(HashSet::new()),
        transaction: Mutex::new(None),
        read_transaction_time_zone: Mutex::new(None),
        ssh_tunnel: ssh_tunnel.clone(),
    });
    let validation = tokio::time::timeout(
        Duration::from_secs(profile.connect_timeout_secs.max(1)),
        session.connection_info(),
    )
    .await;
    match validation {
        Ok(Ok(_)) => Ok(session),
        Ok(Err(_)) if profile.tls.mode == TlsMode::Preferred => {
            let _ = session.pool.clone().disconnect().await;
            let (pool, opts) = create_pool(&connect_profile, &password, TlsMode::Disabled, None)?;
            let fallback = Arc::new(MySqlSession {
                profile,
                pool,
                control_opts: opts,
                running: RwLock::new(HashMap::new()),
                killed_executions: RwLock::new(HashSet::new()),
                transaction: Mutex::new(None),
                read_transaction_time_zone: Mutex::new(None),
                ssh_tunnel,
            });
            fallback.connection_info().await?;
            Ok(fallback)
        }
        Ok(Err(error)) => Err(error),
        Err(_) => Err(CockpitError::Connection(format!(
            "连接超时（{} 秒）",
            profile.connect_timeout_secs
        ))),
    }
}

async fn start_ssh_tunnel(profile: &ConnectionProfile) -> Result<(u16, Arc<SshTunnel>)> {
    let ssh = profile
        .ssh
        .as_ref()
        .ok_or_else(|| CockpitError::InvalidConfig("SSH 配置不存在".into()))?;
    if ssh.auth_method == SshAuthMethod::Password {
        return Err(CockpitError::Unsupported(
            "SSH 密码认证需要交互式终端；请使用 SSH Agent 或私钥".into(),
        ));
    }
    let keyscan = tokio::time::timeout(
        Duration::from_secs(profile.connect_timeout_secs.max(5)),
        tokio::process::Command::new("ssh-keyscan")
            .args(["-p", &ssh.port.to_string(), "-T", "5", ssh.host.trim()])
            .output(),
    )
    .await
    .map_err(|_| CockpitError::Connection("读取 SSH 主机公钥超时".into()))?
    .map_err(|error| CockpitError::Connection(format!("无法运行 ssh-keyscan：{error}")))?;
    if !keyscan.status.success() || keyscan.stdout.is_empty() {
        return Err(CockpitError::Connection(
            "无法读取 SSH 主机公钥，请检查地址和网络".into(),
        ));
    }
    let mut keygen = tokio::process::Command::new("ssh-keygen")
        .args(["-lf", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| CockpitError::Connection(format!("无法运行 ssh-keygen：{error}")))?;
    keygen
        .stdin
        .take()
        .ok_or_else(|| CockpitError::Connection("无法校验 SSH 主机公钥".into()))?
        .write_all(&keyscan.stdout)
        .await
        .map_err(|error| CockpitError::Connection(error.to_string()))?;
    let fingerprint_output = keygen
        .wait_with_output()
        .await
        .map_err(|error| CockpitError::Connection(error.to_string()))?;
    let mut fingerprints = String::from_utf8_lossy(&fingerprint_output.stdout)
        .lines()
        .filter_map(|line| {
            line.split_whitespace()
                .find(|part| part.starts_with("SHA256:"))
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    fingerprints.sort();
    fingerprints.dedup();
    let fingerprints = fingerprints.join(", ");
    if fingerprints.is_empty() {
        return Err(CockpitError::Connection("无法计算 SSH 主机指纹".into()));
    }
    match ssh
        .host_fingerprint
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        None => {
            return Err(CockpitError::InvalidConfig(format!(
                "SSH_HOST_KEY_CONFIRM_REQUIRED|{fingerprints}"
            )));
        }
        Some(expected) if expected != fingerprints => {
            return Err(CockpitError::InvalidConfig(format!(
                "SSH_HOST_KEY_CHANGED|{fingerprints}"
            )));
        }
        Some(_) => {}
    }

    let local_port = std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map_err(|error| CockpitError::Connection(format!("无法分配 SSH 本地端口：{error}")))?
        .port();
    let known_hosts_path =
        std::env::temp_dir().join(format!("cockpit-known-hosts-{}", Uuid::new_v4()));
    tokio::fs::write(&known_hosts_path, &keyscan.stdout)
        .await
        .map_err(|error| CockpitError::Connection(error.to_string()))?;
    let mut command = tokio::process::Command::new("ssh");
    command
        .args([
            "-N",
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "ExitOnForwardFailure=yes",
        ])
        .arg("-o")
        .arg("StrictHostKeyChecking=yes")
        .arg("-o")
        .arg(format!("UserKnownHostsFile={}", known_hosts_path.display()))
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-p")
        .arg(ssh.port.to_string())
        .arg("-L")
        .arg(format!(
            "127.0.0.1:{local_port}:{}:{}",
            profile.host.trim(),
            profile.port
        ));
    if ssh.auth_method == SshAuthMethod::PrivateKey {
        command
            .arg("-i")
            .arg(ssh.private_key_path.as_deref().unwrap_or_default());
    }
    command
        .arg(format!("{}@{}", ssh.username.trim(), ssh.host.trim()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| CockpitError::Connection(format!("无法启动系统 SSH：{error}")))?;
    let ready = async {
        for _ in 0..50 {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| CockpitError::Connection(error.to_string()))?
            {
                return Err(CockpitError::Connection(format!(
                    "SSH 隧道启动失败（{status}）"
                )));
            }
            if tokio::net::TcpStream::connect(("127.0.0.1", local_port))
                .await
                .is_ok()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(CockpitError::Connection("SSH 隧道启动超时".into()))
    }
    .await;
    if let Err(error) = ready {
        let _ = child.start_kill();
        let _ = std::fs::remove_file(&known_hosts_path);
        return Err(error);
    }
    Ok((
        local_port,
        Arc::new(SshTunnel {
            child: Mutex::new(Some(child)),
            known_hosts_path,
        }),
    ))
}

impl SshTunnel {
    async fn close(&self) {
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        let _ = std::fs::remove_file(&self.known_hosts_path);
    }
}

fn create_pool(
    profile: &ConnectionProfile,
    password: &str,
    tls_mode: TlsMode,
    tls_hostname: Option<&str>,
) -> Result<(Pool, Opts)> {
    let mut url =
        Url::parse("mysql://localhost").map_err(|e| CockpitError::InvalidConfig(e.to_string()))?;
    url.set_host(Some(profile.host.trim()))
        .map_err(|_| CockpitError::InvalidConfig("MySQL 主机格式无效".into()))?;
    url.set_port(Some(profile.port))
        .map_err(|_| CockpitError::InvalidConfig("MySQL 端口无效".into()))?;
    url.set_username(profile.username.trim())
        .map_err(|_| CockpitError::InvalidConfig("MySQL 用户名无效".into()))?;
    url.set_password(Some(password))
        .map_err(|_| CockpitError::InvalidConfig("MySQL 密码无法编码".into()))?;
    if let Some(database) = profile.database.as_deref().filter(|v| !v.trim().is_empty()) {
        url.set_path(&format!("/{}", database.trim()));
    }
    let base =
        Opts::from_url(url.as_str()).map_err(|e| CockpitError::InvalidConfig(e.to_string()))?;
    let pool_opts = PoolOpts::new()
        .with_constraints(
            PoolConstraints::new(1, profile.pool_size.clamp(1, 32)).expect("validated pool size"),
        )
        .with_inactive_connection_ttl(Duration::from_secs(300));
    let mut builder = OptsBuilder::from_opts(base)
        .pool_opts(Some(pool_opts))
        .stmt_cache_size(0)
        .client_found_rows(true)
        .prefer_socket(false);

    if tls_mode != TlsMode::Disabled {
        let mut ssl = SslOpts::default();
        match tls_mode {
            TlsMode::Disabled => {}
            TlsMode::Preferred | TlsMode::Required => {
                ssl = ssl
                    .with_danger_accept_invalid_certs(true)
                    .with_danger_skip_domain_validation(true);
            }
            TlsMode::VerifyCa => {
                ssl = ssl.with_danger_skip_domain_validation(true);
            }
            TlsMode::VerifyIdentity => {}
        }
        if let Some(path) = profile
            .tls
            .ca_cert_path
            .as_deref()
            .filter(|v| !v.trim().is_empty())
        {
            ssl = ssl.with_root_certs(vec![PathBuf::from(path).into()]);
        } else if matches!(tls_mode, TlsMode::VerifyCa | TlsMode::VerifyIdentity) {
            return Err(CockpitError::InvalidConfig(
                "校验证书时必须选择 CA 证书".into(),
            ));
        }
        if tls_mode == TlsMode::VerifyIdentity
            && let Some(hostname) = tls_hostname.filter(|value| !value.is_empty())
        {
            ssl = ssl.with_danger_tls_hostname_override(Some(hostname.to_string()));
        }
        match (
            profile
                .tls
                .client_cert_path
                .as_deref()
                .filter(|v| !v.trim().is_empty()),
            profile
                .tls
                .client_key_path
                .as_deref()
                .filter(|v| !v.trim().is_empty()),
        ) {
            (Some(cert), Some(key)) => {
                ssl = ssl.with_client_identity(Some(mysql_async::ClientIdentity::new(
                    PathBuf::from(cert).into(),
                    PathBuf::from(key).into(),
                )));
            }
            (Some(_), None) | (None, Some(_)) => {
                return Err(CockpitError::InvalidConfig(
                    "客户端证书和私钥必须同时配置".into(),
                ));
            }
            (None, None) => {}
        }
        builder = builder.ssl_opts(ssl);
    }

    let opts = Opts::from(builder.clone());
    Ok((Pool::new(builder), opts))
}

#[async_trait]
impl DriverSession for MySqlSession {
    fn connection_id(&self) -> Uuid {
        self.profile.id
    }

    async fn connection_info(&self) -> Result<ConnectionInfo> {
        let mut conn = self.connection().await?;
        let row: Option<Row> = conn
            .query_first("SELECT VERSION() AS version, @@version_comment AS version_comment, CONNECTION_ID() AS connection_id, DATABASE() AS current_database")
            .await
            .map_err(connection_error)?;
        let row = row.ok_or_else(|| CockpitError::Connection("服务器没有返回版本信息".into()))?;
        let tls_cipher = conn
            .query_first::<Row, _>("SHOW STATUS LIKE 'Ssl_cipher'")
            .await
            .ok()
            .flatten()
            .and_then(|row| row_string(&row, 1))
            .filter(|v| !v.is_empty());
        Ok(ConnectionInfo {
            server_version: row_string(&row, "version").unwrap_or_default(),
            server_comment: row_string(&row, "version_comment").filter(|v| !v.is_empty()),
            connection_id: row_u64(&row, "connection_id").unwrap_or_default() as u32,
            current_database: row_string(&row, "current_database").filter(|v| !v.is_empty()),
            tls_cipher,
        })
    }

    async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        let mut conn = self.connection().await?;
        let rows: Vec<Row> = match conn.query("SHOW DATABASES").await {
            Ok(rows) => rows,
            Err(_) => conn
                .query("SELECT SCHEMA_NAME FROM information_schema.SCHEMATA ORDER BY SCHEMA_NAME")
                .await
                .map_err(query_error)?,
        };
        let mut databases = rows
            .into_iter()
            .filter_map(|row| row_string(&row, 0))
            .filter(|v| !v.is_empty())
            .map(|name| DatabaseInfo { name })
            .collect::<Vec<_>>();
        databases.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(databases)
    }

    async fn list_tables(
        &self,
        database: &str,
        filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TableInfo>> {
        let limit = limit.clamp(1, 2_000);
        let mut conn = self.metadata_connection().await?;
        let pattern = format!("%{}%", filter.unwrap_or("").trim());
        let sql = "SELECT TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE, NULLIF(TABLE_COMMENT, '') AS TABLE_COMMENT, TABLE_ROWS, COALESCE(DATA_LENGTH, 0) + COALESCE(INDEX_LENGTH, 0) AS TOTAL_BYTES FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_NAME LIKE ? ORDER BY TABLE_NAME LIMIT ? OFFSET ?";
        let (rows, paging_applied): (Vec<Row>, bool) = match conn
            .exec(sql, (database, pattern, limit as u64, offset as u64))
            .await
        {
            Ok(rows) => (rows, true),
            Err(_) => {
                let quoted = quote_identifier(database);
                let fallback: Vec<Row> = conn
                    .query(format!("SHOW FULL TABLES FROM {quoted}"))
                    .await
                    .map_err(query_error)?;
                (fallback, false)
            }
        };
        let fallback_offset = if paging_applied { 0 } else { offset };
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let name = row_string(&row, "TABLE_NAME").or_else(|| row_string(&row, 0))?;
                if !filter.unwrap_or("").is_empty()
                    && !name
                        .to_ascii_lowercase()
                        .contains(&filter.unwrap_or("").to_ascii_lowercase())
                {
                    return None;
                }
                Some(TableInfo {
                    database: row_string(&row, "TABLE_SCHEMA")
                        .unwrap_or_else(|| database.to_string()),
                    name,
                    table_type: row_string(&row, "TABLE_TYPE")
                        .or_else(|| row_string(&row, 1))
                        .unwrap_or_else(|| "BASE TABLE".into()),
                    comment: row_string(&row, "TABLE_COMMENT").filter(|v| !v.is_empty()),
                    estimated_rows: row_u64(&row, "TABLE_ROWS"),
                    total_bytes: row_u64(&row, "TOTAL_BYTES"),
                })
            })
            .skip(fallback_offset)
            .take(limit)
            .collect())
    }

    async fn list_columns(&self, database: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let mut conn = self.metadata_connection().await?;
        let sql = "SELECT COLUMN_NAME, ORDINAL_POSITION, DATA_TYPE, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, EXTRA, COLUMN_COMMENT, COLUMN_KEY, GENERATION_EXPRESSION, COLLATION_NAME FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION";
        let rows: Vec<Row> = match conn.exec(sql, (database, table)).await {
            Ok(rows) => rows,
            Err(_) => conn
                .query(format!(
                    "SHOW FULL COLUMNS FROM {}.{}",
                    quote_identifier(database),
                    quote_identifier(table)
                ))
                .await
                .map_err(query_error)?,
        };
        Ok(rows
            .into_iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let name = row_string(&row, "COLUMN_NAME").or_else(|| row_string(&row, "Field"))?;
                Some(ColumnInfo {
                    name,
                    ordinal: row_u64(&row, "ORDINAL_POSITION").unwrap_or(index as u64 + 1) as u32,
                    data_type: row_string(&row, "DATA_TYPE")
                        .or_else(|| row_string(&row, "Type"))
                        .unwrap_or_default(),
                    full_type: row_string(&row, "COLUMN_TYPE")
                        .or_else(|| row_string(&row, "Type"))
                        .unwrap_or_default(),
                    nullable: row_string(&row, "IS_NULLABLE")
                        .or_else(|| row_string(&row, "Null"))
                        .is_some_and(|v| v.eq_ignore_ascii_case("YES")),
                    default_value: row_string(&row, "COLUMN_DEFAULT")
                        .or_else(|| row_string(&row, "Default")),
                    extra: row_string(&row, "EXTRA")
                        .or_else(|| row_string(&row, "Extra"))
                        .filter(|v| !v.is_empty()),
                    comment: row_string(&row, "COLUMN_COMMENT")
                        .or_else(|| row_string(&row, "Comment"))
                        .filter(|v| !v.is_empty()),
                    key: row_string(&row, "COLUMN_KEY")
                        .or_else(|| row_string(&row, "Key"))
                        .filter(|v| !v.is_empty()),
                    generation_expression: row_string(&row, "GENERATION_EXPRESSION")
                        .filter(|v| !v.is_empty()),
                    collation: row_string(&row, "COLLATION_NAME")
                        .or_else(|| row_string(&row, "Collation"))
                        .filter(|v| !v.is_empty()),
                })
            })
            .collect())
    }

    async fn table_detail(&self, database: &str, table: &str) -> Result<TableDetail> {
        let columns = self.list_columns(database, table).await?;
        let mut conn = self.metadata_connection().await?;
        let index_rows: Vec<Row> = conn.exec(
            "SELECT INDEX_NAME, COLUMN_NAME, SEQ_IN_INDEX, NON_UNIQUE, INDEX_TYPE FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY INDEX_NAME, SEQ_IN_INDEX",
            (database, table),
        ).await.map_err(query_error)?;
        let mut index_map: HashMap<String, IndexInfo> = HashMap::new();
        for row in index_rows {
            let name = row_string(&row, "INDEX_NAME").unwrap_or_default();
            let entry = index_map.entry(name.clone()).or_insert_with(|| IndexInfo {
                name: name.clone(),
                columns: Vec::new(),
                unique: row_u64(&row, "NON_UNIQUE").unwrap_or(1) == 0,
                primary: name == "PRIMARY",
                index_type: row_string(&row, "INDEX_TYPE"),
            });
            if let Some(column) = row_string(&row, "COLUMN_NAME") {
                entry.columns.push(column);
            }
        }
        let fk_rows: Vec<Row> = conn.exec(
            "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME, r.UPDATE_RULE, r.DELETE_RULE FROM information_schema.KEY_COLUMN_USAGE k LEFT JOIN information_schema.REFERENTIAL_CONSTRAINTS r ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME WHERE k.TABLE_SCHEMA = ? AND k.TABLE_NAME = ? AND k.REFERENCED_TABLE_NAME IS NOT NULL ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION",
            (database, table),
        ).await.map_err(query_error)?;
        let mut fk_map: HashMap<String, ForeignKeyInfo> = HashMap::new();
        for row in fk_rows {
            let name = row_string(&row, "CONSTRAINT_NAME").unwrap_or_default();
            let entry = fk_map
                .entry(name.clone())
                .or_insert_with(|| ForeignKeyInfo {
                    name,
                    columns: Vec::new(),
                    referenced_database: row_string(&row, "REFERENCED_TABLE_SCHEMA")
                        .unwrap_or_default(),
                    referenced_table: row_string(&row, "REFERENCED_TABLE_NAME").unwrap_or_default(),
                    referenced_columns: Vec::new(),
                    on_update: row_string(&row, "UPDATE_RULE"),
                    on_delete: row_string(&row, "DELETE_RULE"),
                });
            if let Some(column) = row_string(&row, "COLUMN_NAME") {
                entry.columns.push(column);
            }
            if let Some(column) = row_string(&row, "REFERENCED_COLUMN_NAME") {
                entry.referenced_columns.push(column);
            }
        }
        let create: Option<Row> = conn
            .query_first(format!(
                "SHOW CREATE TABLE {}.{}",
                quote_identifier(database),
                quote_identifier(table)
            ))
            .await
            .map_err(query_error)?;
        let ddl = create
            .as_ref()
            .and_then(|row| row_string(row, 1))
            .unwrap_or_default();
        let tables = self.list_tables(database, Some(table), 10, 0).await?;
        let table_info = tables
            .into_iter()
            .find(|item| item.name == table)
            .unwrap_or(TableInfo {
                database: database.into(),
                name: table.into(),
                table_type: "BASE TABLE".into(),
                comment: None,
                estimated_rows: None,
                total_bytes: None,
            });
        let mut indexes = index_map.into_values().collect::<Vec<_>>();
        indexes.sort_by(|left, right| {
            right
                .primary
                .cmp(&left.primary)
                .then_with(|| left.name.cmp(&right.name))
        });
        let mut foreign_keys = fk_map.into_values().collect::<Vec<_>>();
        foreign_keys.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(TableDetail {
            table: table_info,
            columns,
            indexes,
            foreign_keys,
            ddl,
        })
    }

    async fn list_routines(&self, database: &str) -> Result<Vec<RoutineInfo>> {
        let mut conn = self.metadata_connection().await?;
        let rows: Vec<Row> = conn
            .exec(
                "SELECT ROUTINE_SCHEMA, ROUTINE_NAME, ROUTINE_TYPE, DATA_TYPE, NULLIF(ROUTINE_COMMENT, '') AS ROUTINE_COMMENT FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = ? ORDER BY ROUTINE_TYPE, ROUTINE_NAME",
                (database,),
            )
            .await
            .map_err(query_error)?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                Some(RoutineInfo {
                    database: row_string(&row, "ROUTINE_SCHEMA")?,
                    name: row_string(&row, "ROUTINE_NAME")?,
                    routine_type: row_string(&row, "ROUTINE_TYPE").unwrap_or_default(),
                    data_type: row_string(&row, "DATA_TYPE").filter(|value| !value.is_empty()),
                    comment: row_string(&row, "ROUTINE_COMMENT").filter(|value| !value.is_empty()),
                })
            })
            .collect())
    }

    async fn list_triggers(&self, database: &str) -> Result<Vec<TriggerInfo>> {
        let mut conn = self.metadata_connection().await?;
        let rows: Vec<Row> = conn
            .exec(
                "SELECT TRIGGER_SCHEMA, TRIGGER_NAME, EVENT_OBJECT_TABLE, ACTION_TIMING, EVENT_MANIPULATION FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA = ? ORDER BY TRIGGER_NAME",
                (database,),
            )
            .await
            .map_err(query_error)?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                Some(TriggerInfo {
                    database: row_string(&row, "TRIGGER_SCHEMA")?,
                    name: row_string(&row, "TRIGGER_NAME")?,
                    table_name: row_string(&row, "EVENT_OBJECT_TABLE").unwrap_or_default(),
                    timing: row_string(&row, "ACTION_TIMING").unwrap_or_default(),
                    event: row_string(&row, "EVENT_MANIPULATION").unwrap_or_default(),
                })
            })
            .collect())
    }

    async fn list_events(&self, database: &str) -> Result<Vec<EventInfo>> {
        let mut conn = self.metadata_connection().await?;
        let rows: Vec<Row> = conn
            .exec(
                "SELECT EVENT_SCHEMA, EVENT_NAME, STATUS, EVENT_TYPE FROM information_schema.EVENTS WHERE EVENT_SCHEMA = ? ORDER BY EVENT_NAME",
                (database,),
            )
            .await
            .map_err(query_error)?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                Some(EventInfo {
                    database: row_string(&row, "EVENT_SCHEMA")?,
                    name: row_string(&row, "EVENT_NAME")?,
                    status: row_string(&row, "STATUS").unwrap_or_default(),
                    event_type: row_string(&row, "EVENT_TYPE").unwrap_or_default(),
                })
            })
            .collect())
    }

    async fn object_definition(
        &self,
        database: &str,
        kind: DatabaseObjectKind,
        name: &str,
    ) -> Result<DatabaseObjectDefinition> {
        let object_type = match kind {
            DatabaseObjectKind::View => "VIEW",
            DatabaseObjectKind::Procedure => "PROCEDURE",
            DatabaseObjectKind::Function => "FUNCTION",
            DatabaseObjectKind::Trigger => "TRIGGER",
            DatabaseObjectKind::Event => "EVENT",
        };
        let ddl_index = match kind {
            DatabaseObjectKind::View => 1,
            DatabaseObjectKind::Procedure
            | DatabaseObjectKind::Function
            | DatabaseObjectKind::Trigger => 2,
            DatabaseObjectKind::Event => 3,
        };
        let sql = format!(
            "SHOW CREATE {object_type} {}.{}",
            quote_identifier(database),
            quote_identifier(name)
        );
        let mut conn = self.metadata_connection().await?;
        let row: Option<Row> = conn.query_first(sql).await.map_err(query_error)?;
        let ddl = row
            .as_ref()
            .and_then(|row| row_string(row, ddl_index))
            .ok_or_else(|| {
                CockpitError::NotFound(format!("未找到 {object_type} {database}.{name}"))
            })?;
        Ok(DatabaseObjectDefinition {
            database: database.into(),
            name: name.into(),
            kind,
            ddl,
        })
    }

    async fn routine_parameters(
        &self,
        database: &str,
        name: &str,
    ) -> Result<Vec<RoutineParameter>> {
        let mut conn = self.connection().await?;
        let rows: Vec<Row> = conn
            .exec(
                "SELECT PARAMETER_NAME, PARAMETER_MODE, DTD_IDENTIFIER, ORDINAL_POSITION FROM information_schema.PARAMETERS WHERE SPECIFIC_SCHEMA = ? AND SPECIFIC_NAME = ? AND ORDINAL_POSITION > 0 ORDER BY ORDINAL_POSITION",
                (database, name),
            )
            .await
            .map_err(query_error)?;
        Ok(rows
            .into_iter()
            .map(|row| RoutineParameter {
                name: row_string(&row, "PARAMETER_NAME"),
                mode: row_string(&row, "PARAMETER_MODE"),
                data_type: row_string(&row, "DTD_IDENTIFIER").unwrap_or_else(|| "unknown".into()),
                ordinal: row_u64(&row, "ORDINAL_POSITION").unwrap_or_default() as u32,
            })
            .collect())
    }

    async fn list_processes(&self) -> Result<Vec<ServerProcessInfo>> {
        let mut conn = self.connection().await?;
        let rows: Vec<Row> = conn
            .query("SHOW FULL PROCESSLIST")
            .await
            .map_err(query_error)?;
        Ok(rows
            .into_iter()
            .map(|row| ServerProcessInfo {
                id: row_u64(&row, "Id").unwrap_or_default(),
                user: row_string(&row, "User").unwrap_or_default(),
                host: row_string(&row, "Host").unwrap_or_default(),
                database: row_string(&row, "db"),
                command: row_string(&row, "Command").unwrap_or_default(),
                time_secs: row_u64(&row, "Time").unwrap_or_default(),
                state: row_string(&row, "State"),
                sql: row_string(&row, "Info"),
            })
            .collect())
    }

    async fn kill_process(&self, process_id: u64) -> Result<()> {
        let mut conn = self.connection().await?;
        conn.query_drop(format!("KILL {process_id}"))
            .await
            .map_err(query_error)
    }

    async fn server_status(&self) -> Result<Vec<ServerMetric>> {
        let mut conn = self.connection().await?;
        let rows: Vec<Row> = conn
            .query("SHOW GLOBAL STATUS")
            .await
            .map_err(query_error)?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                Some(ServerMetric {
                    name: row_string(&row, 0)?,
                    value: row_string(&row, 1).unwrap_or_default(),
                })
            })
            .collect())
    }

    async fn server_variables(&self, filter: Option<&str>) -> Result<Vec<ServerVariable>> {
        let mut conn = self.connection().await?;
        let rows: Vec<Row> = conn
            .query("SHOW GLOBAL VARIABLES")
            .await
            .map_err(query_error)?;
        let filter = filter.unwrap_or_default().to_ascii_lowercase();
        const READ_ONLY: &[&str] = &[
            "basedir",
            "datadir",
            "hostname",
            "license",
            "lower_case_table_names",
            "performance_schema",
            "port",
            "server_uuid",
            "socket",
            "version",
            "version_comment",
            "version_compile_machine",
            "version_compile_os",
        ];
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let name = row_string(&row, 0)?;
                if !filter.is_empty() && !name.to_ascii_lowercase().contains(&filter) {
                    return None;
                }
                Some(ServerVariable {
                    dynamic: !READ_ONLY.contains(&name.as_str()),
                    name,
                    value: row_string(&row, 1).unwrap_or_default(),
                })
            })
            .collect())
    }

    async fn server_locks(&self) -> Result<Vec<ServerLockInfo>> {
        let mut conn = self.connection().await?;
        let mysql8: std::result::Result<Vec<Row>, mysql_async::Error> = conn
            .query(
                "SELECT w.REQUESTING_THREAD_ID waiting_thread_id, w.BLOCKING_THREAD_ID blocking_thread_id, CONCAT(l.OBJECT_SCHEMA, '.', l.OBJECT_NAME) object_name, l.LOCK_TYPE lock_type, l.LOCK_MODE lock_mode, l.LOCK_STATUS lock_status, p.INFO waiting_sql FROM performance_schema.data_lock_waits w JOIN performance_schema.data_locks l ON l.ENGINE_LOCK_ID = w.REQUESTING_ENGINE_LOCK_ID LEFT JOIN information_schema.PROCESSLIST p ON p.ID = w.REQUESTING_THREAD_ID",
            )
            .await;
        let rows = match mysql8 {
            Ok(rows) => rows,
            Err(_) => conn
                .query(
                    "SELECT rt.trx_mysql_thread_id waiting_thread_id, bt.trx_mysql_thread_id blocking_thread_id, rl.lock_table object_name, rl.lock_type lock_type, rl.lock_mode lock_mode, 'WAITING' lock_status, rt.trx_query waiting_sql FROM information_schema.INNODB_LOCK_WAITS w JOIN information_schema.INNODB_TRX rt ON rt.trx_id = w.requesting_trx_id JOIN information_schema.INNODB_TRX bt ON bt.trx_id = w.blocking_trx_id JOIN information_schema.INNODB_LOCKS rl ON rl.lock_id = w.requested_lock_id",
                )
                .await
                .map_err(query_error)?,
        };
        Ok(rows
            .into_iter()
            .map(|row| ServerLockInfo {
                waiting_thread_id: row_u64(&row, "waiting_thread_id").unwrap_or_default(),
                blocking_thread_id: row_u64(&row, "blocking_thread_id"),
                object_name: row_string(&row, "object_name"),
                lock_type: row_string(&row, "lock_type").unwrap_or_default(),
                lock_mode: row_string(&row, "lock_mode").unwrap_or_default(),
                lock_status: row_string(&row, "lock_status").unwrap_or_else(|| "WAITING".into()),
                waiting_sql: row_string(&row, "waiting_sql"),
            })
            .collect())
    }

    async fn list_users(&self) -> Result<Vec<UserAccount>> {
        let mut conn = self.connection().await?;
        let rows: Vec<Row> = conn
            .query("SELECT User, Host, plugin, account_locked FROM mysql.user ORDER BY User, Host")
            .await
            .map_err(query_error)?;
        Ok(rows
            .into_iter()
            .map(|row| UserAccount {
                user: row_string(&row, "User").unwrap_or_default(),
                host: row_string(&row, "Host").unwrap_or_default(),
                plugin: row_string(&row, "plugin"),
                locked: row_string(&row, "account_locked").is_some_and(|value| value == "Y"),
            })
            .collect())
    }

    async fn user_grants(&self, user: &str, host: &str) -> Result<Vec<String>> {
        let mut conn = self.connection().await?;
        let sql = format!(
            "SHOW GRANTS FOR {}@{}",
            quote_string(user),
            quote_string(host)
        );
        let rows: Vec<Row> = conn.query(sql).await.map_err(query_error)?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row_string(&row, 0))
            .collect())
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
        let timeout_secs = request
            .timeout_secs
            .unwrap_or(self.profile.query_timeout_secs)
            .max(1);
        let execution_id = request.execution_id;
        let mut future = Box::pin(self.execute_inner(request));
        match tokio::time::timeout(Duration::from_secs(timeout_secs), &mut future).await {
            Ok(result) => result,
            Err(_) => {
                let cleanup_timeout = Duration::from_secs(self.profile.connect_timeout_secs.max(1));
                let _ = tokio::time::timeout(cleanup_timeout, self.cancel(execution_id)).await;
                // Mark this execution as killed before polling the future
                // again: `execute_inner` then disconnects the pooled
                // connection explicitly instead of letting it return to the
                // pool with a possibly unfinished result set.
                self.killed_executions.write().await.insert(execution_id);
                let drained = tokio::time::timeout(cleanup_timeout, &mut future)
                    .await
                    .is_ok();
                drop(future);
                self.running.write().await.remove(&execution_id);
                self.killed_executions.write().await.remove(&execution_id);
                if !drained && let Some(conn) = self.transaction.lock().await.take() {
                    self.read_transaction_time_zone.lock().await.take();
                    let _ = conn.disconnect().await;
                }
                // When the future did not drain, the pooled connection was
                // dropped mid-result; mysql_async's pool recycles such
                // connections asynchronously (drops the pending result and
                // discards them on fatal errors), so they never reach a
                // later query in a dirty state.
                Err(CockpitError::Timeout)
            }
        }
    }

    async fn mutate_row(&self, request: RowMutationRequest) -> Result<RowMutationResult> {
        if self.profile.read_only {
            return Err(CockpitError::Query("该连接处于只读模式".into()));
        }
        let (sql, values) = build_row_mutation(&request)?;
        let mut transaction = self.transaction.lock().await;
        if let Some(conn) = transaction.as_mut() {
            validate_mutation_key(conn, &request).await?;
            return execute_row_mutation(conn, sql, values, &request).await;
        }
        drop(transaction);
        let mut conn = self.connection().await?;
        validate_mutation_key(&mut conn, &request).await?;
        execute_row_mutation(&mut conn, sql, values, &request).await
    }

    async fn insert_rows(
        &self,
        database: &str,
        table: &str,
        columns: &[String],
        rows: &[Vec<CellValue>],
    ) -> Result<u64> {
        self.insert_rows_with_policy(database, table, columns, rows, ImportConflictPolicy::Error)
            .await
    }

    async fn insert_rows_with_policy(
        &self,
        database: &str,
        table: &str,
        columns: &[String],
        rows: &[Vec<CellValue>],
        policy: ImportConflictPolicy,
    ) -> Result<u64> {
        if self.profile.read_only {
            return Err(CockpitError::Query("该连接处于只读模式".into()));
        }
        if rows.is_empty() {
            return Ok(0);
        }
        if columns.is_empty() || rows.iter().any(|row| row.len() != columns.len()) {
            return Err(CockpitError::InvalidConfig("批量导入的列数不一致".into()));
        }
        let row_placeholders = format!("({})", vec!["?"; columns.len()].join(", "));
        let verb = match policy {
            ImportConflictPolicy::Error | ImportConflictPolicy::Upsert => "INSERT",
            ImportConflictPolicy::Ignore => "INSERT IGNORE",
            ImportConflictPolicy::Replace => "REPLACE",
        };
        let upsert = if policy == ImportConflictPolicy::Upsert {
            format!(
                " ON DUPLICATE KEY UPDATE {}",
                columns
                    .iter()
                    .map(|name| format!("{0}=VALUES({0})", quote_identifier(name)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            String::new()
        };
        let sql = format!(
            "{} INTO {}.{} ({}) VALUES {}{}",
            verb,
            quote_identifier(database),
            quote_identifier(table),
            columns
                .iter()
                .map(|name| quote_identifier(name))
                .collect::<Vec<_>>()
                .join(", "),
            vec![row_placeholders; rows.len()].join(", "),
            upsert,
        );
        let values = rows
            .iter()
            .flatten()
            .map(cell_to_mysql_value)
            .collect::<Result<Vec<_>>>()?;
        let mut transaction = self.transaction.lock().await;
        if let Some(conn) = transaction.as_mut() {
            conn.exec_drop(sql, Params::Positional(values))
                .await
                .map_err(query_error)?;
            return Ok(conn.affected_rows());
        }
        drop(transaction);
        let mut conn = self.connection().await?;
        conn.exec_drop(sql, Params::Positional(values))
            .await
            .map_err(query_error)?;
        Ok(conn.affected_rows())
    }

    async fn begin_transaction(&self) -> Result<()> {
        if self.profile.read_only {
            return Err(CockpitError::Query("只读连接不能开启写事务".into()));
        }
        let mut transaction = self.transaction.lock().await;
        if transaction.is_some() {
            return Err(CockpitError::Query("当前连接已有活动事务".into()));
        }
        let mut conn = self.connection().await?;
        conn.query_drop("START TRANSACTION")
            .await
            .map_err(query_error)?;
        self.read_transaction_time_zone.lock().await.take();
        *transaction = Some(conn);
        Ok(())
    }

    async fn begin_read_transaction(&self) -> Result<()> {
        let mut transaction = self.transaction.lock().await;
        if transaction.is_some() {
            return Err(CockpitError::Query("当前连接已有活动事务".into()));
        }
        let mut conn = self.connection().await?;
        let original_time_zone = conn
            .query_first::<String, _>("SELECT @@SESSION.time_zone")
            .await
            .map_err(query_error)?
            .unwrap_or_else(|| "SYSTEM".into());
        conn.query_drop("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .await
            .map_err(query_error)?;
        conn.query_drop("SET SESSION time_zone = '+00:00'")
            .await
            .map_err(query_error)?;
        if let Err(error) = conn
            .query_drop("START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY")
            .await
        {
            let _ = conn
                .exec_drop("SET SESSION time_zone = ?", (original_time_zone,))
                .await;
            return Err(query_error(error));
        }
        *self.read_transaction_time_zone.lock().await = Some(original_time_zone);
        *transaction = Some(conn);
        Ok(())
    }

    async fn commit_transaction(&self) -> Result<()> {
        let mut transaction = self.transaction.lock().await;
        let mut conn = transaction
            .take()
            .ok_or_else(|| CockpitError::Query("当前连接没有活动事务".into()))?;
        let transaction_result = conn.query_drop("COMMIT").await.map_err(query_error);
        let time_zone = self.read_transaction_time_zone.lock().await.take();
        let restore_result = if let Some(time_zone) = time_zone {
            conn.exec_drop("SET SESSION time_zone = ?", (time_zone,))
                .await
                .map_err(query_error)
        } else {
            Ok(())
        };
        transaction_result.and(restore_result)
    }

    async fn rollback_transaction(&self) -> Result<()> {
        let mut transaction = self.transaction.lock().await;
        let mut conn = transaction
            .take()
            .ok_or_else(|| CockpitError::Query("当前连接没有活动事务".into()))?;
        let transaction_result = conn.query_drop("ROLLBACK").await.map_err(query_error);
        let time_zone = self.read_transaction_time_zone.lock().await.take();
        let restore_result = if let Some(time_zone) = time_zone {
            conn.exec_drop("SET SESSION time_zone = ?", (time_zone,))
                .await
                .map_err(query_error)
        } else {
            Ok(())
        };
        transaction_result.and(restore_result)
    }

    async fn transaction_active(&self) -> bool {
        self.transaction.lock().await.is_some()
    }

    async fn cancel(&self, execution_id: Uuid) -> Result<bool> {
        let connection_id = self.running.read().await.get(&execution_id).copied();
        let Some(connection_id) = connection_id else {
            return Ok(false);
        };
        let mut control = Conn::new(self.control_opts.clone())
            .await
            .map_err(connection_error)?;
        control
            .query_drop(format!("KILL QUERY {connection_id}"))
            .await
            .map_err(query_error)?;
        Ok(true)
    }

    async fn close(&self) -> Result<()> {
        if let Some(mut conn) = self.transaction.lock().await.take() {
            let _ = conn.query_drop("ROLLBACK").await;
        }
        self.read_transaction_time_zone.lock().await.take();
        let result = self
            .pool
            .clone()
            .disconnect()
            .await
            .map_err(connection_error);
        if let Some(tunnel) = &self.ssh_tunnel {
            tunnel.close().await;
        }
        result
    }
}

impl MySqlSession {
    async fn connection(&self) -> Result<Conn> {
        self.pool.get_conn().await.map_err(connection_error)
    }

    async fn metadata_connection(&self) -> Result<Conn> {
        // Reuse an idle pooled connection for metadata reads instead of
        // opening a dedicated TCP+TLS connection on every call. Metadata
        // queries only read information_schema / SHOW with fully qualified
        // names and never mutate session state, so sharing the pool is safe.
        //
        // A single-connection pool (dedicated tab sessions force pool_size 1)
        // must not be drained for metadata: the data query or an open
        // transaction holds the only connection and would deadlock waiting
        // for it. Those sessions keep using a dedicated control connection.
        if self.profile.pool_size > 1 {
            // If every pooled connection is checked out (query or
            // transaction), wait briefly for an idle one, then fall back to a
            // dedicated connection instead of blocking the metadata read.
            match tokio::time::timeout(Duration::from_secs(5), self.pool.get_conn()).await {
                Ok(Ok(conn)) => return Ok(conn),
                Ok(Err(error)) => return Err(connection_error(error)),
                Err(_) => {}
            }
        }
        Conn::new(self.control_opts.clone())
            .await
            .map_err(connection_error)
    }

    async fn execute_inner(&self, request: ExecuteQueryRequest) -> Result<QueryResultPage> {
        let mut transaction = self.transaction.lock().await;
        if let Some(conn) = transaction.as_mut() {
            return self.execute_with_connection(conn, request).await;
        }
        drop(transaction);
        let mut conn = self.connection().await?;
        let execution_id = request.execution_id;
        let result = self.execute_with_connection(&mut conn, request).await;
        if self.killed_executions.read().await.contains(&execution_id) {
            // This query was cancelled by a timeout kill; the connection may
            // still hold an unfinished result set. Disconnect it explicitly
            // so it never returns to the pool in a dirty state.
            let _ = conn.disconnect().await;
        }
        result
    }

    async fn execute_with_connection(
        &self,
        conn: &mut Conn,
        request: ExecuteQueryRequest,
    ) -> Result<QueryResultPage> {
        let connection_id = conn.id();
        self.running
            .write()
            .await
            .insert(request.execution_id, connection_id);
        let started = Instant::now();
        let result = async {
            if let Some(database) = request.database.as_deref().filter(|v| !v.trim().is_empty()) {
                conn.query_drop(format!("USE {}", quote_identifier(database)))
                    .await
                    .map_err(query_error)?;
            }
            execute_on_connection(conn, &request, started).await
        }
        .await;
        self.running.write().await.remove(&request.execution_id);
        result
    }
}

async fn execute_on_connection(
    conn: &mut Conn,
    request: &ExecuteQueryRequest,
    started: Instant,
) -> Result<QueryResultPage> {
    let page_size = request.page_size.clamp(1, 5_000);
    let row_offset = request.row_offset;
    let mut result = conn.query_iter(&request.sql).await.map_err(query_error)?;
    let mut result_sets = Vec::new();
    let mut result_set_index = 0usize;
    loop {
        let columns = result
            .columns_ref()
            .iter()
            .map(column_meta)
            .collect::<Vec<_>>();
        let affected_rows = result.affected_rows();
        let Some(mut stream) = result.stream::<Row>().await.map_err(query_error)? else {
            break;
        };
        let mut rows = Vec::new();
        let mut seen = 0usize;
        let mut has_more = false;
        while let Some(row) = stream.next().await {
            let row = row.map_err(query_error)?;
            if seen < row_offset {
                seen += 1;
                continue;
            }
            if rows.len() < page_size {
                rows.push(row_to_values(&row));
            } else {
                has_more = true;
            }
            seen += 1;
        }
        drop(stream);
        result_sets.push(QueryResultSet {
            columns,
            rows,
            affected_rows,
            truncated: has_more,
            has_more,
            result_set_index,
            row_offset,
            page_size,
        });
        result_set_index += 1;
    }
    result.drop_result().await.map_err(query_error)?;
    let messages = load_warnings(conn).await;
    let mut result_sets = result_sets.into_iter();
    let first = result_sets.next().unwrap_or(QueryResultSet {
        columns: vec![],
        rows: vec![],
        affected_rows: 0,
        truncated: false,
        has_more: false,
        result_set_index: 0,
        row_offset,
        page_size,
    });
    Ok(QueryResultPage {
        execution_id: request.execution_id,
        columns: first.columns,
        rows: first.rows,
        affected_rows: first.affected_rows,
        execution_time_ms: started.elapsed().as_millis(),
        truncated: first.truncated,
        has_more: first.has_more,
        result_set_index: 0,
        messages,
        row_offset,
        page_size,
        additional_result_sets: result_sets.collect(),
    })
}

async fn load_warnings(conn: &mut Conn) -> Vec<QueryMessage> {
    let rows = conn
        .query::<Row, _>("SHOW WARNINGS")
        .await
        .unwrap_or_default();
    rows.into_iter()
        .filter_map(|row| {
            Some(QueryMessage {
                severity: row_string(&row, 0)?,
                code: row_u64(&row, 1).map(|value| value.to_string()),
                message: row_string(&row, 2).unwrap_or_default(),
            })
        })
        .collect()
}

fn column_meta(column: &mysql_async::Column) -> ColumnMeta {
    let flags = column.flags();
    ColumnMeta {
        name: column.name_str().into_owned(),
        database_type: column_type_name(column),
        nullable: !flags.contains(ColumnFlags::NOT_NULL_FLAG),
        unsigned: flags.contains(ColumnFlags::UNSIGNED_FLAG),
        binary: column.character_set() == 63 || flags.contains(ColumnFlags::BINARY_FLAG),
    }
}

fn column_type_name(column: &mysql_async::Column) -> String {
    use ColumnType::*;
    let binary = column.character_set() == 63;
    match column.column_type() {
        MYSQL_TYPE_TINY => "tinyint",
        MYSQL_TYPE_SHORT => "smallint",
        MYSQL_TYPE_INT24 => "mediumint",
        MYSQL_TYPE_LONG => "int",
        MYSQL_TYPE_LONGLONG => "bigint",
        MYSQL_TYPE_FLOAT => "float",
        MYSQL_TYPE_DOUBLE => "double",
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => "decimal",
        MYSQL_TYPE_BIT => "bit",
        MYSQL_TYPE_YEAR => "year",
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => "date",
        MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2 => "time",
        MYSQL_TYPE_DATETIME | MYSQL_TYPE_DATETIME2 => "datetime",
        MYSQL_TYPE_TIMESTAMP | MYSQL_TYPE_TIMESTAMP2 => "timestamp",
        MYSQL_TYPE_JSON => "json",
        MYSQL_TYPE_TINY_BLOB => {
            if binary {
                "tinyblob"
            } else {
                "tinytext"
            }
        }
        MYSQL_TYPE_MEDIUM_BLOB => {
            if binary {
                "mediumblob"
            } else {
                "mediumtext"
            }
        }
        MYSQL_TYPE_LONG_BLOB => {
            if binary {
                "longblob"
            } else {
                "longtext"
            }
        }
        MYSQL_TYPE_BLOB => {
            if binary {
                "blob"
            } else {
                "text"
            }
        }
        MYSQL_TYPE_VARCHAR | MYSQL_TYPE_VAR_STRING => {
            if binary {
                "varbinary"
            } else {
                "varchar"
            }
        }
        MYSQL_TYPE_STRING => {
            if binary {
                "binary"
            } else {
                "char"
            }
        }
        MYSQL_TYPE_GEOMETRY => "geometry",
        MYSQL_TYPE_NULL => "null",
        _ => "unknown",
    }
    .to_string()
}

fn row_to_values(row: &Row) -> Vec<CellValue> {
    (0..row.len())
        .map(|index| {
            let column = &row.columns_ref()[index];
            value_to_cell(row.as_ref(index).unwrap_or(&Value::NULL), column)
        })
        .collect()
}

fn value_to_cell(value: &Value, column: &mysql_async::Column) -> CellValue {
    let column_type = column.column_type();
    match value {
        Value::NULL => CellValue::Null,
        Value::Int(value)
            if matches!(
                column_type,
                ColumnType::MYSQL_TYPE_LONGLONG
                    | ColumnType::MYSQL_TYPE_NEWDECIMAL
                    | ColumnType::MYSQL_TYPE_DECIMAL
            ) =>
        {
            CellValue::Signed(value.to_string())
        }
        Value::Int(value) => CellValue::Signed(value.to_string()),
        Value::UInt(value) => CellValue::Unsigned(value.to_string()),
        Value::Float(value) => CellValue::Float(*value as f64),
        Value::Double(value) => CellValue::Float(*value),
        Value::Date(year, month, day, hour, minute, second, micros) => {
            let date = if *hour == 0 && *minute == 0 && *second == 0 && *micros == 0 {
                format!("{year:04}-{month:02}-{day:02}")
            } else if *micros == 0 {
                format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
            } else {
                format!(
                    "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{micros:06}"
                )
            };
            if matches!(
                column_type,
                ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_NEWDATE
            ) {
                CellValue::Date(date)
            } else {
                CellValue::DateTime(date)
            }
        }
        Value::Time(negative, days, hours, minutes, seconds, micros) => {
            let sign = if *negative { "-" } else { "" };
            let total_hours = days * 24 + *hours as u32;
            CellValue::Time(if *micros == 0 {
                format!("{sign}{total_hours:02}:{minutes:02}:{seconds:02}")
            } else {
                format!("{sign}{total_hours:02}:{minutes:02}:{seconds:02}.{micros:06}")
            })
        }
        Value::Bytes(bytes)
            if matches!(
                column_type,
                ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL
            ) =>
        {
            CellValue::Decimal(String::from_utf8_lossy(bytes).into_owned())
        }
        Value::Bytes(bytes) if column_type == ColumnType::MYSQL_TYPE_BIT => mysql_bit_value(bytes)
            .map_or_else(
                || CellValue::Bytes {
                    base64: BASE64.encode(bytes),
                    preview: None,
                    length: bytes.len(),
                },
                |value| CellValue::Unsigned(value.to_string()),
            ),
        Value::Bytes(bytes) if column_type == ColumnType::MYSQL_TYPE_JSON => {
            CellValue::Json(String::from_utf8_lossy(bytes).into_owned())
        }
        Value::Bytes(bytes) if column_type == ColumnType::MYSQL_TYPE_GEOMETRY => {
            let srid = read_mysql_srid(bytes);
            let wkb = srid.and_then(|_| bytes.get(4..)).unwrap_or(bytes);
            CellValue::Geometry {
                wkb_base64: BASE64.encode(wkb),
                srid,
            }
        }
        Value::Bytes(bytes)
            if let Some(value) = text_protocol_cell(
                bytes,
                column_type,
                column.flags().contains(ColumnFlags::UNSIGNED_FLAG),
            ) =>
        {
            value
        }
        Value::Bytes(bytes) if column.character_set() == 63 => CellValue::Bytes {
            base64: BASE64.encode(bytes),
            preview: printable_preview(bytes),
            length: bytes.len(),
        },
        Value::Bytes(bytes) => CellValue::Text(String::from_utf8_lossy(bytes).into_owned()),
    }
}

fn text_protocol_cell(bytes: &[u8], column_type: ColumnType, unsigned: bool) -> Option<CellValue> {
    let text = std::str::from_utf8(bytes).ok()?;
    match column_type {
        ColumnType::MYSQL_TYPE_TINY
        | ColumnType::MYSQL_TYPE_SHORT
        | ColumnType::MYSQL_TYPE_INT24
        | ColumnType::MYSQL_TYPE_LONG
        | ColumnType::MYSQL_TYPE_LONGLONG
        | ColumnType::MYSQL_TYPE_YEAR => {
            if unsigned {
                text.parse::<u64>().ok()?;
                Some(CellValue::Unsigned(text.into()))
            } else {
                text.parse::<i64>().ok()?;
                Some(CellValue::Signed(text.into()))
            }
        }
        ColumnType::MYSQL_TYPE_FLOAT | ColumnType::MYSQL_TYPE_DOUBLE => text
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(CellValue::Float),
        ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_NEWDATE => {
            Some(CellValue::Date(text.into()))
        }
        ColumnType::MYSQL_TYPE_TIME | ColumnType::MYSQL_TYPE_TIME2 => {
            Some(CellValue::Time(text.into()))
        }
        ColumnType::MYSQL_TYPE_DATETIME
        | ColumnType::MYSQL_TYPE_DATETIME2
        | ColumnType::MYSQL_TYPE_TIMESTAMP
        | ColumnType::MYSQL_TYPE_TIMESTAMP2 => Some(CellValue::DateTime(text.into())),
        _ => None,
    }
}

fn printable_preview(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    text.chars()
        .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
        .then(|| text.chars().take(200).collect())
}

fn mysql_bit_value(bytes: &[u8]) -> Option<u64> {
    (bytes.len() <= std::mem::size_of::<u64>()).then(|| {
        bytes
            .iter()
            .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte))
    })
}

fn read_mysql_srid(bytes: &[u8]) -> Option<u32> {
    (bytes.len() >= 5 && matches!(bytes[4], 0 | 1))
        .then(|| u32::from_le_bytes(bytes[0..4].try_into().expect("four bytes")))
}

fn build_row_mutation(request: &RowMutationRequest) -> Result<(String, Vec<Value>)> {
    if request.database.trim().is_empty() || request.table.trim().is_empty() {
        return Err(CockpitError::InvalidConfig(
            "行操作必须指定数据库和表".into(),
        ));
    }
    let target = format!(
        "{}.{}",
        quote_identifier(request.database.trim()),
        quote_identifier(request.table.trim())
    );
    match request.kind {
        RowMutationKind::Insert => {
            if request.values.is_empty() {
                return Err(CockpitError::InvalidConfig("新增行至少需要一个字段".into()));
            }
            let columns = request
                .values
                .iter()
                .map(|(name, _)| quote_identifier(name))
                .collect::<Vec<_>>()
                .join(", ");
            let placeholders = vec!["?"; request.values.len()].join(", ");
            let values = request
                .values
                .iter()
                .map(|(_, value)| cell_to_mysql_value(value))
                .collect::<Result<Vec<_>>>()?;
            Ok((
                format!("INSERT INTO {target} ({columns}) VALUES ({placeholders})"),
                values,
            ))
        }
        RowMutationKind::Update => {
            if request.values.is_empty() {
                return Err(CockpitError::InvalidConfig("没有需要更新的字段".into()));
            }
            if request.key_values.is_empty() {
                return Err(CockpitError::InvalidConfig(
                    "更新行必须包含主键或唯一键".into(),
                ));
            }
            let assignments = request
                .values
                .iter()
                .map(|(name, _)| format!("{} = ?", quote_identifier(name)))
                .collect::<Vec<_>>()
                .join(", ");
            let mut values = request
                .values
                .iter()
                .map(|(_, value)| cell_to_mysql_value(value))
                .collect::<Result<Vec<_>>>()?;
            let where_sql = mutation_where(request, &mut values)?;
            Ok((
                format!("UPDATE {target} SET {assignments} WHERE {where_sql} LIMIT 1"),
                values,
            ))
        }
        RowMutationKind::Delete => {
            if request.key_values.is_empty() {
                return Err(CockpitError::InvalidConfig(
                    "删除行必须包含主键或唯一键".into(),
                ));
            }
            let mut values = Vec::new();
            let where_sql = mutation_where(request, &mut values)?;
            Ok((
                format!("DELETE FROM {target} WHERE {where_sql} LIMIT 1"),
                values,
            ))
        }
    }
}

#[derive(Debug)]
struct MutationKeyIndex {
    columns: Vec<String>,
    primary: bool,
    all_columns_non_null: bool,
}

async fn validate_mutation_key(conn: &mut Conn, request: &RowMutationRequest) -> Result<()> {
    if request.kind == RowMutationKind::Insert {
        return Ok(());
    }
    let rows: Vec<Row> = conn
        .exec(
            "SELECT s.INDEX_NAME, s.COLUMN_NAME, s.NON_UNIQUE, c.IS_NULLABLE FROM information_schema.STATISTICS s LEFT JOIN information_schema.COLUMNS c ON c.TABLE_SCHEMA = s.TABLE_SCHEMA AND c.TABLE_NAME = s.TABLE_NAME AND c.COLUMN_NAME = s.COLUMN_NAME WHERE s.TABLE_SCHEMA = ? AND s.TABLE_NAME = ? AND (s.INDEX_NAME = 'PRIMARY' OR s.NON_UNIQUE = 0) ORDER BY s.INDEX_NAME, s.SEQ_IN_INDEX",
            (&request.database, &request.table),
        )
        .await
        .map_err(query_error)?;
    let mut by_name: HashMap<String, MutationKeyIndex> = HashMap::new();
    for row in rows {
        let Some(name) = row_string(&row, "INDEX_NAME") else {
            continue;
        };
        let entry = by_name
            .entry(name.clone())
            .or_insert_with(|| MutationKeyIndex {
                columns: Vec::new(),
                primary: name == "PRIMARY",
                all_columns_non_null: true,
            });
        if let Some(column) = row_string(&row, "COLUMN_NAME") {
            entry.columns.push(column);
        }
        entry.all_columns_non_null &= row_string(&row, "IS_NULLABLE")
            .is_some_and(|nullable| nullable.eq_ignore_ascii_case("NO"));
    }
    if mutation_key_is_safe(request, by_name.values()) {
        Ok(())
    } else {
        Err(CockpitError::InvalidConfig(
            "行操作必须使用完整主键或所有字段均非空的唯一索引".into(),
        ))
    }
}

fn mutation_key_is_safe<'a>(
    request: &RowMutationRequest,
    indexes: impl IntoIterator<Item = &'a MutationKeyIndex>,
) -> bool {
    let mut key_names = request
        .key_values
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    key_names.sort_unstable();
    key_names.dedup();
    indexes.into_iter().any(|index| {
        if index.columns.is_empty() || (!index.primary && !index.all_columns_non_null) {
            return false;
        }
        let mut index_names = index.columns.iter().map(String::as_str).collect::<Vec<_>>();
        index_names.sort_unstable();
        index_names == key_names
    })
}

fn mutation_where(request: &RowMutationRequest, values: &mut Vec<Value>) -> Result<String> {
    let mut predicates = Vec::new();
    for (name, value) in request
        .key_values
        .iter()
        .chain(request.original_values.iter())
    {
        predicates.push(format!("{} <=> ?", quote_identifier(name)));
        values.push(cell_to_mysql_value(value)?);
    }
    Ok(predicates.join(" AND "))
}

fn cell_to_mysql_value(value: &CellValue) -> Result<Value> {
    Ok(match value {
        CellValue::Null => Value::NULL,
        CellValue::Bool(value) => Value::Int(i64::from(*value)),
        CellValue::Signed(value) => Value::Int(
            value
                .parse::<i64>()
                .map_err(|_| CockpitError::InvalidConfig(format!("无效的有符号整数：{value}")))?,
        ),
        CellValue::Unsigned(value) => Value::UInt(
            value
                .parse::<u64>()
                .map_err(|_| CockpitError::InvalidConfig(format!("无效的无符号整数：{value}")))?,
        ),
        CellValue::Float(value) if value.is_finite() => Value::Double(*value),
        CellValue::Float(_) => {
            return Err(CockpitError::InvalidConfig("浮点数必须是有限值".into()));
        }
        CellValue::Decimal(value)
        | CellValue::Text(value)
        | CellValue::Date(value)
        | CellValue::Time(value)
        | CellValue::DateTime(value)
        | CellValue::Json(value) => Value::Bytes(value.as_bytes().to_vec()),
        CellValue::Bytes { base64, .. } => Value::Bytes(
            BASE64
                .decode(base64)
                .map_err(|error| CockpitError::InvalidConfig(error.to_string()))?,
        ),
        CellValue::Geometry { wkb_base64, srid } => {
            let wkb = BASE64
                .decode(wkb_base64)
                .map_err(|error| CockpitError::InvalidConfig(error.to_string()))?;
            let mut bytes = Vec::with_capacity(wkb.len() + 4);
            bytes.extend_from_slice(&srid.unwrap_or_default().to_le_bytes());
            bytes.extend_from_slice(&wkb);
            Value::Bytes(bytes)
        }
    })
}

async fn execute_row_mutation(
    conn: &mut Conn,
    sql: String,
    values: Vec<Value>,
    request: &RowMutationRequest,
) -> Result<RowMutationResult> {
    let result = conn
        .exec_iter(sql, Params::Positional(values))
        .await
        .map_err(query_error)?;
    let affected_rows = result.affected_rows();
    result.drop_result().await.map_err(query_error)?;
    Ok(RowMutationResult {
        affected_rows,
        concurrent_change: matches!(
            request.kind,
            RowMutationKind::Update | RowMutationKind::Delete
        ) && affected_rows == 0
            && !request.original_values.is_empty(),
    })
}

fn quote_identifier(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}

fn quote_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}
fn connection_error(error: mysql_async::Error) -> CockpitError {
    CockpitError::Connection(error.to_string())
}
fn query_error(error: mysql_async::Error) -> CockpitError {
    CockpitError::Query(error.to_string())
}

fn row_string<I>(row: &Row, index: I) -> Option<String>
where
    I: mysql_async::prelude::ColumnIndex + Copy,
{
    row.get_opt::<String, I>(index)
        .and_then(|v| v.ok())
        .or_else(|| {
            row.get_opt::<Vec<u8>, I>(index)
                .and_then(|v| v.ok())
                .map(|v| String::from_utf8_lossy(&v).into_owned())
        })
}

fn row_u64<I>(row: &Row, index: I) -> Option<u64>
where
    I: mysql_async::prelude::ColumnIndex + Copy,
{
    row.get_opt::<u64, I>(index)
        .and_then(|v| v.ok())
        .or_else(|| {
            row.get_opt::<i64, I>(index)
                .and_then(|v| v.ok())
                .and_then(|v| u64::try_from(v).ok())
        })
        .or_else(|| row_string(row, index).and_then(|v| v.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn connection_profile() -> ConnectionProfile {
        let now = Utc::now();
        ConnectionProfile {
            id: Uuid::new_v4(),
            driver_kind: cockpit_core::DatabaseKind::MySql,
            group: None,
            name: "test".into(),
            host: "127.0.0.1".into(),
            port: 3306,
            username: "root".into(),
            database: None,
            tls: cockpit_core::TlsOptions {
                mode: TlsMode::VerifyIdentity,
                ca_cert_path: Some("/tmp/test-ca.pem".into()),
                ..Default::default()
            },
            ssh: None,
            connect_timeout_secs: 5,
            query_timeout_secs: 5,
            pool_size: 1,
            read_only: false,
            production: false,
            color: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn identifiers_escape_backticks() {
        assert_eq!(quote_identifier("odd`name"), "`odd``name`");
    }

    #[test]
    fn verify_identity_can_use_the_original_hostname_behind_ssh() {
        let profile = connection_profile();
        let (_pool, opts) = create_pool(
            &profile,
            "",
            TlsMode::VerifyIdentity,
            Some("db.internal.example"),
        )
        .unwrap();
        assert_eq!(
            opts.ssl_opts().and_then(SslOpts::tls_hostname_override),
            Some("db.internal.example")
        );
    }

    #[test]
    fn binary_preview_rejects_control_data() {
        assert_eq!(printable_preview(&[0, 1, 2]), None);
        assert_eq!(printable_preview(b"hello"), Some("hello".into()));
    }

    #[test]
    fn bit_values_are_decoded_as_unsigned_integers() {
        assert_eq!(mysql_bit_value(&[0]), Some(0));
        assert_eq!(mysql_bit_value(&[1]), Some(1));
        assert_eq!(mysql_bit_value(&[1, 0]), Some(256));
        assert_eq!(mysql_bit_value(&[u8::MAX; 8]), Some(u64::MAX));
        assert_eq!(mysql_bit_value(&[0; 9]), None);
    }

    #[test]
    fn text_protocol_values_keep_numeric_and_temporal_types() {
        assert_eq!(
            text_protocol_cell(b"3", ColumnType::MYSQL_TYPE_LONGLONG, true),
            Some(CellValue::Unsigned("3".into()))
        );
        assert_eq!(
            text_protocol_cell(b"-2", ColumnType::MYSQL_TYPE_LONG, false),
            Some(CellValue::Signed("-2".into()))
        );
        assert_eq!(
            text_protocol_cell(b"1.25", ColumnType::MYSQL_TYPE_DOUBLE, false),
            Some(CellValue::Float(1.25))
        );
        assert_eq!(
            text_protocol_cell(b"2026-08-10", ColumnType::MYSQL_TYPE_DATE, false),
            Some(CellValue::Date("2026-08-10".into()))
        );
    }

    #[test]
    fn row_mutations_bind_values_and_require_keys() {
        let request = RowMutationRequest {
            database: "demo".into(),
            table: "odd`table".into(),
            kind: RowMutationKind::Update,
            values: vec![("title".into(), CellValue::Text("new".into()))],
            key_values: vec![("id".into(), CellValue::Unsigned("7".into()))],
            original_values: vec![("title".into(), CellValue::Text("old".into()))],
        };
        let (sql, values) = build_row_mutation(&request).unwrap();
        assert_eq!(
            sql,
            "UPDATE `demo`.`odd``table` SET `title` = ? WHERE `id` <=> ? AND `title` <=> ? LIMIT 1"
        );
        assert_eq!(values.len(), 3);
    }

    #[test]
    fn row_mutations_reject_nullable_unique_keys() {
        let request = RowMutationRequest {
            database: "demo".into(),
            table: "items".into(),
            kind: RowMutationKind::Delete,
            values: vec![],
            key_values: vec![("external_id".into(), CellValue::Null)],
            original_values: vec![("external_id".into(), CellValue::Null)],
        };
        let nullable_unique = MutationKeyIndex {
            columns: vec!["external_id".into()],
            primary: false,
            all_columns_non_null: false,
        };
        assert!(!mutation_key_is_safe(&request, [&nullable_unique]));

        let primary = MutationKeyIndex {
            primary: true,
            ..nullable_unique
        };
        assert!(mutation_key_is_safe(&request, [&primary]));
    }
}
