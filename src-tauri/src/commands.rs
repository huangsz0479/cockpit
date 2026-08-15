use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    io::{BufWriter, Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use argon2::Argon2;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use calamine::{Data, Reader, open_workbook_auto};
use chrono::Utc;
use cockpit_core::{
    CellValue, CockpitError, ColumnInfo, ColumnMeta, ConnectionInfo, ConnectionProfile,
    DatabaseDriver, DatabaseInfo, DatabaseKind, DatabaseObjectDefinition, DatabaseObjectKind,
    ErrorPayload, EventInfo, ExecuteQueryRequest, ExportFormat, ImportConflictPolicy,
    QueryResultPage, ResultExportOptions, ResultStreamWriter, RoutineInfo, RoutineParameter,
    RowMutationRequest, RowMutationResult, ServerLockInfo, ServerMetric, ServerProcessInfo,
    ServerVariable, SqlAssessment, TableDetail, TableInfo, TriggerInfo, UserAccount,
    safety::assess_sql, write_result_page,
};
use cockpit_mysql::MySqlDriver;
use cockpit_postgres::PostgresDriver;
use cockpit_sqlite::SqliteDriver;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use rand_core::{OsRng, RngCore};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::state::AppState;

type CommandResult<T> = std::result::Result<T, ErrorPayload>;

fn payload(error: CockpitError) -> ErrorPayload {
    error.payload()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsInfo {
    version: &'static str,
    log_path: Option<String>,
    logs: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStats {
    open_connection_count: usize,
    tab_session_count: usize,
    memory_bytes: Option<u64>,
}

fn process_tree_memory_bytes<I>(root_pid: u32, processes: I) -> Option<u64>
where
    I: IntoIterator<Item = (u32, Option<u32>, u64)>,
{
    let processes = processes
        .into_iter()
        .map(|(pid, parent_pid, memory_bytes)| (pid, (parent_pid, memory_bytes)))
        .collect::<HashMap<_, _>>();
    if !processes.contains_key(&root_pid) {
        return None;
    }

    let mut included = HashSet::from([root_pid]);
    loop {
        let previous_len = included.len();
        for (&pid, &(parent_pid, _)) in &processes {
            if parent_pid.is_some_and(|parent_pid| included.contains(&parent_pid)) {
                included.insert(pid);
            }
        }
        if included.len() == previous_len {
            break;
        }
    }

    Some(
        included
            .into_iter()
            .fold(0, |total, pid| total.saturating_add(processes[&pid].1)),
    )
}

fn application_memory_bytes() -> Option<u64> {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};

    let root_pid = sysinfo::get_current_pid().ok()?.as_u32();
    let system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_processes(ProcessRefreshKind::nothing().with_memory().without_tasks()),
    );
    process_tree_memory_bytes(
        root_pid,
        system.processes().values().map(|process| {
            (
                process.pid().as_u32(),
                process.parent().map(|pid| pid.as_u32()),
                process.memory(),
            )
        }),
    )
}

#[tauri::command]
pub async fn get_runtime_stats(state: State<'_, Arc<AppState>>) -> CommandResult<RuntimeStats> {
    let open_connection_count = state.sessions.read().await.len();
    let tab_session_count = state.tab_sessions.read().await.len();
    let memory_bytes = tauri::async_runtime::spawn_blocking(application_memory_bytes)
        .await
        .ok()
        .flatten();
    Ok(RuntimeStats {
        open_connection_count,
        tab_session_count,
        memory_bytes,
    })
}

#[tauri::command]
pub async fn get_diagnostics(state: State<'_, Arc<AppState>>) -> CommandResult<DiagnosticsInfo> {
    let log_dir = state.log_dir.clone();
    tauri::async_runtime::spawn_blocking(move || read_diagnostics(&log_dir))
        .await
        .map_err(|error| payload(CockpitError::Other(error.to_string())))?
        .map_err(payload)
}

fn read_diagnostics(log_dir: &Path) -> cockpit_core::Result<DiagnosticsInfo> {
    let entries = match std::fs::read_dir(log_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DiagnosticsInfo {
                version: env!("CARGO_PKG_VERSION"),
                log_path: None,
                logs: "尚未生成应用日志。".into(),
            });
        }
        Err(error) => return Err(exchange_error(error)),
    };
    let mut candidates = entries
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("cockpit"))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|entry| {
        entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
    });
    let Some(entry) = candidates.pop() else {
        return Ok(DiagnosticsInfo {
            version: env!("CARGO_PKG_VERSION"),
            log_path: None,
            logs: "尚未生成应用日志。".into(),
        });
    };
    let path = entry.path();
    let mut file = std::fs::File::open(&path).map_err(exchange_error)?;
    let length = file.metadata().map_err(exchange_error)?.len();
    let start = length.saturating_sub(512 * 1024);
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(start)).map_err(exchange_error)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(exchange_error)?;
    let mut logs = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0
        && let Some(first_line) = logs.find('\n')
    {
        logs.drain(..=first_line);
    }
    Ok(DiagnosticsInfo {
        version: env!("CARGO_PKG_VERSION"),
        log_path: Some(path.to_string_lossy().into_owned()),
        logs: redact_log_text(&logs),
    })
}

fn redact_log_text(value: &str) -> String {
    static SENSITIVE_FIELD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(password|token|secret|authorization)\s*[:=]\s*[^\s,;]+")
            .expect("valid log redaction regex")
    });
    static URL_CREDENTIAL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(://[^:/\s]+:)[^@/\s]+@").expect("valid URL credential regex")
    });
    let value = SENSITIVE_FIELD.replace_all(value, "$1=[REDACTED]");
    URL_CREDENTIAL
        .replace_all(&value, "$1[REDACTED]@")
        .into_owned()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferProgress {
    task_id: Uuid,
    kind: &'static str,
    phase: String,
    completed: u64,
    total: Option<u64>,
    message: Option<String>,
}

const ENCRYPTED_BACKUP_MAGIC: &[u8; 11] = b"COCKPITENC1";
const ENCRYPTION_CHUNK_SIZE: usize = 1024 * 1024;

fn emit_transfer_progress(
    app: &AppHandle,
    task_id: Uuid,
    kind: &'static str,
    phase: impl Into<String>,
    completed: u64,
    total: Option<u64>,
    message: Option<String>,
) {
    let _ = app.emit(
        "transfer-progress",
        TransferProgress {
            task_id,
            kind,
            phase: phase.into(),
            completed,
            total,
            message,
        },
    );
}

fn emit_export_progress(
    app: &AppHandle,
    task_id: Option<Uuid>,
    phase: impl Into<String>,
    completed: u64,
    total: Option<u64>,
    message: Option<String>,
) {
    if let Some(task_id) = task_id {
        emit_transfer_progress(app, task_id, "export", phase, completed, total, message);
    }
}

fn export_total_cell(cell: &CellValue) -> cockpit_core::Result<u64> {
    let parse = |value: &str| {
        value
            .trim()
            .parse::<u64>()
            .map_err(|_| CockpitError::Exchange("无法解析导出总行数".into()))
    };
    match cell {
        CellValue::Signed(value)
        | CellValue::Unsigned(value)
        | CellValue::Decimal(value)
        | CellValue::Text(value) => parse(value),
        CellValue::Float(value)
            if value.is_finite()
                && *value >= 0.0
                && value.fract() == 0.0
                && *value <= u64::MAX as f64 =>
        {
            Ok(*value as u64)
        }
        CellValue::Bytes { base64, .. } => {
            let bytes = BASE64_STANDARD
                .decode(base64)
                .map_err(|_| CockpitError::Exchange("无法解码导出总行数".into()))?;
            let value = std::str::from_utf8(&bytes)
                .map_err(|_| CockpitError::Exchange("导出总行数不是有效文本".into()))?;
            parse(value)
        }
        _ => Err(CockpitError::Exchange(format!(
            "统计导出总行数返回了无效类型：{cell:?}"
        ))),
    }
}

fn export_total_rows(page: &QueryResultPage) -> cockpit_core::Result<u64> {
    page.rows
        .first()
        .and_then(|row| row.first())
        .ok_or_else(|| CockpitError::Exchange("统计导出总行数没有返回结果".into()))
        .and_then(export_total_cell)
}

#[tauri::command]
pub async fn cancel_transfer(
    state: State<'_, Arc<AppState>>,
    task_id: Uuid,
) -> CommandResult<bool> {
    let token = state.transfers.read().await.get(&task_id).cloned();
    if let Some(token) = token {
        token.cancel();
        Ok(true)
    } else {
        Ok(false)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveConnectionRequest {
    pub profile: ConnectionProfile,
    pub password: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveConnectionResponse {
    pub profile: ConnectionProfile,
    pub secret_persisted: bool,
}

#[tauri::command]
pub async fn list_connections(
    state: State<'_, Arc<AppState>>,
) -> CommandResult<Vec<ConnectionProfile>> {
    state.storage.list_connections().map_err(payload)
}

#[tauri::command]
pub async fn has_connection_password(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
) -> CommandResult<bool> {
    state
        .secrets
        .contains(connection_id, "mysql_password")
        .map_err(payload)
}

#[tauri::command]
pub async fn save_connection(
    state: State<'_, Arc<AppState>>,
    request: SaveConnectionRequest,
) -> CommandResult<SaveConnectionResponse> {
    let mut profile = request.profile;
    profile.validate().map_err(payload)?;
    let now = Utc::now();
    if let Ok(Some(existing)) = state.storage.get_connection(profile.id) {
        profile.created_at = existing.created_at;
    }
    profile.updated_at = now;
    state.storage.save_connection(&profile).map_err(payload)?;
    let secret_persisted = match request
        .password
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        Some(password) => state
            .secrets
            .set(profile.id, "mysql_password", password)
            .map_err(payload)?,
        None => true,
    };
    Ok(SaveConnectionResponse {
        profile,
        secret_persisted,
    })
}

#[tauri::command]
pub async fn delete_connection(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
) -> CommandResult<()> {
    if let Some(session) = state.sessions.write().await.remove(&connection_id) {
        let _ = session.close().await;
    }
    close_tab_sessions_for_connection(&state, connection_id).await;
    state
        .storage
        .delete_connection(connection_id)
        .map_err(payload)?;
    state.secrets.delete_connection(connection_id);
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestConnectionRequest {
    pub profile: ConnectionProfile,
    pub password: Option<String>,
}

#[tauri::command]
pub async fn test_connection(
    state: State<'_, Arc<AppState>>,
    request: TestConnectionRequest,
) -> CommandResult<ConnectionInfo> {
    let password = match request.password {
        Some(password) => password,
        None => state
            .secrets
            .get(request.profile.id, "mysql_password")
            .map_err(payload)?
            .unwrap_or_default(),
    };
    match request.profile.driver_kind {
        DatabaseKind::MySql | DatabaseKind::MariaDb => {
            MySqlDriver.test(&request.profile, &password).await
        }
        DatabaseKind::Sqlite => SqliteDriver.test(&request.profile, "").await,
        DatabaseKind::PostgreSql => PostgresDriver.test(&request.profile, &password).await,
    }
    .map_err(payload)
}

async fn open_driver_session(
    state: &AppState,
    connection_id: Uuid,
    dedicated: bool,
) -> CommandResult<Arc<dyn cockpit_core::DriverSession>> {
    let mut profile = state
        .storage
        .get_connection(connection_id)
        .map_err(payload)?
        .ok_or_else(|| payload(CockpitError::NotFound("连接配置不存在".into())))?;
    if dedicated {
        profile.pool_size = 1;
    }
    let password = state
        .secrets
        .get(connection_id, "mysql_password")
        .map_err(payload)?
        .unwrap_or_default();
    match profile.driver_kind {
        DatabaseKind::MySql | DatabaseKind::MariaDb => MySqlDriver.open(profile, password).await,
        DatabaseKind::Sqlite => SqliteDriver.open(profile, String::new()).await,
        DatabaseKind::PostgreSql => PostgresDriver.open(profile, password).await,
    }
    .map_err(payload)
}

async fn close_driver_session(session: Arc<dyn cockpit_core::DriverSession>) -> CommandResult<()> {
    let rollback_result = if session.transaction_active().await {
        session.rollback_transaction().await
    } else {
        Ok(())
    };
    let close_result = session.close().await;
    rollback_result.and(close_result).map_err(payload)
}

async fn close_tab_sessions_for_connection(state: &AppState, connection_id: Uuid) {
    let sessions = {
        let mut tab_sessions = state.tab_sessions.write().await;
        let session_ids = tab_sessions
            .iter()
            .filter_map(|(session_id, (owner_id, _))| {
                (*owner_id == connection_id).then_some(*session_id)
            })
            .collect::<Vec<_>>();
        session_ids
            .into_iter()
            .filter_map(|session_id| tab_sessions.remove(&session_id).map(|(_, session)| session))
            .collect::<Vec<_>>()
    };
    for session in sessions {
        let _ = close_driver_session(session).await;
    }
}

#[tauri::command]
pub async fn connect_connection(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
) -> CommandResult<ConnectionInfo> {
    if let Some(session) = state.sessions.read().await.get(&connection_id).cloned() {
        return session.connection_info().await.map_err(payload);
    }
    let session = open_driver_session(&state, connection_id, false).await?;
    let info = session.connection_info().await.map_err(payload)?;
    state.sessions.write().await.insert(connection_id, session);
    Ok(info)
}

#[tauri::command]
pub async fn open_tab_session(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    session_id: Uuid,
) -> CommandResult<()> {
    if let Some((owner_id, _)) = state.tab_sessions.read().await.get(&session_id) {
        return if *owner_id == connection_id {
            Ok(())
        } else {
            Err(payload(CockpitError::InvalidConfig(
                "标签页会话已绑定到其他连接".into(),
            )))
        };
    }

    let session = open_driver_session(&state, connection_id, true).await?;
    let mut tab_sessions = state.tab_sessions.write().await;
    if let Some((owner_id, _)) = tab_sessions.get(&session_id) {
        let same_connection = *owner_id == connection_id;
        drop(tab_sessions);
        let _ = close_driver_session(session).await;
        return if same_connection {
            Ok(())
        } else {
            Err(payload(CockpitError::InvalidConfig(
                "标签页会话已绑定到其他连接".into(),
            )))
        };
    }
    tab_sessions.insert(session_id, (connection_id, session));
    Ok(())
}

#[tauri::command]
pub async fn close_tab_session(
    state: State<'_, Arc<AppState>>,
    session_id: Uuid,
) -> CommandResult<()> {
    let Some((_, session)) = state.tab_sessions.write().await.remove(&session_id) else {
        return Ok(());
    };
    close_driver_session(session).await
}

#[tauri::command]
pub async fn disconnect_connection(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
) -> CommandResult<()> {
    let close_result = if let Some(session) = state.sessions.write().await.remove(&connection_id) {
        session.close().await.map_err(payload)
    } else {
        Ok(())
    };
    close_tab_sessions_for_connection(&state, connection_id).await;
    close_result
}

async fn session(
    state: &AppState,
    connection_id: Uuid,
) -> CommandResult<Arc<dyn cockpit_core::DriverSession>> {
    state
        .sessions
        .read()
        .await
        .get(&connection_id)
        .cloned()
        .ok_or_else(|| payload(CockpitError::Connection("连接尚未打开".into())))
}

async fn data_session(
    state: &AppState,
    connection_id: Uuid,
    session_id: Option<Uuid>,
) -> CommandResult<Arc<dyn cockpit_core::DriverSession>> {
    let Some(session_id) = session_id else {
        return session(state, connection_id).await;
    };
    let tab_sessions = state.tab_sessions.read().await;
    let (owner_id, session) = tab_sessions
        .get(&session_id)
        .ok_or_else(|| payload(CockpitError::Connection("标签页会话尚未打开".into())))?;
    if *owner_id != connection_id {
        return Err(payload(CockpitError::InvalidConfig(
            "标签页会话与连接不匹配".into(),
        )));
    }
    Ok(session.clone())
}

#[tauri::command]
pub async fn list_databases(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
) -> CommandResult<Vec<DatabaseInfo>> {
    session(&state, connection_id)
        .await?
        .list_databases()
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn list_tables(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    database: String,
    filter: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> CommandResult<Vec<TableInfo>> {
    session(&state, connection_id)
        .await?
        .list_tables(
            &database,
            filter.as_deref(),
            limit.unwrap_or(500),
            offset.unwrap_or(0),
        )
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn list_columns(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    database: String,
    table: String,
) -> CommandResult<Vec<ColumnInfo>> {
    session(&state, connection_id)
        .await?
        .list_columns(&database, &table)
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn get_table_detail(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    database: String,
    table: String,
) -> CommandResult<TableDetail> {
    session(&state, connection_id)
        .await?
        .table_detail(&database, &table)
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn list_routines(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    database: String,
) -> CommandResult<Vec<RoutineInfo>> {
    session(&state, connection_id)
        .await?
        .list_routines(&database)
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn list_triggers(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    database: String,
) -> CommandResult<Vec<TriggerInfo>> {
    session(&state, connection_id)
        .await?
        .list_triggers(&database)
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn list_events(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    database: String,
) -> CommandResult<Vec<EventInfo>> {
    session(&state, connection_id)
        .await?
        .list_events(&database)
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn get_object_definition(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    database: String,
    kind: DatabaseObjectKind,
    name: String,
) -> CommandResult<DatabaseObjectDefinition> {
    session(&state, connection_id)
        .await?
        .object_definition(&database, kind, &name)
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn get_routine_parameters(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    database: String,
    name: String,
) -> CommandResult<Vec<RoutineParameter>> {
    session(&state, connection_id)
        .await?
        .routine_parameters(&database, &name)
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn list_server_processes(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
) -> CommandResult<Vec<ServerProcessInfo>> {
    session(&state, connection_id)
        .await?
        .list_processes()
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn kill_server_process(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    process_id: u64,
) -> CommandResult<()> {
    session(&state, connection_id)
        .await?
        .kill_process(process_id)
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn get_server_status(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
) -> CommandResult<Vec<ServerMetric>> {
    session(&state, connection_id)
        .await?
        .server_status()
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn list_server_variables(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    filter: Option<String>,
) -> CommandResult<Vec<ServerVariable>> {
    session(&state, connection_id)
        .await?
        .server_variables(filter.as_deref())
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn list_server_locks(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
) -> CommandResult<Vec<ServerLockInfo>> {
    session(&state, connection_id)
        .await?
        .server_locks()
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn list_database_users(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
) -> CommandResult<Vec<UserAccount>> {
    session(&state, connection_id)
        .await?
        .list_users()
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn get_user_grants(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    user: String,
    host: String,
) -> CommandResult<Vec<String>> {
    session(&state, connection_id)
        .await?
        .user_grants(&user, &host)
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn assess_query(sql: String) -> SqlAssessment {
    assess_sql(&sql)
}

#[tauri::command]
pub async fn execute_query(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    session_id: Option<Uuid>,
    request: ExecuteQueryRequest,
) -> CommandResult<QueryResultPage> {
    data_session(&state, connection_id, session_id)
        .await?
        .execute(request)
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn load_workspace_state(
    state: State<'_, Arc<AppState>>,
    state_key: String,
) -> CommandResult<Option<String>> {
    state
        .storage
        .load_workspace_state(&state_key)
        .map_err(payload)
}

#[tauri::command]
pub async fn save_workspace_state(
    state: State<'_, Arc<AppState>>,
    state_key: String,
    payload_json: String,
) -> CommandResult<()> {
    state
        .storage
        .save_workspace_state(&state_key, &payload_json)
        .map_err(payload)
}

const MAX_EDITOR_FILE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextFileContent {
    pub path: String,
    pub contents: String,
}

#[tauri::command]
pub async fn read_text_file(input_path: String) -> CommandResult<TextFileContent> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(&input_path);
        let metadata = std::fs::metadata(&path).map_err(exchange_error)?;
        if metadata.len() > MAX_EDITOR_FILE_BYTES {
            return Err(CockpitError::Exchange(format!(
                "文件超过编辑器支持的 32 MB 限制（当前 {:.1} MB）",
                metadata.len() as f64 / 1024.0 / 1024.0
            )));
        }
        let contents = std::fs::read_to_string(&path).map_err(exchange_error)?;
        Ok(TextFileContent {
            path: path.to_string_lossy().into_owned(),
            contents,
        })
    })
    .await
    .map_err(|error| payload(CockpitError::Exchange(error.to_string())))?
    .map_err(payload)
}

#[tauri::command]
pub async fn write_text_file(output_path: String, contents: String) -> CommandResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let output_path = PathBuf::from(output_path);
        let parent = output_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| CockpitError::InvalidConfig("保存路径无效".into()))?;
        let file_name = output_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| CockpitError::InvalidConfig("保存文件名无效".into()))?;
        let temporary_path = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
        let write_result = (|| -> cockpit_core::Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary_path)
                .map_err(exchange_error)?;
            file.write_all(contents.as_bytes())
                .map_err(exchange_error)?;
            file.sync_all().map_err(exchange_error)?;
            drop(file);
            replace_file(&temporary_path, &output_path)
        })();
        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&temporary_path);
            return Err(error);
        }
        Ok(output_path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|error| payload(CockpitError::Exchange(error.to_string())))?
    .map_err(payload)
}

#[tauri::command]
pub async fn reveal_file(input_path: String) -> CommandResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(input_path);
        if !path.is_file() {
            return Err(CockpitError::Exchange("查询文件不存在".into()));
        }
        #[cfg(target_os = "macos")]
        let status = std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .status();
        #[cfg(target_os = "windows")]
        let status = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.to_string_lossy()))
            .status();
        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        let status = std::process::Command::new("xdg-open")
            .arg(path.parent().unwrap_or_else(|| Path::new(".")))
            .status();
        let status = status.map_err(exchange_error)?;
        if status.success() {
            Ok(())
        } else {
            Err(CockpitError::Exchange(
                "无法在文件管理器中显示查询文件".into(),
            ))
        }
    })
    .await
    .map_err(|error| payload(CockpitError::Exchange(error.to_string())))?
    .map_err(payload)
}

/// Upper bound for `write_binary_file` payloads (base64 already decoded).
const MAX_BINARY_FILE_BYTES: u64 = 512 * 1024 * 1024;

#[tauri::command]
pub async fn write_binary_file(output_path: String, base64: String) -> CommandResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        // Reject oversized payloads before decoding: base64 inflates the
        // raw bytes by ~4/3, so the encoded length is a cheap upper bound.
        if base64.len() as u64 > MAX_BINARY_FILE_BYTES * 4 / 3 + 4 {
            return Err(CockpitError::Exchange(format!(
                "二进制文件超过 512 MB 限制（当前约 {:.1} MB）",
                base64.len() as f64 / 1024.0 / 1024.0 / 4.0 * 3.0
            )));
        }
        let bytes = BASE64_STANDARD
            .decode(base64)
            .map_err(|error| CockpitError::InvalidConfig(format!("二进制数据无效：{error}")))?;
        if bytes.len() as u64 > MAX_BINARY_FILE_BYTES {
            return Err(CockpitError::Exchange(format!(
                "二进制文件超过 512 MB 限制（当前 {:.1} MB）",
                bytes.len() as f64 / 1024.0 / 1024.0
            )));
        }
        let output_path = PathBuf::from(output_path);
        let parent = output_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| CockpitError::InvalidConfig("保存路径无效".into()))?;
        let file_name = output_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| CockpitError::InvalidConfig("保存文件名无效".into()))?;
        // Write to a temporary file first, then atomically replace the
        // destination, so a crash mid-write never leaves a truncated file
        // at the requested path.
        let temporary_path = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
        let write_result = (|| -> cockpit_core::Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary_path)
                .map_err(exchange_error)?;
            file.write_all(&bytes).map_err(exchange_error)?;
            file.sync_all().map_err(exchange_error)?;
            drop(file);
            replace_file(&temporary_path, &output_path)
        })();
        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&temporary_path);
            return Err(error);
        }
        Ok(output_path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|error| payload(CockpitError::Exchange(error.to_string())))?
    .map_err(payload)
}

