use std::{
    collections::BTreeMap,
    fs,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use cockpit_core::{
    CellValue, CockpitError, ColumnInfo, ColumnMeta, ConnectionInfo, ConnectionProfile,
    DatabaseDriver, DatabaseInfo, DatabaseObjectDefinition, DatabaseObjectKind, DriverSession,
    EventInfo, ExecuteQueryRequest, ForeignKeyInfo, ImportConflictPolicy, IndexInfo,
    QueryResultPage, QueryResultSet, Result, RiskLevel, RoutineInfo, RoutineParameter,
    RowMutationKind, RowMutationRequest, RowMutationResult, ServerLockInfo, ServerMetric,
    ServerProcessInfo, ServerVariable, TableDetail, TableInfo, TlsMode, TriggerInfo, UserAccount,
    safety::assess_sql,
};
use futures::StreamExt;
use native_tls::{Certificate, Identity, TlsConnector};
use postgres_native_tls::MakeTlsConnector;
use tokio::sync::{Mutex, RwLock};
use tokio_postgres::{Client, Config, NoTls, SimpleQueryMessage, SimpleQueryStream};
use uuid::Uuid;

#[derive(Default)]
pub struct PostgresDriver;

pub struct PostgresSession {
    profile: ConnectionProfile,
    client: Mutex<Option<Client>>,
    cancel_token: tokio_postgres::CancelToken,
    transaction_active: RwLock<bool>,
    /// 最后一次通过 SET search_path 应用的 schema，避免每条查询都重复设置会话状态。
    search_path: Mutex<Option<String>>,
}

fn connection_error(error: impl std::fmt::Display) -> CockpitError {
    CockpitError::Connection(error.to_string())
}

fn query_error(error: impl std::fmt::Display) -> CockpitError {
    CockpitError::Query(error.to_string())
}

/// 只读连接拒绝一切写路径入口（与 MySQL 驱动的错误消息保持一致）。
fn ensure_writable(read_only: bool) -> Result<()> {
    if read_only {
        return Err(CockpitError::Query("该连接处于只读模式".into()));
    }
    Ok(())
}

/// pg_terminate_backend 接收 int4 参数：超出范围的 u64 用 `as i32` 强转会
/// 回绕成错误的（甚至正在使用的）进程号，必须在发送前校验范围。
fn validate_process_pid(process_id: u64) -> Result<i32> {
    i32::try_from(process_id).map_err(|_| {
        CockpitError::InvalidConfig(format!(
            "无效的进程 ID：{process_id}，超出 PostgreSQL 会话 ID 范围"
        ))
    })
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn quote_text(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn client_config(profile: &ConnectionProfile, password: &str) -> Config {
    let mut config = Config::new();
    config
        .host(profile.host.trim())
        .port(profile.port)
        .user(profile.username.trim())
        .password(password)
        .dbname(
            profile
                .database
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("postgres"),
        )
        .connect_timeout(Duration::from_secs(profile.connect_timeout_secs.max(1)));
    config
}

fn tls_connector(profile: &ConnectionProfile) -> Result<MakeTlsConnector> {
    let mut builder = TlsConnector::builder();
    match profile.tls.mode {
        TlsMode::Disabled => {}
        TlsMode::Preferred | TlsMode::Required => {
            builder.danger_accept_invalid_certs(true);
            builder.danger_accept_invalid_hostnames(true);
        }
        TlsMode::VerifyCa => {
            builder.danger_accept_invalid_hostnames(true);
        }
        TlsMode::VerifyIdentity => {}
    }
    if let Some(path) = profile
        .tls
        .ca_cert_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let bytes = fs::read(path)
            .map_err(|error| CockpitError::InvalidConfig(format!("无法读取 CA 证书：{error}")))?;
        let certificate = Certificate::from_pem(&bytes)
            .map_err(|error| CockpitError::InvalidConfig(format!("CA 证书无效：{error}")))?;
        builder.add_root_certificate(certificate);
    } else if matches!(
        profile.tls.mode,
        TlsMode::VerifyCa | TlsMode::VerifyIdentity
    ) {
        return Err(CockpitError::InvalidConfig(
            "校验证书时必须选择 CA 证书".into(),
        ));
    }
    match (
        profile
            .tls
            .client_cert_path
            .as_deref()
            .filter(|value| !value.trim().is_empty()),
        profile
            .tls
            .client_key_path
            .as_deref()
            .filter(|value| !value.trim().is_empty()),
    ) {
        (Some(certificate_path), Some(key_path)) => {
            let certificate = fs::read(certificate_path).map_err(|error| {
                CockpitError::InvalidConfig(format!("无法读取客户端证书：{error}"))
            })?;
            let key = fs::read(key_path).map_err(|error| {
                CockpitError::InvalidConfig(format!("无法读取客户端私钥：{error}"))
            })?;
            let identity = Identity::from_pkcs8(&certificate, &key).map_err(|error| {
                CockpitError::InvalidConfig(format!("客户端证书或私钥无效：{error}"))
            })?;
            builder.identity(identity);
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(CockpitError::InvalidConfig(
                "客户端证书和私钥必须同时配置".into(),
            ));
        }
        (None, None) => {}
    }
    Ok(MakeTlsConnector::new(
        builder.build().map_err(connection_error)?,
    ))
}

async fn connect_client(profile: &ConnectionProfile, password: &str) -> Result<Client> {
    if profile.ssh.is_some() {
        return Err(CockpitError::Unsupported(
            "PostgreSQL 的 SSH 隧道暂不支持；可先建立系统隧道后连接本地端口".into(),
        ));
    }
    let config = client_config(profile, password);
    if profile.tls.mode == TlsMode::Disabled {
        let (client, connection) = config.connect(NoTls).await.map_err(connection_error)?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        return Ok(client);
    }
    let connector = tls_connector(profile)?;
    match config.connect(connector).await {
        Ok((client, connection)) => {
            tokio::spawn(async move {
                let _ = connection.await;
            });
            Ok(client)
        }
        Err(error) if profile.tls.mode == TlsMode::Preferred => {
            let (client, connection) = config.connect(NoTls).await.map_err(|plain_error| {
                connection_error(format!(
                    "TLS 连接失败（{error}），非 TLS 连接也失败（{plain_error}）"
                ))
            })?;
            tokio::spawn(async move {
                let _ = connection.await;
            });
            Ok(client)
        }
        Err(error) => Err(connection_error(error)),
    }
}

async fn connection_info_for(client: &Client) -> Result<ConnectionInfo> {
    let row = client.query_one(
        "SELECT current_setting('server_version'), current_database(), pg_backend_pid(), COALESCE((SELECT ssl::text || COALESCE(' · ' || cipher, '') FROM pg_stat_ssl WHERE pid = pg_backend_pid()), 'false')",
        &[],
    ).await.map_err(query_error)?;
    let tls: String = row.get(3);
    Ok(ConnectionInfo {
        server_version: row.get(0),
        server_comment: Some("PostgreSQL".into()),
        connection_id: row.get::<_, i32>(2).max(0) as u32,
        current_database: Some(row.get(1)),
        tls_cipher: (tls != "false").then_some(tls),
    })
}

#[async_trait]
impl DatabaseDriver for PostgresDriver {
    fn kind(&self) -> &'static str {
        "postgresql"
    }

    async fn test(&self, profile: &ConnectionProfile, password: &str) -> Result<ConnectionInfo> {
        profile.validate()?;
        let client = connect_client(profile, password).await?;
        connection_info_for(&client).await
    }

    async fn open(
        &self,
        profile: ConnectionProfile,
        password: String,
    ) -> Result<Arc<dyn DriverSession>> {
        profile.validate()?;
        let client = connect_client(&profile, &password).await?;
        let cancel_token = client.cancel_token();
        Ok(Arc::new(PostgresSession {
            profile,
            client: Mutex::new(Some(client)),
            cancel_token,
            transaction_active: RwLock::new(false),
            search_path: Mutex::new(None),
        }))
    }
}

impl PostgresSession {
    async fn with_client<T>(&self, action: impl AsyncFnOnce(&Client) -> Result<T>) -> Result<T> {
        let guard = self.client.lock().await;
        let client = guard
            .as_ref()
            .ok_or_else(|| CockpitError::Connection("连接已经关闭".into()))?;
        action(client).await
    }

    async fn execute_inner(&self, request: ExecuteQueryRequest) -> Result<QueryResultPage> {
        let started = Instant::now();
        let guard = self.client.lock().await;
        let client = guard
            .as_ref()
            .ok_or_else(|| CockpitError::Connection("连接已经关闭".into()))?;
        if let Some(schema) = request
            .database
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            // 每个 schema 只在会话中设置一次 search_path，避免每条查询前都产生会话副作用；
            // 事务回滚会还原 search_path，因此回滚时清空缓存（见 rollback_transaction）。
            let mut search_path = self.search_path.lock().await;
            if search_path.as_deref() != Some(schema) {
                client
                    .batch_execute(&format!(
                        "SET search_path TO {}, public",
                        quote_identifier(schema)
                    ))
                    .await
                    .map_err(query_error)?;
                *search_path = Some(schema.to_owned());
            }
        }
        let stream = client
            .simple_query_raw(&request.sql)
            .await
            .map_err(query_error)?;
        simple_stream_to_page(&request, stream, started).await
    }
}

