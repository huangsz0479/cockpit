import { invoke } from "@tauri-apps/api/core";
import type {
  BackupOptions, BackupSummary, ColumnInfo, ConnectionInfo, ConnectionProfile, DatabaseInfo, DatabaseObjectDefinition, DatabaseObjectKind, DiagnosticsInfo, EventInfo, ExecuteQueryRequest, ExportSummary, ImportDataRequest, ImportPreview, ImportPreviewRequest, ImportSummary,
  QueryResultPage, ResultExportOptions, RoutineInfo, RowMutationRequest, RuntimeStats,
  RoutineParameter, RowMutationResult, RedisDatabaseInfo, RedisKeyInfo, RedisReply, RedisScanPage, RedisValue, ServerLockInfo, ServerMetric, ServerProcessInfo, ServerVariable, SqlAssessment, TableDetail, TableInfo, TextFileContent, TriggerInfo, UserAccount, UUID,
} from "@/types";

export const api = {
  listConnections: () => invoke<ConnectionProfile[]>("list_connections"),
  hasConnectionPassword: (connectionId: UUID) => invoke<boolean>("has_connection_password", { connectionId }),
  saveConnection: (profile: ConnectionProfile, password?: string) =>
    invoke<{ profile: ConnectionProfile; secretPersisted: boolean }>("save_connection", { request: { profile, password } }),
  deleteConnection: (connectionId: UUID) => invoke<void>("delete_connection", { connectionId }),
  testConnection: (profile: ConnectionProfile, password?: string) =>
    invoke<ConnectionInfo>("test_connection", { request: { profile, password } }),
  connect: (connectionId: UUID) => invoke<ConnectionInfo>("connect_connection", { connectionId }),
  openTabSession: (connectionId: UUID, sessionId: UUID) =>
    invoke<void>("open_tab_session", { connectionId, sessionId }),
  closeTabSession: (sessionId: UUID) => invoke<void>("close_tab_session", { sessionId }),
  disconnect: (connectionId: UUID) => invoke<void>("disconnect_connection", { connectionId }),
  connectRedis: (connectionId: UUID) => invoke<ConnectionInfo>("connect_redis_connection", { connectionId }),
  disconnectRedis: (connectionId: UUID) => invoke<void>("disconnect_redis_connection", { connectionId }),
  listRedisDatabases: (connectionId: UUID) => invoke<RedisDatabaseInfo[]>("list_redis_databases", { connectionId }),
  scanRedisKeys: (connectionId: UUID, database: number, cursor: number, pattern?: string, count = 200) =>
    invoke<RedisScanPage>("scan_redis_keys", { connectionId, database, cursor, pattern: pattern ?? null, count }),
  redisKeyInfo: (connectionId: UUID, database: number, key: string) =>
    invoke<RedisKeyInfo>("get_redis_key_info", { connectionId, database, key }),
  redisValue: (connectionId: UUID, database: number, key: string, limit = 500) =>
    invoke<RedisValue>("get_redis_value", { connectionId, database, key, limit }),
  setRedisString: (connectionId: UUID, database: number, key: string, valueBase64: string, ttlSecs?: number | null) =>
    invoke<void>("set_redis_string", { connectionId, database, key, valueBase64, ttlSecs: ttlSecs ?? null }),
  deleteRedisKeys: (connectionId: UUID, database: number, keys: string[]) =>
    invoke<number>("delete_redis_keys", { connectionId, database, keys }),
  expireRedisKey: (connectionId: UUID, database: number, key: string, seconds: number) =>
    invoke<boolean>("expire_redis_key", { connectionId, database, key, seconds }),
  renameRedisKey: (connectionId: UUID, database: number, from: string, to: string) =>
    invoke<void>("rename_redis_key", { connectionId, database, from, to }),
  runRedisCommand: (connectionId: UUID, database: number, args: string[], allowWrite: boolean) =>
    invoke<RedisReply>("run_redis_command", { connectionId, database, args, allowWrite }),
  redisServerInfo: (connectionId: UUID) => invoke<ServerMetric[]>("get_redis_server_info", { connectionId }),
  listDatabases: (connectionId: UUID) => invoke<DatabaseInfo[]>("list_databases", { connectionId }),
  listTables: (connectionId: UUID, database: string, filter = "", limit = 500, offset = 0) =>
    invoke<TableInfo[]>("list_tables", { connectionId, database, filter, limit, offset }),
  listColumns: (connectionId: UUID, database: string, table: string) =>
    invoke<ColumnInfo[]>("list_columns", { connectionId, database, table }),
  tableDetail: (connectionId: UUID, database: string, table: string) =>
    invoke<TableDetail>("get_table_detail", { connectionId, database, table }),
  listRoutines: (connectionId: UUID, database: string) =>
    invoke<RoutineInfo[]>("list_routines", { connectionId, database }),
  listTriggers: (connectionId: UUID, database: string) =>
    invoke<TriggerInfo[]>("list_triggers", { connectionId, database }),
  listEvents: (connectionId: UUID, database: string) =>
    invoke<EventInfo[]>("list_events", { connectionId, database }),
  objectDefinition: (connectionId: UUID, database: string, kind: DatabaseObjectKind, name: string) =>
    invoke<DatabaseObjectDefinition>("get_object_definition", { connectionId, database, kind, name }),
  routineParameters: (connectionId: UUID, database: string, name: string) => invoke<RoutineParameter[]>("get_routine_parameters", { connectionId, database, name }),
  serverProcesses: (connectionId: UUID) => invoke<ServerProcessInfo[]>("list_server_processes", { connectionId }),
  killServerProcess: (connectionId: UUID, processId: number) => invoke<void>("kill_server_process", { connectionId, processId }),
  serverStatus: (connectionId: UUID) => invoke<ServerMetric[]>("get_server_status", { connectionId }),
  databaseUsers: (connectionId: UUID) => invoke<UserAccount[]>("list_database_users", { connectionId }),
  userGrants: (connectionId: UUID, user: string, host: string) => invoke<string[]>("get_user_grants", { connectionId, user, host }),
  serverVariables: (connectionId: UUID, filter = "") => invoke<ServerVariable[]>("list_server_variables", { connectionId, filter }),
  serverLocks: (connectionId: UUID) => invoke<ServerLockInfo[]>("list_server_locks", { connectionId }),
  assess: (sql: string) => invoke<SqlAssessment>("assess_query", { sql }),
  execute: (connectionId: UUID, sessionId: UUID | null, request: ExecuteQueryRequest) =>
    invoke<QueryResultPage>("execute_query", { connectionId, sessionId, request }),
  cancel: (connectionId: UUID, sessionId: UUID | null, executionId: UUID) =>
    invoke<boolean>("cancel_query", { connectionId, sessionId, executionId }),
  loadWorkspaceState: (stateKey: string) => invoke<string | null>("load_workspace_state", { stateKey }),
  saveWorkspaceState: (stateKey: string, payloadJson: string) =>
    invoke<void>("save_workspace_state", { stateKey, payloadJson }),
  readTextFile: (inputPath: string) => invoke<TextFileContent>("read_text_file", { inputPath }),
  writeTextFile: (outputPath: string, contents: string) => invoke<string>("write_text_file", { outputPath, contents }),
  revealFile: (inputPath: string) => invoke<void>("reveal_file", { inputPath }),
  writeBinaryFile: (outputPath: string, base64: string) => invoke<string>("write_binary_file", { outputPath, base64 }),
  mutateRow: (connectionId: UUID, sessionId: UUID | null, request: RowMutationRequest) =>
    invoke<RowMutationResult>("mutate_row", { connectionId, sessionId, request }),
  beginTransaction: (connectionId: UUID, sessionId: UUID | null) => invoke<void>("begin_transaction", { connectionId, sessionId }),
  commitTransaction: (connectionId: UUID, sessionId: UUID | null) => invoke<void>("commit_transaction", { connectionId, sessionId }),
  rollbackTransaction: (connectionId: UUID, sessionId: UUID | null) => invoke<void>("rollback_transaction", { connectionId, sessionId }),
  transactionActive: (connectionId: UUID, sessionId: UUID | null) => invoke<boolean>("transaction_active", { connectionId, sessionId }),
  exportResultPage: (outputPath: string, page: QueryResultPage, options: ResultExportOptions) =>
    invoke<ExportSummary>("export_result_page", { outputPath, page, options }),
  exportTable: (connectionId: UUID, database: string, table: string, outputPath: string, options: ResultExportOptions, taskId?: UUID) =>
    invoke<ExportSummary>("export_table", { connectionId, database, table, outputPath, options, taskId: taskId ?? null }),
  exportQuery: (connectionId: UUID, database: string | null, sql: string, outputPath: string, options: ResultExportOptions, taskId?: UUID) =>
    invoke<ExportSummary>("export_query", { connectionId, database, sql, outputPath, options, taskId: taskId ?? null }),
  backupDatabase: (connectionId: UUID, database: string, outputPath: string, options: BackupOptions) =>
    invoke<BackupSummary>("backup_database", {
      request: { connectionId, database, outputPath, includeData: options.includeData,
        compression: options.compression, encryptionPassword: options.encryptionPassword ?? null,
        taskId: options.taskId ?? null },
    }),
  previewImport: (request: ImportPreviewRequest) => invoke<ImportPreview>("preview_import_data", { request }),
  importData: (request: ImportDataRequest) => invoke<ImportSummary>("import_data", { request }),
  cancelTransfer: (taskId: UUID) => invoke<boolean>("cancel_transfer", { taskId }),
  runtimeStats: () => invoke<RuntimeStats>("get_runtime_stats"),
  diagnostics: () => invoke<DiagnosticsInfo>("get_diagnostics"),
};