#[tauri::command]
pub async fn mutate_row(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    session_id: Option<Uuid>,
    request: RowMutationRequest,
) -> CommandResult<RowMutationResult> {
    data_session(&state, connection_id, session_id)
        .await?
        .mutate_row(request)
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn begin_transaction(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    session_id: Option<Uuid>,
) -> CommandResult<()> {
    data_session(&state, connection_id, session_id)
        .await?
        .begin_transaction()
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn commit_transaction(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    session_id: Option<Uuid>,
) -> CommandResult<()> {
    data_session(&state, connection_id, session_id)
        .await?
        .commit_transaction()
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn rollback_transaction(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    session_id: Option<Uuid>,
) -> CommandResult<()> {
    data_session(&state, connection_id, session_id)
        .await?
        .rollback_transaction()
        .await
        .map_err(payload)
}

#[tauri::command]
pub async fn transaction_active(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    session_id: Option<Uuid>,
) -> CommandResult<bool> {
    Ok(data_session(&state, connection_id, session_id)
        .await?
        .transaction_active()
        .await)
}

#[tauri::command]
pub async fn cancel_query(
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    session_id: Option<Uuid>,
    execution_id: Uuid,
) -> CommandResult<bool> {
    data_session(&state, connection_id, session_id)
        .await?
        .cancel(execution_id)
        .await
        .map_err(payload)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub output_path: String,
    pub rows_written: u64,
}

#[tauri::command]
pub async fn export_result_page(
    output_path: String,
    page: QueryResultPage,
    options: Option<ResultExportOptions>,
) -> CommandResult<ExportSummary> {
    tauri::async_runtime::spawn_blocking(move || {
        export_page(
            PathBuf::from(output_path),
            page,
            options.unwrap_or_default(),
        )
    })
    .await
    .map_err(|error| payload(CockpitError::Exchange(error.to_string())))?
    .map_err(payload)
}

#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command parameters mirror the frontend IPC payload"
)]
pub async fn export_table(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    database: String,
    table: String,
    output_path: String,
    options: Option<ResultExportOptions>,
    task_id: Option<Uuid>,
) -> CommandResult<ExportSummary> {
    let driver_kind = state
        .storage
        .get_connection(connection_id)
        .map_err(payload)?
        .ok_or_else(|| payload(CockpitError::NotFound("连接配置不存在".into())))?
        .driver_kind;
    let session = session(&state, connection_id).await?;
    emit_export_progress(
        &app,
        task_id,
        "准备",
        0,
        None,
        Some("正在准备整表导出".into()),
    );
    let output_path = PathBuf::from(output_path);
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| payload(CockpitError::InvalidConfig("导出路径无效".into())))?;
    let file_name = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| payload(CockpitError::InvalidConfig("导出文件名无效".into())))?;
    let temporary_path = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary_path)
        .map_err(|error| payload(exchange_error(error)))?;
    let owns_transaction = !session.transaction_active().await;
    if owns_transaction && let Err(error) = session.begin_read_transaction().await {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(payload(error));
    }
    let mut options = options.unwrap_or_default();
    options.database_name = Some(database.clone());
    options.table_name = Some(table.clone());
    options.database_kind = driver_kind;
    let include_generated_columns = options.format != ExportFormat::Sql;
    let page_size = 2_000usize;
    let mut offset = 0usize;
    let mut writer: Option<ResultStreamWriter<BufWriter<std::fs::File>>> = None;
    let export_result: cockpit_core::Result<u64> = async {
        let detail = session.table_detail(&database, &table).await?;
        if matches!(driver_kind, DatabaseKind::MySql | DatabaseKind::MariaDb)
            && !mysql_table_engine(&detail.ddl)
                .is_some_and(|engine| engine.eq_ignore_ascii_case("InnoDB"))
        {
            return Err(CockpitError::Exchange(
                "为避免导出期间数据变化导致重复或遗漏，整表导出只支持 InnoDB 表".into(),
            ));
        }
        emit_export_progress(
            &app,
            task_id,
            "统计总行数",
            0,
            None,
            Some("正在统计需要导出的数据量".into()),
        );
        let count_page = session
            .execute(ExecuteQueryRequest {
                execution_id: Uuid::new_v4(),
                sql: format!(
                    "SELECT COUNT(*) FROM {}.{}",
                    backup_quote_identifier(&database, driver_kind),
                    backup_quote_identifier(&table, driver_kind),
                ),
                database: Some(database.clone()),
                timeout_secs: None,
                allow_write: false,
                page_size: 1,
                row_offset: 0,
            })
            .await?;
        let total_rows = export_total_rows(&count_page)?;
        emit_export_progress(
            &app,
            task_id,
            "导出数据",
            0,
            Some(total_rows),
            Some(format!("已写入 0 / {total_rows} 行")),
        );
        // Keyset pagination on the primary (or non-null unique) key avoids
        // re-skipping OFFSET rows of the whole table on every page. It is
        // disabled permanently as soon as a page cannot yield a cursor
        // (e.g. a NULL key value), falling back to OFFSET.
        let key_columns = table_keyset_columns(&detail);
        let mut keyset_active = key_columns.is_some();
        let mut cursor: Option<KeysetCursor> = None;
        loop {
            let keyset = keyset_active.then_some(cursor.as_ref()).flatten();
            let sql = table_page_sql(
                &database,
                &table,
                &detail,
                page_size,
                offset,
                keyset,
                driver_kind,
                include_generated_columns,
            )?;
            let page = session
                .execute(ExecuteQueryRequest {
                    execution_id: Uuid::new_v4(),
                    sql,
                    database: Some(database.clone()),
                    timeout_secs: None,
                    allow_write: false,
                    page_size,
                    row_offset: 0,
                })
                .await?;
            if writer.is_none() {
                writer = Some(ResultStreamWriter::new(
                    BufWriter::new(file.try_clone().map_err(exchange_error)?),
                    &page.columns,
                    &options,
                )?);
            }
            let row_count = page.rows.len();
            for row in &page.rows {
                writer
                    .as_mut()
                    .expect("writer initialized")
                    .write_row(row)?;
            }
            offset += row_count;
            if keyset_active {
                let next = match (key_columns.as_deref(), page.rows.last()) {
                    (Some(key_columns), Some(last_row)) => {
                        keyset_from_last_row(key_columns, &page.columns, last_row)
                    }
                    _ => None,
                };
                match next {
                    Some(next)
                        if keyset_condition_sql(&next.columns, &next.last_values, driver_kind)
                            .is_some() =>
                    {
                        cursor = Some(next);
                    }
                    _ => {
                        keyset_active = false;
                        cursor = None;
                    }
                }
            }
            emit_export_progress(
                &app,
                task_id,
                "导出数据",
                offset as u64,
                Some(total_rows),
                Some(format!("已写入 {offset} / {total_rows} 行")),
            );
            if !page.has_more || row_count == 0 {
                break;
            }
        }
        if matches!(driver_kind, DatabaseKind::MySql | DatabaseKind::MariaDb) {
            let current = session.table_detail(&database, &table).await?;
            if current.ddl != detail.ddl {
                return Err(CockpitError::Exchange(
                    "导出期间表结构发生变化，请重试".into(),
                ));
            }
        }
        let writer = writer.ok_or_else(|| CockpitError::Exchange("导出没有返回元数据".into()))?;
        let rows_written = writer.rows_written();
        let mut output = writer.finish()?;
        output.flush().map_err(exchange_error)?;
        output.get_ref().sync_all().map_err(exchange_error)?;
        Ok(rows_written)
    }
    .await;
    let rows_written = match export_result {
        Ok(rows_written) => rows_written,
        Err(error) => {
            if owns_transaction {
                let _ = session.rollback_transaction().await;
            }
            let _ = std::fs::remove_file(&temporary_path);
            return Err(payload(error));
        }
    };
    if owns_transaction && let Err(error) = session.commit_transaction().await {
        let _ = session.rollback_transaction().await;
        let _ = std::fs::remove_file(&temporary_path);
        return Err(payload(error));
    }
    drop(file);
    replace_file(&temporary_path, &output_path).map_err(payload)?;
    emit_export_progress(
        &app,
        task_id,
        "完成",
        rows_written,
        Some(rows_written),
        Some(format!("已导出 {rows_written} 行")),
    );
    Ok(ExportSummary {
        output_path: output_path.to_string_lossy().into_owned(),
        rows_written,
    })
}

#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command parameters mirror the frontend IPC payload"
)]
pub async fn export_query(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    connection_id: Uuid,
    database: Option<String>,
    sql: String,
    output_path: String,
    options: Option<ResultExportOptions>,
    task_id: Option<Uuid>,
) -> CommandResult<ExportSummary> {
    let assessment = assess_sql(&sql);
    if assessment.statement_kind != "SELECT" || assessment.requires_confirmation {
        return Err(payload(CockpitError::InvalidConfig(
            "完整结果导出仅支持无副作用的单条 SELECT 查询".into(),
        )));
    }
    let driver_kind = state
        .storage
        .get_connection(connection_id)
        .map_err(payload)?
        .ok_or_else(|| payload(CockpitError::NotFound("连接配置不存在".into())))?
        .driver_kind;
    let session = session(&state, connection_id).await?;
    emit_export_progress(
        &app,
        task_id,
        "准备",
        0,
        None,
        Some("正在准备完整结果导出".into()),
    );
    let output_path = PathBuf::from(output_path);
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| payload(CockpitError::InvalidConfig("导出路径无效".into())))?;
    let file_name = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| payload(CockpitError::InvalidConfig("导出文件名无效".into())))?;
    let temporary_path = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary_path)
        .map_err(|error| payload(exchange_error(error)))?;
    let owns_transaction = !session.transaction_active().await;
    if owns_transaction && let Err(error) = session.begin_read_transaction().await {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(payload(error));
    }
    let page_size = 2_000usize;
    let export_result: cockpit_core::Result<u64> = async {
        emit_export_progress(
            &app,
            task_id,
            "统计总行数",
            0,
            None,
            Some("正在统计需要导出的数据量".into()),
        );
        let count_page = session
            .execute(ExecuteQueryRequest {
                execution_id: Uuid::new_v4(),
                sql: format!(
                    "SELECT COUNT(*) FROM ({}) AS {}",
                    sql.trim().trim_end_matches(';'),
                    backup_quote_identifier("__cockpit_count", driver_kind),
                ),
                database: database.clone(),
                timeout_secs: None,
                allow_write: false,
                page_size: 1,
                row_offset: 0,
            })
            .await?;
        let total_rows = export_total_rows(&count_page)?;
        emit_export_progress(
            &app,
            task_id,
            "导出数据",
            0,
            Some(total_rows),
            Some(format!("已写入 0 / {total_rows} 行")),
        );
        // Describe the result shape (column names) with LIMIT 0 so every
        // page can be ordered deterministically and keyed by the query's
        // own output columns; no rows are transferred by this query.
        let describe = session
            .execute(ExecuteQueryRequest {
                execution_id: Uuid::new_v4(),
                sql: format!(
                    "SELECT * FROM ({}) AS {} LIMIT 0",
                    sql.trim().trim_end_matches(';'),
                    backup_quote_identifier("__cockpit_page", driver_kind),
                ),
                database: database.clone(),
                timeout_secs: None,
                allow_write: false,
                page_size: 1,
                row_offset: 0,
            })
            .await?;
        let keyset_supported = {
            let mut seen = std::collections::HashSet::new();
            describe
                .columns
                .iter()
                .all(|column| !column.name.is_empty() && seen.insert(column.name.clone()))
        };
        // Probe whether the output columns can actually be ordered: some
        // engines reject ordering on certain column types (e.g. PostgreSQL
        // has no ordering operator for `json`), which would otherwise break
        // the ordered pages below. LIMIT 0 transfers no rows.
        let ordered_columns = if keyset_supported {
            let order = (1..=describe.columns.len())
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let probe = format!(
                "SELECT * FROM ({}) AS {} ORDER BY {order} LIMIT 0",
                sql.trim().trim_end_matches(';'),
                backup_quote_identifier("__cockpit_page", driver_kind),
            );
            match session
                .execute(ExecuteQueryRequest {
                    execution_id: Uuid::new_v4(),
                    sql: probe,
                    database: database.clone(),
                    timeout_secs: None,
                    allow_write: false,
                    page_size: 1,
                    row_offset: 0,
                })
                .await
            {
                Ok(_) => describe.columns.len(),
                Err(_) => 0,
            }
        } else {
            0
        };
        let export_options = {
            let mut options = options.clone().unwrap_or_default();
            options.database_kind = driver_kind;
            options
        };
        let mut rows_written = export_query_pages(
            &app,
            task_id,
            &session,
            database.clone(),
            &sql,
            page_size,
            &file,
            export_options.clone(),
            driver_kind,
            total_rows,
            ordered_columns,
        )
        .await?;
        if ordered_columns > 0 && rows_written != total_rows {
            // Keyset pagination seeks past the cursor with a strict `>`, so
            // duplicate rows (which sort equal to the cursor) would be
            // skipped. When the exported count does not add up, redo the
            // export with plain OFFSET pagination for completeness.
            emit_export_progress(
                &app,
                task_id,
                "导出数据",
                0,
                Some(total_rows),
                Some("检测到重复行，回退为偏移分页重新导出".into()),
            );
            use std::io::{Seek, SeekFrom};
            file.set_len(0).map_err(exchange_error)?;
            file.seek(SeekFrom::Start(0)).map_err(exchange_error)?;
            rows_written = export_query_pages(
                &app,
                task_id,
                &session,
                database.clone(),
                &sql,
                page_size,
                &file,
                export_options,
                driver_kind,
                total_rows,
                0,
            )
            .await?;
        }
        Ok(rows_written)
    }
    .await;
    let rows_written = match export_result {
        Ok(rows) => rows,
        Err(error) => {
            if owns_transaction {
                let _ = session.rollback_transaction().await;
            }
            let _ = std::fs::remove_file(&temporary_path);
            return Err(payload(error));
        }
    };
    if owns_transaction && let Err(error) = session.commit_transaction().await {
        let _ = session.rollback_transaction().await;
        let _ = std::fs::remove_file(&temporary_path);
        return Err(payload(error));
    }
    drop(file);
    replace_file(&temporary_path, &output_path).map_err(payload)?;
    emit_export_progress(
        &app,
        task_id,
        "完成",
        rows_written,
        Some(rows_written),
        Some(format!("已导出 {rows_written} 行")),
    );
    Ok(ExportSummary {
        output_path: output_path.to_string_lossy().into_owned(),
        rows_written,
    })
}

