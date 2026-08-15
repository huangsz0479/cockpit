use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

use async_trait::async_trait;
use base64::Engine as _;
use cockpit_core::{
    CellValue, CockpitError, ColumnInfo, ColumnMeta, ConnectionInfo, ConnectionProfile,
    DatabaseDriver, DatabaseInfo, DatabaseObjectDefinition, DatabaseObjectKind, DriverSession,
    EventInfo, ExecuteQueryRequest, ForeignKeyInfo, ImportConflictPolicy, IndexInfo,
    QueryResultPage, Result, RiskLevel, RoutineInfo, RoutineParameter, RowMutationKind,
    RowMutationRequest, RowMutationResult, ServerMetric, ServerProcessInfo, TableDetail, TableInfo,
    TriggerInfo, UserAccount, safety::assess_sql,
};
use rusqlite::{Connection, OptionalExtension, params_from_iter, types::Value as SqliteValue};
use uuid::Uuid;

#[derive(Default)]
pub struct SqliteDriver;

pub struct SqliteSession {
    profile: ConnectionProfile,
    connection: Arc<Mutex<Connection>>,
    interrupt_handle: rusqlite::InterruptHandle,
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn sqlite_error(error: rusqlite::Error) -> CockpitError {
    CockpitError::Query(error.to_string())
}

fn blocking_error(error: tokio::task::JoinError) -> CockpitError {
    CockpitError::Query(format!("数据库操作线程异常：{error}"))
}

fn lock_connection(connection: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    connection
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn open_connection(profile: &ConnectionProfile) -> Result<Connection> {
    let path = profile.host.trim();
    if path.is_empty() {
        return Err(CockpitError::InvalidConfig(
            "SQLite 文件路径不能为空".into(),
        ));
    }
    if path != ":memory:" {
        let parent = Path::new(path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
        if let Some(parent) = parent {
            std::fs::create_dir_all(parent)
                .map_err(|error| CockpitError::Connection(error.to_string()))?;
        }
    }
    let connection = Connection::open(path).map_err(sqlite_error)?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(sqlite_error)?;
    connection
        .busy_timeout(std::time::Duration::from_secs(
            profile.query_timeout_secs.max(1),
        ))
        .map_err(sqlite_error)?;
    Ok(connection)
}

#[async_trait]
impl DatabaseDriver for SqliteDriver {
    fn kind(&self) -> &'static str {
        "sqlite"
    }

    async fn test(&self, profile: &ConnectionProfile, _password: &str) -> Result<ConnectionInfo> {
        let profile = profile.clone();
        tokio::task::spawn_blocking(move || {
            let connection = open_connection(&profile)?;
            let version: String = connection
                .query_row("SELECT sqlite_version()", [], |row| row.get(0))
                .map_err(sqlite_error)?;
            Ok(ConnectionInfo {
                server_version: version,
                server_comment: Some("SQLite embedded database".into()),
                connection_id: 0,
                current_database: Some("main".into()),
                tls_cipher: None,
            })
        })
        .await
        .map_err(blocking_error)?
    }

    async fn open(
        &self,
        profile: ConnectionProfile,
        _password: String,
    ) -> Result<Arc<dyn DriverSession>> {
        let open_profile = profile.clone();
        let connection = tokio::task::spawn_blocking(move || open_connection(&open_profile))
            .await
            .map_err(blocking_error)??;
        let interrupt_handle = connection.get_interrupt_handle();
        Ok(Arc::new(SqliteSession {
            profile,
            connection: Arc::new(Mutex::new(connection)),
            interrupt_handle,
        }))
    }
}

#[async_trait]
impl DriverSession for SqliteSession {
    fn connection_id(&self) -> Uuid {
        self.profile.id
    }

    async fn connection_info(&self) -> Result<ConnectionInfo> {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            let version: String = connection
                .query_row("SELECT sqlite_version()", [], |row| row.get(0))
                .map_err(sqlite_error)?;
            Ok(ConnectionInfo {
                server_version: version,
                server_comment: Some("SQLite embedded database".into()),
                connection_id: 0,
                current_database: Some("main".into()),
                tls_cipher: None,
            })
        })
        .await
        .map_err(blocking_error)?
    }

