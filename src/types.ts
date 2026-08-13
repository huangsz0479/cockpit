export type UUID = string;
export type DatabaseKind = "mysql" | "mariadb" | "postgresql" | "sqlite";

export type TlsMode = "disabled" | "preferred" | "required" | "verify_ca" | "verify_identity";
export type SshAuthMethod = "password" | "private_key" | "agent";

export interface TlsOptions {
  mode: TlsMode;
  caCertPath?: string | null;
  clientCertPath?: string | null;
  clientKeyPath?: string | null;
}

export interface SshOptions {
  host: string;
  port: number;
  username: string;
  authMethod: SshAuthMethod;
  privateKeyPath?: string | null;
  useAgent: boolean;
  hostFingerprint?: string | null;
}

export interface ConnectionProfile {
  id: UUID;
  driverKind?: DatabaseKind;
  group?: string | null;
  name: string;
  host: string;
  port: number;
  username: string;
  database?: string | null;
  tls: TlsOptions;
  ssh?: SshOptions | null;
  connectTimeoutSecs: number;
  queryTimeoutSecs: number;
  poolSize: number;
  readOnly: boolean;
  production: boolean;
  color?: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ConnectionInfo {
  serverVersion: string;
  serverComment?: string | null;
  connectionId: number;
  currentDatabase?: string | null;
  tlsCipher?: string | null;
}

export interface DatabaseInfo { name: string }
export interface TableInfo {
  database: string;
  name: string;
  tableType: string;
  comment?: string | null;
  estimatedRows?: number | null;
  totalBytes?: number | null;
}

export interface ColumnInfo {
  name: string;
  ordinal: number;
  dataType: string;
  fullType: string;
  nullable: boolean;
  defaultValue?: string | null;
  extra?: string | null;
  comment?: string | null;
  key?: string | null;
  generationExpression?: string | null;
  collation?: string | null;
}

export interface IndexInfo { name: string; columns: string[]; unique: boolean; primary: boolean; indexType?: string | null }
export interface ForeignKeyInfo {
  name: string;
  columns: string[];
  referencedDatabase: string;
  referencedTable: string;
  referencedColumns: string[];
  onUpdate?: string | null;
  onDelete?: string | null;
}
export interface TableDetail { table: TableInfo; columns: ColumnInfo[]; indexes: IndexInfo[]; foreignKeys: ForeignKeyInfo[]; ddl: string }
export interface RoutineInfo { database: string; name: string; routineType: string; dataType?: string | null; comment?: string | null }
export interface TriggerInfo { database: string; name: string; tableName: string; timing: string; event: string }
export interface EventInfo { database: string; name: string; status: string; eventType: string }
export type DatabaseObjectKind = "view" | "procedure" | "function" | "trigger" | "event";
export interface DatabaseObjectDraft {
  mode: "visual" | "ddl";
  kind: DatabaseObjectKind;
  name: string;
  table: string;
  parameters: string;
  returnType: string;
  timing: string;
  event: string;
  schedule: string;
  body: string;
  ddl: string;
}
export interface DatabaseObjectDefinition { database: string; name: string; kind: DatabaseObjectKind; ddl: string }
export interface RoutineParameter { name?: string | null; mode?: string | null; dataType: string; ordinal: number }
export interface ServerProcessInfo { id: number; user: string; host: string; database?: string | null; command: string; timeSecs: number; state?: string | null; sql?: string | null }
export interface ServerMetric { name: string; value: string }
export interface UserAccount { user: string; host: string; plugin?: string | null; locked: boolean }
export interface ServerVariable { name: string; value: string; dynamic: boolean }
export interface ServerLockInfo {
  waitingThreadId: number;
  blockingThreadId?: number | null;
  objectName?: string | null;
  lockType: string;
  lockMode: string;
  lockStatus: string;
  waitingSql?: string | null;
}

export type CellValue =
  | { kind: "null" }
  | { kind: "bool"; value: boolean }
  | { kind: "signed" | "unsigned" | "decimal" | "text" | "date" | "time" | "date_time" | "json"; value: string }
  | { kind: "float"; value: number }
  | { kind: "bytes"; value: { base64: string; preview?: string | null; length: number } }
  | { kind: "geometry"; value: { wkbBase64: string; srid?: number | null } };

export interface ColumnMeta { name: string; databaseType: string; nullable: boolean; unsigned: boolean; binary: boolean }
export interface QueryMessage { severity: string; code?: string | null; message: string }
export interface QueryResultPage {
  executionId: UUID;
  columns: ColumnMeta[];
  rows: CellValue[][];
  affectedRows: number;
  executionTimeMs: number;
  truncated: boolean;
  hasMore: boolean;
  resultSetIndex: number;
  messages: QueryMessage[];
  rowOffset?: number;
  pageSize?: number;
  additionalResultSets?: QueryResultSet[];
}

export interface QueryResultSet {
  columns: ColumnMeta[];
  rows: CellValue[][];
  affectedRows: number;
  truncated: boolean;
  hasMore: boolean;
  resultSetIndex: number;
  rowOffset: number;
  pageSize: number;
}

export interface ExecuteQueryRequest {
  executionId: UUID;
  sql: string;
  database?: string | null;
  timeoutSecs?: number | null;
  allowWrite: boolean;
  pageSize?: number;
  rowOffset?: number;
}

export interface SqlAssessment {
  statementKind: string;
  risk: "safe" | "review" | "destructive";
  requiresConfirmation: boolean;
  reason?: string | null;
}

export type ExportFormat = "txt" | "sql" | "csv" | "excel";

export interface ResultExportOptions {
  format: ExportFormat;
  databaseName?: string | null;
  tableName?: string | null;
  databaseKind?: DatabaseKind;
}

export interface ExportSummary { outputPath: string; rowsWritten: number }
export interface DatabaseBackupSummary { outputPath: string; tablesWritten: number; objectsWritten: number; rowsWritten: number }
export interface TextFileContent { path: string; contents: string }
export interface DiagnosticsInfo { version: string; logPath?: string | null; logs: string }
export interface RuntimeStats { openConnectionCount: number; tabSessionCount: number; memoryBytes: number | null }
export interface AppSettings {
  queryPageSize: number;
  tablePageSize: number;
  showSystemDatabases: boolean;
  autoSaveWorkspace: boolean;
  backupIncludeData: boolean;
  editorFontSize?: number;
  editorTabSize?: number;
  confirmDestructiveQueries?: boolean;
  autoCheckUpdates?: boolean;
  updateManifestUrl?: string;
  defaultExportFormat?: ExportFormat;
  backupCompression?: "none" | "gzip";
  backupEncryption?: boolean;
}
export type ImportFormat = "csv" | "excel" | "sql";
export type ImportConflictStrategy = "error" | "ignore" | "replace" | "upsert";
export interface ImportColumnMapping { source: string; target: string | null }
export interface ImportPreviewRequest {
  inputPath: string;
  format: Exclude<ImportFormat, "sql">;
  hasHeaders: boolean;
  sheetName?: string | null;
  delimiter?: string | null;
  encoding?: "utf-8" | "utf-8-lossy" | "gb18030";
  previewRows?: number;
}
export interface ImportPreview {
  sheets: string[];
  selectedSheet?: string | null;
  columns: string[];
  rows: string[][];
  totalRows: number;
  detectedDelimiter?: string | null;
}
export interface ImportDataRequest {
  connectionId: UUID;
  database: string;
  table?: string | null;
  inputPath: string;
  format: ImportFormat;
  hasHeaders: boolean;
  taskId?: UUID;
  sheetName?: string | null;
  delimiter?: string | null;
  encoding?: "utf-8" | "utf-8-lossy" | "gb18030";
  mappings?: ImportColumnMapping[];
  nullValues?: string[];
  trimValues?: boolean;
  conflictStrategy?: ImportConflictStrategy;
  batchSize?: number;
  continueOnError?: boolean;
  encryptionPassword?: string | null;
}
export interface ImportRowError { rowNumber: number; message: string }
export interface ImportSummary {
  rowsImported: number;
  rowsSkipped?: number;
  statementsExecuted: number;
  errors?: ImportRowError[];
}

export interface TransferProgress {
  taskId: UUID;
  kind: "import" | "backup" | "restore" | "export";
  phase: string;
  completed: number;
  total?: number | null;
  message?: string | null;
}

export interface TransferTask extends TransferProgress {
  title: string;
  status: "running" | "completed" | "failed" | "cancelled";
  cancellable?: boolean;
  startedAt: string;
  finishedAt?: string | null;
  outputPath?: string | null;
  checksumSha256?: string | null;
  error?: string | null;
}

export interface BackupSchedule {
  enabled: boolean;
  connectionId: UUID;
  database: string;
  directory: string;
  intervalHours: number;
  nextRunAt: string;
  compression: "none" | "gzip";
  includeData: boolean;
}

export interface BackupOptions {
  includeData: boolean;
  compression: "none" | "gzip";
  encryptionPassword?: string | null;
  taskId?: UUID;
}
export interface BackupSummary extends DatabaseBackupSummary {
  checksumSha256?: string | null;
  bytesWritten?: number;
  encrypted?: boolean;
}

export interface QuerySnippet {
  id: UUID;
  name: string;
  sql: string;
  tags: string[];
}

export type RowMutationKind = "insert" | "update" | "delete";
export interface RowMutationRequest {
  database: string;
  table: string;
  kind: RowMutationKind;
  values: [string, CellValue][];
  keyValues: [string, CellValue][];
  originalValues: [string, CellValue][];
}
export interface RowMutationResult { affectedRows: number; concurrentChange: boolean }

export interface CommandError { code: string; message: string }