/// Writes one full export pass of an arbitrary query result to `file`,
/// fetching `page_size` rows at a time. With `ordered_columns > 0` the pages
/// are ordered by the query's own output columns and fetched with keyset
/// pagination (seek past the last row) instead of `LIMIT/OFFSET`, which
/// turns the export from O(n²) into O(n) for seekable queries. Keyset mode
/// is abandoned permanently as soon as a page cannot yield a valid cursor
/// (NULL or otherwise unrepresentable key values), falling back to OFFSET.
/// With `ordered_columns == 0` the pass uses plain `LIMIT/OFFSET` without
/// ordering (e.g. when the result columns cannot be ordered at all).
#[allow(
    clippy::too_many_arguments,
    reason = "export pass bundles the session, target file and progress context"
)]
async fn export_query_pages(
    app: &AppHandle,
    task_id: Option<Uuid>,
    session: &Arc<dyn cockpit_core::DriverSession>,
    database: Option<String>,
    sql: &str,
    page_size: usize,
    file: &std::fs::File,
    options: ResultExportOptions,
    driver_kind: DatabaseKind,
    total_rows: u64,
    ordered_columns: usize,
) -> cockpit_core::Result<u64> {
    let mut offset = 0usize;
    let mut writer: Option<ResultStreamWriter<BufWriter<std::fs::File>>> = None;
    let mut keyset_active = ordered_columns > 0;
    let mut cursor: Option<KeysetCursor> = None;
    loop {
        let keyset = keyset_active.then_some(cursor.as_ref()).flatten();
        let page = session
            .execute(ExecuteQueryRequest {
                execution_id: Uuid::new_v4(),
                sql: paged_select_sql(sql, page_size, offset, keyset, driver_kind, ordered_columns),
                database: database.clone(),
                timeout_secs: None,
                allow_write: false,
                page_size,
                row_offset: 0,
            })
            .await?;
        if writer.is_none() {
            writer = Some(ResultStreamWriter::new(
                BufWriter::new(file.try_clone().map_err(exchange_error)?),
                &page.columns,
                &options,
            )?);
        }
        let row_count = page.rows.len();
        for row in &page.rows {
            writer
                .as_mut()
                .expect("writer initialized")
                .write_row(row)?;
        }
        offset += row_count;
        if keyset_active && let Some(last_row) = page.rows.last() {
            let next = KeysetCursor {
                columns: page
                    .columns
                    .iter()
                    .map(|column| column.name.clone())
                    .collect::<Vec<_>>(),
                last_values: last_row.to_vec(),
            };
            if keyset_condition_sql(&next.columns, &next.last_values, driver_kind).is_some() {
                cursor = Some(next);
            } else {
                keyset_active = false;
                cursor = None;
            }
        }
        emit_export_progress(
            app,
            task_id,
            "导出数据",
            offset as u64,
            Some(total_rows),
            Some(format!("已写入 {offset} / {total_rows} 行")),
        );
        if !page.has_more || row_count == 0 {
            break;
        }
    }
    let writer = writer.ok_or_else(|| CockpitError::Exchange("导出没有返回元数据".into()))?;
    let rows_written = writer.rows_written();
    let mut output = writer.finish()?;
    output.flush().map_err(exchange_error)?;
    output.get_ref().sync_all().map_err(exchange_error)?;
    Ok(rows_written)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseBackupSummary {
    pub output_path: String,
    pub tables_written: u64,
    pub objects_written: u64,
    pub rows_written: u64,
    pub checksum_sha256: String,
    pub bytes_written: u64,
    pub encrypted: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupDatabaseRequest {
    connection_id: Uuid,
    database: String,
    output_path: String,
    include_data: Option<bool>,
    compression: Option<String>,
    encryption_password: Option<String>,
    task_id: Option<Uuid>,
}

#[tauri::command]
pub async fn backup_database(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    request: BackupDatabaseRequest,
) -> CommandResult<DatabaseBackupSummary> {
    let BackupDatabaseRequest {
        connection_id,
        database,
        output_path,
        include_data,
        compression,
        encryption_password,
        task_id,
    } = request;
    let driver_kind = state
        .storage
        .get_connection(connection_id)
        .map_err(payload)?
        .ok_or_else(|| payload(CockpitError::NotFound("连接配置不存在".into())))?
        .driver_kind;
    let session = session(&state, connection_id).await?;
    if session.transaction_active().await {
        return Err(payload(CockpitError::Query(
            "备份前请先提交或回滚当前事务".into(),
        )));
    }
    let output_path = PathBuf::from(output_path);
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| payload(CockpitError::InvalidConfig("备份路径无效".into())))?;
    let file_name = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| payload(CockpitError::InvalidConfig("备份文件名无效".into())))?;
    let include_data = include_data.unwrap_or(true);
    let compression = compression.unwrap_or_else(|| "none".into());
    if compression != "none" && compression != "gzip" {
        return Err(payload(CockpitError::InvalidConfig(
            "备份压缩格式只支持 none 或 gzip".into(),
        )));
    }
    if encryption_password
        .as_deref()
        .is_some_and(|password| password.chars().count() < 8)
    {
        return Err(payload(CockpitError::InvalidConfig(
            "备份加密密码至少需要 8 个字符".into(),
        )));
    }
    let temporary_path = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary_path)
        .map_err(|error| payload(exchange_error(error)))?;
    let task_id = task_id.unwrap_or_else(Uuid::new_v4);
    let token = tokio_util::sync::CancellationToken::new();
    state.transfers.write().await.insert(task_id, token.clone());
    if let Err(error) = session.begin_read_transaction().await {
        let _ = std::fs::remove_file(&temporary_path);
        state.transfers.write().await.remove(&task_id);
        return Err(payload(error));
    }
    emit_transfer_progress(&app, task_id, "backup", "读取结构", 0, None, None);
    let backup_result: cockpit_core::Result<(u64, u64, u64)> = async {
        let mut output = BufWriter::new(file);
        writeln!(output, "-- Cockpit database backup").map_err(exchange_error)?;
        writeln!(output, "-- Database: {}\n", database).map_err(exchange_error)?;
        let mut captured_database_ddl = None;
        match driver_kind {
            DatabaseKind::MySql | DatabaseKind::MariaDb => {
                writeln!(output, "SET NAMES utf8mb4;").map_err(exchange_error)?;
                writeln!(output, "SET @COCKPIT_OLD_SQL_MODE=@@SESSION.SQL_MODE;")
                    .map_err(exchange_error)?;
                writeln!(output, "SET @COCKPIT_OLD_TIME_ZONE=@@SESSION.TIME_ZONE;")
                    .map_err(exchange_error)?;
                writeln!(
                    output,
                    "SET @COCKPIT_OLD_FOREIGN_KEY_CHECKS=@@SESSION.FOREIGN_KEY_CHECKS;"
                )
                .map_err(exchange_error)?;
                writeln!(output, "SET SESSION SQL_MODE='NO_AUTO_VALUE_ON_ZERO';")
                    .map_err(exchange_error)?;
                writeln!(output, "SET SESSION TIME_ZONE='+00:00';").map_err(exchange_error)?;
                writeln!(output, "SET FOREIGN_KEY_CHECKS=0;").map_err(exchange_error)?;
                let database_ddl = mysql_database_definition(&session, &database).await?;
                captured_database_ddl = Some(database_ddl.clone());
                writeln!(output, "{};", database_ddl.trim_end_matches(';'))
                    .map_err(exchange_error)?;
                if let Some(alter_database) = mysql_alter_database_definition(&database_ddl) {
                    writeln!(output, "{alter_database};").map_err(exchange_error)?;
                }
                writeln!(
                    output,
                    "USE {};\n",
                    backup_quote_identifier(&database, driver_kind)
                )
                .map_err(exchange_error)?;
            }
            DatabaseKind::PostgreSql => {
                writeln!(output, "SET client_encoding = 'UTF8';").map_err(exchange_error)?;
                writeln!(
                    output,
                    "CREATE SCHEMA IF NOT EXISTS {};",
                    backup_quote_identifier(&database, driver_kind)
                )
                .map_err(exchange_error)?;
                writeln!(
                    output,
                    "SET search_path TO {}, public;\n",
                    backup_quote_identifier(&database, driver_kind)
                )
                .map_err(exchange_error)?;
            }
            DatabaseKind::Sqlite => {
                writeln!(output, "PRAGMA foreign_keys=OFF;\n").map_err(exchange_error)?;
            }
        }

        let tables = list_all_tables(&session, &database).await?;
        let base_tables = tables
            .iter()
            .filter(|table| !table.table_type.contains("VIEW"))
            .collect::<Vec<_>>();
        let views = tables
            .iter()
            .filter(|table| table.table_type.contains("VIEW"))
            .collect::<Vec<_>>();
        let ordered_view_names =
            if matches!(driver_kind, DatabaseKind::MySql | DatabaseKind::MariaDb) {
                mysql_ordered_view_names(
                    &session,
                    &database,
                    &views
                        .iter()
                        .map(|view| view.name.clone())
                        .collect::<Vec<_>>(),
                )
                .await?
            } else {
                views.iter().map(|view| view.name.clone()).collect()
            };

        let mut table_details = HashMap::new();
        for (index, table) in base_tables.iter().enumerate() {
            if token.is_cancelled() {
                return Err(CockpitError::Query("备份任务已取消".into()));
            }
            let detail = session.table_detail(&database, &table.name).await?;
            writeln!(
                output,
                "DROP TABLE IF EXISTS {}.{};",
                backup_quote_identifier(&database, driver_kind),
                backup_quote_identifier(&table.name, driver_kind)
            )
            .map_err(exchange_error)?;
            writeln!(output, "{};\n", detail.ddl.trim_end_matches(';')).map_err(exchange_error)?;
            table_details.insert(table.name.clone(), detail);
            emit_transfer_progress(
                &app,
                task_id,
                "backup",
                "读取表结构",
                (index + 1) as u64,
                Some(base_tables.len() as u64),
                Some(table.name.clone()),
            );
        }
        if include_data && matches!(driver_kind, DatabaseKind::MySql | DatabaseKind::MariaDb) {
            let non_transactional = table_details
                .iter()
                .filter_map(|(name, detail)| match mysql_table_engine(&detail.ddl) {
                    Some(engine) if engine.eq_ignore_ascii_case("InnoDB") => None,
                    Some(engine) => Some(format!("{name} ({engine})")),
                    None => Some(format!("{name} (未知引擎)")),
                })
                .collect::<Vec<_>>();
            if !non_transactional.is_empty() {
                return Err(CockpitError::Exchange(format!(
                    "为避免生成不一致备份，暂不支持备份非 InnoDB 表的数据：{}",
                    non_transactional.join(", ")
                )));
            }
        }

        let mut rows_written = 0u64;
        if include_data {
            for (table_index, table) in base_tables.iter().enumerate() {
                if token.is_cancelled() {
                    return Err(CockpitError::Query("备份任务已取消".into()));
                }
                let page_size = 2_000usize;
                let mut offset = 0usize;
                let detail = table_details.get(&table.name).ok_or_else(|| {
                    CockpitError::Exchange(format!("备份缺少表结构：{}", table.name))
                })?;
                // Keyset pagination on the table key (same SQL builder as
                // exports); falls back to OFFSET when no cursor is available.
                let key_columns = table_keyset_columns(detail);
                let mut keyset_active = key_columns.is_some();
                let mut cursor: Option<KeysetCursor> = None;
                let first_page = session
                    .execute(ExecuteQueryRequest {
                        execution_id: Uuid::new_v4(),
                        sql: backup_table_page_sql(
                            &database,
                            &table.name,
                            detail,
                            page_size,
                            offset,
                            keyset_active.then_some(cursor.as_ref()).flatten(),
                            driver_kind,
                        )?,
                        database: Some(database.clone()),
                        timeout_secs: None,
                        allow_write: false,
                        page_size,
                        row_offset: 0,
                    })
                    .await?;
                let options = ResultExportOptions {
                    format: ExportFormat::Sql,
                    database_name: Some(database.clone()),
                    table_name: Some(table.name.clone()),
                    database_kind: driver_kind,
                };
                let mut writer = ResultStreamWriter::new(output, &first_page.columns, &options)?;
                let mut page = first_page;
                loop {
                    if token.is_cancelled() {
                        return Err(CockpitError::Query("备份任务已取消".into()));
                    }
                    let row_count = page.rows.len();
                    for row in &page.rows {
                        writer.write_row(row)?;
                    }
                    offset += row_count;
                    if keyset_active {
                        let next = match (key_columns.as_deref(), page.rows.last()) {
                            (Some(key_columns), Some(last_row)) => {
                                keyset_from_last_row(key_columns, &page.columns, last_row)
                            }
                            _ => None,
                        };
                        match next {
                            Some(next)
                                if keyset_condition_sql(
                                    &next.columns,
                                    &next.last_values,
                                    driver_kind,
                                )
                                .is_some() =>
                            {
                                cursor = Some(next);
                            }
                            _ => {
                                keyset_active = false;
                                cursor = None;
                            }
                        }
                    }
                    emit_transfer_progress(
                        &app,
                        task_id,
                        "backup",
                        "导出表数据",
                        (table_index + 1) as u64,
                        Some(base_tables.len() as u64),
                        Some(format!("{} · {} 行", table.name, offset)),
                    );
                    if !page.has_more || row_count == 0 {
                        break;
                    }
                    page = session
                        .execute(ExecuteQueryRequest {
                            execution_id: Uuid::new_v4(),
                            sql: backup_table_page_sql(
                                &database,
                                &table.name,
                                detail,
                                page_size,
                                offset,
                                keyset_active.then_some(cursor.as_ref()).flatten(),
                                driver_kind,
                            )?,
                            database: Some(database.clone()),
                            timeout_secs: None,
                            allow_write: false,
                            page_size,
                            row_offset: 0,
                        })
                        .await?;
                }
                rows_written += writer.rows_written();
                output = writer.finish()?;
                writeln!(output).map_err(exchange_error)?;
            }
        }

        let routines = session.list_routines(&database).await?;
        let triggers = session.list_triggers(&database).await?;
        let events = session.list_events(&database).await?;
        let mut objects_written = 0u64;
        let mut object_definitions = Vec::new();
        for routine in &routines {
            if token.is_cancelled() {
                return Err(CockpitError::Query("备份任务已取消".into()));
            }
            let kind = if routine.routine_type.eq_ignore_ascii_case("FUNCTION") {
                DatabaseObjectKind::Function
            } else {
                DatabaseObjectKind::Procedure
            };
            let definition = session
                .object_definition(&database, kind, &routine.name)
                .await?;
            object_definitions.push((kind, routine.name.clone(), definition.ddl.clone()));
            if matches!(driver_kind, DatabaseKind::MySql | DatabaseKind::MariaDb) {
                writeln!(
                    output,
                    "DROP {} IF EXISTS {}.{};",
                    if kind == DatabaseObjectKind::Function {
                        "FUNCTION"
                    } else {
                        "PROCEDURE"
                    },
                    backup_quote_identifier(&database, driver_kind),
                    backup_quote_identifier(&routine.name, driver_kind)
                )
                .map_err(exchange_error)?;
                write_delimited_definition(&mut output, &definition.ddl)?;
            } else {
                writeln!(output, "{};\n", definition.ddl.trim_end_matches(';'))
                    .map_err(exchange_error)?;
            }
            objects_written += 1;
        }
        for view_name in &ordered_view_names {
            if token.is_cancelled() {
                return Err(CockpitError::Query("备份任务已取消".into()));
            }
            let definition = session
                .object_definition(&database, DatabaseObjectKind::View, view_name)
                .await?;
            object_definitions.push((
                DatabaseObjectKind::View,
                view_name.clone(),
                definition.ddl.clone(),
            ));
            writeln!(
                output,
                "DROP VIEW IF EXISTS {}.{};",
                backup_quote_identifier(&database, driver_kind),
                backup_quote_identifier(view_name, driver_kind)
            )
            .map_err(exchange_error)?;
            writeln!(
                output,
                "{};\n",
                strip_definer_clause(&definition.ddl).trim_end_matches(';')
            )
            .map_err(exchange_error)?;
        }

        for trigger in &triggers {
            if token.is_cancelled() {
                return Err(CockpitError::Query("备份任务已取消".into()));
            }
            let definition = session
                .object_definition(&database, DatabaseObjectKind::Trigger, &trigger.name)
                .await?;
            object_definitions.push((
                DatabaseObjectKind::Trigger,
                trigger.name.clone(),
                definition.ddl.clone(),
            ));
            if matches!(driver_kind, DatabaseKind::MySql | DatabaseKind::MariaDb) {
                writeln!(
                    output,
                    "DROP TRIGGER IF EXISTS {}.{};",
                    backup_quote_identifier(&database, driver_kind),
                    backup_quote_identifier(&trigger.name, driver_kind)
                )
                .map_err(exchange_error)?;
                write_delimited_definition(&mut output, &definition.ddl)?;
            } else {
                writeln!(output, "{};\n", definition.ddl.trim_end_matches(';'))
                    .map_err(exchange_error)?;
            }
            objects_written += 1;
        }
        for event in &events {
            if token.is_cancelled() {
                return Err(CockpitError::Query("备份任务已取消".into()));
            }
            let definition = session
                .object_definition(&database, DatabaseObjectKind::Event, &event.name)
                .await?;
            object_definitions.push((
                DatabaseObjectKind::Event,
                event.name.clone(),
                definition.ddl.clone(),
            ));
            writeln!(
                output,
                "DROP EVENT IF EXISTS {}.{};",
                backup_quote_identifier(&database, driver_kind),
                backup_quote_identifier(&event.name, driver_kind)
            )
            .map_err(exchange_error)?;
            write_delimited_definition(&mut output, &definition.ddl)?;
            objects_written += 1;
        }
        if matches!(driver_kind, DatabaseKind::MySql | DatabaseKind::MariaDb) {
            let current_database_ddl = mysql_database_definition(&session, &database).await?;
            if captured_database_ddl.as_deref() != Some(current_database_ddl.as_str()) {
                return Err(CockpitError::Exchange(
                    "备份期间数据库默认字符集或排序规则发生变化，请重试".into(),
                ));
            }
            let final_tables = list_all_tables(&session, &database).await?;
            let original_names = tables
                .iter()
                .map(|table| (&table.name, &table.table_type))
                .collect::<Vec<_>>();
            let final_names = final_tables
                .iter()
                .map(|table| (&table.name, &table.table_type))
                .collect::<Vec<_>>();
            if original_names != final_names {
                return Err(CockpitError::Exchange(
                    "备份期间数据库对象列表发生变化，请重试".into(),
                ));
            }
            for table in &base_tables {
                let current = session.table_detail(&database, &table.name).await?;
                let original = table_details.get(&table.name).ok_or_else(|| {
                    CockpitError::Exchange(format!("备份缺少表结构：{}", table.name))
                })?;
                if current.ddl != original.ddl {
                    return Err(CockpitError::Exchange(format!(
                        "备份期间表结构发生变化：{}",
                        table.name
                    )));
                }
            }
            if session.list_routines(&database).await? != routines
                || session.list_triggers(&database).await? != triggers
                || session.list_events(&database).await? != events
            {
                return Err(CockpitError::Exchange(
                    "备份期间存储过程、触发器或事件列表发生变化，请重试".into(),
                ));
            }
            for (kind, name, ddl) in &object_definitions {
                let current = session.object_definition(&database, *kind, name).await?;
                if current.ddl != *ddl {
                    return Err(CockpitError::Exchange(format!(
                        "备份期间数据库对象定义发生变化：{name}"
                    )));
                }
            }
        }
        match driver_kind {
            DatabaseKind::MySql | DatabaseKind::MariaDb => {
                writeln!(
                    output,
                    "SET FOREIGN_KEY_CHECKS=@COCKPIT_OLD_FOREIGN_KEY_CHECKS;"
                )
                .map_err(exchange_error)?;
                writeln!(output, "SET SESSION TIME_ZONE=@COCKPIT_OLD_TIME_ZONE;")
                    .map_err(exchange_error)?;
                writeln!(output, "SET SESSION SQL_MODE=@COCKPIT_OLD_SQL_MODE;")
                    .map_err(exchange_error)?
            }
            DatabaseKind::Sqlite => {
                writeln!(output, "PRAGMA foreign_keys=ON;").map_err(exchange_error)?
            }
            DatabaseKind::PostgreSql => {}
        }
        output.flush().map_err(exchange_error)?;
        output.get_ref().sync_all().map_err(exchange_error)?;
        Ok((tables.len() as u64, objects_written, rows_written))
    }
    .await;
    let (tables_written, objects_written, rows_written) = match backup_result {
        Ok(summary) => summary,
        Err(error) => {
            let _ = session.rollback_transaction().await;
            let _ = std::fs::remove_file(&temporary_path);
            state.transfers.write().await.remove(&task_id);
            return Err(payload(error));
        }
    };
    if let Err(error) = session.commit_transaction().await {
        let _ = session.rollback_transaction().await;
        let _ = std::fs::remove_file(&temporary_path);
        state.transfers.write().await.remove(&task_id);
        return Err(payload(error));
    }
    let mut final_temporary_path = if compression == "gzip" {
        emit_transfer_progress(&app, task_id, "backup", "压缩", 0, None, None);
        let compressed_path = parent.join(format!(".{file_name}.{}.gz.tmp", Uuid::new_v4()));
        let compress_token = token.clone();
        let compress_input = temporary_path.clone();
        let compress_output = compressed_path.clone();
        // gzip is CPU/IO bound; run it on the blocking pool so the async
        // runtime is never stalled. The custom writer checks the
        // cancellation token on every compressed chunk, so cancel_transfer
        // stops the compression promptly instead of waiting for the whole
        // file to be processed.
        let compress_result =
            tokio::task::spawn_blocking(move || -> cockpit_core::Result<PathBuf> {
                let mut input = std::fs::File::open(&compress_input).map_err(exchange_error)?;
                let compressed_file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&compress_output)
                    .map_err(exchange_error)?;
                let writer = CancelCheckWriter {
                    inner: compressed_file,
                    token: compress_token,
                };
                let mut encoder = GzEncoder::new(writer, Compression::default());
                std::io::copy(&mut input, &mut encoder).map_err(exchange_error)?;
                let file = encoder.finish().map_err(exchange_error)?.into_inner();
                file.sync_all().map_err(exchange_error)?;
                Ok(compress_output)
            })
            .await
            .map_err(|error| payload(CockpitError::Exchange(error.to_string())))?;
        match compress_result {
            Ok(path) => {
                let _ = std::fs::remove_file(&temporary_path);
                path
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temporary_path);
                let _ = std::fs::remove_file(&compressed_path);
                state.transfers.write().await.remove(&task_id);
                return Err(payload(error));
            }
        }
    } else {
        temporary_path
    };
    let encrypted = encryption_password.is_some();
    if let Some(password) = encryption_password.as_deref() {
        emit_transfer_progress(&app, task_id, "backup", "加密", 0, None, None);
        let encrypted_path = parent.join(format!(".{file_name}.{}.enc.tmp", Uuid::new_v4()));
        let encrypt_token = token.clone();
        let encrypt_input = final_temporary_path.clone();
        let encrypt_output = encrypted_path.clone();
        let encrypt_password = password.to_string();
        let encrypt_result = tokio::task::spawn_blocking(move || {
            encrypt_backup_file(
                &encrypt_input,
                &encrypt_output,
                &encrypt_password,
                &encrypt_token,
            )
        })
        .await
        .map_err(|error| payload(CockpitError::Exchange(error.to_string())))?;
        if let Err(error) = encrypt_result {
            let _ = std::fs::remove_file(&final_temporary_path);
            let _ = std::fs::remove_file(&encrypted_path);
            state.transfers.write().await.remove(&task_id);
            return Err(payload(error));
        }
        let _ = std::fs::remove_file(&final_temporary_path);
        final_temporary_path = encrypted_path;
    }
    let hash_path = final_temporary_path.clone();
    let hash_token = token.clone();
    let hash_result = tokio::task::spawn_blocking(move || file_sha256(&hash_path, &hash_token))
        .await
        .map_err(|error| payload(CockpitError::Exchange(error.to_string())))?;
    let (checksum_sha256, bytes_written) = match hash_result {
        Ok(result) => result,
        Err(error) => {
            let _ = std::fs::remove_file(&final_temporary_path);
            state.transfers.write().await.remove(&task_id);
            return Err(payload(error));
        }
    };
    replace_file(&final_temporary_path, &output_path).map_err(payload)?;
    state.transfers.write().await.remove(&task_id);
    emit_transfer_progress(
        &app,
        task_id,
        "backup",
        "完成",
        1,
        Some(1),
        Some(checksum_sha256.clone()),
    );
    Ok(DatabaseBackupSummary {
        output_path: output_path.to_string_lossy().into_owned(),
        tables_written,
        objects_written,
        rows_written,
        checksum_sha256,
        bytes_written,
        encrypted,
    })
}