#[async_trait]
impl DriverSession for PostgresSession {
    fn connection_id(&self) -> Uuid {
        self.profile.id
    }

    async fn connection_info(&self) -> Result<ConnectionInfo> {
        self.with_client(connection_info_for).await
    }

    async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        self.with_client(async |client| {
            let rows = client.query(
                "SELECT schema_name FROM information_schema.schemata WHERE schema_name <> 'information_schema' AND schema_name NOT LIKE 'pg_%' ORDER BY schema_name",
                &[],
            ).await.map_err(query_error)?;
            Ok(rows.into_iter().map(|row| DatabaseInfo { name: row.get(0) }).collect())
        }).await
    }

    async fn list_tables(
        &self,
        database: &str,
        filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TableInfo>> {
        let database = database.to_owned();
        let filter = filter.unwrap_or_default().to_owned();
        self.with_client(async move |client| {
            let rows = client.query(
                "SELECT t.table_name, t.table_type, COALESCE(c.reltuples::bigint, 0), COALESCE(pg_total_relation_size(c.oid), 0) FROM information_schema.tables t LEFT JOIN pg_namespace n ON n.nspname=t.table_schema LEFT JOIN pg_class c ON c.relnamespace=n.oid AND c.relname=t.table_name WHERE t.table_schema=$1 AND ($2='' OR t.table_name ILIKE '%' || $2 || '%') ORDER BY t.table_name LIMIT $3 OFFSET $4",
                &[&database, &filter, &(limit as i64), &(offset as i64)],
            ).await.map_err(query_error)?;
            Ok(rows.into_iter().map(|row| TableInfo {
                database: database.clone(),
                name: row.get(0),
                table_type: row.get(1),
                comment: None,
                estimated_rows: Some(row.get::<_, i64>(2).max(0) as u64),
                total_bytes: Some(row.get::<_, i64>(3).max(0) as u64),
            }).collect())
        }).await
    }

    async fn list_columns(&self, database: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let database = database.to_owned();
        let table = table.to_owned();
        self.with_client(async move |client| load_columns(client, &database, &table).await)
            .await
    }

    async fn table_detail(&self, database: &str, table: &str) -> Result<TableDetail> {
        let database = database.to_owned();
        let table = table.to_owned();
        self.with_client(async move |client| {
            let columns = load_columns(client, &database, &table).await?;
            let table_type = client.query_opt("SELECT table_type FROM information_schema.tables WHERE table_schema=$1 AND table_name=$2", &[&database, &table]).await.map_err(query_error)?
                .map(|row| row.get(0)).ok_or_else(|| CockpitError::NotFound(format!("对象不存在：{database}.{table}")))?;
            let index_rows = client.query("SELECT indexname, indexdef FROM pg_indexes WHERE schemaname=$1 AND tablename=$2 ORDER BY indexname", &[&database, &table]).await.map_err(query_error)?;
            let mut indexes = Vec::new();
            let mut index_definitions = Vec::new();
            for row in index_rows {
                let name: String = row.get(0);
                let definition: String = row.get(1);
                indexes.push(IndexInfo {
                    primary: definition.contains(" UNIQUE INDEX ") && name.ends_with("_pkey"),
                    unique: definition.contains(" UNIQUE INDEX "),
                    columns: parse_index_columns(&definition),
                    index_type: definition.split(" USING ").nth(1).and_then(|value| value.split_whitespace().next()).map(str::to_uppercase),
                    name,
                });
                index_definitions.push(definition);
            }
            let foreign_keys = load_foreign_keys(client, &database, &table).await?;
            let mut ddl = format!("CREATE TABLE {}.{} (\n", quote_identifier(&database), quote_identifier(&table));
            let mut definitions = columns.iter().map(column_ddl).collect::<Vec<_>>();
            for index in indexes.iter().filter(|index| index.primary) {
                definitions.push(format!("CONSTRAINT {} PRIMARY KEY ({})", quote_identifier(&index.name), index.columns.iter().map(|column| quote_identifier(column)).collect::<Vec<_>>().join(", ")));
            }
            for foreign_key in &foreign_keys {
                definitions.push(format!("CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {}.{} ({}){}{}",
                    quote_identifier(&foreign_key.name),
                    foreign_key.columns.iter().map(|column| quote_identifier(column)).collect::<Vec<_>>().join(", "),
                    quote_identifier(&foreign_key.referenced_database), quote_identifier(&foreign_key.referenced_table),
                    foreign_key.referenced_columns.iter().map(|column| quote_identifier(column)).collect::<Vec<_>>().join(", "),
                    foreign_key.on_update.as_deref().map(|rule| format!(" ON UPDATE {rule}")).unwrap_or_default(),
                    foreign_key.on_delete.as_deref().map(|rule| format!(" ON DELETE {rule}")).unwrap_or_default(),
                ));
            }
            ddl.push_str(&definitions.into_iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join(",\n"));
            ddl.push_str("\n);");
            for (index, definition) in indexes.iter().zip(index_definitions).filter(|(index, _)| !index.primary) {
                let _ = index;
                ddl.push('\n');
                ddl.push_str(definition.trim_end_matches(';'));
                ddl.push(';');
            }
            let table_info = TableInfo { database: database.clone(), name: table.clone(), table_type, comment: None, estimated_rows: None, total_bytes: None };
            Ok(TableDetail { table: table_info, columns, indexes, foreign_keys, ddl })
        }).await
    }

    async fn list_routines(&self, database: &str) -> Result<Vec<RoutineInfo>> {
        let database = database.to_owned();
        self.with_client(async move |client| {
            let rows = client.query("SELECT routine_name, routine_type, data_type, routine_definition FROM information_schema.routines WHERE routine_schema=$1 ORDER BY routine_name", &[&database]).await.map_err(query_error)?;
            Ok(rows.into_iter().map(|row| RoutineInfo { database: database.clone(), name: row.get(0), routine_type: row.get(1), data_type: row.get(2), comment: None }).collect())
        }).await
    }

    async fn list_triggers(&self, database: &str) -> Result<Vec<TriggerInfo>> {
        let database = database.to_owned();
        self.with_client(async move |client| {
            let rows = client.query("SELECT trigger_name, event_object_table, action_timing, event_manipulation FROM information_schema.triggers WHERE trigger_schema=$1 ORDER BY trigger_name", &[&database]).await.map_err(query_error)?;
            Ok(rows.into_iter().map(|row| TriggerInfo { database: database.clone(), name: row.get(0), table_name: row.get(1), timing: row.get(2), event: row.get(3) }).collect())
        }).await
    }

    async fn list_events(&self, _database: &str) -> Result<Vec<EventInfo>> {
        Ok(Vec::new())
    }

    async fn object_definition(
        &self,
        database: &str,
        kind: DatabaseObjectKind,
        name: &str,
    ) -> Result<DatabaseObjectDefinition> {
        let database = database.to_owned();
        let name = name.to_owned();
        self.with_client(async move |client| {
            let ddl = match kind {
                DatabaseObjectKind::View => client.query_opt("SELECT 'CREATE OR REPLACE VIEW ' || quote_ident($1) || '.' || quote_ident($2) || ' AS ' || pg_get_viewdef(c.oid, true) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2 AND c.relkind IN ('v','m')", &[&database, &name]).await.map_err(query_error)?,
                DatabaseObjectKind::Procedure | DatabaseObjectKind::Function => client.query_opt("SELECT pg_get_functiondef(p.oid) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname=$1 AND p.proname=$2 ORDER BY p.oid LIMIT 1", &[&database, &name]).await.map_err(query_error)?,
                DatabaseObjectKind::Trigger => client.query_opt("SELECT pg_get_triggerdef(t.oid, true) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND t.tgname=$2 AND NOT t.tgisinternal LIMIT 1", &[&database, &name]).await.map_err(query_error)?,
                DatabaseObjectKind::Event => return Err(CockpitError::Unsupported("PostgreSQL 没有 MySQL 事件对象".into())),
            };
            let ddl = ddl.map(|row| row.get(0)).ok_or_else(|| CockpitError::NotFound(format!("对象不存在：{name}")))?;
            Ok(DatabaseObjectDefinition { database, name, kind, ddl })
        }).await
    }

    async fn routine_parameters(
        &self,
        database: &str,
        name: &str,
    ) -> Result<Vec<RoutineParameter>> {
        let database = database.to_owned();
        let name = name.to_owned();
        self.with_client(async move |client| {
            let rows = client.query("SELECT parameter_name, parameter_mode, data_type, ordinal_position FROM information_schema.parameters WHERE specific_schema=$1 AND specific_name LIKE $2 || '%' ORDER BY ordinal_position", &[&database, &name]).await.map_err(query_error)?;
            Ok(rows.into_iter().map(|row| RoutineParameter { name: row.get(0), mode: row.get(1), data_type: row.get(2), ordinal: row.get::<_, i32>(3).max(0) as u32 }).collect())
        }).await
    }

    async fn list_processes(&self) -> Result<Vec<ServerProcessInfo>> {
        self.with_client(async |client| {
            let rows = client.query("SELECT pid, usename, COALESCE(client_addr::text, 'local'), datname, state, EXTRACT(EPOCH FROM (clock_timestamp()-COALESCE(query_start, backend_start)))::bigint, wait_event_type || COALESCE(':' || wait_event, ''), query FROM pg_stat_activity ORDER BY pid", &[]).await.map_err(query_error)?;
            Ok(rows.into_iter().map(|row| ServerProcessInfo {
                id: row.get::<_, i32>(0).max(0) as u64,
                user: row.get::<_, Option<String>>(1).unwrap_or_default(),
                host: row.get(2), database: row.get(3), command: row.get::<_, Option<String>>(4).unwrap_or_default(),
                time_secs: row.get::<_, i64>(5).max(0) as u64, state: row.get(6), sql: row.get(7),
            }).collect())
        }).await
    }

    /// 先校验 PID 范围再下发终止命令，避免 `as i32` 回绕误伤其他会话。
    async fn kill_process(&self, process_id: u64) -> Result<()> {
        let process_id = validate_process_pid(process_id)?;
        self.with_client(async move |client| {
            let stopped: bool = client
                .query_one("SELECT pg_terminate_backend($1)", &[&process_id])
                .await
                .map_err(query_error)?
                .get(0);
            if stopped {
                Ok(())
            } else {
                Err(CockpitError::Query("服务器未终止该会话".into()))
            }
        })
        .await
    }

    async fn server_status(&self) -> Result<Vec<ServerMetric>> {
        self.with_client(async |client| {
            let row = client.query_one("SELECT numbackends::bigint, xact_commit, xact_rollback, blks_read, blks_hit, tup_returned, tup_fetched, tup_inserted, tup_updated, tup_deleted, conflicts, deadlocks FROM pg_stat_database WHERE datname=current_database()", &[]).await.map_err(query_error)?;
            let names = ["numbackends", "xact_commit", "xact_rollback", "blks_read", "blks_hit", "tup_returned", "tup_fetched", "tup_inserted", "tup_updated", "tup_deleted", "conflicts", "deadlocks"];
            Ok(names.into_iter().enumerate().map(|(index, name)| ServerMetric { name: name.into(), value: row.get::<_, i64>(index).to_string() }).collect())
        }).await
    }

    async fn server_variables(&self, filter: Option<&str>) -> Result<Vec<ServerVariable>> {
        let filter = filter.unwrap_or_default().to_owned();
        self.with_client(async move |client| {
            let rows = client.query("SELECT name, setting, context NOT IN ('internal','postmaster') FROM pg_settings WHERE ($1='' OR name ILIKE '%' || $1 || '%') ORDER BY name LIMIT 1000", &[&filter]).await.map_err(query_error)?;
            Ok(rows.into_iter().map(|row| ServerVariable { name: row.get(0), value: row.get(1), dynamic: row.get(2) }).collect())
        }).await
    }

    async fn server_locks(&self) -> Result<Vec<ServerLockInfo>> {
        self.with_client(async |client| {
            let rows = client.query("SELECT a.pid, blocker, COALESCE(a.wait_event_type, 'Lock'), COALESCE(a.wait_event, 'unknown'), COALESCE(a.state, 'waiting'), a.query FROM pg_stat_activity a CROSS JOIN LATERAL unnest(pg_blocking_pids(a.pid)) blocker ORDER BY a.pid", &[]).await.map_err(query_error)?;
            Ok(rows.into_iter().map(|row| ServerLockInfo {
                waiting_thread_id: row.get::<_, i32>(0).max(0) as u64,
                blocking_thread_id: Some(row.get::<_, i32>(1).max(0) as u64),
                object_name: None, lock_type: row.get(2), lock_mode: row.get(3), lock_status: row.get(4), waiting_sql: row.get(5),
            }).collect())
        }).await
    }

    async fn list_users(&self) -> Result<Vec<UserAccount>> {
        self.with_client(async |client| {
            let rows = client
                .query(
                    "SELECT rolname, rolcanlogin, rolvaliduntil FROM pg_roles ORDER BY rolname",
                    &[],
                )
                .await
                .map_err(query_error)?;
            Ok(rows
                .into_iter()
                .map(|row| UserAccount {
                    user: row.get(0),
                    host: "*".into(),
                    plugin: Some("PostgreSQL role".into()),
                    locked: !row.get::<_, bool>(1),
                })
                .collect())
        })
        .await
    }

    async fn user_grants(&self, user: &str, _host: &str) -> Result<Vec<String>> {
        let user = user.to_owned();
        self.with_client(async move |client| {
            let rows = client.query("SELECT 'ROLE ' || quote_ident(r.rolname) FROM pg_auth_members m JOIN pg_roles r ON r.oid=m.roleid JOIN pg_roles u ON u.oid=m.member WHERE u.rolname=$1 UNION ALL SELECT privilege_type || ' ON ' || quote_ident(table_schema) || '.' || quote_ident(table_name) FROM information_schema.role_table_grants WHERE grantee=$1 ORDER BY 1", &[&user]).await.map_err(query_error)?;
            Ok(rows.into_iter().map(|row| row.get(0)).collect())
        }).await
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
        let timeout = Duration::from_secs(
            request
                .timeout_secs
                .unwrap_or(self.profile.query_timeout_secs)
                .max(1),
        );
        let mut future = Box::pin(self.execute_inner(request));
        match tokio::time::timeout(timeout, &mut future).await {
            Ok(result) => result,
            Err(_) => {
                let cleanup_timeout = Duration::from_secs(self.profile.connect_timeout_secs.max(1));
                // 1) 通知服务端取消仍在运行的查询，而不是只丢弃客户端 future：
                //    否则服务端会继续执行（如长 SELECT / UPDATE），把会话卡住。
                let _ =
                    tokio::time::timeout(cleanup_timeout, self.cancel_token.cancel_query(NoTls))
                        .await;
                // 2) 等待连接把被取消查询的剩余响应排空（回到 ReadyForQuery），恢复可复用状态。
                let drained = tokio::time::timeout(cleanup_timeout, &mut future)
                    .await
                    .is_ok();
                drop(future);
                if !drained {
                    // 3) 取消后连接未能恢复：断开连接，避免后续查询复用一个坏会话。
                    let mut guard = self.client.lock().await;
                    let _ = guard.take();
                    *self.transaction_active.write().await = false;
                    self.search_path.lock().await.take();
                }
                Err(CockpitError::Timeout)
            }
        }
    }

    async fn mutate_row(&self, request: RowMutationRequest) -> Result<RowMutationResult> {
        ensure_writable(self.profile.read_only)?;
        let target = format!(
            "{}.{}",
            quote_identifier(&request.database),
            quote_identifier(&request.table)
        );
        let sql = match request.kind {
            RowMutationKind::Insert => {
                if request.values.is_empty() {
                    format!("INSERT INTO {target} DEFAULT VALUES")
                } else {
                    format!(
                        "INSERT INTO {target} ({}) VALUES ({})",
                        request
                            .values
                            .iter()
                            .map(|(name, _)| quote_identifier(name))
                            .collect::<Vec<_>>()
                            .join(", "),
                        request
                            .values
                            .iter()
                            .map(|(_, value)| cell_literal(value))
                            .collect::<Result<Vec<_>>>()?
                            .join(", ")
                    )
                }
            }
            RowMutationKind::Update => {
                if request.values.is_empty() {
                    return Err(CockpitError::Query("没有需要更新的字段".into()));
                }
                let conditions = mutation_conditions(&request)?;
                format!(
                    "UPDATE {target} SET {} WHERE {conditions}",
                    request
                        .values
                        .iter()
                        .map(|(name, value)| Ok(format!(
                            "{} = {}",
                            quote_identifier(name),
                            cell_literal(value)?
                        )))
                        .collect::<Result<Vec<_>>>()?
                        .join(", ")
                )
            }
            RowMutationKind::Delete => format!(
                "DELETE FROM {target} WHERE {}",
                mutation_conditions(&request)?
            ),
        };
        let affected = self
            .with_client(async move |client| client.execute(&sql, &[]).await.map_err(query_error))
            .await?;
        Ok(RowMutationResult {
            affected_rows: affected,
            concurrent_change: request.kind != RowMutationKind::Insert && affected == 0,
        })
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
        ensure_writable(self.profile.read_only)?;
        if columns.is_empty() || rows.is_empty() {
            return Ok(0);
        }
        let values = rows
            .iter()
            .map(|row| {
                if row.len() != columns.len() {
                    return Err(CockpitError::Exchange(
                        "导入行的字段数与映射列不一致".into(),
                    ));
                }
                Ok(format!(
                    "({})",
                    row.iter()
                        .map(cell_literal)
                        .collect::<Result<Vec<_>>>()?
                        .join(", ")
                ))
            })
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        let target = format!("{}.{}", quote_identifier(database), quote_identifier(table));
        let conflict = match policy {
            ImportConflictPolicy::Error => String::new(),
            ImportConflictPolicy::Ignore => " ON CONFLICT DO NOTHING".into(),
            ImportConflictPolicy::Replace | ImportConflictPolicy::Upsert => format!(
                " ON CONFLICT DO UPDATE SET {}",
                columns
                    .iter()
                    .map(|name| format!(
                        "{} = EXCLUDED.{}",
                        quote_identifier(name),
                        quote_identifier(name)
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let sql = format!(
            "INSERT INTO {target} ({}) VALUES {values}{conflict}",
            columns
                .iter()
                .map(|name| quote_identifier(name))
                .collect::<Vec<_>>()
                .join(", ")
        );
        self.with_client(async move |client| client.execute(&sql, &[]).await.map_err(query_error))
            .await
    }

    async fn begin_transaction(&self) -> Result<()> {
        ensure_writable(self.profile.read_only)?;
        self.with_client(async |client| client.batch_execute("BEGIN").await.map_err(query_error))
            .await?;
        *self.transaction_active.write().await = true;
        Ok(())
    }

    async fn begin_read_transaction(&self) -> Result<()> {
        self.with_client(async |client| {
            client
                .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .await
                .map_err(query_error)
        })
        .await?;
        *self.transaction_active.write().await = true;
        Ok(())
    }

    async fn commit_transaction(&self) -> Result<()> {
        self.with_client(async |client| client.batch_execute("COMMIT").await.map_err(query_error))
            .await?;
        *self.transaction_active.write().await = false;
        Ok(())
    }

    async fn rollback_transaction(&self) -> Result<()> {
        self.with_client(async |client| {
            client.batch_execute("ROLLBACK").await.map_err(query_error)
        })
        .await?;
        *self.transaction_active.write().await = false;
        // ROLLBACK 会还原事务内 SET search_path 的副作用，缓存可能已失效。
        self.search_path.lock().await.take();
        Ok(())
    }

    async fn transaction_active(&self) -> bool {
        *self.transaction_active.read().await
    }

    async fn cancel(&self, _execution_id: Uuid) -> Result<bool> {
        self.cancel_token
            .cancel_query(NoTls)
            .await
            .map_err(query_error)?;
        Ok(true)
    }

    async fn close(&self) -> Result<()> {
        self.client.lock().await.take();
        *self.transaction_active.write().await = false;
        self.search_path.lock().await.take();
        Ok(())
    }
}

async fn load_columns(client: &Client, database: &str, table: &str) -> Result<Vec<ColumnInfo>> {
    let rows = client.query(
        "SELECT c.column_name, c.ordinal_position, c.data_type, COALESCE((SELECT format_type(a.atttypid, a.atttypmod) FROM pg_attribute a JOIN pg_class pc ON pc.oid=a.attrelid JOIN pg_namespace pn ON pn.oid=pc.relnamespace WHERE pn.nspname=c.table_schema AND pc.relname=c.table_name AND a.attname=c.column_name AND a.attnum>0), c.udt_name, c.data_type), c.is_nullable='YES', c.column_default, c.identity_generation, c.generation_expression, c.collation_name, CASE WHEN EXISTS (SELECT 1 FROM information_schema.table_constraints tc JOIN information_schema.key_column_usage kcu ON tc.constraint_schema=kcu.constraint_schema AND tc.constraint_name=kcu.constraint_name WHERE tc.table_schema=c.table_schema AND tc.table_name=c.table_name AND kcu.column_name=c.column_name AND tc.constraint_type='PRIMARY KEY') THEN 'PRI' WHEN EXISTS (SELECT 1 FROM information_schema.table_constraints tc JOIN information_schema.key_column_usage kcu ON tc.constraint_schema=kcu.constraint_schema AND tc.constraint_name=kcu.constraint_name WHERE tc.table_schema=c.table_schema AND tc.table_name=c.table_name AND kcu.column_name=c.column_name AND tc.constraint_type='UNIQUE') THEN 'UNI' END FROM information_schema.columns c WHERE c.table_schema=$1 AND c.table_name=$2 ORDER BY c.ordinal_position",
        &[&database, &table],
    ).await.map_err(query_error)?;
    Ok(rows
        .into_iter()
        .map(|row| ColumnInfo {
            name: row.get(0),
            ordinal: row.get::<_, i32>(1).max(0) as u32,
            data_type: row.get(2),
            full_type: row.get(3),
            nullable: row.get(4),
            default_value: row.get(5),
            extra: row.get(6),
            comment: None,
            key: row.get(9),
            generation_expression: row.get(7),
            collation: row.get(8),
        })
        .collect())
}

async fn load_foreign_keys(
    client: &Client,
    database: &str,
    table: &str,
) -> Result<Vec<ForeignKeyInfo>> {
    let rows = client.query(
        "SELECT tc.constraint_name, kcu.column_name, referenced.table_schema, referenced.table_name, referenced.column_name, rc.update_rule, rc.delete_rule, kcu.ordinal_position FROM information_schema.table_constraints tc JOIN information_schema.key_column_usage kcu ON tc.constraint_name=kcu.constraint_name AND tc.constraint_schema=kcu.constraint_schema JOIN information_schema.referential_constraints rc ON rc.constraint_name=tc.constraint_name AND rc.constraint_schema=tc.constraint_schema JOIN information_schema.key_column_usage referenced ON referenced.constraint_name=rc.unique_constraint_name AND referenced.constraint_schema=rc.unique_constraint_schema AND referenced.ordinal_position=kcu.position_in_unique_constraint WHERE tc.constraint_type='FOREIGN KEY' AND tc.table_schema=$1 AND tc.table_name=$2 ORDER BY tc.constraint_name, kcu.ordinal_position",
        &[&database, &table],
    ).await.map_err(query_error)?;
    let mut grouped: BTreeMap<String, ForeignKeyInfo> = BTreeMap::new();
    for row in rows {
        let name: String = row.get(0);
        let entry = grouped
            .entry(name.clone())
            .or_insert_with(|| ForeignKeyInfo {
                name,
                columns: Vec::new(),
                referenced_database: row.get(2),
                referenced_table: row.get(3),
                referenced_columns: Vec::new(),
                on_update: row.get(5),
                on_delete: row.get(6),
            });
        entry.columns.push(row.get(1));
        entry.referenced_columns.push(row.get(4));
    }
    Ok(grouped.into_values().collect())
}

fn parse_index_columns(definition: &str) -> Vec<String> {
    definition
        .rsplit_once('(')
        .and_then(|(_, tail)| tail.split_once(')'))
        .map(|(columns, _)| {
            columns
                .split(',')
                .map(|name| name.trim().trim_matches('"').to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn column_ddl(column: &ColumnInfo) -> String {
    let mut ddl = format!("{} {}", quote_identifier(&column.name), column.full_type);
    if !column.nullable {
        ddl.push_str(" NOT NULL");
    }
    if let Some(default) = &column.default_value {
        ddl.push_str(" DEFAULT ");
        ddl.push_str(default);
    }
    ddl
}

fn cell_literal(value: &CellValue) -> Result<String> {
    Ok(match value {
        CellValue::Null => "NULL".into(),
        CellValue::Bool(value) => {
            if *value {
                "TRUE".into()
            } else {
                "FALSE".into()
            }
        }
        CellValue::Signed(value) | CellValue::Unsigned(value) | CellValue::Decimal(value) => {
            quote_text(value)
        }
        CellValue::Float(value) => {
            if value.is_finite() {
                value.to_string()
            } else {
                quote_text(&value.to_string())
            }
        }
        CellValue::Text(value)
        | CellValue::Date(value)
        | CellValue::Time(value)
        | CellValue::DateTime(value)
        | CellValue::Json(value) => quote_text(value),
        CellValue::Bytes { base64, .. } => format!("decode({}, 'base64')", quote_text(base64)),
        CellValue::Geometry { wkb_base64, .. } => {
            format!("decode({}, 'base64')", quote_text(wkb_base64))
        }
    })
}

fn mutation_conditions(request: &RowMutationRequest) -> Result<String> {
    if request.key_values.is_empty() {
        return Err(CockpitError::Query("更新或删除需要主键/唯一键".into()));
    }
    request
        .key_values
        .iter()
        .chain(request.original_values.iter())
        .map(|(name, value)| {
            Ok(if matches!(value, CellValue::Null) {
                format!("{} IS NULL", quote_identifier(name))
            } else {
                format!("{} = {}", quote_identifier(name), cell_literal(value)?)
            })
        })
        .collect::<Result<Vec<_>>>()
        .map(|values| values.join(" AND "))
}

/// 流式消费结果行时只保留一页数据：跳过 `row_offset` 行、最多收集 `page_size` 行，
/// 之后的任意一行都标记 `has_more`（仍需继续消费，让连接回到可复用状态）。
struct PageCollector {
    rows: Vec<Vec<CellValue>>,
    skipped: usize,
    has_more: bool,
    row_offset: usize,
    page_size: usize,
}

impl PageCollector {
    fn new(row_offset: usize, page_size: usize) -> Self {
        Self {
            rows: Vec::with_capacity(page_size.min(1024)),
            skipped: 0,
            has_more: false,
            row_offset,
            page_size,
        }
    }

    fn push(&mut self, row: Vec<CellValue>) {
        if self.skipped < self.row_offset {
            self.skipped += 1;
        } else if self.rows.len() < self.page_size {
            self.rows.push(row);
        } else {
            self.has_more = true;
        }
    }
}

/// 把 `simple_query_raw` 的流式消息转换为分页结果：边消费边丢弃跳过/超出页面的行，
/// 避免把整个结果集物化到内存；每条语句（结果集）独立应用 row_offset/page_size 分页。
async fn simple_stream_to_page(
    request: &ExecuteQueryRequest,
    stream: SimpleQueryStream,
    started: Instant,
) -> Result<QueryResultPage> {
    let page_size = request.page_size.max(1);
    let row_offset = request.row_offset;
    let mut sets: Vec<QueryResultSet> = Vec::new();
    let mut columns: Vec<ColumnMeta> = Vec::new();
    let mut affected = 0u64;
    let mut page = PageCollector::new(row_offset, page_size);
    let mut result_set_index = 0usize;
    let push_set = |sets: &mut Vec<QueryResultSet>,
                    columns: &mut Vec<ColumnMeta>,
                    page: &mut PageCollector,
                    affected: &mut u64,
                    result_set_index: &mut usize| {
        if !(columns.is_empty() && page.rows.is_empty() && *affected == 0) {
            sets.push(QueryResultSet {
                columns: std::mem::take(columns),
                rows: std::mem::take(&mut page.rows),
                affected_rows: *affected,
                truncated: page.has_more,
                has_more: page.has_more,
                result_set_index: *result_set_index,
                row_offset,
                page_size,
            });
            *result_set_index += 1;
        }
        *affected = 0;
        *page = PageCollector::new(row_offset, page_size);
    };
    let mut stream = Box::pin(stream);
    while let Some(message) = stream.next().await {
        match message.map_err(query_error)? {
            SimpleQueryMessage::RowDescription(description) => {
                push_set(
                    &mut sets,
                    &mut columns,
                    &mut page,
                    &mut affected,
                    &mut result_set_index,
                );
                columns = description
                    .iter()
                    .map(|column| ColumnMeta {
                        name: column.name().into(),
                        database_type: "TEXT".into(),
                        nullable: true,
                        unsigned: false,
                        binary: false,
                    })
                    .collect();
            }
            SimpleQueryMessage::Row(row) => {
                if columns.is_empty() {
                    columns = row
                        .columns()
                        .iter()
                        .map(|column| ColumnMeta {
                            name: column.name().into(),
                            database_type: "TEXT".into(),
                            nullable: true,
                            unsigned: false,
                            binary: false,
                        })
                        .collect();
                }
                page.push(
                    (0..row.len())
                        .map(|index| {
                            row.get(index)
                                .map(|value| CellValue::Text(value.into()))
                                .unwrap_or(CellValue::Null)
                        })
                        .collect(),
                );
            }
            SimpleQueryMessage::CommandComplete(count) => {
                // 只有不带结果行的语句（纯 DML）时 count 才是受影响行数；
                // 带结果行的语句（SELECT、INSERT ... RETURNING 等）count 是返回行数，
                // 不应当作 affected_rows 上报（与 MySQL 驱动语义一致）。
                if page.rows.is_empty() && columns.is_empty() {
                    affected = count;
                }
                push_set(
                    &mut sets,
                    &mut columns,
                    &mut page,
                    &mut affected,
                    &mut result_set_index,
                );
            }
            _ => {}
        }
    }
    push_set(
        &mut sets,
        &mut columns,
        &mut page,
        &mut affected,
        &mut result_set_index,
    );
    let first = if sets.is_empty() {
        QueryResultSet {
            columns: Vec::new(),
            rows: Vec::new(),
            affected_rows: 0,
            truncated: false,
            has_more: false,
            result_set_index: 0,
            row_offset,
            page_size,
        }
    } else {
        sets.remove(0)
    };
    Ok(QueryResultPage {
        execution_id: request.execution_id,
        columns: first.columns,
        rows: first.rows,
        affected_rows: first.affected_rows,
        execution_time_ms: started.elapsed().as_millis(),
        truncated: first.truncated,
        has_more: first.has_more,
        result_set_index: 0,
        messages: Vec::new(),
        row_offset: first.row_offset,
        page_size: first.page_size,
        additional_result_sets: sets,
        source_table: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_identifiers_and_cell_values_without_sql_injection() {
        assert_eq!(quote_identifier("odd\"name"), "\"odd\"\"name\"");
        assert_eq!(
            cell_literal(&CellValue::Text("O'Reilly".into())).unwrap(),
            "'O''Reilly'"
        );
        assert_eq!(
            cell_literal(&CellValue::Signed("1; DROP TABLE t".into())).unwrap(),
            "'1; DROP TABLE t'"
        );
    }

    #[test]
    fn parses_regular_index_columns() {
        assert_eq!(
            parse_index_columns(
                "CREATE INDEX idx ON public.items USING btree (id, \"created at\")"
            ),
            vec!["id", "created at"]
        );
    }

    #[test]
    fn read_only_connections_reject_writes() {
        assert!(ensure_writable(false).is_ok());
        assert!(matches!(
            ensure_writable(true),
            Err(CockpitError::Query(message)) if message.contains("只读模式")
        ));
    }

    #[test]
    fn page_collector_skips_offset_and_keeps_only_one_page() {
        let mut page = PageCollector::new(2, 2);
        for value in 0..5 {
            page.push(vec![CellValue::Signed(value.to_string())]);
        }
        assert_eq!(
            page.rows,
            vec![
                vec![CellValue::Signed("2".into())],
                vec![CellValue::Signed("3".into())],
            ]
        );
        assert!(page.has_more);
    }

    #[test]
    fn page_collector_reports_no_more_when_result_is_exhausted() {
        let mut page = PageCollector::new(1, 3);
        for value in 0..4 {
            page.push(vec![CellValue::Signed(value.to_string())]);
        }
        assert_eq!(page.rows.len(), 3);
        assert!(!page.has_more);
    }

    #[test]
    fn page_collector_returns_empty_page_when_offset_past_the_end() {
        let mut page = PageCollector::new(10, 2);
        for value in 0..3 {
            page.push(vec![CellValue::Signed(value.to_string())]);
        }
        assert!(page.rows.is_empty());
        assert!(!page.has_more);
    }

    #[test]
    fn process_pid_must_fit_postgres_int4() {
        assert_eq!(validate_process_pid(0).unwrap(), 0);
        assert_eq!(validate_process_pid(12345).unwrap(), 12345);
        assert_eq!(validate_process_pid(i32::MAX as u64).unwrap(), i32::MAX);
        // 超过 int4 范围的 PID 不能被静默回绕成错误会话。
        assert!(
            matches!(validate_process_pid(i32::MAX as u64 + 1), Err(CockpitError::InvalidConfig(message)) if message.contains("无效的进程 ID"))
        );
        assert!(matches!(
            validate_process_pid(u64::MAX),
            Err(CockpitError::InvalidConfig(_))
        ));
    }
}