    async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            let mut statement = connection
                .prepare("PRAGMA database_list")
                .map_err(sqlite_error)?;
            statement
                .query_map([], |row| Ok(DatabaseInfo { name: row.get(1)? }))
                .map_err(sqlite_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite_error)
        })
        .await
        .map_err(blocking_error)?
    }

    async fn list_tables(
        &self,
        database: &str,
        filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TableInfo>> {
        let database = database.to_string();
        let filter = filter.map(str::to_string);
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            let sql = format!(
                "SELECT name, type, COALESCE(sql, '') FROM {}.sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' AND (?1 = '' OR lower(name) LIKE '%' || lower(?1) || '%') ORDER BY name LIMIT ?2 OFFSET ?3",
                quote_identifier(&database),
            );
            let mut statement = connection.prepare(&sql).map_err(sqlite_error)?;
            statement
                .query_map(
                    (
                        filter.as_deref().unwrap_or_default(),
                        limit as i64,
                        offset as i64,
                    ),
                    |row| {
                        let kind: String = row.get(1)?;
                        Ok(TableInfo {
                            database: database.clone(),
                            name: row.get(0)?,
                            table_type: if kind == "view" {
                                "VIEW".into()
                            } else {
                                "BASE TABLE".into()
                            },
                            comment: None,
                            estimated_rows: None,
                            total_bytes: None,
                        })
                    },
                )
                .map_err(sqlite_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite_error)
        })
        .await
        .map_err(blocking_error)?
    }

    async fn list_columns(&self, database: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let database = database.to_string();
        let table = table.to_string();
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            list_columns_sync(&connection, &database, &table)
        })
        .await
        .map_err(blocking_error)?
    }

    async fn table_detail(&self, database: &str, table: &str) -> Result<TableDetail> {
        let database = database.to_string();
        let table = table.to_string();
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            let columns = list_columns_sync(&connection, &database, &table)?;
            let mut ddl: String = connection
                .query_row(
                    &format!(
                        "SELECT sql FROM {}.sqlite_master WHERE name = ?1",
                        quote_identifier(&database)
                    ),
                    [table.as_str()],
                    |row| row.get(0),
                )
                .map_err(sqlite_error)?;
            let table_type: String = connection
                .query_row(
                    &format!(
                        "SELECT type FROM {}.sqlite_master WHERE name = ?1",
                        quote_identifier(&database)
                    ),
                    [table.as_str()],
                    |row| row.get(0),
                )
                .map_err(sqlite_error)?;
            let mut indexes = Vec::new();
            let mut index_statement = connection
                .prepare(&format!(
                    "PRAGMA {}.index_list({})",
                    quote_identifier(&database),
                    quote_identifier(&table)
                ))
                .map_err(sqlite_error)?;
            let index_rows = index_statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)? != 0,
                        row.get::<_, String>(3).unwrap_or_default(),
                    ))
                })
                .map_err(sqlite_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite_error)?;
            for (name, unique, origin) in index_rows {
                let mut column_statement = connection
                    .prepare(&format!(
                        "PRAGMA {}.index_info({})",
                        quote_identifier(&database),
                        quote_identifier(&name)
                    ))
                    .map_err(sqlite_error)?;
                let index_columns = column_statement
                    .query_map([], |row| row.get::<_, String>(2))
                    .map_err(sqlite_error)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(sqlite_error)?;
                indexes.push(IndexInfo {
                    name,
                    columns: index_columns,
                    unique,
                    primary: origin == "pk",
                    index_type: Some("BTREE".into()),
                });
            }
            let mut ddl_statement = connection
                .prepare(&format!(
                    "SELECT sql FROM {}.sqlite_master WHERE type='index' AND tbl_name=?1 AND sql IS NOT NULL ORDER BY name",
                    quote_identifier(&database)
                ))
                .map_err(sqlite_error)?;
            let index_ddls = ddl_statement
                .query_map([table.as_str()], |row| row.get::<_, String>(0))
                .map_err(sqlite_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite_error)?;
            for index_ddl in index_ddls {
                ddl.push_str(";\n");
                ddl.push_str(index_ddl.trim_end_matches(';'));
            }
            let mut foreign_keys = Vec::new();
            let mut fk_statement = connection
                .prepare(&format!(
                    "PRAGMA {}.foreign_key_list({})",
                    quote_identifier(&database),
                    quote_identifier(&table)
                ))
                .map_err(sqlite_error)?;
            let rows = fk_statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })
                .map_err(sqlite_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite_error)?;
            for (id, referenced_table, column, referenced_column, on_update, on_delete) in rows {
                foreign_keys.push(ForeignKeyInfo {
                    name: format!("fk_{table}_{id}"),
                    columns: vec![column],
                    referenced_database: database.clone(),
                    referenced_table,
                    referenced_columns: vec![referenced_column],
                    on_update: Some(on_update),
                    on_delete: Some(on_delete),
                });
            }
            Ok(TableDetail {
                table: TableInfo {
                    database: database.clone(),
                    name: table.clone(),
                    table_type: if table_type == "view" {
                        "VIEW".into()
                    } else {
                        "BASE TABLE".into()
                    },
                    comment: None,
                    estimated_rows: None,
                    total_bytes: None,
                },
                columns,
                indexes,
                foreign_keys,
                ddl,
            })
        })
        .await
        .map_err(blocking_error)?
    }

    async fn list_routines(&self, _database: &str) -> Result<Vec<RoutineInfo>> {
        Ok(Vec::new())
    }

    async fn list_triggers(&self, database: &str) -> Result<Vec<TriggerInfo>> {
        let database = database.to_string();
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            let sql = format!(
                "SELECT name, tbl_name, sql FROM {}.sqlite_master WHERE type='trigger' ORDER BY name",
                quote_identifier(&database)
            );
            let mut statement = connection.prepare(&sql).map_err(sqlite_error)?;
            statement
                .query_map([], |row| {
                    Ok(TriggerInfo {
                        database: database.clone(),
                        name: row.get(0)?,
                        table_name: row.get(1)?,
                        timing: "".into(),
                        event: "".into(),
                    })
                })
                .map_err(sqlite_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite_error)
        })
        .await
        .map_err(blocking_error)?
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
        let object_type = match kind {
            DatabaseObjectKind::View => "view",
            DatabaseObjectKind::Trigger => "trigger",
            _ => {
                return Err(CockpitError::Unsupported(
                    "SQLite 仅支持视图和触发器定义".into(),
                ));
            }
        };
        let database = database.to_string();
        let name = name.to_string();
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            let ddl = connection
                .query_row(
                    &format!(
                        "SELECT sql FROM {}.sqlite_master WHERE type=?1 AND name=?2",
                        quote_identifier(&database)
                    ),
                    (object_type, name.as_str()),
                    |row| row.get(0),
                )
                .optional()
                .map_err(sqlite_error)?
                .ok_or_else(|| CockpitError::NotFound(format!("对象不存在：{name}")))?;
            Ok(DatabaseObjectDefinition {
                database,
                name,
                kind,
                ddl,
            })
        })
        .await
        .map_err(blocking_error)?
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
        Err(CockpitError::Unsupported("SQLite 没有服务器会话".into()))
    }
    async fn server_status(&self) -> Result<Vec<ServerMetric>> {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            let page_count: i64 = connection
                .query_row("PRAGMA page_count", [], |row| row.get(0))
                .map_err(sqlite_error)?;
            let page_size: i64 = connection
                .query_row("PRAGMA page_size", [], |row| row.get(0))
                .map_err(sqlite_error)?;
            let journal_mode: String = connection
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .map_err(sqlite_error)?;
            Ok(vec![
                ServerMetric {
                    name: "page_count".into(),
                    value: page_count.to_string(),
                },
                ServerMetric {
                    name: "page_size".into(),
                    value: page_size.to_string(),
                },
                ServerMetric {
                    name: "database_bytes".into(),
                    value: (page_count * page_size).to_string(),
                },
                ServerMetric {
                    name: "journal_mode".into(),
                    value: journal_mode,
                },
            ])
        })
        .await
        .map_err(blocking_error)?
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
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            execute_sync(&connection, request)
        })
        .await
        .map_err(blocking_error)?
    }

    async fn mutate_row(&self, request: RowMutationRequest) -> Result<RowMutationResult> {
        if self.profile.read_only {
            return Err(CockpitError::Query("该连接处于只读模式".into()));
        }
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            let table = format!(
                "{}.{}",
                quote_identifier(&request.database),
                quote_identifier(&request.table)
            );
            let (sql, values) = match request.kind {
                RowMutationKind::Insert => {
                    let columns = request
                        .values
                        .iter()
                        .map(|(name, _)| quote_identifier(name))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let placeholders = vec!["?"; request.values.len()].join(", ");
                    (
                        format!("INSERT INTO {table} ({columns}) VALUES ({placeholders})"),
                        request
                            .values
                            .iter()
                            .map(|(_, value)| cell_to_sqlite(value))
                            .collect::<Result<Vec<_>>>()?,
                    )
                }
                RowMutationKind::Update => {
                    let set = request
                        .values
                        .iter()
                        .map(|(name, _)| format!("{} = ?", quote_identifier(name)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let (where_sql, mut where_values) = mutation_where(&request)?;
                    let mut values = request
                        .values
                        .iter()
                        .map(|(_, value)| cell_to_sqlite(value))
                        .collect::<Result<Vec<_>>>()?;
                    values.append(&mut where_values);
                    (
                        format!("UPDATE {table} SET {set} WHERE {where_sql}"),
                        values,
                    )
                }
                RowMutationKind::Delete => {
                    let (where_sql, values) = mutation_where(&request)?;
                    (format!("DELETE FROM {table} WHERE {where_sql}"), values)
                }
            };
            let affected = connection
                .execute(&sql, params_from_iter(values))
                .map_err(sqlite_error)?;
            Ok(RowMutationResult {
                affected_rows: affected as u64,
                concurrent_change: request.kind != RowMutationKind::Insert && affected == 0,
            })
        })
        .await
        .map_err(blocking_error)?
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
        let database = database.to_string();
        let table = table.to_string();
        let columns = columns.to_vec();
        let rows = rows.to_vec();
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            let verb = match policy {
                ImportConflictPolicy::Error | ImportConflictPolicy::Upsert => "INSERT",
                ImportConflictPolicy::Ignore => "INSERT OR IGNORE",
                ImportConflictPolicy::Replace => "INSERT OR REPLACE",
            };
            let placeholders = format!("({})", vec!["?"; columns.len()].join(", "));
            let upsert = if policy == ImportConflictPolicy::Upsert {
                format!(
                    " ON CONFLICT DO UPDATE SET {}",
                    columns
                        .iter()
                        .map(|column| format!("{0}=excluded.{0}", quote_identifier(column)))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                String::new()
            };
            let sql = format!(
                "{verb} INTO {}.{} ({}) VALUES {}{upsert}",
                quote_identifier(&database),
                quote_identifier(&table),
                columns
                    .iter()
                    .map(|column| quote_identifier(column))
                    .collect::<Vec<_>>()
                    .join(", "),
                vec![placeholders; rows.len()].join(", ")
            );
            let values = rows
                .iter()
                .flatten()
                .map(cell_to_sqlite)
                .collect::<Result<Vec<_>>>()?;
            let affected = connection
                .execute(&sql, params_from_iter(values))
                .map_err(sqlite_error)?;
            Ok(affected as u64)
        })
        .await
        .map_err(blocking_error)?
    }

    async fn begin_transaction(&self) -> Result<()> {
        if self.profile.read_only {
            return Err(CockpitError::Query("只读连接不能开启写事务".into()));
        }
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            if !connection.is_autocommit() {
                return Err(CockpitError::Query("当前连接已有活动事务".into()));
            }
            connection
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(sqlite_error)
        })
        .await
        .map_err(blocking_error)?
    }
    async fn commit_transaction(&self) -> Result<()> {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            if connection.is_autocommit() {
                return Err(CockpitError::Query("当前连接没有活动事务".into()));
            }
            connection.execute_batch("COMMIT").map_err(sqlite_error)
        })
        .await
        .map_err(blocking_error)?
    }
    async fn rollback_transaction(&self) -> Result<()> {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            if connection.is_autocommit() {
                return Ok(());
            }
            connection.execute_batch("ROLLBACK").map_err(sqlite_error)
        })
        .await
        .map_err(blocking_error)?
    }
    async fn transaction_active(&self) -> bool {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let connection = lock_connection(&connection);
            !connection.is_autocommit()
        })
        .await
        .unwrap_or(false)
    }
    async fn cancel(&self, _execution_id: Uuid) -> Result<bool> {
        self.interrupt_handle.interrupt();
        Ok(true)
    }
    async fn close(&self) -> Result<()> {
        Ok(())
    }
}