/// Write wrapper that aborts the surrounding `std::io::copy` as soon as the
/// backup task is cancelled, so gzip compression stops promptly instead of
/// blocking a worker thread until the whole file has been processed.
struct CancelCheckWriter<W> {
    inner: W,
    token: tokio_util::sync::CancellationToken,
}

impl<W> CancelCheckWriter<W> {
    fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for CancelCheckWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if self.token.is_cancelled() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "备份任务已取消",
            ));
        }
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn encryption_key(password: &str, salt: &[u8; 16]) -> cockpit_core::Result<[u8; 32]> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|error| CockpitError::Exchange(format!("无法生成备份加密密钥：{error}")))?;
    Ok(key)
}

fn encrypt_backup_file(
    input_path: &Path,
    output_path: &Path,
    password: &str,
    token: &tokio_util::sync::CancellationToken,
) -> cockpit_core::Result<()> {
    let mut salt = [0u8; 16];
    let mut nonce_prefix = [0u8; 4];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce_prefix);
    let key = encryption_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(exchange_error)?;
    let mut input = std::fs::File::open(input_path).map_err(exchange_error)?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output_path)
        .map_err(exchange_error)?;
    output
        .write_all(ENCRYPTED_BACKUP_MAGIC)
        .map_err(exchange_error)?;
    output.write_all(&salt).map_err(exchange_error)?;
    output.write_all(&nonce_prefix).map_err(exchange_error)?;
    let mut buffer = vec![0u8; ENCRYPTION_CHUNK_SIZE];
    let mut counter = 0u64;
    loop {
        if token.is_cancelled() {
            return Err(CockpitError::Query("备份任务已取消".into()));
        }
        let read = input.read(&mut buffer).map_err(exchange_error)?;
        if read == 0 {
            break;
        }
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&nonce_prefix);
        nonce[4..].copy_from_slice(&counter.to_be_bytes());
        let encrypted = cipher
            .encrypt(Nonce::from_slice(&nonce), &buffer[..read])
            .map_err(|_| CockpitError::Exchange("备份加密失败".into()))?;
        let length = u32::try_from(encrypted.len())
            .map_err(|_| CockpitError::Exchange("加密分块过大".into()))?;
        output
            .write_all(&length.to_be_bytes())
            .map_err(exchange_error)?;
        output.write_all(&encrypted).map_err(exchange_error)?;
        counter = counter
            .checked_add(1)
            .ok_or_else(|| CockpitError::Exchange("加密分块计数溢出".into()))?;
    }
    output
        .write_all(&0u32.to_be_bytes())
        .map_err(exchange_error)?;
    output.sync_all().map_err(exchange_error)
}

/// Decrypts a chunked encrypted backup file incrementally, so the encrypted
/// bytes never have to be held in memory together with the decrypted output
/// (a large backup would otherwise peak at ~1.5x its size). The plaintext is
/// still returned as one buffer because the SQL restore pipeline needs the
/// full text in memory to split statements.
fn decrypt_backup_file(path: &Path, password: &str) -> cockpit_core::Result<Vec<u8>> {
    let mut file = std::fs::File::open(path).map_err(exchange_error)?;
    let header_len = ENCRYPTED_BACKUP_MAGIC.len() + 16 + 4;
    let mut header = vec![0u8; header_len];
    file.read_exact(&mut header).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            CockpitError::Exchange("加密备份文件头无效".into())
        } else {
            exchange_error(error)
        }
    })?;
    if !header.starts_with(ENCRYPTED_BACKUP_MAGIC) {
        return Err(CockpitError::Exchange("加密备份文件头无效".into()));
    }
    let mut salt = [0u8; 16];
    salt.copy_from_slice(&header[ENCRYPTED_BACKUP_MAGIC.len()..ENCRYPTED_BACKUP_MAGIC.len() + 16]);
    let nonce_prefix = &header[ENCRYPTED_BACKUP_MAGIC.len() + 16..header_len];
    let key = encryption_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(exchange_error)?;
    let mut output = Vec::new();
    let mut counter = 0u64;
    loop {
        let mut length_bytes = [0u8; 4];
        file.read_exact(&mut length_bytes).map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                CockpitError::Exchange("加密备份文件不完整".into())
            } else {
                exchange_error(error)
            }
        })?;
        let length = u32::from_be_bytes(length_bytes) as usize;
        if length == 0 {
            let mut trailing = [0u8; 1];
            if file.read(&mut trailing).map_err(exchange_error)? != 0 {
                return Err(CockpitError::Exchange("加密备份尾部包含无效数据".into()));
            }
            break;
        }
        let mut encrypted = vec![0u8; length];
        file.read_exact(&mut encrypted).map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                CockpitError::Exchange("加密备份分块不完整".into())
            } else {
                exchange_error(error)
            }
        })?;
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(nonce_prefix);
        nonce[4..].copy_from_slice(&counter.to_be_bytes());
        let decrypted = cipher
            .decrypt(Nonce::from_slice(&nonce), encrypted.as_slice())
            .map_err(|_| CockpitError::Exchange("备份密码错误或文件已损坏".into()))?;
        output.extend_from_slice(&decrypted);
        counter = counter
            .checked_add(1)
            .ok_or_else(|| CockpitError::Exchange("加密分块计数溢出".into()))?;
    }
    Ok(output)
}

fn read_backup_text(path: &Path, password: Option<&str>) -> cockpit_core::Result<String> {
    // Peek at the magic without reading the whole file, so encrypted backups
    // are decrypted incrementally instead of loading the ciphertext fully.
    let mut magic = [0u8; ENCRYPTED_BACKUP_MAGIC.len()];
    let magic_len = {
        let mut file = std::fs::File::open(path).map_err(exchange_error)?;
        file.read(&mut magic).map_err(exchange_error)?
    };
    let mut bytes = if magic_len >= ENCRYPTED_BACKUP_MAGIC.len() && magic == *ENCRYPTED_BACKUP_MAGIC
    {
        let password = password
            .filter(|value| !value.is_empty())
            .ok_or_else(|| CockpitError::InvalidConfig("该备份已加密，请输入密码".into()))?;
        decrypt_backup_file(path, password)?
    } else {
        std::fs::read(path).map_err(exchange_error)?
    };
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut decoded = Vec::new();
        GzDecoder::new(Cursor::new(bytes))
            .read_to_end(&mut decoded)
            .map_err(exchange_error)?;
        bytes = decoded;
    }
    String::from_utf8(bytes)
        .map_err(|error| CockpitError::Exchange(format!("备份不是有效的 UTF-8 SQL：{error}")))
}

fn strip_cockpit_backup_header(sql: &str) -> (bool, &str) {
    match sql.split_once('\n') {
        Some((first_line, body)) if first_line.trim() == "-- Cockpit database backup" => {
            (true, body)
        }
        None if sql.trim() == "-- Cockpit database backup" => (true, ""),
        _ => (false, sql),
    }
}

fn file_sha256(
    path: &Path,
    token: &tokio_util::sync::CancellationToken,
) -> cockpit_core::Result<(String, u64)> {
    let mut file = std::fs::File::open(path).map_err(exchange_error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        if token.is_cancelled() {
            return Err(CockpitError::Query("备份任务已取消".into()));
        }
        let read = file.read(&mut buffer).map_err(exchange_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        bytes += read as u64;
    }
    Ok((format!("{:x}", digest.finalize()), bytes))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportFormat {
    Csv,
    Excel,
    Sql,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreviewRequest {
    pub input_path: String,
    pub format: ImportFormat,
    #[serde(default = "default_true")]
    pub has_headers: bool,
    pub sheet_name: Option<String>,
    pub delimiter: Option<String>,
    pub encoding: Option<String>,
    #[serde(default = "default_preview_rows")]
    pub preview_rows: usize,
}

fn default_preview_rows() -> usize {
    50
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub sheets: Vec<String>,
    pub selected_sheet: Option<String>,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub total_rows: usize,
    pub detected_delimiter: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportColumnMapping {
    pub source: String,
    pub target: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDataRequest {
    pub connection_id: Uuid,
    pub database: String,
    pub table: Option<String>,
    pub input_path: String,
    pub format: ImportFormat,
    #[serde(default = "default_true")]
    pub has_headers: bool,
    pub task_id: Option<Uuid>,
    pub sheet_name: Option<String>,
    pub delimiter: Option<String>,
    pub encoding: Option<String>,
    #[serde(default)]
    pub mappings: Vec<ImportColumnMapping>,
    #[serde(default = "default_null_values")]
    pub null_values: Vec<String>,
    #[serde(default = "default_true")]
    pub trim_values: bool,
    #[serde(default)]
    pub conflict_strategy: ImportConflictPolicy,
    #[serde(default = "default_import_batch_size")]
    pub batch_size: usize,
    #[serde(default)]
    pub continue_on_error: bool,
    pub encryption_password: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_null_values() -> Vec<String> {
    vec!["NULL".into(), "\\N".into()]
}

fn default_import_batch_size() -> usize {
    250
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRowError {
    pub row_number: usize,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub rows_imported: u64,
    pub rows_skipped: u64,
    pub statements_executed: u64,
    pub errors: Vec<ImportRowError>,
}

#[tauri::command]
pub async fn preview_import_data(request: ImportPreviewRequest) -> CommandResult<ImportPreview> {
    tauri::async_runtime::spawn_blocking(move || build_import_preview(&request))
        .await
        .map_err(|error| payload(CockpitError::Exchange(error.to_string())))?
        .map_err(payload)
}

#[tauri::command]
pub async fn import_data(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    request: ImportDataRequest,
) -> CommandResult<ImportSummary> {
    let task_id = request.task_id.unwrap_or_else(Uuid::new_v4);
    let kind = if matches!(request.format, ImportFormat::Sql) {
        "restore"
    } else {
        "import"
    };
    let token = tokio_util::sync::CancellationToken::new();
    state.transfers.write().await.insert(task_id, token.clone());
    emit_transfer_progress(&app, task_id, kind, "准备", 0, None, None);
    let result = import_data_inner(&app, &state, request, task_id, token).await;
    state.transfers.write().await.remove(&task_id);
    if let Err(error) = &result {
        let message = error.to_string();
        let phase = if message.contains("取消") {
            "已取消"
        } else {
            "失败"
        };
        emit_transfer_progress(&app, task_id, kind, phase, 0, None, Some(message));
    }
    result.map_err(payload)
}

async fn import_data_inner(
    app: &AppHandle,
    state: &AppState,
    request: ImportDataRequest,
    task_id: Uuid,
    token: tokio_util::sync::CancellationToken,
) -> cockpit_core::Result<ImportSummary> {
    let driver_kind = state
        .storage
        .get_connection(request.connection_id)?
        .ok_or_else(|| CockpitError::NotFound("连接配置不存在".into()))?
        .driver_kind;
    let session = state
        .sessions
        .read()
        .await
        .get(&request.connection_id)
        .cloned()
        .ok_or_else(|| CockpitError::Connection("连接尚未打开".into()))?;
    if session.transaction_active().await {
        return Err(CockpitError::Query("导入前请先提交或回滚当前事务".into()));
    }
    if matches!(request.format, ImportFormat::Sql) {
        emit_transfer_progress(
            app,
            task_id,
            "restore",
            "读取 SQL",
            0,
            None,
            Some("正在读取 SQL 文件".into()),
        );
        let input_path = PathBuf::from(&request.input_path);
        let encryption_password = request.encryption_password.clone();
        let sql_text = tokio::task::spawn_blocking(move || {
            read_backup_text(&input_path, encryption_password.as_deref())
        })
        .await
        .map_err(|error| CockpitError::Exchange(error.to_string()))??;
        if token.is_cancelled() {
            return Err(CockpitError::Query("SQL 文件执行已取消".into()));
        }
        let (cockpit_backup, sql_text) = strip_cockpit_backup_header(&sql_text);
        let execution_database = (!cockpit_backup).then_some(request.database.as_str());

        session.begin_transaction().await?;
        let mysql_session_settings =
            if matches!(driver_kind, DatabaseKind::MySql | DatabaseKind::MariaDb) {
                match capture_mysql_session_settings(&session).await {
                    Ok(settings) => Some(settings),
                    Err(error) => {
                        let _ = session.rollback_transaction().await;
                        return Err(error);
                    }
                }
            } else {
                None
            };
        let import_result: cockpit_core::Result<u64> = async {
            let mut parser = SqlScriptParser::default();
            let mut executed = 0u64;
            for line in sql_text.split_inclusive('\n') {
                if token.is_cancelled() {
                    return Err(CockpitError::Query("SQL 文件执行已取消".into()));
                }
                for statement in parser.push_line(line)? {
                    execute_import_statement(&session, execution_database, statement, executed + 1)
                        .await?;
                    executed += 1;
                    emit_transfer_progress(
                        app,
                        task_id,
                        "restore",
                        "执行 SQL",
                        executed,
                        None,
                        Some(format!("已执行 {executed} 条语句")),
                    );
                }
            }
            if let Some(statement) = parser.finish() {
                execute_import_statement(&session, execution_database, statement, executed + 1)
                    .await?;
                executed += 1;
                emit_transfer_progress(
                    app,
                    task_id,
                    "restore",
                    "执行 SQL",
                    executed,
                    None,
                    Some(format!("已执行 {executed} 条语句")),
                );
            }
            if executed == 0 {
                return Err(CockpitError::InvalidConfig("SQL 文件没有可执行语句".into()));
            }
            emit_transfer_progress(
                app,
                task_id,
                "restore",
                "提交事务",
                executed,
                None,
                Some(format!("已执行 {executed} 条语句，正在提交")),
            );
            Ok(executed)
        }
        .await;
        let settings_restore_result = if let Some(settings) = &mysql_session_settings {
            restore_mysql_session_settings(&session, settings).await
        } else {
            Ok(())
        };
        let executed = match import_result {
            Ok(executed) => executed,
            Err(error) => {
                let _ = session.rollback_transaction().await;
                return match settings_restore_result {
                    Ok(()) => Err(error),
                    Err(restore_error) => Err(CockpitError::Exchange(format!(
                        "{error}；同时无法恢复 MySQL 会话设置：{restore_error}"
                    ))),
                };
            }
        };
        if let Err(error) = settings_restore_result {
            let _ = session.rollback_transaction().await;
            return Err(CockpitError::Exchange(format!(
                "SQL 已执行，但无法恢复 MySQL 会话设置：{error}"
            )));
        }
        if let Err(error) = session.commit_transaction().await {
            let _ = session.rollback_transaction().await;
            return Err(error);
        }
        emit_transfer_progress(
            app,
            task_id,
            "restore",
            "完成",
            executed,
            None,
            Some(format!("已执行 {executed} 条语句")),
        );
        return Ok(ImportSummary {
            rows_imported: 0,
            rows_skipped: 0,
            statements_executed: executed,
            errors: Vec::new(),
        });
    }
    let table = request
        .table
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CockpitError::InvalidConfig("数据导入必须选择目标表".into()))?;
    let detail = session.table_detail(&request.database, table).await?;
    let fallback_columns = detail
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let valid_columns = detail
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<Vec<_>>();
    let parse_request = ImportParseRequest {
        input_path: request.input_path.clone(),
        format: request.format,
        has_headers: request.has_headers,
        fallback_columns,
        sheet_name: request.sheet_name.clone(),
        delimiter: request.delimiter.clone(),
        encoding: request.encoding.clone(),
        null_values: request.null_values.clone(),
        trim_values: request.trim_values,
    };
    let batch_size = request.batch_size.clamp(1, 2_000);
    // The file is parsed on a blocking thread and streamed to this loop in
    // batches (CSV row by row; Excel in row chunks), so imports no longer
    // load the whole file into memory.
    let (mut receiver, reader_task) = spawn_import_reader(parse_request, batch_size, token.clone());
    session.begin_transaction().await?;
    let mut imported = 0u64;
    let mut processed = 0u64;
    let mut errors = Vec::new();
    let mut projection: Option<ImportProjection> = None;
    let mut known_total: Option<u64> = None;
    loop {
        let message = receiver
            .recv()
            .await
            .ok_or_else(|| CockpitError::Exchange("导入文件读取线程意外终止".into()))?;
        let batch = match message {
            Ok(ImportStreamMessage::Start {
                columns: source_columns,
                total,
            }) => {
                let resolved =
                    resolve_import_projection(source_columns, &request.mappings, &valid_columns)?;
                if resolved.columns.is_empty() {
                    let _ = session.rollback_transaction().await;
                    return Err(CockpitError::InvalidConfig("导入文件没有可用列".into()));
                }
                known_total = total;
                projection = Some(resolved);
                continue;
            }
            Ok(ImportStreamMessage::Rows(rows)) => rows,
            Ok(ImportStreamMessage::End) => break,
            Err(error) => {
                let _ = session.rollback_transaction().await;
                return Err(error);
            }
        };
        if token.is_cancelled() {
            let _ = session.rollback_transaction().await;
            return Err(CockpitError::Query("导入任务已取消".into()));
        }
        let projection = projection
            .as_ref()
            .ok_or_else(|| CockpitError::Exchange("导入流缺少表头信息".into()))?;
        let batch = batch
            .iter()
            .map(|row| project_import_row(projection, row))
            .collect::<Vec<_>>();
        let result = session
            .insert_rows_with_policy(
                &request.database,
                table,
                &projection.columns,
                &batch,
                request.conflict_strategy,
            )
            .await;
        match result {
            Ok(affected) => imported += affected,
            Err(error) if !request.continue_on_error => {
                let _ = session.rollback_transaction().await;
                return Err(error);
            }
            Err(_) => {
                for (offset, row) in batch.iter().enumerate() {
                    match session
                        .insert_rows_with_policy(
                            &request.database,
                            table,
                            &projection.columns,
                            std::slice::from_ref(row),
                            request.conflict_strategy,
                        )
                        .await
                    {
                        Ok(affected) => imported += affected,
                        Err(error) => {
                            if errors.len() < 200 {
                                errors.push(ImportRowError {
                                    row_number: processed as usize
                                        + offset
                                        + if request.has_headers { 2 } else { 1 },
                                    message: error.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
        processed += batch.len() as u64;
        emit_transfer_progress(
            app,
            task_id,
            "import",
            "写入数据",
            processed,
            known_total,
            None,
        );
    }
    reader_task
        .await
        .map_err(|error| CockpitError::Exchange(format!("导入文件读取线程异常：{error}")))?;
    if let Err(error) = session.commit_transaction().await {
        let _ = session.rollback_transaction().await;
        return Err(error);
    }
    // CSV streaming does not know the total up front; use the processed
    // count so the summary semantics match the previous whole-file import.
    let total = known_total.unwrap_or(processed);
    emit_transfer_progress(app, task_id, "import", "完成", total, Some(total), None);
    Ok(ImportSummary {
        rows_imported: imported,
        rows_skipped: total.saturating_sub(imported.min(total)),
        statements_executed: 0,
        errors,
    })
}

struct MySqlSessionSettings {
    sql_mode: String,
    time_zone: String,
    foreign_key_checks: u64,
}

async fn capture_mysql_session_settings(
    session: &Arc<dyn cockpit_core::DriverSession>,
) -> cockpit_core::Result<MySqlSessionSettings> {
    let page = session
        .execute(ExecuteQueryRequest {
            execution_id: Uuid::new_v4(),
            sql: "SELECT @@SESSION.SQL_MODE, @@SESSION.TIME_ZONE, @@SESSION.FOREIGN_KEY_CHECKS"
                .into(),
            database: None,
            timeout_secs: None,
            allow_write: false,
            page_size: 1,
            row_offset: 0,
        })
        .await?;
    let row = page
        .rows
        .first()
        .ok_or_else(|| CockpitError::Exchange("无法读取 MySQL 会话设置".into()))?;
    let sql_mode = mysql_setting_text(row.first(), "SQL_MODE")?;
    let time_zone = mysql_setting_text(row.get(1), "TIME_ZONE")?;
    let foreign_key_checks = mysql_setting_text(row.get(2), "FOREIGN_KEY_CHECKS")?
        .parse::<u64>()
        .map_err(|_| CockpitError::Exchange("MySQL FOREIGN_KEY_CHECKS 值无效".into()))?;
    Ok(MySqlSessionSettings {
        sql_mode,
        time_zone,
        foreign_key_checks,
    })
}

fn mysql_setting_text(value: Option<&CellValue>, name: &str) -> cockpit_core::Result<String> {
    value
        .and_then(cell_value_text)
        .map(str::to_string)
        .ok_or_else(|| CockpitError::Exchange(format!("无法读取 MySQL {name} 会话设置")))
}

fn cell_value_text(value: &CellValue) -> Option<&str> {
    match value {
        CellValue::Text(value)
        | CellValue::Signed(value)
        | CellValue::Unsigned(value)
        | CellValue::Decimal(value)
        | CellValue::Date(value)
        | CellValue::Time(value)
        | CellValue::DateTime(value)
        | CellValue::Json(value) => Some(value),
        CellValue::Bytes {
            preview: Some(value),
            ..
        } => Some(value),
        _ => None,
    }
}

fn mysql_utf8_sql_literal(value: &str) -> String {
    let hex = value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    format!("CONVERT(X'{hex}' USING utf8mb4)")
}

async fn restore_mysql_session_settings(
    session: &Arc<dyn cockpit_core::DriverSession>,
    settings: &MySqlSessionSettings,
) -> cockpit_core::Result<()> {
    let statements = [
        format!(
            "SET SESSION FOREIGN_KEY_CHECKS={}",
            settings.foreign_key_checks
        ),
        format!(
            "SET SESSION TIME_ZONE={}",
            mysql_utf8_sql_literal(&settings.time_zone)
        ),
        format!(
            "SET SESSION SQL_MODE={}",
            mysql_utf8_sql_literal(&settings.sql_mode)
        ),
    ];
    for statement in statements {
        execute_import_statement(session, None, statement, 0).await?;
    }
    Ok(())
}

struct ImportParseRequest {
    input_path: String,
    format: ImportFormat,
    has_headers: bool,
    fallback_columns: Vec<String>,
    sheet_name: Option<String>,
    delimiter: Option<String>,
    encoding: Option<String>,
    null_values: Vec<String>,
    trim_values: bool,
}

/// Messages sent by the blocking import reader to the async insert loop.
/// The first message is always `Start` (source columns plus the total row
/// count when it is known up front), followed by `Rows` batches and `End`.
enum ImportStreamMessage {
    Start {
        columns: Vec<String>,
        total: Option<u64>,
    },
    Rows(Vec<Vec<CellValue>>),
    End,
}

type ImportStreamResult = std::result::Result<ImportStreamMessage, CockpitError>;

/// Excel (calamine) materializes the whole sheet in memory and cannot be
/// streamed; cap the number of data rows so a misconfigured import cannot
/// exhaust the process memory.
const MAX_IMPORT_ROWS: usize = 1_000_000;

/// Starts a blocking reader thread that parses the import file and sends
/// `Start`/`Rows`/`End` batches over a bounded channel. The async insert
/// loop consumes the batches so the file is never loaded in full at once
/// (CSV is parsed row by row; Excel stays bounded by `MAX_IMPORT_ROWS`).
fn spawn_import_reader(
    request: ImportParseRequest,
    batch_size: usize,
    token: tokio_util::sync::CancellationToken,
) -> (
    tokio::sync::mpsc::Receiver<ImportStreamResult>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let handle = tokio::task::spawn_blocking(move || {
        let result = match request.format {
            ImportFormat::Csv => read_csv_stream(&request, batch_size, &token, &tx),
            ImportFormat::Excel => read_excel_stream(&request, batch_size, &token, &tx),
            ImportFormat::Sql => unreachable!("SQL restore is handled separately"),
        };
        if let Err(error) = result {
            let _ = tx.blocking_send(Err(error));
        }
    });
    (rx, handle)
}

fn read_csv_stream(
    request: &ImportParseRequest,
    batch_size: usize,
    token: &tokio_util::sync::CancellationToken,
    tx: &tokio::sync::mpsc::Sender<ImportStreamResult>,
) -> cockpit_core::Result<()> {
    let file = std::fs::File::open(&request.input_path).map_err(exchange_error)?;
    let transcoding =
        TranscodingReader::new(file, request.encoding.as_deref()).map_err(exchange_error)?;
    let mut buffered = std::io::BufReader::new(transcoding);
    // Sniff the delimiter from the first decoded chunk, then hand the whole
    // stream (sniffed prefix + remainder) to the CSV reader.
    let mut sniff = Vec::with_capacity(64 * 1024);
    std::io::Read::by_ref(&mut buffered)
        .take(64 * 1024)
        .read_to_end(&mut sniff)
        .map_err(exchange_error)?;
    let sniff_text = String::from_utf8_lossy(&sniff);
    let delimiter = delimiter_byte(request.delimiter.as_deref(), &sniff_text)?;
    let source = Cursor::new(sniff).chain(buffered);
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(request.has_headers)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(source);
    let columns = if request.has_headers {
        reader
            .headers()
            .map_err(exchange_error)?
            .iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
    } else {
        request.fallback_columns.clone()
    };
    if tx
        .blocking_send(Ok(ImportStreamMessage::Start {
            columns,
            total: None,
        }))
        .is_err()
    {
        return Ok(());
    }
    let mut batch = Vec::with_capacity(batch_size);
    for record in reader.records() {
        if token.is_cancelled() {
            return Err(CockpitError::Query("导入任务已取消".into()));
        }
        let record = record.map_err(exchange_error)?;
        batch.push(
            record
                .iter()
                .map(|value| import_text_cell(value, &request.null_values, request.trim_values))
                .collect::<Vec<_>>(),
        );
        if batch.len() >= batch_size {
            let rows = std::mem::take(&mut batch);
            if tx
                .blocking_send(Ok(ImportStreamMessage::Rows(rows)))
                .is_err()
            {
                return Ok(());
            }
        }
    }
    if !batch.is_empty()
        && tx
            .blocking_send(Ok(ImportStreamMessage::Rows(batch)))
            .is_err()
    {
        return Ok(());
    }
    if tx.blocking_send(Ok(ImportStreamMessage::End)).is_err() {
        return Ok(());
    }
    Ok(())
}

fn read_excel_stream(
    request: &ImportParseRequest,
    batch_size: usize,
    token: &tokio_util::sync::CancellationToken,
    tx: &tokio::sync::mpsc::Sender<ImportStreamResult>,
) -> cockpit_core::Result<()> {
    let mut workbook = open_workbook_auto(&request.input_path).map_err(exchange_error)?;
    let sheet_names = workbook.sheet_names().to_vec();
    let sheet_name = sheet_names
        .iter()
        .find(|name| request.sheet_name.as_deref() == Some(name.as_str()))
        .or_else(|| sheet_names.first())
        .cloned()
        .ok_or_else(|| CockpitError::Exchange("Excel 文件没有工作表".into()))?;
    let range = workbook
        .worksheet_range(&sheet_name)
        .map_err(exchange_error)?;
    let data_rows = range
        .height()
        .saturating_sub(usize::from(request.has_headers));
    if data_rows > MAX_IMPORT_ROWS {
        return Err(CockpitError::Exchange(format!(
            "Excel 工作表超过 {MAX_IMPORT_ROWS} 行上限（当前 {data_rows} 行），无法导入"
        )));
    }
    let mut rows = range.rows();
    let columns = if request.has_headers {
        rows.next()
            .map(|row| row.iter().map(ToString::to_string).collect())
            .unwrap_or_default()
    } else {
        request.fallback_columns.clone()
    };
    if tx
        .blocking_send(Ok(ImportStreamMessage::Start {
            columns,
            total: Some(data_rows as u64),
        }))
        .is_err()
    {
        return Ok(());
    }
    let mut batch = Vec::with_capacity(batch_size);
    for row in rows {
        if token.is_cancelled() {
            return Err(CockpitError::Query("导入任务已取消".into()));
        }
        batch.push(
            row.iter()
                .map(|value| match value {
                    Data::Empty => CellValue::Null,
                    _ => import_text_cell(
                        &value.to_string(),
                        &request.null_values,
                        request.trim_values,
                    ),
                })
                .collect::<Vec<_>>(),
        );
        if batch.len() >= batch_size {
            let rows = std::mem::take(&mut batch);
            if tx
                .blocking_send(Ok(ImportStreamMessage::Rows(rows)))
                .is_err()
            {
                return Ok(());
            }
        }
    }
    if !batch.is_empty()
        && tx
            .blocking_send(Ok(ImportStreamMessage::Rows(batch)))
            .is_err()
    {
        return Ok(());
    }
    if tx.blocking_send(Ok(ImportStreamMessage::End)).is_err() {
        return Ok(());
    }
    Ok(())
}

/// Incremental transcoding reader: decodes the raw file bytes (UTF-8 strict,
/// UTF-8 lossy, or GB18030) into UTF-8 on the fly so CSV files never need to
/// be loaded into memory as a whole. Strict UTF-8 mode reports invalid input
/// as an I/O error, mirroring the previous whole-file behavior.
struct TranscodingReader<R> {
    inner: R,
    decoder: encoding_rs::Decoder,
    raw_buffer: Vec<u8>,
    raw_pos: usize,
    raw_len: usize,
    out_buffer: Vec<u8>,
    out_pos: usize,
    eof: bool,
    strict: bool,
}

impl<R: Read> TranscodingReader<R> {
    fn new(inner: R, encoding: Option<&str>) -> std::io::Result<Self> {
        let encoding = encoding.unwrap_or("utf-8");
        let decoder = match encoding {
            "utf-8" => encoding_rs::UTF_8.new_decoder_without_bom_handling(),
            "utf-8-lossy" => encoding_rs::UTF_8.new_decoder_without_bom_handling(),
            "gb18030" => encoding_rs::GB18030.new_decoder_without_bom_handling(),
            value => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("不支持的文本编码：{value}"),
                ));
            }
        };
        Ok(Self {
            inner,
            decoder,
            raw_buffer: vec![0u8; 64 * 1024],
            raw_pos: 0,
            raw_len: 0,
            out_buffer: Vec::new(),
            out_pos: 0,
            eof: false,
            strict: encoding == "utf-8",
        })
    }

    /// Decodes more input into `out_buffer`; returns `false` on EOF.
    fn fill(&mut self) -> std::io::Result<bool> {
        let mut decoded = [0u8; 8192];
        loop {
            if self.raw_pos < self.raw_len {
                let (result, read, written, had_errors) = self.decoder.decode_to_utf8(
                    &self.raw_buffer[self.raw_pos..self.raw_len],
                    &mut decoded,
                    false,
                );
                self.raw_pos += read;
                if had_errors && self.strict {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "CSV 不是有效 UTF-8；可改用宽松 UTF-8 或 GB18030",
                    ));
                }
                if written > 0 {
                    self.out_buffer.extend_from_slice(&decoded[..written]);
                    return Ok(true);
                }
                if matches!(result, encoding_rs::CoderResult::InputEmpty) {
                    // All buffered raw bytes consumed; need more input.
                }
            }
            if self.eof {
                let mut tail = [0u8; 16];
                let (_, _, written, had_errors) = self.decoder.decode_to_utf8(&[], &mut tail, true);
                if had_errors && self.strict {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "CSV 不是有效 UTF-8；可改用宽松 UTF-8 或 GB18030",
                    ));
                }
                if written > 0 {
                    self.out_buffer.extend_from_slice(&tail[..written]);
                    return Ok(true);
                }
                return Ok(false);
            }
            let read = self.inner.read(&mut self.raw_buffer)?;
            self.raw_pos = 0;
            self.raw_len = read;
            if read == 0 {
                self.eof = true;
            }
        }
    }
}

impl<R: Read> Read for TranscodingReader<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.out_pos < self.out_buffer.len() {
                let count = std::cmp::min(out.len(), self.out_buffer.len() - self.out_pos);
                out[..count].copy_from_slice(&self.out_buffer[self.out_pos..self.out_pos + count]);
                self.out_pos += count;
                if self.out_pos == self.out_buffer.len() {
                    self.out_buffer.clear();
                    self.out_pos = 0;
                }
                return Ok(count);
            }
            if !self.fill()? {
                return Ok(0);
            }
        }
    }
}

/// Resolved column projection for one import: target column names plus the
/// source index each one is read from.
struct ImportProjection {
    columns: Vec<String>,
    indices: Vec<usize>,
}

fn resolve_import_projection(
    source_columns: Vec<String>,
    mappings: &[ImportColumnMapping],
    valid_columns: &[&str],
) -> cockpit_core::Result<ImportProjection> {
    let resolved = if mappings.is_empty() {
        source_columns
            .iter()
            .enumerate()
            .filter(|(_, column)| valid_columns.contains(&column.as_str()))
            .map(|(index, column)| (index, column.clone()))
            .collect::<Vec<_>>()
    } else {
        mappings
            .iter()
            .filter_map(|mapping| mapping.target.as_ref().map(|target| (mapping, target)))
            .map(|(mapping, target)| {
                if !valid_columns.contains(&target.as_str()) {
                    return Err(CockpitError::InvalidConfig(format!(
                        "目标字段不存在：{target}"
                    )));
                }
                let index = source_columns
                    .iter()
                    .position(|source| source == &mapping.source)
                    .ok_or_else(|| {
                        CockpitError::InvalidConfig(format!("来源字段不存在：{}", mapping.source))
                    })?;
                Ok((index, target.clone()))
            })
            .collect::<cockpit_core::Result<Vec<_>>>()?
    };
    let mut seen = std::collections::HashSet::new();
    if resolved
        .iter()
        .any(|(_, target)| !seen.insert(target.clone()))
    {
        return Err(CockpitError::InvalidConfig(
            "同一目标字段不能映射多次".into(),
        ));
    }
    Ok(ImportProjection {
        columns: resolved
            .iter()
            .map(|(_, target)| target.clone())
            .collect::<Vec<_>>(),
        indices: resolved.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
    })
}

fn project_import_row(projection: &ImportProjection, row: &[CellValue]) -> Vec<CellValue> {
    projection
        .indices
        .iter()
        .map(|index| row.get(*index).cloned().unwrap_or(CellValue::Null))
        .collect::<Vec<_>>()
}

fn import_text_cell(value: &str, null_values: &[String], trim_values: bool) -> CellValue {
    let value = if trim_values { value.trim() } else { value };
    if null_values.iter().any(|null| null == value) {
        CellValue::Null
    } else {
        CellValue::Text(value.into())
    }
}

fn delimiter_byte(delimiter: Option<&str>, contents: &str) -> cockpit_core::Result<u8> {
    if let Some(value) = delimiter.filter(|value| !value.is_empty() && *value != "auto") {
        let bytes = value.as_bytes();
        if bytes.len() != 1 || !bytes[0].is_ascii() {
            return Err(CockpitError::InvalidConfig(
                "分隔符必须是单个 ASCII 字符".into(),
            ));
        }
        return Ok(bytes[0]);
    }
    let first_line = contents.lines().next().unwrap_or_default();
    Ok(*b",\t;|"
        .iter()
        .max_by_key(|candidate| {
            first_line
                .as_bytes()
                .iter()
                .filter(|byte| *byte == *candidate)
                .count()
        })
        .unwrap_or(&b','))
}

fn build_import_preview(request: &ImportPreviewRequest) -> cockpit_core::Result<ImportPreview> {
    match request.format {
        ImportFormat::Sql => Err(CockpitError::InvalidConfig("SQL 文件不支持表格预览".into())),
        ImportFormat::Csv => {
            let file = std::fs::File::open(&request.input_path).map_err(exchange_error)?;
            let transcoding = TranscodingReader::new(file, request.encoding.as_deref())
                .map_err(exchange_error)?;
            let mut buffered = std::io::BufReader::new(transcoding);
            let mut sniff = Vec::with_capacity(64 * 1024);
            std::io::Read::by_ref(&mut buffered)
                .take(64 * 1024)
                .read_to_end(&mut sniff)
                .map_err(exchange_error)?;
            let sniff_text = String::from_utf8_lossy(&sniff);
            let delimiter = delimiter_byte(request.delimiter.as_deref(), &sniff_text)?;
            let source = Cursor::new(sniff).chain(buffered);
            let mut reader = csv::ReaderBuilder::new()
                .has_headers(request.has_headers)
                .flexible(true)
                .delimiter(delimiter)
                .from_reader(source);
            let mut columns = if request.has_headers {
                reader
                    .headers()
                    .map_err(exchange_error)?
                    .iter()
                    .map(str::to_string)
                    .collect()
            } else {
                Vec::new()
            };
            let mut rows = Vec::new();
            let mut total_rows = 0;
            for record in reader.records() {
                let record = record.map_err(exchange_error)?;
                if columns.is_empty() {
                    columns = (1..=record.len())
                        .map(|index| format!("column_{index}"))
                        .collect();
                }
                if rows.len() < request.preview_rows.clamp(1, 200) {
                    rows.push(record.iter().map(str::to_string).collect());
                }
                total_rows += 1;
            }
            Ok(ImportPreview {
                sheets: Vec::new(),
                selected_sheet: None,
                columns,
                rows,
                total_rows,
                detected_delimiter: Some((delimiter as char).to_string()),
            })
        }
        ImportFormat::Excel => {
            let mut workbook = open_workbook_auto(&request.input_path).map_err(exchange_error)?;
            let sheets = workbook.sheet_names().to_vec();
            let selected_sheet = request
                .sheet_name
                .as_ref()
                .filter(|name| sheets.contains(name))
                .cloned()
                .or_else(|| sheets.first().cloned())
                .ok_or_else(|| CockpitError::Exchange("Excel 文件没有工作表".into()))?;
            let range = workbook
                .worksheet_range(&selected_sheet)
                .map_err(exchange_error)?;
            let total_rows = range
                .height()
                .saturating_sub(usize::from(request.has_headers));
            let mut source = range.rows();
            let mut columns = if request.has_headers {
                source
                    .next()
                    .map(|row| row.iter().map(ToString::to_string).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let rows = source
                .take(request.preview_rows.clamp(1, 200))
                .map(|row| row.iter().map(ToString::to_string).collect::<Vec<_>>())
                .collect::<Vec<_>>();
            if columns.is_empty() {
                let width = rows.first().map(Vec::len).unwrap_or(range.width());
                columns = (1..=width).map(|index| format!("column_{index}")).collect();
            }
            Ok(ImportPreview {
                sheets,
                selected_sheet: Some(selected_sheet),
                columns,
                rows,
                total_rows,
                detected_delimiter: None,
            })
        }
    }
}

fn export_page(
    output_path: PathBuf,
    page: QueryResultPage,
    options: ResultExportOptions,
) -> cockpit_core::Result<ExportSummary> {
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| CockpitError::InvalidConfig("导出路径无效".into()))?;
    let file_name = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CockpitError::InvalidConfig("导出文件名无效".into()))?;
    let temporary_path = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary_path)
        .map_err(exchange_error)?;
    let export_result = write_result_page(&mut file, &page, &options)
        .and_then(|_| file.sync_all().map_err(exchange_error));
    if let Err(error) = export_result {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error);
    }
    drop(file);
    replace_file(&temporary_path, &output_path)?;
    Ok(ExportSummary {
        output_path: output_path.to_string_lossy().into_owned(),
        rows_written: page.rows.len() as u64,
    })
}

fn replace_file(temporary_path: &Path, output_path: &Path) -> cockpit_core::Result<()> {
    match std::fs::rename(temporary_path, output_path) {
        Ok(()) => Ok(()),
        Err(error)
            if cfg!(windows)
                && matches!(
                    error.kind(),
                    std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
                ) =>
        {
            // Windows `rename` cannot overwrite an existing destination
            // (`MoveFileEx` without `MOVEFILE_REPLACE_EXISTING`). std does
            // not expose ReplaceFile, so remove the destination first and
            // retry with a single rename.
            if let Err(remove_error) = std::fs::remove_file(output_path)
                && remove_error.kind() != std::io::ErrorKind::NotFound
            {
                let _ = std::fs::remove_file(temporary_path);
                return Err(exchange_error(remove_error));
            }
            std::fs::rename(temporary_path, output_path).map_err(|error| {
                let _ = std::fs::remove_file(temporary_path);
                exchange_error(error)
            })
        }
        Err(error) => {
            let _ = std::fs::remove_file(temporary_path);
            Err(exchange_error(error))
        }
    }
}

fn strip_definer_clause(ddl: &str) -> String {
    let mut result = ddl.to_string();
    loop {
        let upper = result.to_ascii_uppercase();
        let Some(start) = upper.find("DEFINER=") else {
            break;
        };
        let end = result[start..]
            .find(char::is_whitespace)
            .map(|offset| start + offset)
            .unwrap_or(result.len());
        result.replace_range(start..end, "");
    }
    result
}

fn write_delimited_definition(output: &mut impl Write, ddl: &str) -> cockpit_core::Result<()> {
    let ddl = strip_definer_clause(ddl);
    let mut delimiter = "$$".to_string();
    let mut suffix = 1u32;
    while ddl.contains(&delimiter) {
        delimiter = format!("$COCKPIT_{suffix}$");
        suffix += 1;
    }
    writeln!(output, "DELIMITER {delimiter}").map_err(exchange_error)?;
    writeln!(output, "{}{delimiter}", ddl.trim_end_matches(';')).map_err(exchange_error)?;
    writeln!(output, "DELIMITER ;\n").map_err(exchange_error)
}

async fn execute_import_statement(
    session: &Arc<dyn cockpit_core::DriverSession>,
    database: Option<&str>,
    statement: String,
    statement_number: u64,
) -> cockpit_core::Result<()> {
    session
        .execute(ExecuteQueryRequest {
            execution_id: Uuid::new_v4(),
            sql: statement,
            database: database.map(str::to_string),
            timeout_secs: None,
            allow_write: true,
            page_size: 1,
            row_offset: 0,
        })
        .await
        .map(|_| ())
        .map_err(|error| {
            CockpitError::Exchange(format!(
                "SQL 文件在第 {statement_number} 条语句执行失败：{error}"
            ))
        })
}

#[cfg(test)]
fn split_sql_script(script: &str) -> cockpit_core::Result<Vec<String>> {
    let mut parser = SqlScriptParser::default();
    let mut statements = Vec::new();
    for line in script.split_inclusive('\n') {
        statements.extend(parser.push_line(line)?);
    }
    if let Some(statement) = parser.finish() {
        statements.push(statement);
    }
    Ok(statements)
}

struct SqlScriptParser {
    delimiter: String,
    buffer: String,
}

impl Default for SqlScriptParser {
    fn default() -> Self {
        Self {
            delimiter: ";".into(),
            buffer: String::new(),
        }
    }
}

impl SqlScriptParser {
    fn push_line(&mut self, line: &str) -> cockpit_core::Result<Vec<String>> {
        let trimmed = line.trim();
        if trimmed
            .get(..9)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("DELIMITER"))
            && trimmed.chars().nth(9).is_some_and(char::is_whitespace)
        {
            if fragment_has_sql(&self.buffer) {
                return Err(CockpitError::InvalidConfig(
                    "DELIMITER 指令前存在未结束的 SQL 语句".into(),
                ));
            }
            self.buffer.clear();
            let next = trimmed[9..].trim();
            if next.is_empty() || next.len() > 16 {
                return Err(CockpitError::InvalidConfig(
                    "DELIMITER 必须是 1 到 16 个字符".into(),
                ));
            }
            self.delimiter = next.to_string();
            return Ok(Vec::new());
        }
        self.buffer.push_str(line);
        if !line.contains(&self.delimiter) {
            return Ok(Vec::new());
        }
        let mut statements = Vec::new();
        while let Some(index) = find_sql_delimiter(&self.buffer, &self.delimiter) {
            let statement = self.buffer[..index].trim();
            if fragment_has_sql(statement) {
                statements.push(statement.to_string());
            }
            self.buffer.drain(..index + self.delimiter.len());
        }
        Ok(statements)
    }

    fn finish(self) -> Option<String> {
        fragment_has_sql(&self.buffer).then(|| self.buffer.trim().to_string())
    }
}

fn find_sql_delimiter(input: &str, delimiter: &str) -> Option<usize> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ScanState {
        Normal,
        Single,
        Double,
        Backtick,
        LineComment,
        BlockComment,
    }

    let bytes = input.as_bytes();
    let delimiter = delimiter.as_bytes();
    let mut state = ScanState::Normal;
    let mut dollar_tag: Option<Vec<u8>> = None;
    let mut index = 0usize;
    while index < bytes.len() {
        if let Some(tag) = dollar_tag.as_ref() {
            if bytes[index..].starts_with(tag) {
                index += tag.len();
                dollar_tag = None;
            } else {
                index += 1;
            }
            continue;
        }
        match state {
            ScanState::Normal => {
                if bytes[index..].starts_with(delimiter) {
                    return Some(index);
                }
                match bytes[index] {
                    b'\'' => state = ScanState::Single,
                    b'"' => state = ScanState::Double,
                    b'`' => state = ScanState::Backtick,
                    b'#' => state = ScanState::LineComment,
                    b'-' if bytes.get(index + 1) == Some(&b'-') => {
                        state = ScanState::LineComment;
                        index += 1;
                    }
                    b'/' if bytes.get(index + 1) == Some(&b'*') => {
                        state = ScanState::BlockComment;
                        index += 1;
                    }
                    b'$' => {
                        if let Some(end) = bytes[index + 1..].iter().position(|byte| *byte == b'$')
                        {
                            let end = index + 1 + end;
                            if bytes[index + 1..end]
                                .iter()
                                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                            {
                                let tag = bytes[index..=end].to_vec();
                                index = end;
                                dollar_tag = Some(tag);
                            }
                        }
                    }
                    _ => {}
                }
            }
            ScanState::Single | ScanState::Double | ScanState::Backtick => {
                let quote = match state {
                    ScanState::Single => b'\'',
                    ScanState::Double => b'"',
                    _ => b'`',
                };
                if bytes[index] == b'\\' && state != ScanState::Backtick {
                    index += usize::from(index + 1 < bytes.len());
                } else if bytes[index] == quote {
                    if bytes.get(index + 1) == Some(&quote) {
                        index += 1;
                    } else {
                        state = ScanState::Normal;
                    }
                }
            }
            ScanState::LineComment => {
                if bytes[index] == b'\n' {
                    state = ScanState::Normal;
                }
            }
            ScanState::BlockComment => {
                if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    state = ScanState::Normal;
                    index += 1;
                }
            }
        }
        index += 1;
    }
    None
}