fn list_columns_sync(
    connection: &Connection,
    database: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>> {
    let mut statement = connection
        .prepare(&format!(
            "PRAGMA {}.table_xinfo({})",
            quote_identifier(database),
            quote_identifier(table)
        ))
        .map_err(sqlite_error)?;
    statement
        .query_map([], |row| {
            let primary: i64 = row.get(5)?;
            Ok(ColumnInfo {
                name: row.get(1)?,
                ordinal: row.get::<_, i64>(0)? as u32 + 1,
                data_type: row
                    .get::<_, String>(2)?
                    .split('(')
                    .next()
                    .unwrap_or("TEXT")
                    .to_ascii_lowercase(),
                full_type: row.get(2)?,
                nullable: row.get::<_, i64>(3)? == 0,
                default_value: row.get(4)?,
                extra: None,
                comment: None,
                key: if primary > 0 {
                    Some("PRI".into())
                } else {
                    None
                },
                generation_expression: None,
                collation: None,
            })
        })
        .map_err(sqlite_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sqlite_error)
}

fn sqlite_ref_to_cell(value: rusqlite::types::ValueRef<'_>) -> CellValue {
    match value {
        rusqlite::types::ValueRef::Null => CellValue::Null,
        rusqlite::types::ValueRef::Integer(value) => CellValue::Signed(value.to_string()),
        rusqlite::types::ValueRef::Real(value) => CellValue::Float(value),
        rusqlite::types::ValueRef::Text(value) => {
            CellValue::Text(String::from_utf8_lossy(value).into_owned())
        }
        rusqlite::types::ValueRef::Blob(value) => CellValue::Bytes {
            base64: base64::engine::general_purpose::STANDARD.encode(value),
            preview: None,
            length: value.len(),
        },
    }
}

fn cell_to_sqlite(value: &CellValue) -> Result<SqliteValue> {
    Ok(match value {
        CellValue::Null => SqliteValue::Null,
        CellValue::Bool(value) => SqliteValue::Integer(i64::from(*value)),
        CellValue::Signed(value) => SqliteValue::Integer(
            value
                .parse()
                .map_err(|_| CockpitError::Query("整数超出 SQLite 范围".into()))?,
        ),
        CellValue::Unsigned(value) => SqliteValue::Integer(
            value
                .parse()
                .map_err(|_| CockpitError::Query("无符号整数超出 SQLite 范围".into()))?,
        ),
        CellValue::Float(value) => SqliteValue::Real(*value),
        CellValue::Decimal(value)
        | CellValue::Text(value)
        | CellValue::Date(value)
        | CellValue::Time(value)
        | CellValue::DateTime(value)
        | CellValue::Json(value) => SqliteValue::Text(value.clone()),
        CellValue::Bytes { base64, .. } => SqliteValue::Blob(
            base64::engine::general_purpose::STANDARD
                .decode(base64)
                .map_err(|error| CockpitError::Query(error.to_string()))?,
        ),
        CellValue::Geometry { wkb_base64, .. } => SqliteValue::Blob(
            base64::engine::general_purpose::STANDARD
                .decode(wkb_base64)
                .map_err(|error| CockpitError::Query(error.to_string()))?,
        ),
    })
}

fn mutation_where(request: &RowMutationRequest) -> Result<(String, Vec<SqliteValue>)> {
    if request.key_values.is_empty() {
        return Err(CockpitError::Query("更新或删除需要主键/唯一键".into()));
    }
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    for (name, value) in request
        .key_values
        .iter()
        .chain(request.original_values.iter())
    {
        if matches!(value, CellValue::Null) {
            clauses.push(format!("{} IS NULL", quote_identifier(name)));
        } else {
            clauses.push(format!("{} = ?", quote_identifier(name)));
            values.push(cell_to_sqlite(value)?);
        }
    }
    Ok((clauses.join(" AND "), values))
}

fn execute_sync(connection: &Connection, request: ExecuteQueryRequest) -> Result<QueryResultPage> {
    let started = Instant::now();
    let page_size = request.page_size.clamp(1, 5_000);
    let row_offset = request.row_offset;
    let mut statement = connection.prepare(&request.sql).map_err(sqlite_error)?;
    let column_count = statement.column_count();
    if column_count == 0 {
        drop(statement);
        let affected_rows = connection
            .execute_batch(&request.sql)
            .map_err(sqlite_error)
            .map(|_| connection.changes())?;
        return Ok(empty_result(
            request.execution_id,
            affected_rows,
            started,
            row_offset,
            page_size,
        ));
    }
    let columns = statement
        .column_names()
        .iter()
        .map(|name| ColumnMeta {
            name: (*name).into(),
            database_type: "DYNAMIC".into(),
            nullable: true,
            unsigned: false,
            binary: false,
        })
        .collect::<Vec<_>>();
    let mut cursor = statement.query([]).map_err(sqlite_error)?;
    let mut rows = Vec::new();
    let mut seen = 0usize;
    let mut has_more = false;
    while let Some(row) = cursor.next().map_err(sqlite_error)? {
        if seen < row_offset {
            seen += 1;
            continue;
        }
        if rows.len() < page_size {
            let values = (0..column_count)
                .map(|index| {
                    row.get_ref(index)
                        .map(sqlite_ref_to_cell)
                        .map_err(sqlite_error)
                })
                .collect::<Result<Vec<_>>>()?;
            rows.push(values);
        } else {
            has_more = true;
        }
        seen += 1;
    }
    drop(cursor);
    drop(statement);
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
        row_offset,
        page_size,
        additional_result_sets: Vec::new(),
    })
}