fn fragment_has_sql(fragment: &str) -> bool {
    let mut in_block_comment = false;
    for line in fragment.lines() {
        let mut remaining = line.trim();
        loop {
            if in_block_comment {
                let Some(end) = remaining.find("*/") else {
                    break;
                };
                in_block_comment = false;
                remaining = remaining[end + 2..].trim_start();
                continue;
            }
            if remaining.is_empty() || remaining.starts_with("--") || remaining.starts_with('#') {
                break;
            }
            if remaining.starts_with("/*") && !remaining.starts_with("/*!") {
                if let Some(end) = remaining.find("*/") {
                    remaining = remaining[end + 2..].trim_start();
                    continue;
                }
                in_block_comment = true;
                break;
            }
            return true;
        }
    }
    false
}

fn exchange_error(error: impl std::fmt::Display) -> CockpitError {
    CockpitError::Exchange(error.to_string())
}

fn quote_identifier(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}

fn backup_quote_identifier(value: &str, database_kind: DatabaseKind) -> String {
    match database_kind {
        DatabaseKind::MySql | DatabaseKind::MariaDb => quote_identifier(value),
        DatabaseKind::PostgreSql | DatabaseKind::Sqlite => {
            format!("\"{}\"", value.replace('"', "\"\""))
        }
    }
}

async fn list_all_tables(
    session: &Arc<dyn cockpit_core::DriverSession>,
    database: &str,
) -> cockpit_core::Result<Vec<TableInfo>> {
    const PAGE_SIZE: usize = 2_000;
    let mut tables = Vec::new();
    let mut offset = 0usize;
    loop {
        let page = session
            .list_tables(database, None, PAGE_SIZE, offset)
            .await?;
        let page_len = page.len();
        tables.extend(page);
        if page_len < PAGE_SIZE {
            break;
        }
        offset = offset
            .checked_add(page_len)
            .ok_or_else(|| CockpitError::Exchange("数据库对象数量超过支持范围".into()))?;
    }
    Ok(tables)
}

async fn mysql_database_definition(
    session: &Arc<dyn cockpit_core::DriverSession>,
    database: &str,
) -> cockpit_core::Result<String> {
    let page = session
        .execute(ExecuteQueryRequest {
            execution_id: Uuid::new_v4(),
            sql: format!(
                "SHOW CREATE DATABASE {}",
                backup_quote_identifier(database, DatabaseKind::MySql)
            ),
            database: None,
            timeout_secs: None,
            allow_write: false,
            page_size: 1,
            row_offset: 0,
        })
        .await?;
    page.rows
        .first()
        .and_then(|row| row.get(1))
        .and_then(cell_value_text)
        .map(str::to_string)
        .ok_or_else(|| CockpitError::Exchange("MySQL 未返回数据库创建语句".into()))
}

async fn mysql_ordered_view_names(
    session: &Arc<dyn cockpit_core::DriverSession>,
    database: &str,
    view_names: &[String],
) -> cockpit_core::Result<Vec<String>> {
    if view_names.len() < 2 {
        return Ok(view_names.to_vec());
    }
    let sql = format!(
        "SELECT VIEW_NAME, TABLE_NAME FROM information_schema.VIEW_TABLE_USAGE WHERE VIEW_SCHEMA = {0} AND TABLE_SCHEMA = {0} ORDER BY VIEW_NAME, TABLE_NAME",
        mysql_utf8_sql_literal(database)
    );
    let mut dependencies: HashMap<String, HashSet<String>> = HashMap::new();
    let mut offset = 0usize;
    loop {
        let page = session
            .execute(ExecuteQueryRequest {
                execution_id: Uuid::new_v4(),
                sql: sql.clone(),
                database: None,
                timeout_secs: None,
                allow_write: false,
                page_size: 5_000,
                row_offset: offset,
            })
            .await?;
        for row in &page.rows {
            let Some(view) = row.first().and_then(cell_value_text) else {
                continue;
            };
            let Some(dependency) = row.get(1).and_then(cell_value_text) else {
                continue;
            };
            dependencies
                .entry(view.to_string())
                .or_default()
                .insert(dependency.to_string());
        }
        offset += page.rows.len();
        if !page.has_more || page.rows.is_empty() {
            break;
        }
    }
    order_view_names(view_names, &dependencies)
}

fn order_view_names(
    view_names: &[String],
    dependencies: &HashMap<String, HashSet<String>>,
) -> cockpit_core::Result<Vec<String>> {
    let mut remaining = view_names.iter().cloned().collect::<HashSet<_>>();
    let mut ordered = Vec::with_capacity(view_names.len());
    while !remaining.is_empty() {
        let ready = view_names
            .iter()
            .filter(|name| {
                remaining.contains(*name)
                    && dependencies
                        .get(*name)
                        .is_none_or(|items| items.iter().all(|item| !remaining.contains(item)))
            })
            .cloned()
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return Err(CockpitError::Exchange(
                "视图依赖关系存在循环，无法生成可顺序恢复的备份".into(),
            ));
        }
        for name in ready {
            remaining.remove(&name);
            ordered.push(name);
        }
    }
    Ok(ordered)
}

fn mysql_alter_database_definition(create_ddl: &str) -> Option<String> {
    let database = Regex::new(
        r"(?i)CREATE\s+DATABASE(?:\s+(?:/\*![\s\S]*?\*/|IF\s+NOT\s+EXISTS))*\s+(`(?:``|[^`])+`)",
    )
    .ok()?
    .captures(create_ddl)?
    .get(1)?
    .as_str();
    let charset = Regex::new(r"(?i)DEFAULT\s+CHARACTER\s+SET\s+([A-Za-z0-9_]+)")
        .ok()?
        .captures(create_ddl)?
        .get(1)?
        .as_str();
    let collation = Regex::new(r"(?i)\bCOLLATE\s+([A-Za-z0-9_]+)")
        .ok()?
        .captures(create_ddl)
        .and_then(|captures| captures.get(1))
        .map(|value| format!(" COLLATE {}", value.as_str()))
        .unwrap_or_default();
    Some(format!(
        "ALTER DATABASE {database} DEFAULT CHARACTER SET {charset}{collation}"
    ))
}

fn mysql_column_is_generated(column: &ColumnInfo) -> bool {
    column
        .generation_expression
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || column
            .extra
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().contains("generated"))
}