fn empty_result(
    execution_id: Uuid,
    affected_rows: u64,
    started: Instant,
    row_offset: usize,
    page_size: usize,
) -> QueryResultPage {
    QueryResultPage {
        execution_id,
        columns: Vec::new(),
        rows: Vec::new(),
        affected_rows,
        execution_time_ms: started.elapsed().as_millis(),
        truncated: false,
        has_more: false,
        result_set_index: 0,
        messages: Vec::new(),
        row_offset,
        page_size,
        additional_result_sets: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use cockpit_core::{DatabaseKind, DatabaseObjectKind, ExecuteQueryRequest, TlsOptions};

    use super::*;

    fn profile() -> ConnectionProfile {
        let now = Utc::now();
        ConnectionProfile {
            id: Uuid::new_v4(),
            driver_kind: DatabaseKind::Sqlite,
            group: None,
            name: "memory".into(),
            host: ":memory:".into(),
            port: 1,
            username: String::new(),
            database: Some("main".into()),
            tls: TlsOptions::default(),
            ssh: None,
            connect_timeout_secs: 5,
            query_timeout_secs: 30,
            pool_size: 1,
            read_only: false,
            production: false,
            color: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn request(sql: &str, allow_write: bool) -> ExecuteQueryRequest {
        ExecuteQueryRequest {
            execution_id: Uuid::new_v4(),
            sql: sql.into(),
            database: Some("main".into()),
            timeout_secs: None,
            allow_write,
            page_size: 100,
            row_offset: 0,
        }
    }

    #[tokio::test]
    async fn browses_queries_and_mutates_an_in_memory_database() {
        let session = SqliteDriver.open(profile(), String::new()).await.unwrap();
        session.execute(request("CREATE TABLE items(id INTEGER PRIMARY KEY, name TEXT NOT NULL); INSERT INTO items(name) VALUES ('a'), ('b')", true)).await.unwrap();
        assert_eq!(session.list_databases().await.unwrap()[0].name, "main");
        assert_eq!(
            session.list_tables("main", None, 20, 0).await.unwrap()[0].name,
            "items"
        );
        let detail = session.table_detail("main", "items").await.unwrap();
        assert_eq!(detail.columns.len(), 2);
        let page = session
            .execute(request("SELECT id, name FROM items ORDER BY id", false))
            .await
            .unwrap();
        assert_eq!(page.rows.len(), 2);
        session
            .mutate_row(RowMutationRequest {
                database: "main".into(),
                table: "items".into(),
                kind: RowMutationKind::Update,
                values: vec![("name".into(), CellValue::Text("updated".into()))],
                key_values: vec![("id".into(), CellValue::Signed("1".into()))],
                original_values: Vec::new(),
            })
            .await
            .unwrap();
        assert!(
            session
                .execute(request("SELECT name FROM items WHERE id=1", false))
                .await
                .unwrap()
                .rows
                .iter()
                .flatten()
                .any(|value| value == &CellValue::Text("updated".into()))
        );
        assert!(
            session
                .object_definition("main", DatabaseObjectKind::View, "missing")
                .await
                .is_err()
        );
    }
}