fn mysql_table_engine(ddl: &str) -> Option<&str> {
    let upper = ddl.to_ascii_uppercase();
    let start = upper.find("ENGINE=")? + "ENGINE=".len();
    let end = ddl[start..]
        .find(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .map_or(ddl.len(), |offset| start + offset);
    ddl.get(start..end).filter(|value| !value.is_empty())
}

/// Cursor for keyset (seek) pagination: the ordered key columns plus the
/// values of the last emitted row. The next page is fetched with
/// `WHERE k > last ORDER BY k LIMIT n` so the server seeks to the cursor
/// instead of re-skipping `OFFSET` rows of the whole result set every page
/// (linear total cost instead of quadratic).
#[derive(Debug, Clone)]
struct KeysetCursor {
    columns: Vec<String>,
    last_values: Vec<CellValue>,
}

/// Builds the `WHERE` clause that seeks past `(last_values...)` in
/// `(columns...)` order, or `None` when the cursor cannot be represented:
/// a NULL key value cannot be compared with `>` (NULL comparisons are
/// unknown), so callers must fall back to `OFFSET` pagination.
///
/// The comparison is written as an explicit OR-of-AND chain
/// (`k1 > v1 OR (k1 = v1 AND k2 > v2) OR ...`) rather than the row
/// constructor `(k1, k2) > (v1, v2)`: it is equivalent lexicographically
/// but works identically on MySQL, MariaDB, PostgreSQL and SQLite, and
/// avoids the NULL edge cases of MySQL row-constructor comparisons.
fn keyset_condition_sql(
    columns: &[String],
    last_values: &[CellValue],
    database_kind: DatabaseKind,
) -> Option<String> {
    if columns.len() != last_values.len() {
        return None;
    }
    let literals = last_values
        .iter()
        .map(|value| keyset_literal(value, database_kind))
        .collect::<Option<Vec<_>>>()?;
    let mut predicates = Vec::with_capacity(columns.len());
    for index in 0..columns.len() {
        let mut parts = Vec::with_capacity(index + 1);
        for equal in 0..index {
            parts.push(format!(
                "{} = {}",
                backup_quote_identifier(&columns[equal], database_kind),
                literals[equal]
            ));
        }
        parts.push(format!(
            "{} > {}",
            backup_quote_identifier(&columns[index], database_kind),
            literals[index]
        ));
        predicates.push(parts.join(" AND "));
    }
    Some(predicates.join(" OR "))
}

/// Renders a single key value as a SQL literal for keyset comparisons.
/// Returns `None` for values that cannot be embedded safely (NULL, or a
/// non-finite float). Text-like values are single-quoted; binary and
/// geometry values are embedded as hex literals so byte order is preserved.
fn keyset_literal(value: &CellValue, database_kind: DatabaseKind) -> Option<String> {
    let quoted = |text: String| -> String {
        let escaped = match database_kind {
            // MySQL/MariaDB treat backslash as an escape character by
            // default, so backslashes must be doubled as well.
            DatabaseKind::MySql | DatabaseKind::MariaDb => {
                text.replace('\\', "\\\\").replace('\'', "''")
            }
            // PostgreSQL (standard_conforming_strings) and SQLite treat
            // backslash literally; only single quotes need doubling.
            DatabaseKind::PostgreSql | DatabaseKind::Sqlite => text.replace('\'', "''"),
        };
        format!("'{escaped}'")
    };
    let hex_literal = |bytes: &[u8]| -> String {
        let hex = bytes
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>();
        match database_kind {
            DatabaseKind::MySql | DatabaseKind::MariaDb | DatabaseKind::Sqlite => {
                format!("X'{hex}'")
            }
            DatabaseKind::PostgreSql => format!("'\\x{hex}'"),
        }
    };
    match value {
        CellValue::Null => None,
        CellValue::Bool(value) => Some(quoted(if *value { "1".into() } else { "0".into() })),
        CellValue::Signed(value)
        | CellValue::Unsigned(value)
        | CellValue::Decimal(value)
        | CellValue::Text(value)
        | CellValue::Date(value)
        | CellValue::Time(value)
        | CellValue::DateTime(value)
        | CellValue::Json(value) => Some(quoted(value.clone())),
        CellValue::Float(value) if value.is_finite() => Some(quoted(format!("{value}"))),
        CellValue::Float(_) => None,
        CellValue::Bytes { base64, .. } => {
            let bytes = BASE64_STANDARD.decode(base64).ok()?;
            Some(hex_literal(&bytes))
        }
        CellValue::Geometry { wkb_base64, srid } => {
            let wkb = BASE64_STANDARD.decode(wkb_base64).ok()?;
            let mut bytes = Vec::with_capacity(wkb.len() + 4);
            bytes.extend_from_slice(&srid.unwrap_or_default().to_le_bytes());
            bytes.extend_from_slice(&wkb);
            Some(hex_literal(&bytes))
        }
    }
}

/// Returns the ordered key columns usable for keyset pagination of a table:
/// the primary key, or the first unique index whose columns are all
/// declared NOT NULL (mirrors the ordering choice of `table_page_sql`).
/// `None` means no reliable key exists and `OFFSET` must be used.
fn table_keyset_columns(detail: &TableDetail) -> Option<Vec<String>> {
    let primary = detail
        .indexes
        .iter()
        .find(|index| index.primary && !index.columns.is_empty());
    let non_null_unique = detail.indexes.iter().find(|index| {
        index.unique
            && !index.columns.is_empty()
            && index.columns.iter().all(|name| {
                detail
                    .columns
                    .iter()
                    .find(|column| column.name == *name)
                    .is_some_and(|column| !column.nullable)
            })
    });
    primary
        .or(non_null_unique)
        .map(|index| index.columns.clone())
}

/// Builds a keyset cursor from the last row of a fetched page, aligned with
/// the page's column metadata. Returns `None` when a key column is missing
/// from the projection (e.g. a generated key column excluded from export).
fn keyset_from_last_row(
    key_columns: &[String],
    page_columns: &[ColumnMeta],
    last_row: &[CellValue],
) -> Option<KeysetCursor> {
    let mut last_values = Vec::with_capacity(key_columns.len());
    for name in key_columns {
        let index = page_columns
            .iter()
            .position(|column| column.name == *name)?;
        last_values.push(last_row.get(index)?.clone());
    }
    Some(KeysetCursor {
        columns: key_columns.to_vec(),
        last_values,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "pagination builder mirrors the table detail and cursor inputs"
)]
fn table_page_sql(
    database: &str,
    table: &str,
    detail: &TableDetail,
    page_size: usize,
    offset: usize,
    keyset: Option<&KeysetCursor>,
    database_kind: DatabaseKind,
    include_generated_columns: bool,
) -> cockpit_core::Result<String> {
    if !matches!(database_kind, DatabaseKind::MySql | DatabaseKind::MariaDb) {
        let key_columns = table_keyset_columns(detail);
        if let (Some(key_columns), Some(cursor)) = (key_columns.as_ref(), keyset)
            && cursor.columns == *key_columns
            && let Some(condition) =
                keyset_condition_sql(&cursor.columns, &cursor.last_values, database_kind)
        {
            let order = key_columns
                .iter()
                .map(|column| backup_quote_identifier(column, database_kind))
                .collect::<Vec<_>>()
                .join(", ");
            return Ok(format!(
                "SELECT * FROM {}.{} WHERE {condition} ORDER BY {order} LIMIT {}",
                backup_quote_identifier(database, database_kind),
                backup_quote_identifier(table, database_kind),
                page_size + 1
            ));
        }
        if let Some(key_columns) = key_columns {
            let order = key_columns
                .iter()
                .map(|column| backup_quote_identifier(column, database_kind))
                .collect::<Vec<_>>()
                .join(", ");
            return Ok(format!(
                "SELECT * FROM {}.{} ORDER BY {order} LIMIT {} OFFSET {}",
                backup_quote_identifier(database, database_kind),
                backup_quote_identifier(table, database_kind),
                page_size + 1,
                offset
            ));
        }
        return Ok(format!(
            "SELECT * FROM {}.{} LIMIT {} OFFSET {}",
            backup_quote_identifier(database, database_kind),
            backup_quote_identifier(table, database_kind),
            page_size + 1,
            offset
        ));
    }
    let selected_columns = detail
        .columns
        .iter()
        .filter(|column| include_generated_columns || !mysql_column_is_generated(column))
        .collect::<Vec<_>>();
    if selected_columns.is_empty() {
        return Err(CockpitError::Exchange(format!(
            "表 {table} 没有可导出的普通字段"
        )));
    }
    let primary = detail
        .indexes
        .iter()
        .find(|index| index.primary && !index.columns.is_empty());
    let non_null_unique = detail.indexes.iter().find(|index| {
        index.unique
            && !index.columns.is_empty()
            && index.columns.iter().all(|name| {
                detail
                    .columns
                    .iter()
                    .find(|column| column.name == *name)
                    .is_some_and(|column| !column.nullable)
            })
    });
    let key_columns = primary
        .or(non_null_unique)
        .map(|index| index.columns.clone());
    let projection = selected_columns
        .iter()
        .map(|column| backup_quote_identifier(&column.name, database_kind))
        .collect::<Vec<_>>()
        .join(", ");
    let order = if let Some(index) = primary.or(non_null_unique) {
        index
            .columns
            .iter()
            .map(|column| backup_quote_identifier(column, database_kind))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        selected_columns
            .iter()
            .map(|column| {
                format!(
                    "SHA2(COALESCE(HEX({}), 'NULL'), 256)",
                    backup_quote_identifier(&column.name, database_kind)
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    if let (Some(key_columns), Some(cursor)) = (key_columns.as_ref(), keyset)
        && cursor.columns == *key_columns
        && let Some(condition) =
            keyset_condition_sql(&cursor.columns, &cursor.last_values, database_kind)
    {
        return Ok(format!(
            "SELECT {projection} FROM {}.{} WHERE {condition} ORDER BY {order} LIMIT {}",
            backup_quote_identifier(database, database_kind),
            backup_quote_identifier(table, database_kind),
            page_size + 1
        ));
    }
    Ok(format!(
        "SELECT {projection} FROM {}.{} ORDER BY {order} LIMIT {} OFFSET {}",
        backup_quote_identifier(database, database_kind),
        backup_quote_identifier(table, database_kind),
        page_size + 1,
        offset
    ))
}

fn backup_table_page_sql(
    database: &str,
    table: &str,
    detail: &TableDetail,
    page_size: usize,
    offset: usize,
    keyset: Option<&KeysetCursor>,
    database_kind: DatabaseKind,
) -> cockpit_core::Result<String> {
    table_page_sql(
        database,
        table,
        detail,
        page_size,
        offset,
        keyset,
        database_kind,
        false,
    )
}

fn paged_select_sql(
    sql: &str,
    page_size: usize,
    offset: usize,
    keyset: Option<&KeysetCursor>,
    database_kind: DatabaseKind,
    ordered_columns: usize,
) -> String {
    let source = format!(
        "SELECT * FROM ({}) AS {}",
        sql.trim().trim_end_matches(';'),
        backup_quote_identifier("__cockpit_page", database_kind)
    );
    let limit = page_size + 1;
    if let Some(cursor) = keyset
        && let Some(condition) =
            keyset_condition_sql(&cursor.columns, &cursor.last_values, database_kind)
    {
        // Deterministic ordering by the query's own output columns
        // (ordinals work even when the columns have duplicate names).
        let order = (1..=cursor.columns.len())
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return format!("{source} WHERE {condition} ORDER BY {order} LIMIT {limit}");
    }
    if ordered_columns > 0 {
        // Keep page 1 and any OFFSET fallback pages ordered identically to
        // the keyset order, so the export stays consistent and repeatable.
        let order = (1..=ordered_columns)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return format!("{source} ORDER BY {order} LIMIT {limit} OFFSET {offset}");
    }
    format!("{source} LIMIT {limit} OFFSET {offset}")
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        io::Write as _,
    };

    use super::{
        DatabaseKind, KeysetCursor, RuntimeStats, decrypt_backup_file, encrypt_backup_file,
        export_total_cell, keyset_condition_sql, mysql_alter_database_definition, order_view_names,
        paged_select_sql, process_tree_memory_bytes, read_backup_text, split_sql_script,
        strip_cockpit_backup_header, strip_definer_clause, table_keyset_columns, table_page_sql,
        write_delimited_definition,
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use cockpit_core::CellValue;

    #[test]
    fn serializes_runtime_stats_for_the_frontend_contract() {
        let value = serde_json::to_value(RuntimeStats {
            open_connection_count: 2,
            tab_session_count: 3,
            memory_bytes: Some(128 * 1024 * 1024),
        })
        .unwrap();
        assert_eq!(value["openConnectionCount"], 2);
        assert_eq!(value["tabSessionCount"], 3);
        assert_eq!(value["memoryBytes"], 128 * 1024 * 1024);
    }

    #[test]
    fn totals_memory_for_the_root_process_and_all_descendants() {
        let memory_bytes = process_tree_memory_bytes(
            10,
            [
                (10, Some(1), 100),
                (11, Some(10), 40),
                (12, Some(11), 20),
                (13, Some(10), 30),
                (20, Some(1), 500),
            ],
        );

        assert_eq!(memory_bytes, Some(190));
    }

    #[test]
    fn returns_none_when_the_root_process_cannot_be_sampled() {
        assert_eq!(
            process_tree_memory_bytes(10, [(11, Some(10), 40), (20, Some(1), 500)]),
            None
        );
    }

    #[test]
    fn chooses_a_delimiter_that_does_not_collide_with_routine_body() {
        let mut output = Vec::new();
        write_delimited_definition(
            &mut output,
            "CREATE PROCEDURE demo() BEGIN SELECT '$$'; END",
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.starts_with("DELIMITER $COCKPIT_1$\n"));
        assert!(text.contains("END$COCKPIT_1$\n"));
        let statements = split_sql_script(&text).unwrap();
        assert_eq!(
            statements,
            ["CREATE PROCEDURE demo() BEGIN SELECT '$$'; END"]
        );
    }

    #[test]
    fn restores_mysql_database_charset_even_when_database_already_exists() {
        assert_eq!(
            mysql_alter_database_definition(
                "CREATE DATABASE /*!32312 IF NOT EXISTS*/ `demo``db` /*!40100 DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci */"
            )
            .as_deref(),
            Some("ALTER DATABASE `demo``db` DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci")
        );
    }

    #[test]
    fn orders_mysql_views_after_their_view_dependencies() {
        let names = vec![
            "summary".to_string(),
            "base_view".to_string(),
            "detail".to_string(),
        ];
        let dependencies = HashMap::from([
            ("summary".to_string(), HashSet::from(["detail".to_string()])),
            (
                "detail".to_string(),
                HashSet::from(["base_view".to_string()]),
            ),
        ]);
        assert_eq!(
            order_view_names(&names, &dependencies).unwrap(),
            ["base_view", "detail", "summary"]
        );
    }

    #[test]
    fn mysql_backup_pages_include_invisible_and_skip_generated_columns() {
        let detail = cockpit_core::TableDetail {
            table: cockpit_core::TableInfo {
                database: "demo".into(),
                name: "items".into(),
                table_type: "BASE TABLE".into(),
                comment: None,
                estimated_rows: None,
                total_bytes: None,
            },
            columns: vec![
                cockpit_core::ColumnInfo {
                    name: "id".into(),
                    ordinal: 1,
                    data_type: "bigint".into(),
                    full_type: "bigint".into(),
                    nullable: false,
                    default_value: None,
                    extra: None,
                    comment: None,
                    key: Some("PRI".into()),
                    generation_expression: None,
                    collation: None,
                },
                cockpit_core::ColumnInfo {
                    name: "secret".into(),
                    ordinal: 2,
                    data_type: "varchar".into(),
                    full_type: "varchar(20)".into(),
                    nullable: true,
                    default_value: None,
                    extra: Some("INVISIBLE".into()),
                    comment: None,
                    key: None,
                    generation_expression: None,
                    collation: None,
                },
                cockpit_core::ColumnInfo {
                    name: "computed".into(),
                    ordinal: 3,
                    data_type: "bigint".into(),
                    full_type: "bigint".into(),
                    nullable: true,
                    default_value: None,
                    extra: Some("VIRTUAL GENERATED".into()),
                    comment: None,
                    key: None,
                    generation_expression: Some("(`id` + 1)".into()),
                    collation: None,
                },
            ],
            indexes: vec![cockpit_core::IndexInfo {
                name: "PRIMARY".into(),
                columns: vec!["id".into()],
                unique: true,
                primary: true,
                index_type: Some("BTREE".into()),
            }],
            foreign_keys: vec![],
            ddl: String::new(),
        };
        let sql = table_page_sql(
            "demo",
            "items",
            &detail,
            500,
            1_000,
            None,
            cockpit_core::DatabaseKind::MySql,
            false,
        )
        .unwrap();
        assert!(sql.starts_with("SELECT `id`, `secret` FROM `demo`.`items` ORDER BY `id`"));
        assert!(!sql.contains("computed"));
        assert!(sql.ends_with("LIMIT 501 OFFSET 1000"));
    }

    #[test]
    fn keyset_condition_uses_quoted_composite_key_predicates() {
        let columns = vec!["order`id".to_string(), "created_at".to_string()];
        let values = vec![
            CellValue::Unsigned("42".into()),
            CellValue::Text("2024-01-02 03:04:05".into()),
        ];
        let condition =
            keyset_condition_sql(&columns, &values, DatabaseKind::MySql).expect("valid cursor");
        // AND binds tighter than OR, so the composite predicate needs no
        // extra parentheses; backticks inside identifiers are doubled.
        assert_eq!(
            condition,
            "`order``id` > '42' OR `order``id` = '42' AND `created_at` > '2024-01-02 03:04:05'"
        );
    }

    #[test]
    fn keyset_condition_escapes_literals_per_dialect() {
        let columns = vec!["note".to_string()];
        let values = vec![CellValue::Text("it's a \\ test".into())];
        assert_eq!(
            keyset_condition_sql(&columns, &values, DatabaseKind::MySql).unwrap(),
            "`note` > 'it''s a \\\\ test'"
        );
        assert_eq!(
            keyset_condition_sql(&columns, &values, DatabaseKind::PostgreSql).unwrap(),
            "\"note\" > 'it''s a \\ test'"
        );
    }

    #[test]
    fn keyset_condition_rejects_null_and_mismatched_cursors() {
        let columns = vec!["id".to_string()];
        assert_eq!(
            keyset_condition_sql(&columns, &[CellValue::Null], DatabaseKind::MySql),
            None
        );
        assert_eq!(
            keyset_condition_sql(&columns, &[], DatabaseKind::MySql),
            None
        );
        assert_eq!(
            keyset_condition_sql(&columns, &[CellValue::Float(f64::NAN)], DatabaseKind::MySql),
            None
        );
    }

    #[test]
    fn table_page_sql_uses_keyset_cursor_when_valid() {
        let detail = cockpit_core::TableDetail {
            table: cockpit_core::TableInfo {
                database: "demo".into(),
                name: "items".into(),
                table_type: "BASE TABLE".into(),
                comment: None,
                estimated_rows: None,
                total_bytes: None,
            },
            columns: vec![cockpit_core::ColumnInfo {
                name: "id".into(),
                ordinal: 1,
                data_type: "bigint".into(),
                full_type: "bigint".into(),
                nullable: false,
                default_value: None,
                extra: None,
                comment: None,
                key: Some("PRI".into()),
                generation_expression: None,
                collation: None,
            }],
            indexes: vec![cockpit_core::IndexInfo {
                name: "PRIMARY".into(),
                columns: vec!["id".into()],
                unique: true,
                primary: true,
                index_type: Some("BTREE".into()),
            }],
            foreign_keys: vec![],
            ddl: String::new(),
        };
        assert_eq!(table_keyset_columns(&detail), Some(vec!["id".to_string()]));
        let cursor = KeysetCursor {
            columns: vec!["id".into()],
            last_values: vec![CellValue::Unsigned("100".into())],
        };
        let sql = table_page_sql(
            "demo",
            "items",
            &detail,
            500,
            1_000,
            Some(&cursor),
            DatabaseKind::MySql,
            true,
        )
        .unwrap();
        assert!(sql.contains("WHERE `id` > '100'"));
        assert!(sql.ends_with("LIMIT 501"));
        assert!(!sql.contains("OFFSET"));
    }

    #[test]
    fn table_page_sql_falls_back_to_offset_for_keyless_tables() {
        let detail = cockpit_core::TableDetail {
            table: cockpit_core::TableInfo {
                database: "demo".into(),
                name: "logs".into(),
                table_type: "BASE TABLE".into(),
                comment: None,
                estimated_rows: None,
                total_bytes: None,
            },
            columns: vec![cockpit_core::ColumnInfo {
                name: "message".into(),
                ordinal: 1,
                data_type: "text".into(),
                full_type: "text".into(),
                nullable: true,
                default_value: None,
                extra: None,
                comment: None,
                key: None,
                generation_expression: None,
                collation: None,
            }],
            indexes: vec![],
            foreign_keys: vec![],
            ddl: String::new(),
        };
        assert_eq!(table_keyset_columns(&detail), None);
        let sql = table_page_sql(
            "demo",
            "logs",
            &detail,
            500,
            1_000,
            None,
            DatabaseKind::PostgreSql,
            true,
        )
        .unwrap();
        assert!(sql.starts_with("SELECT * FROM \"demo\".\"logs\" LIMIT 501 OFFSET 1000"));
    }

    #[test]
    fn paged_select_sql_seeks_with_keyset_and_orders_by_ordinals() {
        let cursor = KeysetCursor {
            columns: vec!["id".into(), "name".into()],
            last_values: vec![
                CellValue::Unsigned("7".into()),
                CellValue::Text("ada".into()),
            ],
        };
        let sql = paged_select_sql(
            "SELECT id, name FROM users ORDER BY id;",
            2_000,
            0,
            Some(&cursor),
            DatabaseKind::MySql,
            2,
        );
        assert!(sql.starts_with(
            "SELECT * FROM (SELECT id, name FROM users ORDER BY id) AS `__cockpit_page`"
        ));
        assert!(sql.contains("WHERE `id` > '7' OR `id` = '7' AND `name` > 'ada'"));
        assert!(sql.ends_with("ORDER BY 1, 2 LIMIT 2001"));
        assert!(!sql.contains("OFFSET"));
    }

    #[test]
    fn paged_select_sql_orders_first_and_fallback_pages_by_ordinals() {
        // Without a cursor the first page is still ordered by the output
        // columns (ordinals), so later keyset pages stay consistent; the
        // unordered LIMIT/OFFSET form is only used when ordering is known
        // to be impossible.
        let sql = paged_select_sql(
            "SELECT id, name FROM users",
            2_000,
            4_000,
            None,
            DatabaseKind::PostgreSql,
            2,
        );
        assert!(sql.contains("ORDER BY 1, 2 LIMIT 2001 OFFSET 4000"));
        let unordered = paged_select_sql(
            "SELECT id FROM users",
            2_000,
            4_000,
            None,
            DatabaseKind::PostgreSql,
            0,
        );
        assert!(unordered.ends_with("LIMIT 2001 OFFSET 4000"));
        assert!(!unordered.contains("ORDER BY"));
    }

    #[test]
    fn parses_export_totals_from_numeric_and_binary_driver_values() {
        assert_eq!(
            export_total_cell(&CellValue::Float(5_000.0)).unwrap(),
            5_000
        );
        assert_eq!(
            export_total_cell(&CellValue::Bytes {
                base64: BASE64_STANDARD.encode(b"112000"),
                preview: Some("112000".into()),
                length: 6,
            })
            .unwrap(),
            112_000
        );
    }

    #[test]
    fn reads_encrypted_gzip_sql_backups_and_removes_header() {
        let suffix = uuid::Uuid::new_v4();
        let compressed = std::env::temp_dir().join(format!("cockpit-{suffix}.sql.gz"));
        let encrypted = std::env::temp_dir().join(format!("cockpit-{suffix}.enc"));
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(b"-- Cockpit database backup\nSELECT 1;\nSELECT 2;")
            .unwrap();
        std::fs::write(&compressed, encoder.finish().unwrap()).unwrap();
        encrypt_backup_file(
            &compressed,
            &encrypted,
            "backup-password",
            &tokio_util::sync::CancellationToken::new(),
        )
        .unwrap();

        let sql = read_backup_text(&encrypted, Some("backup-password")).unwrap();
        let (cockpit_backup, body) = strip_cockpit_backup_header(&sql);

        assert!(cockpit_backup);
        assert_eq!(split_sql_script(body).unwrap(), ["SELECT 1", "SELECT 2"]);
        let _ = std::fs::remove_file(compressed);
        let _ = std::fs::remove_file(encrypted);
    }

    #[test]
    fn keeps_regular_sql_and_accepts_crlf_backup_headers() {
        let regular = "SELECT 1;";
        assert_eq!(strip_cockpit_backup_header(regular), (false, regular));
        assert_eq!(
            strip_cockpit_backup_header("-- Cockpit database backup\r\nSELECT 2;"),
            (true, "SELECT 2;")
        );
    }

    #[test]
    fn splits_regular_sql_without_touching_literal_semicolons() {
        let statements = split_sql_script("INSERT INTO t VALUES ('a;b');\nSELECT 1;").unwrap();
        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("a;b"));
        assert_eq!(statements[1], "SELECT 1");
    }

    #[test]
    fn supports_mysql_delimiter_blocks() {
        let script = "DELIMITER $$\nCREATE PROCEDURE p() BEGIN\nSELECT 1;\nSELECT 2;\nEND$$\nDELIMITER ;\nCALL p();";
        let statements = split_sql_script(script).unwrap();
        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("SELECT 2;"));
        assert_eq!(statements[1], "CALL p()");
    }

    #[test]
    fn supports_postgresql_dollar_quoted_bodies() {
        let script = "CREATE FUNCTION f() RETURNS void LANGUAGE plpgsql AS $body$\nBEGIN\n  PERFORM 1;\nEND\n$body$;\nSELECT 2;";
        let statements = split_sql_script(script).unwrap();
        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("PERFORM 1;"));
        assert_eq!(statements[1], "SELECT 2");
    }

    #[test]
    fn encrypted_backup_round_trips_and_rejects_wrong_password() {
        let suffix = uuid::Uuid::new_v4();
        let input = std::env::temp_dir().join(format!("cockpit-{suffix}.sql"));
        let encrypted = std::env::temp_dir().join(format!("cockpit-{suffix}.enc"));
        std::fs::write(&input, "SELECT '中文';").unwrap();
        encrypt_backup_file(
            &input,
            &encrypted,
            "correct-password",
            &tokio_util::sync::CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(decrypt_backup_file(&encrypted, "correct-password").unwrap())
                .unwrap(),
            "SELECT '中文';"
        );
        assert!(decrypt_backup_file(&encrypted, "wrong-password").is_err());
        assert_eq!(
            read_backup_text(&encrypted, Some("correct-password")).unwrap(),
            "SELECT '中文';"
        );
        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(encrypted);
    }

    #[test]
    fn reads_gzip_sql_backups() {
        let path = std::env::temp_dir().join(format!("cockpit-{}.sql.gz", uuid::Uuid::new_v4()));
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"SELECT 42;").unwrap();
        std::fs::write(&path, encoder.finish().unwrap()).unwrap();
        assert_eq!(read_backup_text(&path, None).unwrap(), "SELECT 42;");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn accepts_delimiter_after_comment_only_lines() {
        let script = "-- procedure below\nDELIMITER //\nCREATE PROCEDURE p() SELECT 'https://example.test'//\nDELIMITER ;";
        let statements = split_sql_script(script).unwrap();
        assert_eq!(statements.len(), 1);
        assert!(statements[0].contains("https://example.test"));
    }

    #[test]
    fn removes_definer_for_portable_backup() {
        assert_eq!(
            strip_definer_clause("CREATE DEFINER=`root`@`localhost` PROCEDURE p() SELECT 1"),
            "CREATE  PROCEDURE p() SELECT 1"
        );
    }
}
