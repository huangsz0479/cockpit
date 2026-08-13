<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import { open, save } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { join } from "@tauri-apps/api/path";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ArrowDownWideNarrow, ArrowUpDown, ArrowUpNarrowWide, Braces, ChevronDown, ChevronLeft, ChevronRight, CircleCheck, CircleMinus, Clipboard, Columns3, Copy, Database, Download, ExternalLink, Eye, FileCode2, FileUp, FolderOpen, KeyRound, ListFilter, MoreHorizontal, Pencil, Play, Plus, RefreshCw, RotateCcw, Search, Table2, Trash2, Unplug, X } from "lucide-vue-next";
import commitIcon from "../src-tauri/icons/database/commit.svg";
import databaseIcon from "../src-tauri/icons/database/database.svg";
import exportIcon from "../src-tauri/icons/database/export.svg";
import formatIcon from "../src-tauri/icons/database/fmatter.svg";
import mysqlIcon from "../src-tauri/icons/database/mysql.svg";
import postgresqlIcon from "../src-tauri/icons/database/pgsql.svg";
import rollbackIcon from "../src-tauri/icons/database/rollback.svg";
import runIcon from "../src-tauri/icons/database/run.svg";
import saveQueryIcon from "../src-tauri/icons/database/save.svg";
import sqliteIcon from "../src-tauri/icons/database/sql-lite.svg";
import stopIcon from "../src-tauri/icons/database/stop.svg";
import transactionIcon from "../src-tauri/icons/database/transaction.svg";
import { format } from "sql-formatter";
import ConnectionDialog from "@/components/ConnectionDialog.vue";
import ContextMenu from "@/components/ContextMenu.vue";
import CreateTableEditor from "@/components/CreateTableEditor.vue";
import CellViewer from "@/components/CellViewer.vue";
import InlineDatePicker from "@/components/InlineDatePicker.vue";
import ImportDataDialog from "@/components/ImportDataDialog.vue";
import QueryExportDialog from "@/components/QueryExportDialog.vue";
import TransferCenter from "@/components/TransferCenter.vue";
import SqlParameterDialog from "@/components/SqlParameterDialog.vue";
import SnippetDialog from "@/components/SnippetDialog.vue";
import ResultInsightsDialog from "@/components/ResultInsightsDialog.vue";
import BatchEditDialog from "@/components/BatchEditDialog.vue";
import ColumnManagerDialog from "@/components/ColumnManagerDialog.vue";
import DatabaseObjectEditor from "@/components/DatabaseObjectEditor.vue";
import SettingsDialog from "@/components/SettingsDialog.vue";
import DiagnosticsDialog from "@/components/DiagnosticsDialog.vue";
import ServerAdminPanel from "@/components/ServerAdminPanel.vue";
import RedisManager from "@/components/redis/RedisManager.vue";
import SchemaCompareDialog from "@/components/SchemaCompareDialog.vue";
import ActionDialog from "@/components/ActionDialog.vue";
import AppSelect from "@/components/AppSelect.vue";
import TableExportControl from "@/components/TableExportControl.vue";
import SqlEditor from "@/components/SqlEditor.vue";
import AppToolbar from "@/components/app-shell/AppToolbar.vue";
import NavigationSidebar from "@/components/app-shell/NavigationSidebar.vue";
import WorkspaceEmpty from "@/components/app-shell/WorkspaceEmpty.vue";
import WorkspaceTabs from "@/components/app-shell/WorkspaceTabs.vue";
import type { NavigationContextTarget as ContextTarget, ObjectGroupKind } from "@/components/app-shell/types";
import { cellText } from "@/lib/cell";
import { columnIsNullable, hasDatabaseDefault, isGeneratedColumn, parseRowCell, rowCellChanged, rowDraftValue, rowInputType, rowInputValue, type RowDraftCell } from "@/lib/rowEditing";
import { findSqlParameters } from "@/lib/sqlParameters";
import { useActionDialog } from "@/lib/actionDialog";
import { api } from "@/lib/api";
import { fetchLatestGitHubRelease, isNewerVersion } from "@/lib/githubRelease";
import { alterTableSql, canAppendSelectQueryLimit, canPageSelectQuery, createDefaultTableDefinition, createTableSql, quoteIdentifier, quoteMysqlIdentifier, selectPreviewSql, selectQueryPageSql, selectTablePageSql, singleTableSelectAllTargets, tableDetailToDefinition } from "@/lib/sql";
import type { CreateTableDefinition } from "@/lib/sql";
import { useAppStore } from "@/stores/app";
import type { AppSettings, BackupSchedule, CellValue, ColumnMeta, ConnectionProfile, DatabaseObjectDraft, DatabaseObjectKind, ExportFormat, QueryResultPage, QueryResultSet, QuerySnippet, RedisDatabaseInfo, RoutineInfo, RuntimeStats, TableDetail, TableInfo, TransferProgress, TransferTask, UUID } from "@/types";

type ResultView = "result" | "summary";

interface WorkspaceTab {
  id: string;
  sessionId: UUID;
  kind: "console" | "table" | "create-table" | "alter-table" | "database-object";
  title: string;
  connectionId?: UUID | null;
  database: string | null;
  createTableDefinition?: CreateTableDefinition;
  originalTableDefinition?: CreateTableDefinition;
  tableDetail?: TableDetail;
  editableTable?: TableInfo | null;
  editableResultSets?: Record<number, TableDetail>;
  resultSql?: string | null;
  sql: string;
  result: QueryResultPage | null;
  hasExecuted?: boolean;
  resultPanelClosed?: boolean;
  resultView?: ResultView;
  columnWidths: Record<string, number>;
  closable: boolean;
  resultSetIndex?: number;
  selectedRowIndex?: number | null;
  selectedRowIndexes?: number[];
  pageSize?: number;
  pageable?: boolean;
  generatedTitle?: boolean;
  filePath?: string | null;
  persistedSql?: string;
  filter?: string;
  appliedFilter?: string;
  sortColumn?: string | null;
  sortDirection?: "asc" | "desc";
  selectedCell?: { row: number; column: number } | null;
  pagingSql?: string | null;
  pagingUsesDriverOffset?: boolean;
  pinned?: boolean;
  resultFilter?: string;
  columnOrder?: string[];
  hiddenColumns?: string[];
  frozenColumnCount?: number;
  databaseObjectDraft?: DatabaseObjectDraft;
  databaseObjectPersistedDraft?: string;
  databaseObjectOriginalName?: string;
}

interface InlineRowEditorState {
  mode: "insert" | "update";
  tabId: string;
  rowIndex: number;
  columns: ColumnMeta[];
  row: CellValue[];
  draft: Record<string, RowDraftCell>;
  activeColumn: string;
  datePickerPlacement: "top-start" | "top-end" | "bottom-start" | "bottom-end";
}

const store = useAppStore();
const {
  actionDialog,
  confirmAction,
  promptAction,
  showNotice,
  acceptActionDialog,
  cancelActionDialog,
} = useActionDialog();
const { connections, connectionInfo, activeConnectionId, connected, databases, selectedDatabase, tables, tableHasMore, selectedTable, tableDetail, routines, triggers, events, transactionSessions, executingId, busy, error } = storeToRefs(store);
const showDialog = ref(false);
const showSettings = ref(false);
const appVersion = ref("—");
const updateCheckPending = ref(false);
const runtimeStats = ref<RuntimeStats | null>(null);
const runtimeStatsState = ref<"loading" | "ready" | "unavailable">("__TAURI_INTERNALS__" in window ? "loading" : "unavailable");
const showDiagnostics = ref(false);
const showServerAdmin = ref(false);
const redisManagerConnection = ref<ConnectionProfile | null>(null);
const redisManagerDatabase = ref<number | null>(null);
const redisDatabases = ref<Record<UUID, RedisDatabaseInfo[]>>({});
const redisLoading = ref<Record<UUID, boolean>>({});
const showImportDialog = ref(false);
const showTransferCenter = ref(false);
const showSnippetDialog = ref(false);
const parameterSql = ref<string | null>(null);
const showResultInsights = ref(false);
const showBatchEdit = ref(false);
const batchEditColumn = ref<string | null>(null);
const batchEditRowIndexes = ref<number[]>([]);
const columnBatchEditValue = ref("");
const columnBatchEditNull = ref(false);
const columnBatchEditError = ref("");
const columnDirectEditActive = ref(false);
const showColumnManager = ref(false);
const showQueryExportDialog = ref(false);
const tableActionsOpen = ref(false);
const tableActionsMenu = ref<HTMLElement | null>(null);
const columnMenuColumn = ref<string | null>(null);
const compareDatabase = ref<string | null>(null);
const editing = ref<ConnectionProfile | null>(null);
const creatingTableTabId = ref<string | null>(null);
const savingDatabaseObjectTabId = ref<string | null>(null);
const workspaceTabs = ref<WorkspaceTab[]>([]);
const activeWorkspaceTabId = ref<string | null>(null);
const expandedConnectionId = ref<UUID | null>(null);
const expandedDatabase = ref<string | null>(null);
const expandedTableGroup = ref(false);
const expandedViewGroup = ref(false);
const expandedFunctionGroup = ref(false);
const expandedTriggerGroup = ref(false);
const expandedEventGroup = ref(false);
const contextMenu = ref<{ x: number; y: number; target: ContextTarget } | null>(null);
const queryFileNotice = ref<{ kind: "info" | "success" | "warning"; message: string } | null>(null);
const inlineRowEditor = ref<InlineRowEditorState | null>(null);
const cellViewer = ref<{ column: string; value: CellValue } | null>(null);
const workspaceRestored = ref(false);
let workspaceSaveTimer: ReturnType<typeof setTimeout> | null = null;
let queryFileNoticeTimer: ReturnType<typeof setTimeout> | null = null;
let inlineRowSavePromise: Promise<boolean> | null = null;
const WORKSPACE_STATE_KEY = "main";
const SETTINGS_STATE_KEY = "settings";
const TRANSFER_TASKS_STORAGE_KEY = "cockpit:transfer-tasks";
const BACKUP_SCHEDULE_STORAGE_KEY = "cockpit:backup-schedule";
const SNIPPETS_STORAGE_KEY = "cockpit:snippets";
const shortcutModifier = /Macintosh|Mac OS X/.test(navigator.userAgent) ? "⌘" : "Ctrl+";
function storedJson<T>(key: string, fallback: T): T {
  try { return JSON.parse(localStorage.getItem(key) ?? "") as T; } catch { return fallback; }
}
const transferTasks = ref<TransferTask[]>(storedJson<TransferTask[]>(TRANSFER_TASKS_STORAGE_KEY, []));
const activeExportTaskId = ref<UUID | null>(null);
const activeExportTask = computed(() => activeExportTaskId.value
  ? transferTasks.value.find((task) => task.taskId === activeExportTaskId.value) ?? null
  : null);
const activeExportStatusLabel = computed(() => {
  if (activeExportTask.value?.status === "running") return "正在导出";
  if (activeExportTask.value?.status === "completed") return "导出成功";
  if (activeExportTask.value?.status === "cancelled") return "导出已取消";
  return "导出失败";
});
const activeExportPercent = computed(() => {
  const task = activeExportTask.value;
  if (!task || task.total == null) return null;
  if (task.status === "completed") return 100;
  if (task.total === 0) return 0;
  return Math.min(99, Math.floor((task.completed / task.total) * 100));
});
const backupSchedule = ref<BackupSchedule | null>(storedJson<BackupSchedule | null>(BACKUP_SCHEDULE_STORAGE_KEY, null));
const snippets = ref<QuerySnippet[]>(storedJson<QuerySnippet[]>(SNIPPETS_STORAGE_KEY, []));
let unlistenTransferProgress: UnlistenFn | null = null;
let backupScheduleTimer: ReturnType<typeof setInterval> | null = null;
let runtimeStatsTimer: ReturnType<typeof setInterval> | null = null;
let runtimeStatsRequestPending = false;
const RUNTIME_STATS_REFRESH_MS = 10_000;
const SYSTEM_DATABASES = new Set(["information_schema", "mysql", "performance_schema", "sys"]);
const DEFAULT_SETTINGS: AppSettings = {
  queryPageSize: 500,
  tablePageSize: 100,
  showSystemDatabases: false,
  autoSaveWorkspace: true,
  backupIncludeData: true,
  editorFontSize: 12,
  editorTabSize: 2,
  confirmDestructiveQueries: true,
  autoCheckUpdates: true,
  defaultExportFormat: "excel",
  backupCompression: "none",
  backupEncryption: false,
};
const settings = ref<AppSettings>({ ...DEFAULT_SETTINGS });
const DEFAULT_NAVIGATION_WIDTH = 292;
const MIN_NAVIGATION_WIDTH = 250;
const MAX_NAVIGATION_WIDTH = 460;
const NAVIGATION_WIDTH_STORAGE_KEY = "cockpit:navigation-width";
const storedNavigationWidth = Number.parseInt(localStorage.getItem(NAVIGATION_WIDTH_STORAGE_KEY) ?? "", 10);
const navigationWidth = ref(Number.isFinite(storedNavigationWidth) ? storedNavigationWidth : DEFAULT_NAVIGATION_WIDTH);
const isNavigationResizing = ref(false);
const workspaceContent = ref<HTMLElement | null>(null);
const DEFAULT_RESULT_PANEL_RATIO = 0.38;
const MIN_WORKSPACE_PANEL_HEIGHT = 160;
const RESULT_RESIZER_HEIGHT = 7;
const resultPanelRatio = ref(DEFAULT_RESULT_PANEL_RATIO);
const resultPanelHeight = ref<number | null>(null);
const isResultResizing = ref(false);
const DEFAULT_DATA_COLUMN_WIDTH = 160;
const MIN_DATA_COLUMN_WIDTH = 72;
const MAX_DATA_COLUMN_WIDTH = 1200;
const ROW_NUMBER_COLUMN_WIDTH = 42;
const COLUMN_RESIZE_STEP = 12;
const DATA_GRID_HEADER_HEIGHT = 36;
const DATA_GRID_ROW_HEIGHT = 24;
const DATA_GRID_OVERSCAN = 8;
const DATA_GRID_FALLBACK_VIEWPORT_HEIGHT = 480;
const DATA_GRID_INITIAL_ROW_COUNT = Math.ceil(DATA_GRID_FALLBACK_VIEWPORT_HEIGHT / DATA_GRID_ROW_HEIGHT) + DATA_GRID_OVERSCAN;
const isColumnResizing = ref(false);
const gridScroll = ref<HTMLElement | null>(null);
const gridRowWindow = ref({ start: 0, end: DATA_GRID_INITIAL_ROW_COUNT });
const EXPORT_FORMATS: { value: ExportFormat; label: string; extension: string }[] = [
  { value: "txt", label: "TXT", extension: "txt" },
  { value: "sql", label: "SQL", extension: "sql" },
  { value: "csv", label: "CSV", extension: "csv" },
  { value: "excel", label: "Excel", extension: "xlsx" },
];
const exportFormat = ref<ExportFormat>("excel");
const resultPanelStyle = computed(() => ({
  flexBasis: resultPanelHeight.value === null
    ? `calc(${DEFAULT_RESULT_PANEL_RATIO * 100}% - ${RESULT_RESIZER_HEIGHT * DEFAULT_RESULT_PANEL_RATIO}px)`
    : `${resultPanelHeight.value}px`,
}));
const baseTables = computed(() => tables.value.filter((table) => !table.tableType.includes("VIEW")));
const views = computed(() => tables.value.filter((table) => table.tableType.includes("VIEW")));
const filteredBaseTables = baseTables;
const filteredViews = views;
const filteredRoutines = routines;
const filteredTriggers = triggers;
const filteredEvents = events;
const connectionGroups = computed(() => {
  const groups = new Map<string, ConnectionProfile[]>();
  for (const connection of connections.value) {
    const name = connection.group?.trim() || "未分组";
    const group = groups.get(name) ?? [];
    group.push(connection);
    groups.set(name, group);
  }
  return [...groups]
    .sort(([left], [right]) => left === "未分组" ? 1 : right === "未分组" ? -1 : left.localeCompare(right, "zh-CN"))
    .map(([name, items]) => ({ name, connections: items }));
});
const editorSchema = computed<Record<string, readonly string[]>>(() => Object.fromEntries(
  tables.value.map((table) => [
    table.name,
    tableDetail.value?.table.name === table.name ? tableDetail.value.columns.map((column) => column.name) : [],
  ]),
));
const filteredDatabases = computed(() => {
  const driverKind = connections.value.find((connection) => connection.id === activeConnectionId.value)?.driverKind;
  const hidesMysqlSystemDatabases = (driverKind === "mysql" || driverKind === "mariadb") && !settings.value.showSystemDatabases;
  return hidesMysqlSystemDatabases
    ? databases.value.filter((database) => !SYSTEM_DATABASES.has(database.name))
    : databases.value;
});
const activeWorkspaceTab = computed<WorkspaceTab | null>(() => workspaceTabs.value.find((tab) => tab.id === activeWorkspaceTabId.value) ?? null);
const editorColumnCache = new Map<string, readonly string[]>();
const editorColumnRequests = new Map<string, Promise<readonly string[]>>();

function editorColumnCacheKey(connectionId: UUID, database: string, table: string) {
  return `${connectionId}\0${database}\0${table}`;
}

async function loadEditorTableColumns(table: string, database?: string): Promise<readonly string[]> {
  const tab = activeWorkspaceTab.value;
  const connectionId = tab?.connectionId ?? activeConnectionId.value;
  const targetDatabase = database || tab?.database || selectedDatabase.value;
  if (!connectionId || !targetDatabase) return [];
  const key = editorColumnCacheKey(connectionId, targetDatabase, table);
  const cached = editorColumnCache.get(key);
  if (cached) return cached;
  const pending = editorColumnRequests.get(key);
  if (pending) return pending;
  const request: Promise<readonly string[]> = api.listColumns(connectionId, targetDatabase, table)
    .then((columns) => {
      const names = columns.map((column) => column.name);
      editorColumnCache.set(key, names);
      return names;
    })
    .catch(() => [])
    .finally(() => editorColumnRequests.delete(key));
  editorColumnRequests.set(key, request);
  return request;
}

const sqlEditor = ref<InstanceType<typeof SqlEditor> | null>(null);
const activeWorkspaceTitle = computed(() => {
  const tab = activeWorkspaceTab.value;
  return tab && (!tab.generatedTitle || tab.title !== untitledQueryTitle(tab.connectionId)) ? tab.title : "";
});
const dirtyWorkspaceTabIds = computed(() => workspaceTabs.value.filter(workspaceTabIsDirty).map((tab) => tab.id));
const activeQueryConnection = computed(() => connections.value.find((connection) => connection.id === activeWorkspaceTab.value?.connectionId) ?? null);
const activeQueryConnectionIcon = computed(() => {
  const connection = activeQueryConnection.value;
  if (!connection) return databaseIcon;
  const kind = connection.driverKind;
  if (kind === "postgresql") return postgresqlIcon;
  if (kind === "sqlite") return sqliteIcon;
  if (kind === "mysql" || kind === "mariadb" || !kind) return mysqlIcon;
  return databaseIcon;
});
const activeConnectionKind = computed(() => connections.value.find((connection) => connection.id === activeConnectionId.value)?.driverKind ?? "mysql");
const activeCreateTableConnection = computed(() => connections.value.find((connection) => connection.id === activeWorkspaceTab.value?.connectionId) ?? null);
const sql = computed(() => activeWorkspaceTab.value?.sql ?? "");

function commitWorkspaceTabSql(tabId: string | null, value: string) {
  if (!tabId) return;
  const tab = workspaceTabs.value.find((item) => item.id === tabId);
  if (tab) tab.sql = value;
}

function flushSqlEditor() {
  sqlEditor.value?.flushTextInput?.();
}

function syncActiveSqlEditor() {
  const tab = activeWorkspaceTab.value;
  const value = sqlEditor.value?.currentValue?.();
  if (tab?.kind === "console" && typeof value === "string") tab.sql = value;
}
const result = computed(() => activeWorkspaceTab.value?.result ?? null);
const activeResultView = computed<ResultView>(() => activeWorkspaceTab.value?.resultView === "summary" ? "summary" : "result");
const activeQueryPanelVisible = computed(() => Boolean(activeWorkspaceTab.value?.hasExecuted && !activeWorkspaceTab.value.resultPanelClosed));
const allResultSets = computed<QueryResultSet[]>(() => {
  const page = activeWorkspaceTab.value?.result;
  if (!page) return [];
  return [{
    columns: page.columns,
    rows: page.rows,
    affectedRows: page.affectedRows,
    truncated: page.truncated,
    hasMore: page.hasMore,
    resultSetIndex: 0,
    rowOffset: page.rowOffset ?? 0,
    pageSize: page.pageSize ?? activeWorkspaceTab.value?.pageSize ?? 500,
  }, ...(page.additionalResultSets ?? [])];
});
const displayedResult = computed<QueryResultPage | null>(() => {
  const page = activeWorkspaceTab.value?.result;
  if (!page) return null;
  const selected = allResultSets.value[activeWorkspaceTab.value?.resultSetIndex ?? 0] ?? allResultSets.value[0];
  return selected ? { ...page, ...selected, additionalResultSets: page.additionalResultSets } : page;
});
const totalResultRows = computed(() => allResultSets.value.reduce((total, resultSet) => total + resultSet.rows.length, 0));
const totalAffectedRows = computed(() => allResultSets.value.reduce((total, resultSet) => total + resultSet.affectedRows, 0));
const resultHasMore = computed(() => allResultSets.value.some((resultSet) => resultSet.hasMore || resultSet.truncated));
const visibleResultRows = computed(() => {
  const rows = displayedResult.value?.rows ?? [];
  const filter = activeWorkspaceTab.value?.resultFilter?.trim().toLocaleLowerCase() ?? "";
  const visibleRows = rows
    .map((row, rowIndex) => ({ row, rowIndex }))
    .filter(({ row }) => !filter || row.some((cell) => cellText(cell).toLocaleLowerCase().includes(filter)));
  const editor = inlineRowEditor.value;
  if (editor?.mode === "insert" && editor.tabId === activeWorkspaceTab.value?.id) {
    visibleRows.push({ row: editor.row, rowIndex: editor.rowIndex });
  }
  return visibleRows;
});
const visibleColumnEntries = computed(() => {
  const columns = displayedResult.value?.columns ?? [];
  const order = activeWorkspaceTab.value?.columnOrder ?? [];
  const orderRanks = new Map(order.map((name, index) => [name, index]));
  const hidden = new Set(activeWorkspaceTab.value?.hiddenColumns ?? []);
  const ranked = [...columns]
    .map((column, sourceIndex) => ({ column, sourceIndex }))
    .sort((left, right) => {
      const leftRank = orderRanks.get(left.column.name) ?? Number.MAX_SAFE_INTEGER;
      const rightRank = orderRanks.get(right.column.name) ?? Number.MAX_SAFE_INTEGER;
      return leftRank - rightRank
        || left.sourceIndex - right.sourceIndex;
    });
  return ranked.filter((entry) => !hidden.has(entry.column.name));
});
const virtualResultRowWindow = computed(() => {
  const total = visibleResultRows.value.length;
  let start = Math.min(gridRowWindow.value.start, Math.max(0, total - 1));
  let end = Math.min(total, Math.max(start, gridRowWindow.value.end));
  const editor = inlineRowEditor.value?.tabId === activeWorkspaceTabId.value ? inlineRowEditor.value : null;
  const editingRowIndex = editor?.rowIndex;
  const editingRowPosition = editingRowIndex == null
    ? -1
    : visibleResultRows.value.findIndex((entry) => entry.rowIndex === editingRowIndex);
  if (editingRowPosition >= 0 && (editingRowPosition < start || editingRowPosition >= end)) {
    start = Math.max(0, editingRowPosition - DATA_GRID_OVERSCAN);
    end = Math.min(total, start + DATA_GRID_INITIAL_ROW_COUNT);
  }
  return { start, end };
});
const renderedResultRows = computed(() => {
  const { start, end } = virtualResultRowWindow.value;
  return visibleResultRows.value.slice(start, end).map((entry, index) => ({
    ...entry,
    visibleIndex: start + index,
  }));
});
const virtualResultPaddingTop = computed(() => virtualResultRowWindow.value.start * DATA_GRID_ROW_HEIGHT);
const virtualResultPaddingBottom = computed(() => (
  visibleResultRows.value.length - virtualResultRowWindow.value.end
) * DATA_GRID_ROW_HEIGHT);
const activeTransaction = computed(() => Boolean(
  activeWorkspaceTab.value?.sessionId
  && transactionSessions.value[activeWorkspaceTab.value.sessionId],
));
const activeInlineRowEditor = computed(() => inlineRowEditor.value?.tabId === activeWorkspaceTabId.value
  ? inlineRowEditor.value
  : null);
const selectedRowIndexes = computed(() => activeWorkspaceTab.value?.selectedRowIndexes ?? []);
const selectedVisibleRowCount = computed(() => {
  const selected = new Set(selectedRowIndexes.value);
  return visibleResultRows.value.reduce((count, entry) => count + Number(selected.has(entry.rowIndex)), 0);
});
function editableTableForTab(tab: WorkspaceTab | null | undefined) {
  if (!tab) return null;
  if (tab.kind === "table") return tab.tableDetail?.table ?? null;
  return tab.kind === "console" ? tab.editableTable ?? null : null;
}
function selectConsoleEditableResult(tab: WorkspaceTab, resultSetIndex: number) {
  if (tab.kind !== "console") return;
  const detail = tab.editableResultSets?.[resultSetIndex];
  tab.editableTable = detail?.table ?? null;
  tab.tableDetail = detail;
}
const activeEditableTable = computed(() => {
  return editableTableForTab(activeWorkspaceTab.value);
});
const activeTableHasUniqueKey = computed(() => Boolean(
  activeWorkspaceTab.value?.tableDetail && uniqueKeyColumns(activeWorkspaceTab.value.tableDetail).length,
));
const batchEditableColumns = computed(() => {
  const tab = activeWorkspaceTab.value;
  const page = displayedResult.value;
  if (!tab?.tableDetail || !page) return [];
  return page.columns.filter((column) => !isGeneratedColumn(
    tab.tableDetail?.columns.find((detail) => detail.name === column.name),
  ));
});
const activeBatchEditColumn = computed(() => batchEditableColumns.value.find(
  (column) => column.name === batchEditColumn.value,
) ?? null);
const columnBatchEditInputType = computed(() => activeBatchEditColumn.value
  ? rowInputType(activeBatchEditColumn.value)
  : "text");
const selectedForeignKey = computed(() => {
  const tab = activeWorkspaceTab.value;
  const selected = tab?.selectedCell;
  const page = displayedResult.value;
  if (!tab?.tableDetail || !selected || !page) return null;
  const column = page.columns[selected.column]?.name;
  if (!column) return null;
  const foreignKey = tab.tableDetail.foreignKeys.find((item) => item.columns.includes(column));
  if (!foreignKey) return null;
  const position = foreignKey.columns.indexOf(column);
  return { foreignKey, referencedColumn: foreignKey.referencedColumns[position]!, value: page.rows[selected.row]?.[selected.column] };
});
let navigationResizeStartX = 0;
let navigationResizeStartWidth = 0;
let resultResizeStartY = 0;
let resultResizeStartHeight = 0;
let columnResizeStartX = 0;
let columnResizeStartWidth = 0;
let resizingColumnKey: string | null = null;
let resizingColumnTabId: string | null = null;
let gridResizeObserver: ResizeObserver | null = null;

function activateWorkspaceTabById(id: string) {
  const tab = workspaceTabs.value.find((item) => item.id === id);
  if (tab) activateWorkspaceTab(tab);
}

function closeTableActionsMenu() {
  tableActionsOpen.value = false;
}

function closeColumnMenu() {
  columnMenuColumn.value = null;
}

function toggleColumnMenu(columnName: string) {
  columnMenuColumn.value = columnMenuColumn.value === columnName ? null : columnName;
}

function handleOutsidePointer(event: PointerEvent) {
  const target = event.target;
  if (tableActionsOpen.value && target instanceof Node && !tableActionsMenu.value?.contains(target)) closeTableActionsMenu();
  if (columnMenuColumn.value && !(target instanceof Element && target.closest(".column-header-menu"))) closeColumnMenu();
  if (
    batchEditColumn.value
    && !(target instanceof Element && target.closest("th.selected-column, .column-batch-edit-bar"))
  ) cancelColumnBatchEdit();
}

function handleTableActionsMenuClick(event: MouseEvent) {
  const target = event.target;
  if (target instanceof Element && target.closest("button")) closeTableActionsMenu();
}

async function refreshRuntimeStats() {
  if (runtimeStatsRequestPending) return;
  runtimeStatsRequestPending = true;
  try {
    runtimeStats.value = await api.runtimeStats();
    runtimeStatsState.value = "ready";
  } catch {
    runtimeStatsState.value = "unavailable";
  } finally {
    runtimeStatsRequestPending = false;
  }
}

onMounted(async () => {
  applySettingsEffects();
  constrainNavigationWidth();
  window.addEventListener("resize", constrainNavigationWidth);
  window.addEventListener("resize", constrainResultPanelHeight);
  window.addEventListener("keydown", handleGlobalShortcut);
  window.addEventListener("beforeunload", handleBeforeUnload);
  document.addEventListener("pointerdown", handleOutsidePointer);
  if ("__TAURI_INTERNALS__" in window) {
    void refreshRuntimeStats();
    runtimeStatsTimer = setInterval(() => { void refreshRuntimeStats(); }, RUNTIME_STATS_REFRESH_MS);
    try { appVersion.value = await getVersion(); } catch { appVersion.value = "—"; }
    unlistenTransferProgress = await listen<TransferProgress>("transfer-progress", ({ payload }) => updateTransferProgress(payload));
    backupScheduleTimer = setInterval(() => { void checkBackupSchedule(); }, 60_000);
    await loadSettings();
    if (settings.value.autoCheckUpdates) void checkForUpdates(false);
    await store.loadConnections();
    if (settings.value.autoSaveWorkspace) await restoreWorkspace();
    else workspaceRestored.value = true;
    void checkBackupSchedule();
  } else {
    appVersion.value = "开发预览";
    workspaceRestored.value = true;
  }
});

onBeforeUnmount(() => {
  window.removeEventListener("resize", constrainNavigationWidth);
  window.removeEventListener("resize", constrainResultPanelHeight);
  window.removeEventListener("keydown", handleGlobalShortcut);
  window.removeEventListener("beforeunload", handleBeforeUnload);
  document.removeEventListener("pointerdown", handleOutsidePointer);
  if (workspaceSaveTimer) clearTimeout(workspaceSaveTimer);
  if (queryFileNoticeTimer) clearTimeout(queryFileNoticeTimer);
  unlistenTransferProgress?.();
  if (backupScheduleTimer) clearInterval(backupScheduleTimer);
  if (runtimeStatsTimer) clearInterval(runtimeStatsTimer);
  gridResizeObserver?.disconnect();
});

function persistTransferTasks() {
  const retained = transferTasks.value.slice(0, 100);
  transferTasks.value = retained;
  localStorage.setItem(TRANSFER_TASKS_STORAGE_KEY, JSON.stringify(retained));
}

function updateTransferProgress(progress: TransferProgress) {
  let task = transferTasks.value.find((item) => item.taskId === progress.taskId);
  if (!task) {
    task = {
      ...progress,
      title: progress.kind === "import" ? "数据导入" : progress.kind === "restore" ? "SQL 恢复" : progress.kind === "backup" ? "数据库备份" : "数据导出",
      status: "running",
      startedAt: new Date().toISOString(),
    };
    transferTasks.value.unshift(task);
  } else {
    Object.assign(task, progress);
  }
  if (progress.phase === "完成") {
    task.status = "completed";
    task.finishedAt = new Date().toISOString();
  } else if (progress.phase === "已取消") {
    task.status = "cancelled";
    task.finishedAt = new Date().toISOString();
  } else if (progress.phase === "失败") {
    task.status = "failed";
    task.error = progress.message;
    task.finishedAt = new Date().toISOString();
  }
  persistTransferTasks();
}

function createTransferTask(task: TransferTask) {
  transferTasks.value.unshift(task);
  persistTransferTasks();
}

function finishTransferTask(taskId: UUID, status: TransferTask["status"], details: Partial<TransferTask> = {}) {
  const task = transferTasks.value.find((item) => item.taskId === taskId);
  if (task) Object.assign(task, details, { status, finishedAt: new Date().toISOString() });
  persistTransferTasks();
}

function confirmDestructiveAction(message: string, detail = "此操作不可撤销。") {
  return confirmAction({
    title: "确认危险操作",
    message,
    detail,
    tone: "danger",
    confirmLabel: "继续操作",
  });
}

async function cancelTransferTask(taskId: UUID) {
  if (await api.cancelTransfer(taskId)) finishTransferTask(taskId, "cancelled", { phase: "已取消" });
}

async function clearTransferTasks() {
  const finishedCount = transferTasks.value.filter((task) => task.status !== "running").length;
  if (!finishedCount || !await confirmDestructiveAction(
    `清空 ${finishedCount} 条已结束的任务记录？`,
    "运行中的任务不会受影响；清空后的记录无法恢复。",
  )) return;
  transferTasks.value = transferTasks.value.filter((task) => task.status === "running");
  persistTransferTasks();
}

function saveBackupSchedule(schedule: BackupSchedule | null) {
  backupSchedule.value = schedule;
  if (schedule) localStorage.setItem(BACKUP_SCHEDULE_STORAGE_KEY, JSON.stringify(schedule));
  else localStorage.removeItem(BACKUP_SCHEDULE_STORAGE_KEY);
}

function persistSnippets() {
  localStorage.setItem(SNIPPETS_STORAGE_KEY, JSON.stringify(snippets.value));
}

function saveSnippet(snippet: QuerySnippet) {
  snippets.value.push(snippet);
  snippets.value.sort((left, right) => left.name.localeCompare(right.name));
  persistSnippets();
}

async function removeSnippet(snippet: QuerySnippet) {
  if (!await confirmDestructiveAction(`删除 SQL 片段“${snippet.name}”？`)) return;
  snippets.value = snippets.value.filter((item) => item.id !== snippet.id);
  persistSnippets();
}

function openSnippet(snippet: QuerySnippet) {
  createQueryTab(snippet.sql, selectedDatabase.value);
  showSnippetDialog.value = false;
}

function handleBeforeUnload(event: BeforeUnloadEvent) {
  if (!workspaceTabs.value.some(workspaceTabIsDirty)) return;
  event.preventDefault();
  event.returnValue = "";
}

function handleGlobalShortcut(event: KeyboardEvent) {
  if (event.key === "Escape") {
    if (event.defaultPrevented) return;
    let closed = true;
    if (actionDialog.value) cancelActionDialog();
    else if (contextMenu.value) closeContextMenu();
    else if (columnMenuColumn.value) closeColumnMenu();
    else if (showQueryExportDialog.value) showQueryExportDialog.value = false;
    else if (tableActionsOpen.value) closeTableActionsMenu();
    else if (showDiagnostics.value) showDiagnostics.value = false;
    else if (cellViewer.value) cellViewer.value = null;
    else if (parameterSql.value) parameterSql.value = null;
    else if (showSnippetDialog.value) showSnippetDialog.value = false;
    else if (showResultInsights.value) showResultInsights.value = false;
    else if (showBatchEdit.value) closeBatchEditDialog();
    else if (activeBatchEditColumn.value) cancelColumnBatchEdit();
    else if (showColumnManager.value) showColumnManager.value = false;
    else if (showServerAdmin.value) showServerAdmin.value = false;
    else if (showTransferCenter.value) showTransferCenter.value = false;
    else if (compareDatabase.value) compareDatabase.value = null;
    else if (showSettings.value) showSettings.value = false;
    else if (showDialog.value) { showDialog.value = false; editing.value = null; }
    else closed = false;
    if (closed) event.preventDefault();
    return;
  }
  if (!(event.metaKey || event.ctrlKey) || event.altKey) return;
  if (
    actionDialog.value
    || showDialog.value
    || showSettings.value
    || showDiagnostics.value
    || showServerAdmin.value
    || showImportDialog.value
    || showTransferCenter.value
    || showSnippetDialog.value
    || parameterSql.value
    || showResultInsights.value
    || showBatchEdit.value
    || showColumnManager.value
    || compareDatabase.value
    || cellViewer.value
  ) return;
  const key = event.key.toLowerCase();
  if (key === "n") {
    event.preventDefault();
    createQuery();
  } else if (key === "o") {
    event.preventDefault();
    void openSqlFile();
  } else if (key === "s") {
    event.preventDefault();
    void saveCurrentQuery(event.shiftKey);
  } else if (key === "w" && activeWorkspaceTabId.value) {
    event.preventDefault();
    closeWorkspaceTab(activeWorkspaceTabId.value);
  } else if (key === ",") {
    event.preventDefault();
    showSettings.value = true;
  }
}

// 有限深度仍覆盖页签字段与 columnWidths，但不会遍历 result.rows 的每个单元格。
watch([workspaceTabs, activeWorkspaceTabId], () => {
  if (!settings.value.autoSaveWorkspace || !workspaceRestored.value || !("__TAURI_INTERNALS__" in window)) return;
  if (workspaceSaveTimer) clearTimeout(workspaceSaveTimer);
  workspaceSaveTimer = setTimeout(() => { void persistWorkspace(); }, 350);
}, { deep: 4 });

watch([
  activeWorkspaceTabId,
  () => activeWorkspaceTab.value?.resultSetIndex,
  () => activeWorkspaceTab.value?.resultFilter,
  () => displayedResult.value?.rows,
], resetGridRowWindow);

watch([activeWorkspaceTabId, () => displayedResult.value?.executionId], () => { showQueryExportDialog.value = false; });

watch(gridScroll, (element) => {
  gridResizeObserver?.disconnect();
  gridResizeObserver = null;
  if (element && typeof ResizeObserver !== "undefined") {
    gridResizeObserver = new ResizeObserver(() => updateGridRowWindow(element));
    gridResizeObserver.observe(element);
  }
  void nextTick(() => updateGridRowWindow(element));
}, { flush: "post" });

async function loadSettings() {
  try {
    const payload = await api.loadWorkspaceState(SETTINGS_STATE_KEY);
    if (payload) {
      const loaded = { ...DEFAULT_SETTINGS, ...JSON.parse(payload) as Partial<AppSettings> } as AppSettings & { theme?: unknown; updateManifestUrl?: unknown };
      delete loaded.theme;
      delete loaded.updateManifestUrl;
      settings.value = loaded;
    }
    applySettingsEffects();
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
  }
}

async function saveSettings(next: AppSettings) {
  settings.value = { ...DEFAULT_SETTINGS, ...next };
  applySettingsEffects();
  for (const tab of workspaceTabs.value) {
    if (tab.kind === "console") tab.pageSize = settings.value.queryPageSize;
    else if (tab.kind === "table") tab.pageSize = settings.value.tablePageSize;
  }
  showSettings.value = false;
  if (!("__TAURI_INTERNALS__" in window)) return;
  try {
    await api.saveWorkspaceState(SETTINGS_STATE_KEY, JSON.stringify(settings.value));
    if (settings.value.autoSaveWorkspace) {
      workspaceRestored.value = true;
      await persistWorkspace();
    }
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
  }
}

function applySettingsEffects() {
  delete document.documentElement.dataset.theme;
  exportFormat.value = settings.value.defaultExportFormat ?? "excel";
  document.documentElement.style.setProperty("--editor-font-size", `${settings.value.editorFontSize ?? 12}px`);
}

async function checkForUpdates(notifyWhenCurrent = true) {
  if (updateCheckPending.value) return;
  updateCheckPending.value = true;
  let showFailure = notifyWhenCurrent;
  try {
    const [release, currentVersion] = await Promise.all([
      fetchLatestGitHubRelease(),
      getVersion(),
    ]);
    if (!isNewerVersion(release.version, currentVersion)) {
      if (notifyWhenCurrent) await showNotice({
        title: "已是最新版本",
        message: `当前版本 ${currentVersion} 已是最新版本。`,
        tone: "success",
      });
      return;
    }

    showFailure = true;
    const openDownloadPage = await confirmAction({
      title: "发现新版本",
      message: `Cockpit ${release.version} 已发布（当前版本 ${currentVersion}）。`,
      detail: release.notes?.trim() || "可前往 GitHub Releases 查看说明并下载安装包。",
      tone: "success",
      confirmLabel: "前往 GitHub 下载",
      cancelLabel: "稍后",
    });
    if (openDownloadPage) await openUrl(release.url);
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : String(cause);
    if (showFailure) await showNotice({
      title: "检查更新失败",
      message,
      detail: "请检查网络连接后重试，也可直接打开 Cockpit 的 GitHub Releases 页面。",
      tone: "warning",
    });
    else console.warn(`automatic update check failed: ${message}`);
  } finally {
    updateCheckPending.value = false;
  }
}

async function restoreWorkspace() {
  try {
    const payload = await api.loadWorkspaceState(WORKSPACE_STATE_KEY);
    if (payload) {
      const snapshot = JSON.parse(payload) as { tabs?: WorkspaceTab[]; activeTabId?: string | null };
      const restoredTabs: WorkspaceTab[] = [];
      let skippedFileTabs = 0;
      for (const originalTab of (snapshot.tabs ?? []).filter((tab) => !isDatabaseViewTab(tab))) {
        let tab = originalTab;
        if (tab.kind === "console" && tab.filePath) {
          try {
            const file = await api.readTextFile(tab.filePath);
            tab = { ...tab, sql: file.contents, persistedSql: file.contents, filePath: file.path };
          } catch {
            skippedFileTabs += 1;
            continue;
          }
        }
        const generatedTitle = tab.kind === "console" && (tab.generatedTitle
          ?? (!tab.filePath && /^查询 \d+$/.test(tab.title)));
        restoredTabs.push({
          ...tab,
          sessionId: crypto.randomUUID(),
          title: generatedTitle ? untitledQueryTitle(tab.connectionId) : tab.title,
          generatedTitle,
          result: null,
          columnWidths: tab.columnWidths ?? {},
          closable: true,
          selectedRowIndex: null,
          selectedRowIndexes: [],
          selectedCell: null,
          tableDetail: tab.kind === "console" ? undefined : tab.tableDetail,
          editableTable: null,
          pageSize: tab.pageSize ?? (tab.kind === "table" ? settings.value.tablePageSize : settings.value.queryPageSize),
          persistedSql: tab.persistedSql ?? tab.sql,
        });
      }
      workspaceTabs.value = restoredTabs;
      activeWorkspaceTabId.value = workspaceTabs.value.some((tab) => tab.id === snapshot.activeTabId)
        ? snapshot.activeTabId ?? null
        : workspaceTabs.value[0]?.id ?? null;
      if (activeWorkspaceTab.value) await applyQueryContext(activeWorkspaceTab.value);
      if (skippedFileTabs) showQueryFileNotice(`已跳过 ${skippedFileTabs} 个文件不存在或无法读取的查询页签。`, "warning");
    }
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    workspaceRestored.value = true;
  }
}

async function persistWorkspace() {
  try {
    const tabs = workspaceTabs.value.map((tab) => {
      const { sessionId: _sessionId, ...persistedTab } = tab;
      return {
        ...persistedTab,
        result: null,
        selectedRowIndex: null,
        tableDetail: tab.kind === "console" ? undefined : tab.tableDetail,
        editableTable: null,
      };
    });
    await api.saveWorkspaceState(WORKSPACE_STATE_KEY, JSON.stringify({
      tabs,
      activeTabId: activeWorkspaceTabId.value,
    }));
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
  }
}

function collapseObjectGroups() {
  expandedTableGroup.value = false;
  expandedViewGroup.value = false;
  expandedFunctionGroup.value = false;
  expandedTriggerGroup.value = false;
  expandedEventGroup.value = false;
}

function navigationWidthBounds() {
  const detailWidth = window.innerWidth <= 1180 ? 0 : 300;
  const workspaceWidth = window.innerWidth <= 1180 ? 450 : 480;
  return {
    min: MIN_NAVIGATION_WIDTH,
    max: Math.max(MIN_NAVIGATION_WIDTH, Math.min(MAX_NAVIGATION_WIDTH, window.innerWidth - detailWidth - workspaceWidth)),
  };
}

function setNavigationWidth(width: number, persist = false) {
  const { min, max } = navigationWidthBounds();
  navigationWidth.value = Math.round(Math.min(max, Math.max(min, width)));
  if (persist) localStorage.setItem(NAVIGATION_WIDTH_STORAGE_KEY, String(navigationWidth.value));
}

function constrainNavigationWidth() {
  setNavigationWidth(navigationWidth.value);
}

function startNavigationResize(event: PointerEvent) {
  event.preventDefault();
  navigationResizeStartX = event.clientX;
  navigationResizeStartWidth = navigationWidth.value;
  isNavigationResizing.value = true;
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
}

function resizeNavigation(event: PointerEvent) {
  if (!isNavigationResizing.value) return;
  setNavigationWidth(navigationResizeStartWidth + event.clientX - navigationResizeStartX);
}

function finishNavigationResize(event: PointerEvent) {
  if (!isNavigationResizing.value) return;
  isNavigationResizing.value = false;
  localStorage.setItem(NAVIGATION_WIDTH_STORAGE_KEY, String(navigationWidth.value));
  const target = event.currentTarget as HTMLElement;
  if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
}

function resetNavigationWidth() {
  setNavigationWidth(DEFAULT_NAVIGATION_WIDTH, true);
}

function resizeNavigationWithKeyboard(event: KeyboardEvent) {
  if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  const { min, max } = navigationWidthBounds();
  if (event.key === "Home") setNavigationWidth(min, true);
  else if (event.key === "End") setNavigationWidth(max, true);
  else setNavigationWidth(navigationWidth.value + (event.key === "ArrowLeft" ? -12 : 12), true);
}

function resultPanelBounds() {
  const availableHeight = Math.max(0, (workspaceContent.value?.clientHeight ?? 0) - RESULT_RESIZER_HEIGHT);
  const minimumHeight = Math.min(MIN_WORKSPACE_PANEL_HEIGHT, availableHeight / 2);
  return {
    availableHeight,
    min: minimumHeight,
    max: Math.max(minimumHeight, availableHeight - minimumHeight),
  };
}

function setResultPanelHeight(height: number) {
  const { availableHeight, min, max } = resultPanelBounds();
  if (!availableHeight) return;
  resultPanelHeight.value = Math.min(max, Math.max(min, height));
  resultPanelRatio.value = resultPanelHeight.value / availableHeight;
}

function currentResultPanelHeight() {
  if (resultPanelHeight.value !== null) return resultPanelHeight.value;
  return workspaceContent.value?.querySelector<HTMLElement>(".result-card")?.getBoundingClientRect().height ?? 0;
}

function constrainResultPanelHeight() {
  if (resultPanelHeight.value !== null) setResultPanelHeight(resultPanelHeight.value);
}

function startResultResize(event: PointerEvent) {
  event.preventDefault();
  const { availableHeight } = resultPanelBounds();
  if (!availableHeight) return;
  resultResizeStartY = event.clientY;
  resultResizeStartHeight = currentResultPanelHeight();
  isResultResizing.value = true;
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
}

function resizeResult(event: PointerEvent) {
  if (!isResultResizing.value) return;
  setResultPanelHeight(resultResizeStartHeight - (event.clientY - resultResizeStartY));
}

function finishResultResize(event: PointerEvent) {
  if (!isResultResizing.value) return;
  isResultResizing.value = false;
  const target = event.currentTarget as HTMLElement;
  if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
}

function resetResultPanelHeight() {
  resultPanelHeight.value = null;
  resultPanelRatio.value = DEFAULT_RESULT_PANEL_RATIO;
}

function resizeResultWithKeyboard(event: KeyboardEvent) {
  if (!["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  const { availableHeight, min, max } = resultPanelBounds();
  if (!availableHeight) return;
  if (event.key === "Home") setResultPanelHeight(min);
  else if (event.key === "End") setResultPanelHeight(max);
  else setResultPanelHeight(currentResultPanelHeight() + (event.key === "ArrowUp" ? 16 : -16));
}

function updateGridRowWindow(element = gridScroll.value) {
  const rowCount = visibleResultRows.value.length;
  const viewportHeight = element?.clientHeight || DATA_GRID_FALLBACK_VIEWPORT_HEIGHT;
  const bodyScrollTop = Math.max(0, (element?.scrollTop ?? 0) - DATA_GRID_HEADER_HEIGHT);
  const firstVisibleRow = Math.floor(bodyScrollTop / DATA_GRID_ROW_HEIGHT);
  const start = Math.max(0, firstVisibleRow - DATA_GRID_OVERSCAN);
  const end = Math.min(
    rowCount,
    Math.ceil((bodyScrollTop + viewportHeight) / DATA_GRID_ROW_HEIGHT) + DATA_GRID_OVERSCAN,
  );
  if (gridRowWindow.value.start !== start || gridRowWindow.value.end !== end) {
    gridRowWindow.value = { start, end };
  }
}

function handleGridScroll(event: Event) {
  updateGridRowWindow(event.currentTarget as HTMLElement);
}

function resetGridRowWindow() {
  gridRowWindow.value = { start: 0, end: DATA_GRID_INITIAL_ROW_COUNT };
  if (gridScroll.value) gridScroll.value.scrollTop = 0;
  void nextTick(() => updateGridRowWindow());
}

function ensureGridRowVisible(rowIndex: number) {
  const rowPosition = visibleResultRows.value.findIndex((entry) => entry.rowIndex === rowIndex);
  if (rowPosition < 0) return false;
  const element = gridScroll.value;
  if (element) {
    const viewportHeight = element.clientHeight || DATA_GRID_FALLBACK_VIEWPORT_HEIGHT;
    const rowTop = DATA_GRID_HEADER_HEIGHT + rowPosition * DATA_GRID_ROW_HEIGHT;
    const rowBottom = rowTop + DATA_GRID_ROW_HEIGHT;
    const visibleTop = element.scrollTop + DATA_GRID_HEADER_HEIGHT;
    const visibleBottom = element.scrollTop + viewportHeight;
    if (rowTop < visibleTop) element.scrollTop = Math.max(0, rowTop - DATA_GRID_HEADER_HEIGHT);
    else if (rowBottom > visibleBottom) element.scrollTop = Math.max(0, rowBottom - viewportHeight);
    updateGridRowWindow(element);
  } else {
    const start = Math.max(0, rowPosition - DATA_GRID_OVERSCAN);
    gridRowWindow.value = { start, end: start + DATA_GRID_INITIAL_ROW_COUNT };
  }
  return true;
}

function columnKey(column: ColumnMeta, index: number) {
  return `${index}:${column.name}`;
}

function columnWidth(column: ColumnMeta, index: number) {
  return activeWorkspaceTab.value?.columnWidths[columnKey(column, index)] ?? DEFAULT_DATA_COLUMN_WIDTH;
}

function gridLeadingWidth() {
  return activeWorkspaceTab.value?.kind === "table"
    ? ROW_NUMBER_COLUMN_WIDTH + 30
    : activeEditableTable.value ? 30 : 0;
}

function gridWidth(entries: readonly { column: ColumnMeta; sourceIndex: number }[]) {
  return gridLeadingWidth()
    + entries.reduce((width, entry) => width + columnWidth(entry.column, entry.sourceIndex), 0);
}

const frozenColumnOffsets = computed(() => {
  let left = gridLeadingWidth();
  return visibleColumnEntries.value.map((entry) => {
    const offset = left;
    left += columnWidth(entry.column, entry.sourceIndex);
    return offset;
  });
});

function frozenColumnStyle(displayIndex: number, column: ColumnMeta, sourceIndex: number) {
  const count = activeWorkspaceTab.value?.frozenColumnCount ?? 0;
  if (displayIndex >= count) return undefined;
  const left = frozenColumnOffsets.value[displayIndex] ?? gridLeadingWidth();
  return { position: "sticky" as const, left: `${left}px`, zIndex: 3, width: `${columnWidth(column, sourceIndex)}px` };
}

function applyColumnConfiguration(order: string[], hidden: string[], frozenCount: number) {
  const tab = activeWorkspaceTab.value;
  if (!tab) return;
  tab.columnOrder = order;
  tab.hiddenColumns = hidden;
  tab.frozenColumnCount = frozenCount;
  showColumnManager.value = false;
}

function setColumnWidth(tab: WorkspaceTab, key: string, width: number) {
  tab.columnWidths[key] = Math.round(Math.min(MAX_DATA_COLUMN_WIDTH, Math.max(MIN_DATA_COLUMN_WIDTH, width)));
}

function startColumnResize(event: PointerEvent, column: ColumnMeta, index: number) {
  const tab = activeWorkspaceTab.value;
  if (!tab) return;
  event.preventDefault();
  event.stopPropagation();
  const target = event.currentTarget as HTMLElement;
  resizingColumnKey = columnKey(column, index);
  resizingColumnTabId = tab.id;
  columnResizeStartX = event.clientX;
  columnResizeStartWidth = target.closest("th")?.getBoundingClientRect().width ?? columnWidth(column, index);
  setColumnWidth(tab, resizingColumnKey, columnResizeStartWidth);
  isColumnResizing.value = true;
  target.setPointerCapture(event.pointerId);
}

function resizeColumn(event: PointerEvent) {
  if (!isColumnResizing.value || !resizingColumnKey || !resizingColumnTabId) return;
  const tab = workspaceTabs.value.find((item) => item.id === resizingColumnTabId);
  if (tab) setColumnWidth(tab, resizingColumnKey, columnResizeStartWidth + event.clientX - columnResizeStartX);
}

function finishColumnResize(event: PointerEvent) {
  if (!isColumnResizing.value) return;
  isColumnResizing.value = false;
  resizingColumnKey = null;
  resizingColumnTabId = null;
  const target = event.currentTarget as HTMLElement;
  if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
}

function resetColumnWidth(column: ColumnMeta, index: number) {
  const tab = activeWorkspaceTab.value;
  if (tab) delete tab.columnWidths[columnKey(column, index)];
}

function resizeColumnWithKeyboard(event: KeyboardEvent, column: ColumnMeta, index: number) {
  if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
  const tab = activeWorkspaceTab.value;
  if (!tab) return;
  event.preventDefault();
  const key = columnKey(column, index);
  if (event.key === "Home") setColumnWidth(tab, key, MIN_DATA_COLUMN_WIDTH);
  else if (event.key === "End") setColumnWidth(tab, key, MAX_DATA_COLUMN_WIDTH);
  else setColumnWidth(tab, key, columnWidth(column, index) + (event.key === "ArrowLeft" ? -COLUMN_RESIZE_STEP : COLUMN_RESIZE_STEP));
}

async function saveConnection(profile: ConnectionProfile, password?: string) {
  if (await store.saveConnection(profile, password)) { showDialog.value = false; editing.value = null; }
}

function editConnection(profile: ConnectionProfile) { editing.value = profile; showDialog.value = true; }

function openRedisManager(connection: ConnectionProfile, database?: number) {
  redisManagerConnection.value = connection;
  redisManagerDatabase.value = database ?? null;
}

function closeRedisManager() {
  redisManagerConnection.value = null;
  redisManagerDatabase.value = null;
}

function openContextMenu(event: MouseEvent, target: ContextTarget) {
  contextMenu.value = { x: event.clientX, y: event.clientY, target };
}

function closeContextMenu() {
  contextMenu.value = null;
}

async function copyText(value: string) {
  try {
    await navigator.clipboard.writeText(value);
  } catch {
    error.value = "复制失败，请检查剪贴板权限";
  }
}

async function runConnectionContextAction(action: "toggle" | "edit" | "disconnect" | "remove" | "redis") {
  const target = contextMenu.value?.target;
  if (!target || target.kind !== "connection") return;
  const connection = target.connection;
  closeContextMenu();
  if (action === "toggle") await toggleConnection(connection);
  else if (action === "redis") openRedisManager(connection);
  else if (action === "edit") editConnection(connection);
  else if (action === "disconnect") await disconnectConnection(connection.id);
  else await removeConnection(connection);
}

async function runDatabaseContextAction(action: "open" | "refresh" | "backup" | "restore" | "compare" | "copy" | "drop") {
  const target = contextMenu.value?.target;
  if (!target || target.kind !== "database") return;
  const database = target.database;
  closeContextMenu();
  if (action === "copy") await copyText(database);
  else if (action === "drop") await dropDatabase(database);
  else if (action === "backup") await backupDatabase(database);
  else if (action === "restore") await importSqlFile(database);
  else if (action === "compare") compareDatabase.value = database;
  else if (action === "open") await openDatabase(database);
  else if (selectedDatabase.value !== database) await openDatabase(database);
  else await Promise.all([
    store.loadTables("", false),
    store.loadDatabaseObjects(database),
  ]);
}

async function backupDatabase(database: string) {
  if (!activeConnectionId.value) return;
  const compression = settings.value.backupCompression ?? "none";
  let encryptionPassword: string | null = null;
  if (settings.value.backupEncryption) {
    encryptionPassword = await promptAction({
      title: "加密数据库备份",
      message: `为数据库“${database}”的备份设置密码。`,
      detail: "该密码不会被保存。恢复备份时必须输入相同密码。",
      inputLabel: "备份密码",
      inputType: "password",
      inputPlaceholder: "至少 8 个字符",
      inputMinLength: 8,
      inputRequired: true,
      inputValidationMessage: "密码至少需要 8 个字符",
      trimInput: false,
      confirmLabel: "继续备份",
    });
    if (encryptionPassword === null) return;
  }
  const encryptedSuffix = encryptionPassword ? ".enc" : "";
  const outputPath = await save({
    title: `备份数据库 ${database}`,
    defaultPath: `${database}-${new Date().toISOString().slice(0, 10)}.sql${compression === "gzip" ? ".gz" : ""}${encryptedSuffix}`,
    filters: [{ name: encryptionPassword ? "加密备份" : compression === "gzip" ? "Gzip SQL" : "SQL", extensions: [encryptionPassword ? "enc" : compression === "gzip" ? "gz" : "sql"] }],
  });
  if (!outputPath) return;
  const normalizedPath = normalizeBackupPath(outputPath, compression, Boolean(encryptionPassword));
  await runBackup(activeConnectionId.value, database, normalizedPath, settings.value.backupIncludeData, compression, true, encryptionPassword);
}

function normalizeBackupPath(path: string, compression: "none" | "gzip", encrypted = false) {
  const lower = path.toLocaleLowerCase();
  if (encrypted) {
    if (lower.endsWith(".enc")) return path;
    return `${path}.enc`;
  }
  if (compression === "gzip") {
    if (lower.endsWith(".sql.gz") || lower.endsWith(".gz")) return path;
    if (lower.endsWith(".sql")) return `${path}.gz`;
    return `${path}.sql.gz`;
  }
  return lower.endsWith(".sql") ? path : `${path}.sql`;
}

async function runBackup(
  connectionId: UUID,
  database: string,
  outputPath: string,
  includeData: boolean,
  compression: "none" | "gzip",
  notify = false,
  encryptionPassword: string | null = null,
) {
  const taskId = crypto.randomUUID();
  const connection = connections.value.find((item) => item.id === connectionId);
  createTransferTask({
    taskId,
    kind: "backup",
    title: `备份 ${connection?.name ?? "连接"}.${database}`,
    phase: "准备",
    completed: 0,
    status: "running",
    startedAt: new Date().toISOString(),
    outputPath,
  });
  busy.value = true;
  error.value = null;
  try {
    const summary = await api.backupDatabase(
      connectionId,
      database,
      outputPath,
      {
        includeData,
        compression,
        encryptionPassword,
        taskId,
      },
    );
    finishTransferTask(taskId, "completed", { phase: "完成", checksumSha256: summary.checksumSha256, message: `${summary.rowsWritten} 行 · SHA-256 ${summary.checksumSha256?.slice(0, 12) ?? "—"}…` });
    if (notify) await showNotice({
      title: "备份完成",
      message: `已备份 ${summary.tablesWritten} 个表/视图、${summary.objectsWritten} 个数据库对象和 ${summary.rowsWritten} 行数据。`,
      detail: `SHA-256：${summary.checksumSha256 ?? "—"}`,
      tone: "success",
      confirmLabel: "完成",
    });
  } catch (cause) {
    const message = typeof cause === "object" && cause && "message" in cause ? String(cause.message) : String(cause);
    error.value = message;
    finishTransferTask(taskId, message.includes("取消") ? "cancelled" : "failed", { phase: message.includes("取消") ? "已取消" : "失败", error: message });
  } finally {
    busy.value = false;
  }
}

async function runBackupScheduleNow(schedule: BackupSchedule) {
  const fileName = `${schedule.database}-${new Date().toISOString().replace(/[:.]/g, "-")}.sql${schedule.compression === "gzip" ? ".gz" : ""}`;
  const outputPath = await join(schedule.directory, fileName);
  await runBackup(schedule.connectionId, schedule.database, outputPath, schedule.includeData, schedule.compression);
}

async function checkBackupSchedule() {
  const schedule = backupSchedule.value;
  if (!schedule?.enabled || new Date(schedule.nextRunAt).getTime() > Date.now()) return;
  schedule.nextRunAt = new Date(Date.now() + Math.max(1, schedule.intervalHours) * 3_600_000).toISOString();
  saveBackupSchedule({ ...schedule });
  if (!connectionInfo.value[schedule.connectionId]) {
    const taskId = crypto.randomUUID();
    createTransferTask({
      taskId, kind: "backup", title: `定时备份 ${schedule.database}`, phase: "等待连接", completed: 0,
      status: "failed", startedAt: new Date().toISOString(), finishedAt: new Date().toISOString(),
      error: "计划连接当前未打开；已保留计划并顺延到下次运行。",
    });
    return;
  }
  await runBackupScheduleNow(schedule);
}

async function runTableGroupContextAction(action: "create" | "view" | "query" | "toggle" | "refresh") {
  const target = contextMenu.value?.target;
  if (!target || target.kind !== "table-group") return;
  const database = target.database;
  closeContextMenu();
  if (action === "toggle") {
    expandedTableGroup.value = !expandedTableGroup.value;
  } else if (action === "refresh") {
    expandedTableGroup.value = true;
    if (selectedDatabase.value !== database) await openDatabase(database);
    else await store.loadTables("", false);
  } else if (action === "create") {
    error.value = null;
    if (["mysql", "mariadb", "sqlite"].includes(activeConnectionKind.value)) createTableTab(database);
    else createQueryTab(`CREATE TABLE ${quoteIdentifier(database, "postgresql")}.${quoteIdentifier("new_table", "postgresql")} (\n  id BIGSERIAL PRIMARY KEY\n);`, database);
  } else if (action === "view") {
    createDatabaseObjectTab(database, "view");
  } else {
    createQueryTab("", database);
  }
}

function objectGroupLabel(group: ObjectGroupKind) {
  return ({
    view: "视图",
    routine: "函数",
    trigger: "触发器",
    event: "事件",
  } as const)[group];
}

function objectGroupExpanded(group: ObjectGroupKind) {
  if (group === "view") return expandedViewGroup.value;
  if (group === "routine") return expandedFunctionGroup.value;
  if (group === "trigger") return expandedTriggerGroup.value;
  return expandedEventGroup.value;
}

function setObjectGroupExpanded(group: ObjectGroupKind, expanded: boolean) {
  if (group === "view") expandedViewGroup.value = expanded;
  else if (group === "routine") expandedFunctionGroup.value = expanded;
  else if (group === "trigger") expandedTriggerGroup.value = expanded;
  else expandedEventGroup.value = expanded;
}

function createObjectFromGroup(database: string, kind: DatabaseObjectKind) {
  if (activeConnectionKind.value === "sqlite" && (kind === "procedure" || kind === "function" || kind === "event")) {
    error.value = "SQLite 仅支持创建视图和触发器。";
    return;
  }
  if (activeConnectionKind.value === "postgresql" && kind === "event") {
    error.value = "PostgreSQL 不支持 MySQL EVENT，请使用外部调度器或 pg_cron。";
    return;
  }
  createDatabaseObjectTab(database, kind);
}

async function runObjectGroupContextAction(action: "create-view" | "create-function" | "create-procedure" | "create-trigger" | "create-event" | "new-query" | "toggle" | "refresh") {
  const target = contextMenu.value?.target;
  if (!target || target.kind !== "object-group") return;
  const { database, group } = target;
  closeContextMenu();
  if (action === "toggle") {
    setObjectGroupExpanded(group, !objectGroupExpanded(group));
    return;
  }
  if (action === "refresh") {
    const changedDatabase = selectedDatabase.value !== database;
    if (changedDatabase) await openDatabase(database);
    if (!changedDatabase && group === "view") await store.loadTables("", false);
    else if (!changedDatabase) await store.loadDatabaseObjects(database);
    setObjectGroupExpanded(group, true);
    return;
  }
  if (action === "new-query") {
    createQueryTab("", database);
    return;
  }
  const kind = ({
    "create-view": "view",
    "create-function": "function",
    "create-procedure": "procedure",
    "create-trigger": "trigger",
    "create-event": "event",
  } as const)[action];
  createObjectFromGroup(database, kind);
}

async function createDatabase() {
  if (!activeConnectionId.value) {
    error.value = "请先连接数据库";
    return;
  }
  if (activeConnectionKind.value === "sqlite") {
    error.value = "SQLite 数据库对应连接文件；请通过“新建连接”创建或打开另一个文件。";
    return;
  }
  const postgresql = activeConnectionKind.value === "postgresql";
  const name = await promptAction({
    title: postgresql ? "新建 Schema" : "新建数据库",
    message: postgresql ? "输入要创建的 PostgreSQL Schema 名称。" : "输入要创建的 MySQL 数据库名称。",
    inputLabel: postgresql ? "Schema 名称" : "数据库名称",
    inputPlaceholder: postgresql ? "例如 app_public" : "例如 application_db",
    inputRequired: true,
    inputValidationMessage: "名称不能为空",
    confirmLabel: "创建",
  });
  if (!name) return;
  const statement = activeConnectionKind.value === "postgresql"
    ? `CREATE SCHEMA ${quoteIdentifier(name, "postgresql")};`
    : `CREATE DATABASE ${quoteMysqlIdentifier(name)} DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci;`;
  const page = await store.execute(null, statement, true);
  if (page) await store.loadDatabases();
}

async function dropDatabase(database: string) {
  if (!await confirmDestructiveAction(`永久删除数据库“${database}”及其中全部对象？`)) return;
  if (activeConnectionKind.value === "sqlite") { error.value = "SQLite 主数据库不能在当前连接中删除。"; return; }
  const page = await store.execute(null, activeConnectionKind.value === "postgresql"
    ? `DROP SCHEMA ${quoteIdentifier(database, "postgresql")} CASCADE;`
    : `DROP DATABASE ${quoteMysqlIdentifier(database)};`, true);
  if (page) {
    expandedDatabase.value = null;
    await store.loadDatabases();
  }
}

async function runTableContextAction(action: "preview" | "generate" | "design" | "truncate" | "drop" | "copy" | "copy-qualified") {
  const target = contextMenu.value?.target;
  if (!target || target.kind !== "table") return;
  const table = target.table;
  closeContextMenu();
  if (action === "preview") await previewTable(table);
  else if (action === "generate") generateSelect(table);
  else if (action === "design") {
    if (table.tableType.includes("VIEW")) await openDatabaseObject(table.database, "view", table.name);
    else await designTable(table);
  } else if (action === "truncate") {
    if (!table.tableType.includes("VIEW")) {
      const target = `${quoteIdentifier(table.database, activeConnectionKind.value)}.${quoteIdentifier(table.name, activeConnectionKind.value)}`;
      await runTableDdl(table, activeConnectionKind.value === "sqlite" ? `DELETE FROM ${target};` : `TRUNCATE TABLE ${target};`, "清空表中全部数据");
    }
  }
  else if (action === "drop") {
    const objectType = table.tableType.includes("VIEW") ? "VIEW" : "TABLE";
    await runTableDdl(table, `DROP ${objectType} ${quoteIdentifier(table.database, activeConnectionKind.value)}.${quoteIdentifier(table.name, activeConnectionKind.value)};`, `永久删除该${objectType === "VIEW" ? "视图" : "表"}`);
  } else if (action === "copy") await copyText(table.name);
  else if (action === "copy-qualified") await copyText(`${table.database}.${table.name}`);
}

async function runTableDdl(table: TableInfo, statement: string, description: string) {
  if (!await confirmDestructiveAction(`${description}“${table.name}”？`)) return;
  const page = await store.execute(null, statement, true);
  if (!page) return;
  await store.loadTables("", false);
  const tabId = `table:${activeConnectionId.value ?? ""}:${table.database}:${table.name}`;
  const tab = workspaceTabs.value.find((item) => item.id === tabId);
  if (tab) await closeWorkspaceTab(tab.id);
}

function isDatabaseViewTab(tab: { kind: string }) {
  // `table-detail` is retained only to discard tabs saved by older versions.
  return tab.kind === "table" || tab.kind === "table-detail";
}

function databaseTabMatches(tab: WorkspaceTab, connectionId: UUID, database?: string) {
  return isDatabaseViewTab(tab)
    && tab.connectionId === connectionId
    && (!database || tab.database === database);
}

async function closeDatabaseWorkspaceTabs(connectionId: UUID, database?: string, confirmTransactions = true) {
  const targetTabs = workspaceTabs.value.filter((tab) => databaseTabMatches(tab, connectionId, database));
  if (!targetTabs.length) return true;
  const transactionTabs = targetTabs.filter((tab) => transactionSessions.value[tab.sessionId]);
  if (confirmTransactions && transactionTabs.length && !await confirmAction({
    title: "关闭事务中的标签页？",
    message: `${transactionTabs.length} 个标签页中还有尚未提交的事务。`,
    detail: "继续后将分别回滚这些标签页的事务，其他标签页不受影响。",
    tone: "warning",
    confirmLabel: "回滚并关闭",
  })) return false;
  const targetIds = new Set(targetTabs.map((tab) => tab.id));
  const editor = inlineRowEditor.value;
  if (editor && targetIds.has(editor.tabId) && !await finishInlineRowEdit(editor)) return false;
  for (const tab of targetTabs) {
    if (!await store.closeTabSession(tab.sessionId)) return false;
  }
  const activeIndex = workspaceTabs.value.findIndex((tab) => tab.id === activeWorkspaceTabId.value);
  const activeWasClosed = activeWorkspaceTabId.value != null && targetIds.has(activeWorkspaceTabId.value);
  workspaceTabs.value = workspaceTabs.value.filter((tab) => !targetIds.has(tab.id));
  if (activeWasClosed) {
    activeWorkspaceTabId.value = workspaceTabs.value[Math.min(activeIndex, workspaceTabs.value.length - 1)]?.id ?? null;
  }
  if (inlineRowEditor.value && targetIds.has(inlineRowEditor.value.tabId)) inlineRowEditor.value = null;
  cellViewer.value = null;
  showImportDialog.value = false;
  closeBatchEditDialog();
  cancelColumnBatchEdit();
  showColumnManager.value = false;
  showResultInsights.value = false;
  showQueryExportDialog.value = false;
  tableActionsOpen.value = false;
  return true;
}

async function loadRedisDatabases(connection: ConnectionProfile) {
  redisLoading.value = { ...redisLoading.value, [connection.id]: true };
  try {
    await api.connectRedis(connection.id);
    const databases = await api.listRedisDatabases(connection.id);
    redisDatabases.value = { ...redisDatabases.value, [connection.id]: databases };
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    redisLoading.value = { ...redisLoading.value, [connection.id]: false };
  }
}

async function toggleConnection(connection: ConnectionProfile) {
  if (connection.driverKind === "redis") {
    if (expandedConnectionId.value === connection.id) {
      expandedConnectionId.value = null;
      expandedDatabase.value = null;
      collapseObjectGroups();
      void api.disconnectRedis(connection.id).catch(() => {});
      return;
    }
    if (expandedConnectionId.value && !await closeDatabaseWorkspaceTabs(expandedConnectionId.value)) return;
    expandedConnectionId.value = connection.id;
    expandedDatabase.value = null;
    collapseObjectGroups();
    await loadRedisDatabases(connection);
    return;
  }
  if (expandedConnectionId.value === connection.id) {
    if (!await closeDatabaseWorkspaceTabs(connection.id)) return;
    if (activeConnectionId.value === connection.id) store.closeDatabase();
    expandedConnectionId.value = null;
    expandedDatabase.value = null;
    collapseObjectGroups();
    return;
  }
  if (expandedConnectionId.value && !await closeDatabaseWorkspaceTabs(expandedConnectionId.value)) return;
  expandedConnectionId.value = connection.id;
  expandedDatabase.value = null;
  collapseObjectGroups();
  await store.connect(connection.id);
}

async function toggleDatabase(database: string) {
  if (expandedDatabase.value === database) {
    if (activeConnectionId.value && !await closeDatabaseWorkspaceTabs(activeConnectionId.value, database)) return;
    store.closeDatabase();
    expandedDatabase.value = null;
    collapseObjectGroups();
    return;
  }
  if (activeConnectionId.value && expandedDatabase.value
    && !await closeDatabaseWorkspaceTabs(activeConnectionId.value, expandedDatabase.value)) return;
  expandedDatabase.value = database;
  collapseObjectGroups();
  await store.selectDatabase(database);
}

async function disconnectConnection(connectionId: UUID) {
  const transactionCount = workspaceTabs.value.filter((tab) => tab.connectionId === connectionId && transactionSessions.value[tab.sessionId]).length;
  if (transactionCount && !await confirmAction({
    title: "断开包含活动事务的连接？",
    message: `${transactionCount} 个标签页中还有尚未提交的事务。`,
    detail: "断开连接会分别回滚这些事务，其他连接不受影响。",
    tone: "warning",
    confirmLabel: "回滚并断开",
  })) return;
  if (!await closeDatabaseWorkspaceTabs(connectionId, undefined, false)) return;
  await store.disconnect(connectionId);
  if (expandedConnectionId.value === connectionId) {
    expandedConnectionId.value = null;
    expandedDatabase.value = null;
    collapseObjectGroups();
  }
}

async function removeConnection(connection: ConnectionProfile) {
  const transactionCount = workspaceTabs.value.filter((tab) => tab.connectionId === connection.id && transactionSessions.value[tab.sessionId]).length;
  if (!await confirmDestructiveAction(
    `删除连接“${connection.name}”？`,
    `只会删除本地连接配置，不会删除数据库中的数据。${transactionCount ? ` ${transactionCount} 个标签页中的事务将被回滚。` : ""}`,
  )) return;
  if (!await closeDatabaseWorkspaceTabs(connection.id, undefined, false)) return;
  await store.removeConnection(connection.id);
  if (expandedConnectionId.value === connection.id) {
    expandedConnectionId.value = null;
    expandedDatabase.value = null;
    collapseObjectGroups();
  }
}

async function openDatabase(database: string) {
  if (activeConnectionId.value && expandedDatabase.value && expandedDatabase.value !== database
    && !await closeDatabaseWorkspaceTabs(activeConnectionId.value, expandedDatabase.value)) return;
  if (expandedDatabase.value !== database) collapseObjectGroups();
  expandedDatabase.value = database;
  await store.selectDatabase(database);
}


function routineObjectKind(routine: RoutineInfo): DatabaseObjectKind {
  return routine.routineType.toUpperCase() === "FUNCTION" ? "function" : "procedure";
}

function objectKindLabel(kind: DatabaseObjectKind) {
  return ({ view: "视图", procedure: "存储过程", function: "函数", trigger: "触发器", event: "事件" } as const)[kind];
}

function databaseObjectDraft(kind: DatabaseObjectKind, name = "", ddl = ""): DatabaseObjectDraft {
  return {
    mode: ddl ? "ddl" : "visual",
    kind,
    name: name || `new_${kind}`,
    table: "",
    parameters: "",
    returnType: "VARCHAR(255)",
    timing: "BEFORE",
    event: "INSERT",
    schedule: "EVERY 1 DAY",
    body: kind === "view" ? "SELECT 1 AS value" : "BEGIN\n  -- SQL body\nEND",
    ddl,
  };
}

function createDatabaseObjectTab(database: string, kind: DatabaseObjectKind) {
  const connectionId = activeConnectionId.value;
  if (!connectionId) return null;
  const existingTab = workspaceTabs.value.find((tab) => tab.kind === "database-object"
    && !tab.databaseObjectOriginalName
    && tab.connectionId === connectionId
    && tab.database === database
    && tab.databaseObjectDraft?.kind === kind);
  if (existingTab) {
    void activateWorkspaceTab(existingTab);
    return existingTab;
  }
  const draft = databaseObjectDraft(kind);
  const tab: WorkspaceTab = {
    id: `database-object:new:${crypto.randomUUID()}`,
    sessionId: crypto.randomUUID(),
    kind: "database-object",
    title: `新建${objectKindLabel(kind)}`,
    connectionId,
    database,
    databaseObjectDraft: draft,
    databaseObjectPersistedDraft: JSON.stringify(draft),
    sql: "",
    result: null,
    columnWidths: {},
    closable: true,
  };
  workspaceTabs.value.push(tab);
  activeWorkspaceTabId.value = tab.id;
  return tab;
}

async function openDatabaseObject(database: string, kind: DatabaseObjectKind, name: string) {
  if (!activeConnectionId.value) return;
  const connectionId = activeConnectionId.value;
  const tabId = `database-object:${connectionId}:${database}:${kind}:${name}`;
  const existingTab = workspaceTabs.value.find((tab) => tab.id === tabId);
  if (existingTab) {
    await activateWorkspaceTab(existingTab);
    return;
  }
  error.value = null;
  try {
    if (selectedDatabase.value !== database) await openDatabase(database);
    const definition = await api.objectDefinition(connectionId, database, kind, name);
    const ddl = `${definition.ddl.trimEnd().replace(/;+$/, "")};`;
    const draft = databaseObjectDraft(kind, name, ddl);
    const tab: WorkspaceTab = {
      id: tabId,
      sessionId: crypto.randomUUID(),
      kind: "database-object",
      title: `${objectKindLabel(kind)} · ${name}`,
      connectionId,
      database,
      databaseObjectDraft: draft,
      databaseObjectPersistedDraft: JSON.stringify(draft),
      databaseObjectOriginalName: name,
      sql: "",
      result: null,
      columnWidths: {},
      closable: true,
    };
    workspaceTabs.value.push(tab);
    activeWorkspaceTabId.value = tab.id;
  } catch (cause) {
    error.value = typeof cause === "object" && cause && "message" in cause ? String(cause.message) : String(cause);
  }
}

function createDatabaseObject() {
  if (!selectedDatabase.value) { error.value = "请先选择数据库"; return; }
  createDatabaseObjectTab(selectedDatabase.value, "view");
}

function updateDatabaseObjectDraft(tab: WorkspaceTab, draft: DatabaseObjectDraft) {
  if (tab.kind !== "database-object") return;
  error.value = null;
  tab.databaseObjectDraft = draft;
  const label = objectKindLabel(draft.kind);
  tab.title = tab.databaseObjectOriginalName
    ? `${label} · ${draft.name || tab.databaseObjectOriginalName}`
    : draft.name.trim() && draft.name !== `new_${draft.kind}` ? `新建${label} · ${draft.name.trim()}` : `新建${label}`;
}

function openObjectSql(tab: WorkspaceTab, statement: string) {
  if (tab.kind !== "database-object" || !tab.databaseObjectDraft) return;
  const queryTab = createQueryTab(statement, tab.database, tab.connectionId);
  queryTab.title = `${objectKindLabel(tab.databaseObjectDraft.kind)} SQL · ${tab.databaseObjectDraft.name}`;
  queryTab.generatedTitle = false;
}

async function saveDatabaseObject(tab: WorkspaceTab, statement: string) {
  if (tab.kind !== "database-object" || !tab.databaseObjectDraft || !tab.connectionId || !tab.database
    || savingDatabaseObjectTabId.value || busy.value) return;
  const connection = connections.value.find((item) => item.id === tab.connectionId);
  if (connection?.readOnly) {
    error.value = "当前为只读连接，不能保存数据库对象";
    return;
  }
  const name = tab.databaseObjectDraft.name.trim();
  if (!name || !statement.trim()) {
    error.value = "请填写对象名称和定义";
    return;
  }
  const targetTabId = `database-object:${tab.connectionId}:${tab.database}:${tab.databaseObjectDraft.kind}:${name}`;
  const conflictingTab = workspaceTabs.value.find((item) => item !== tab && item.id === targetTabId);
  if (conflictingTab && transactionSessions.value[conflictingTab.sessionId]) {
    error.value = "同一数据库对象的另一个标签页正在事务中，请先提交或回滚后再保存";
    return;
  }
  const originalTabId = tab.id;
  const wasActive = activeWorkspaceTabId.value === originalTabId;
  savingDatabaseObjectTabId.value = originalTabId;
  error.value = null;
  try {
    if (!await applyQueryContext(tab)) return;
    const page = await store.execute(tab.sessionId, statement, true);
    if (!page) return;

    await Promise.all([
      store.loadTables("", false),
      store.loadDatabaseObjects(tab.database),
    ]);
    expandedConnectionId.value = tab.connectionId;
    expandedDatabase.value = tab.database;
    setObjectGroupExpanded(tab.databaseObjectDraft.kind === "view" ? "view"
      : tab.databaseObjectDraft.kind === "trigger" ? "trigger"
        : tab.databaseObjectDraft.kind === "event" ? "event" : "routine", true);

    const savedDraft: DatabaseObjectDraft = { ...tab.databaseObjectDraft, ddl: statement };
    const savedId = targetTabId;
    const savedTitle = `${objectKindLabel(savedDraft.kind)} · ${name}`;
    const duplicate = workspaceTabs.value.find((item) => item !== tab && item.id === savedId);
    if (duplicate) {
      if (!await store.closeTabSession(duplicate.sessionId)) return;
      workspaceTabs.value = workspaceTabs.value.filter((item) => item !== duplicate);
    }
    tab.id = savedId;
    tab.databaseObjectDraft = savedDraft;
    tab.databaseObjectPersistedDraft = JSON.stringify(savedDraft);
    tab.databaseObjectOriginalName = name;
    tab.title = savedTitle;
    if (wasActive) activeWorkspaceTabId.value = savedId;
    showQueryFileNotice(`${objectKindLabel(savedDraft.kind)}“${name}”已保存到数据库。`, "success");
  } finally {
    savingDatabaseObjectTabId.value = null;
  }
}

async function runObjectContextAction(action: "open" | "invoke" | "toggle" | "copy" | "copy-qualified" | "drop") {
  const target = contextMenu.value?.target;
  if (!target || target.kind !== "object") return;
  closeContextMenu();
  if (action === "open") {
    await openDatabaseObject(target.database, target.objectKind, target.name);
    return;
  }
  if (action === "copy" || action === "copy-qualified") {
    await copyText(action === "copy" ? target.name : `${target.database}.${target.name}`);
    return;
  }
  if (action === "invoke") {
    const databaseKind = activeConnectionKind.value;
    const qualified = `${quoteIdentifier(target.database, databaseKind)}.${quoteIdentifier(target.name, databaseKind)}`;
    let parameters = [] as Awaited<ReturnType<typeof api.routineParameters>>;
    try {
      parameters = await api.routineParameters(activeConnectionId.value!, target.database, target.name);
    } catch (cause) {
      error.value = typeof cause === "object" && cause && "message" in cause ? String(cause.message) : String(cause);
      return;
    }
    const argumentsSql = parameters.filter((parameter) => databaseKind !== "postgresql" || parameter.mode !== "OUT").map((parameter) => {
      const name = parameter.name || `arg${parameter.ordinal}`;
      return databaseKind !== "postgresql" && (parameter.mode === "OUT" || parameter.mode === "INOUT")
        ? `@${name}`
        : `/* ${name} ${parameter.dataType} */ NULL`;
    }).join(", ");
    const statement = target.objectKind === "procedure"
      ? `CALL ${qualified}(${argumentsSql});${parameters.some((item) => item.mode === "OUT" || item.mode === "INOUT") ? `\nSELECT ${parameters.filter((item) => item.mode === "OUT" || item.mode === "INOUT").map((item) => `@${item.name || `arg${item.ordinal}`}`).join(", ")};` : ""}`
      : `SELECT ${qualified}(${argumentsSql});`;
    createQueryTab(statement, target.database);
    return;
  }
  if (action === "toggle" && target.objectKind === "event") {
    const nextState = target.status === "ENABLED" ? "DISABLE" : "ENABLE";
    const page = await store.execute(null, `ALTER EVENT ${quoteMysqlIdentifier(target.database)}.${quoteMysqlIdentifier(target.name)} ${nextState};`, true);
    if (page) await store.loadDatabaseObjects(target.database);
    return;
  }
  if (!await confirmDestructiveAction(`永久删除${target.label}“${target.name}”？`)) return;
  const qualified = `${quoteIdentifier(target.database, activeConnectionKind.value)}.${quoteIdentifier(target.name, activeConnectionKind.value)}`;
  const triggerTable = target.objectKind === "trigger" ? triggers.value.find((item) => item.name === target.name)?.tableName : null;
  const dropStatement = activeConnectionKind.value === "postgresql" && target.objectKind === "trigger" && triggerTable
    ? `DROP TRIGGER ${quoteIdentifier(target.name, "postgresql")} ON ${quoteIdentifier(target.database, "postgresql")}.${quoteIdentifier(triggerTable, "postgresql")};`
    : `DROP ${target.objectKind.toUpperCase()} ${qualified};`;
  const page = await store.execute(
    null,
    dropStatement,
    true,
  );
  if (page) await store.loadDatabaseObjects(target.database);
}

function generateSelect(table: TableInfo) {
  const kind = connections.value.find((item) => item.id === activeConnectionId.value)?.driverKind ?? "mysql";
  createQueryTab(selectPreviewSql(table.database, table.name, kind), table.database);
}

function createTableTab(database: string) {
  const tab: WorkspaceTab = {
    id: `create-table:${crypto.randomUUID()}`,
    sessionId: crypto.randomUUID(),
    kind: "create-table",
    title: "新建表",
    connectionId: activeConnectionId.value,
    database,
    createTableDefinition: createDefaultTableDefinition(activeConnectionKind.value),
    sql: "",
    result: null,
    columnWidths: {},
    closable: true,
  };
  workspaceTabs.value.push(tab);
  activeWorkspaceTabId.value = tab.id;
  return tab;
}

async function designTable(table: TableInfo) {
  if (!activeConnectionId.value) return;
  const detail = await api.tableDetail(activeConnectionId.value, table.database, table.name);
  if (activeConnectionKind.value !== "mysql" && activeConnectionKind.value !== "mariadb") {
    createQueryTab(`${detail.ddl.trimEnd().replace(/;+$/, "")};`, table.database);
    return;
  }
  const definition = tableDetailToDefinition(detail);
  const tabId = `alter-table:${activeConnectionId.value}:${table.database}:${table.name}`;
  let tab = workspaceTabs.value.find((item) => item.id === tabId);
  if (!tab) {
    tab = {
      id: tabId,
      sessionId: crypto.randomUUID(),
      kind: "alter-table",
      title: `设计 · ${table.name}`,
      connectionId: activeConnectionId.value,
      database: table.database,
      createTableDefinition: structuredClone(definition),
      originalTableDefinition: structuredClone(definition),
      tableDetail: detail,
      sql: "",
      result: null,
      columnWidths: {},
      closable: true,
    };
    workspaceTabs.value.push(tab);
  }
  activeWorkspaceTabId.value = tab.id;
}

function updateCreateTableDefinition(tab: WorkspaceTab, definition: CreateTableDefinition) {
  if (tab.kind !== "create-table" && tab.kind !== "alter-table") return;
  tab.createTableDefinition = definition;
  tab.title = tab.kind === "alter-table"
    ? `设计 · ${definition.name.trim() || tab.originalTableDefinition?.name || "表"}`
    : definition.name.trim() ? `新建表 · ${definition.name.trim()}` : "新建表";
}

async function createTableFromTab(tab: WorkspaceTab, definition: CreateTableDefinition) {
  if ((tab.kind !== "create-table" && tab.kind !== "alter-table") || !tab.connectionId || !tab.database || busy.value || creatingTableTabId.value) return;
  const connection = connections.value.find((item) => item.id === tab.connectionId);
  if (connection?.readOnly) {
    error.value = "当前为只读连接，不能创建表";
    return;
  }
  creatingTableTabId.value = tab.id;
  error.value = null;
  let atomicScope: BatchTransactionScope | null = null;
  try {
    const statement = tab.kind === "alter-table" && tab.originalTableDefinition
      ? alterTableSql(tab.database, tab.originalTableDefinition, definition)
      : createTableSql(tab.database, definition, connection?.driverKind ?? "mysql");
    if (!statement) {
      error.value = "当前没有结构变更";
      return;
    }
    const destructiveChanges = statement.match(/\bDROP\s+(?:COLUMN|PRIMARY\s+KEY|INDEX|FOREIGN\s+KEY|CHECK)\b/gi)?.length ?? 0;
    if (destructiveChanges && !await confirmDestructiveAction(
      `应用表“${definition.name.trim() || tab.originalTableDefinition?.name || tab.title}”的结构修改，并执行 ${destructiveChanges} 项删除？`,
      "被删除字段中的数据、索引或约束无法恢复。",
    )) return;
    const needsAtomicSqliteCreate = tab.kind === "create-table"
      && connection?.driverKind === "sqlite"
      && Boolean(definition.indexes?.length);
    if (needsAtomicSqliteCreate) {
      atomicScope = await beginBatchTransactionScope(tab);
      if (!atomicScope) return;
    } else if (!await applyQueryContext(tab)) return;
    const page = await store.execute(tab.sessionId, statement, true);
    if (!page) {
      if (atomicScope) {
        const executionError = error.value;
        await rollbackBatchTransactionScope(tab, atomicScope);
        atomicScope = null;
        error.value = executionError ?? error.value;
      }
      return;
    }
    if (atomicScope) {
      if (!await commitBatchTransactionScope(tab, atomicScope)) {
        const commitError = error.value;
        await rollbackBatchTransactionScope(tab, atomicScope);
        atomicScope = null;
        error.value = commitError ?? error.value;
        return;
      }
      atomicScope = null;
    }
    expandedConnectionId.value = tab.connectionId;
    expandedDatabase.value = tab.database;
    expandedTableGroup.value = true;
    await store.loadTables("", false);
    creatingTableTabId.value = null;
    await closeWorkspaceTab(tab.id);
  } catch (cause) {
    const failure = cause instanceof Error ? cause.message : String(cause);
    if (atomicScope) await rollbackBatchTransactionScope(tab, atomicScope);
    error.value = failure;
  } finally {
    if (creatingTableTabId.value === tab.id) creatingTableTabId.value = null;
  }
}

function untitledQueryTitle(connectionId: UUID | null | undefined) {
  const connectionName = connections.value.find((connection) => connection.id === connectionId)?.name ?? "未选择连接";
  return `无标题@${connectionName}`;
}

function createQueryTab(initialSql = "", database = selectedDatabase.value, connectionId = activeConnectionId.value) {
  const tab: WorkspaceTab = {
    id: `query:${crypto.randomUUID()}`,
    sessionId: crypto.randomUUID(),
    kind: "console",
    title: untitledQueryTitle(connectionId),
    connectionId,
    database,
    sql: initialSql,
    result: null,
    columnWidths: {},
    closable: true,
    resultSetIndex: 0,
    selectedRowIndex: null,
    pageSize: settings.value.queryPageSize,
    persistedSql: initialSql,
    generatedTitle: true,
  };
  workspaceTabs.value.push(tab);
  activeWorkspaceTabId.value = tab.id;
  return tab;
}

function createQuery() {
  const tab = createQueryTab();
  void applyQueryContext(tab);
}

function toggleActiveTabPin() {
  if (activeWorkspaceTab.value) activeWorkspaceTab.value.pinned = !activeWorkspaceTab.value.pinned;
}

function openAdminSql(statement: string) {
  createQueryTab(statement);
  showServerAdmin.value = false;
}

function pathFileName(path: string) {
  return path.split(/[\\/]/).pop() || "query.sql";
}

function queryFileError(cause: unknown) {
  return typeof cause === "object" && cause && "message" in cause ? String(cause.message) : String(cause);
}

function showQueryFileNotice(message: string, kind: "info" | "success" | "warning" = "info") {
  queryFileNotice.value = { kind, message };
  if (queryFileNoticeTimer) clearTimeout(queryFileNoticeTimer);
  queryFileNoticeTimer = setTimeout(() => { queryFileNotice.value = null; }, 5_000);
}

function queryTabIsDirty(tab: WorkspaceTab) {
  return tab.kind === "console" && tab.sql !== (tab.persistedSql ?? "");
}

function databaseObjectTabIsDirty(tab: WorkspaceTab) {
  return tab.kind === "database-object"
    && Boolean(tab.databaseObjectDraft)
    && JSON.stringify(tab.databaseObjectDraft) !== tab.databaseObjectPersistedDraft;
}

function workspaceTabIsDirty(tab: WorkspaceTab) {
  return queryTabIsDirty(tab) || databaseObjectTabIsDirty(tab);
}

async function openSqlFile() {
  const inputPath = await open({ multiple: false, directory: false, filters: [{ name: "SQL 文件", extensions: ["sql"] }] });
  if (!inputPath || Array.isArray(inputPath)) return;
  try {
    const file = await api.readTextFile(inputPath);
    const tab = createQueryTab(file.contents);
    tab.filePath = file.path;
    tab.persistedSql = file.contents;
    tab.title = pathFileName(file.path);
    tab.generatedTitle = false;
  } catch (cause) {
    showQueryFileNotice(`打开 SQL 文件失败：${queryFileError(cause)}`, "warning");
  }
}

async function saveSqlFile(forceSaveAs = false, linkToTab = true): Promise<string | null> {
  const tab = activeWorkspaceTab.value;
  if (!tab || tab.kind !== "console") return null;
  let outputPath = forceSaveAs ? null : tab.filePath ?? null;
  if (!outputPath) {
    outputPath = await save({
      title: "保存 SQL 文件",
      defaultPath: tab.filePath ?? `${tab.title.replace(/\.sql$/i, "")}.sql`,
      filters: [{ name: "SQL", extensions: ["sql"] }],
    });
  }
  if (!outputPath) return null;
  if (!/\.sql$/i.test(outputPath)) outputPath += ".sql";
  try {
    const savedPath = await api.writeTextFile(outputPath, tab.sql);
    if (linkToTab) {
      tab.filePath = savedPath;
      tab.persistedSql = tab.sql;
      tab.title = pathFileName(savedPath);
      tab.generatedTitle = false;
    } else {
      showQueryFileNotice(`已导出“${pathFileName(savedPath)}”。`, "success");
    }
    return savedPath;
  } catch (cause) {
    showQueryFileNotice(`保存 SQL 文件失败：${queryFileError(cause)}`, "warning");
    return null;
  }
}

async function applyQueryContext(tab: WorkspaceTab): Promise<boolean> {
  if ((tab.kind !== "console" && tab.kind !== "create-table" && tab.kind !== "alter-table" && tab.kind !== "database-object" && tab.kind !== "table") || !tab.closable || !tab.connectionId) return false;
  if (activeConnectionId.value !== tab.connectionId) await store.connect(tab.connectionId);
  if (activeConnectionId.value !== tab.connectionId) return false;
  if (!await store.openTabSession(tab.connectionId, tab.sessionId)) return false;
  if (activeConnectionId.value === tab.connectionId && tab.database && selectedDatabase.value !== tab.database) {
    await store.selectDatabase(tab.database);
  }
  return !tab.database || selectedDatabase.value === tab.database;
}

async function activateWorkspaceTab(tab: WorkspaceTab) {
  const editor = activeInlineRowEditor.value;
  if (editor && editor.tabId !== tab.id && !await finishInlineRowEdit(editor)) return;
  flushSqlEditor();
  activeWorkspaceTabId.value = tab.id;
  await applyQueryContext(tab);
}

async function selectQueryConnection(value: string | null) {
  const tab = activeWorkspaceTab.value;
  if (!tab || tab.kind !== "console" || !tab.closable) return;
  const connectionId = value || null;
  if (connectionId === tab.connectionId) return;
  if (transactionSessions.value[tab.sessionId] && !await confirmAction({
    title: "切换当前标签页的连接？",
    message: "切换连接会回滚这个标签页中尚未提交的事务。",
    detail: "其他标签页的事务不会受到影响。",
    tone: "warning",
    confirmLabel: "回滚并切换",
  })) return;
  if (!await store.closeTabSession(tab.sessionId)) return;
  tab.sessionId = crypto.randomUUID();
  tab.connectionId = connectionId;
  tab.database = null;
  tab.editableTable = null;
  tab.tableDetail = undefined;
  tab.editableResultSets = {};
  tab.resultSql = null;
  cancelColumnBatchEdit();
  if (tab.generatedTitle) tab.title = untitledQueryTitle(connectionId);
  if (connectionId) {
    await store.connect(connectionId);
    if (activeConnectionId.value === connectionId) await store.openTabSession(connectionId, tab.sessionId);
  }
}

async function selectQueryDatabase(value: string | null) {
  const tab = activeWorkspaceTab.value;
  if (!tab || tab.kind !== "console" || !tab.closable) return;
  const database = value || null;
  tab.database = database;
  tab.editableTable = null;
  tab.tableDetail = undefined;
  tab.editableResultSets = {};
  tab.resultSql = null;
  cancelColumnBatchEdit();
  if (database) await store.selectDatabase(database);
}

async function previewTable(table: TableInfo) {
  if (selectedDatabase.value !== table.database) await openDatabase(table.database);
  const tabId = `table:${activeConnectionId.value ?? ""}:${table.database}:${table.name}`;
  let tab = workspaceTabs.value.find((item) => item.id === tabId);
  if (!tab) {
    tab = {
      id: tabId,
      sessionId: crypto.randomUUID(),
      kind: "table",
      title: table.name,
      connectionId: activeConnectionId.value,
      database: table.database,
      tableDetail: tableDetail.value?.table.database === table.database && tableDetail.value.table.name === table.name
        ? tableDetail.value
        : undefined,
      sql: selectTablePageSql(table.database, table.name, settings.value.tablePageSize, 0, "", null, "asc", connections.value.find((item) => item.id === activeConnectionId.value)?.driverKind ?? "mysql"),
      result: null,
      columnWidths: {},
      closable: true,
      resultSetIndex: 0,
      selectedRowIndex: null,
      selectedRowIndexes: [],
      pageSize: settings.value.tablePageSize,
      pageable: true,
      filter: "",
      appliedFilter: "",
      sortColumn: null,
      sortDirection: "asc",
      selectedCell: null,
    };
    workspaceTabs.value.push(tab);
    tab = workspaceTabs.value[workspaceTabs.value.length - 1]!;
  }
  activeWorkspaceTabId.value = tab.id;
  const targetTab = tab;
  const connectionId = activeConnectionId.value;
  if (connectionId && !targetTab.tableDetail) {
    void api.tableDetail(connectionId, table.database, table.name)
      .then((detail) => { targetTab.tableDetail = detail; })
      .catch(() => { /* 结构信息失败不阻止数据浏览。 */ });
  }
  if (!await applyQueryContext(tab)) return;
  const page = await store.execute(tab.sessionId, tab.sql, false, 0, tab.pageSize ?? 100);
  if (page) {
    page.rowOffset = 0;
    tab.result = page;
  }
}

async function closeWorkspaceTab(tabId: string) {
  if (creatingTableTabId.value === tabId) return;
  if (activeWorkspaceTabId.value === tabId) syncActiveSqlEditor();
  const index = workspaceTabs.value.findIndex((tab) => tab.id === tabId);
  if (index < 0 || !workspaceTabs.value[index]!.closable || workspaceTabs.value[index]!.pinned) return;
  const tab = workspaceTabs.value[index]!;
  const hasPendingEditorText = activeWorkspaceTabId.value === tabId
    && Boolean(sqlEditor.value?.hasPendingTextInput?.());
  if (inlineRowEditor.value?.tabId === tabId) {
    if (inlineEditorHasChanges(inlineRowEditor.value) && !await confirmAction({
      title: "放弃行修改？",
      message: `“${tab.title}”中有未保存的行修改。`,
      detail: "关闭标签页后，这些修改将无法恢复。",
      tone: "warning",
      confirmLabel: "放弃并关闭",
    })) return;
    inlineRowEditor.value = null;
  }
  if ((workspaceTabIsDirty(tab) || hasPendingEditorText) && !await confirmAction({
    title: "关闭未保存的标签页？",
    message: `“${tab.title}”有未保存的修改。`,
    detail: "关闭后，未保存的内容将无法恢复。",
    tone: "warning",
    confirmLabel: "放弃并关闭",
  })) return;
  if (transactionSessions.value[tab.sessionId] && !await confirmAction({
    title: "关闭事务中的标签页？",
    message: `“${tab.title}”中还有尚未提交的事务。`,
    detail: "关闭后只会回滚这个标签页的事务，其他标签页不受影响。",
    tone: "warning",
    confirmLabel: "回滚并关闭",
  })) return;
  if (!await store.closeTabSession(tab.sessionId)) return;
  if (activeWorkspaceTabId.value === tabId) flushSqlEditor();
  workspaceTabs.value.splice(index, 1);
  if (activeWorkspaceTabId.value === tabId) {
    activeWorkspaceTabId.value = workspaceTabs.value[Math.max(0, index - 1)]?.id ?? null;
    if (activeWorkspaceTab.value) await applyQueryContext(activeWorkspaceTab.value);
  }
}

async function execute(overrideSql?: unknown) {
  syncActiveSqlEditor();
  const tab = activeWorkspaceTab.value;
  if (!tab) return;
  if (tab.kind === "console" && tab.closable) {
    if (!tab.connectionId) {
      error.value = "请先为当前查询选择连接";
      return;
    }
    if (!await applyQueryContext(tab)) return;
  }
  const sqlToRun = typeof overrideSql === "string" && overrideSql.trim() ? overrideSql : tab.sql;
  if (findSqlParameters(sqlToRun).length) {
    parameterSql.value = sqlToRun;
    return;
  }
  const assessment = await api.assess(sqlToRun);
  let allowWrite = false;
  if (assessment.requiresConfirmation) {
    const mustConfirm = assessment.risk === "destructive"
      || settings.value.confirmDestructiveQueries !== false
      || activeQueryConnection.value?.production;
    allowWrite = !mustConfirm || await confirmAction({
      title: assessment.risk === "destructive" ? "确认执行高风险 SQL" : "确认执行写入 SQL",
      message: assessment.reason ?? "该语句可能修改数据。",
      detail: activeQueryConnection.value?.production
        ? "当前连接标记为生产环境，请再次确认执行目标和影响范围。"
        : "请确认 SQL 内容和当前连接后再执行。",
      tone: assessment.risk === "destructive" ? "danger" : "warning",
      confirmLabel: "确认执行",
    });
    if (!allowWrite) return;
  }
  const safeSelect = assessment.statementKind === "SELECT" && !assessment.requiresConfirmation;
  const canPageQuery = tab.kind === "console" && safeSelect && canPageSelectQuery(sqlToRun);
  const appendPageLimit = canPageQuery && canAppendSelectQueryLimit(sqlToRun);
  const editableTargets = tab.kind === "console" && !assessment.requiresConfirmation
    ? singleTableSelectAllTargets(sqlToRun, tab.database)
    : [];
  if (tab.kind === "console") {
    tab.editableTable = null;
    tab.tableDetail = undefined;
    tab.editableResultSets = {};
    tab.resultSql = null;
    cancelColumnBatchEdit();
  }
  const requestSql = appendPageLimit ? selectQueryPageSql(sqlToRun, tab.pageSize ?? 500, 0, activeQueryConnection.value?.driverKind ?? "mysql") : sqlToRun;
  if (tab.kind === "console") {
    tab.hasExecuted = true;
    tab.resultPanelClosed = false;
    tab.resultView = "result";
  }
  const page = await store.execute(tab.sessionId, requestSql, allowWrite, 0, tab.pageSize ?? 500);
  if (page) {
    tab.result = page;
    tab.resultSql = tab.kind === "console" ? sqlToRun : null;
    tab.resultSetIndex = 0;
    tab.selectedRowIndex = null;
    tab.selectedRowIndexes = [];
    tab.selectedCell = null;
    tab.pageable = canPageQuery || tab.kind === "table";
    tab.pagingSql = canPageQuery ? sqlToRun : null;
    tab.pagingUsesDriverOffset = canPageQuery && !appendPageLimit;
    if (editableTargets.length && tab.kind === "console" && tab.connectionId) {
      const executedPage = page;
      const resultSetCount = 1 + (page.additionalResultSets?.length ?? 0);
      editableTargets.slice(0, resultSetCount).forEach((editableTarget, resultSetIndex) => {
        if (!editableTarget) return;
        void api.tableDetail(tab.connectionId!, editableTarget.database, editableTarget.table)
          .then((detail) => {
            if (tab.result?.executionId !== executedPage.executionId || tab.resultSql !== sqlToRun || detail.table.tableType.includes("VIEW")) return;
            tab.editableResultSets ??= {};
            tab.editableResultSets[resultSetIndex] = detail;
            if ((tab.resultSetIndex ?? 0) === resultSetIndex) selectConsoleEditableResult(tab, resultSetIndex);
          })
          .catch(() => { /* 无法确认表结构时保持对应结果集只读。 */ });
      });
    }
  }
}

async function executeParameterizedSql(rendered: string) {
  parameterSql.value = null;
  await execute(rendered);
}

function closeResultPanel() {
  const tab = activeWorkspaceTab.value;
  if (tab?.kind === "console") tab.resultPanelClosed = true;
}

function selectResultView(view: ResultView) {
  const tab = activeWorkspaceTab.value;
  if (tab?.kind === "console") tab.resultView = view;
}

async function changeResultPage(direction: -1 | 1) {
  const tab = activeWorkspaceTab.value;
  const page = displayedResult.value;
  if (!tab || !page || (tab.resultSetIndex ?? 0) !== 0 || !tab.pageable) return;
  const pageSize = page.pageSize ?? tab.pageSize ?? 500;
  const currentOffset = page.rowOffset ?? 0;
  const nextOffset = Math.max(0, currentOffset + direction * pageSize);
  if (nextOffset === currentOffset || (direction > 0 && !page.hasMore)) return;
  let requestSql = tab.sql;
  let requestOffset = nextOffset;
  if (tab.kind === "table" && tab.database) {
    requestSql = selectTablePageSql(
      tab.database,
      tab.title,
      pageSize,
      nextOffset,
      tab.appliedFilter,
      tab.sortColumn,
      tab.sortDirection,
      connections.value.find((item) => item.id === tab.connectionId)?.driverKind ?? "mysql",
    );
    requestOffset = 0;
  } else if (tab.kind === "console" && tab.pagingSql) {
    if (tab.pagingUsesDriverOffset) {
      requestSql = tab.pagingSql;
      requestOffset = nextOffset;
    } else {
      requestSql = selectQueryPageSql(tab.pagingSql, pageSize, nextOffset, connections.value.find((item) => item.id === tab.connectionId)?.driverKind ?? "mysql");
      requestOffset = 0;
    }
  }
  if (!await applyQueryContext(tab)) return;
  const next = await store.execute(tab.sessionId, requestSql, false, requestOffset, pageSize);
  if (next) {
    next.rowOffset = nextOffset;
    tab.result = next;
    tab.resultSetIndex = 0;
    tab.selectedRowIndex = null;
    tab.selectedRowIndexes = [];
    tab.selectedCell = null;
    if (tab.kind === "table") tab.sql = requestSql;
  }
}

function selectResultSet(index: number) {
  const tab = activeWorkspaceTab.value;
  if (!tab) return;
  tab.resultSetIndex = index;
  selectConsoleEditableResult(tab, index);
  tab.selectedRowIndex = null;
  tab.selectedRowIndexes = [];
  tab.selectedCell = null;
}

function formatSql() {
  syncActiveSqlEditor();
  const kind = activeQueryConnection.value?.driverKind ?? "mysql";
  const language = kind === "postgresql" ? "postgresql" : kind === "sqlite" ? "sqlite" : kind === "mariadb" ? "mariadb" : "mysql";
  try {
    const tab = activeWorkspaceTab.value;
    if (tab) tab.sql = format(sql.value, { language, keywordCase: "upper" });
  } catch { /* keep original */ }
}

async function explainQuery() {
  const tab = activeWorkspaceTab.value;
  if (!tab || tab.kind !== "console" || !tab.sql.trim()) return;
  const assessment = await api.assess(tab.sql);
  if (assessment.risk !== "safe") {
    error.value = "执行计划只支持只读查询";
    return;
  }
  await execute(`EXPLAIN ${tab.sql.trim().replace(/;+$/, "")}`);
}

function loadMoreTables() {
  if (tableHasMore.value && !busy.value) void store.loadTables("", true);
}

async function saveCurrentQuery(forceSaveAs = false) {
  syncActiveSqlEditor();
  const tab = activeWorkspaceTab.value;
  if (!tab || tab.kind !== "console" || !tab.sql.trim()) return;
  const savedPath = await saveSqlFile(forceSaveAs || !tab.filePath);
  if (!savedPath) return;
  showQueryFileNotice(`已保存“${pathFileName(savedPath)}”。`, "success");
}

async function beginTransaction() {
  const tab = activeWorkspaceTab.value;
  if (!tab?.connectionId) return;
  if (await applyQueryContext(tab)) await store.beginTransaction(tab.sessionId);
}
async function commitTransaction() {
  const tab = activeWorkspaceTab.value;
  if (!tab?.connectionId) return;
  if (await applyQueryContext(tab)) await store.commitTransaction(tab.sessionId);
}
async function rollbackTransaction() {
  const tab = activeWorkspaceTab.value;
  if (!tab?.connectionId) return;
  if (await confirmAction({
    title: "回滚当前事务？",
    message: "将撤销当前事务中的全部未提交修改。",
    detail: "回滚完成后无法恢复这些修改。",
    tone: "danger",
    confirmLabel: "确认回滚",
  })) {
    if (await applyQueryContext(tab)) await store.rollbackTransaction(tab.sessionId);
  }
}

function selectDataRow(index: number) {
  if (activeWorkspaceTab.value) activeWorkspaceTab.value.selectedRowIndex = index;
}

function toggleDataRow(index: number, checked: boolean) {
  const tab = activeWorkspaceTab.value;
  if (!tab) return;
  const selected = new Set(tab.selectedRowIndexes ?? []);
  if (checked) selected.add(index);
  else selected.delete(index);
  tab.selectedRowIndexes = [...selected].sort((left, right) => left - right);
  tab.selectedRowIndex = checked ? index : tab.selectedRowIndex === index ? tab.selectedRowIndexes[0] ?? null : tab.selectedRowIndex;
}

function toggleAllVisibleRows(checked: boolean) {
  const tab = activeWorkspaceTab.value;
  if (!tab) return;
  tab.selectedRowIndexes = checked ? visibleResultRows.value.map((entry) => entry.rowIndex) : [];
  tab.selectedRowIndex = tab.selectedRowIndexes[0] ?? null;
}

function closeBatchEditDialog() {
  showBatchEdit.value = false;
  batchEditRowIndexes.value = [];
}

function cancelColumnBatchEdit() {
  batchEditColumn.value = null;
  columnBatchEditValue.value = "";
  columnBatchEditNull.value = false;
  columnBatchEditError.value = "";
  columnDirectEditActive.value = false;
}

function openSelectedRowsBatchEdit() {
  if (!selectedRowIndexes.value.length) return;
  cancelColumnBatchEdit();
  batchEditRowIndexes.value = [...selectedRowIndexes.value];
  showBatchEdit.value = true;
}

function columnBatchEditDisabled(columnName: string) {
  const tab = activeWorkspaceTab.value;
  const detail = tab?.tableDetail?.columns.find((column) => column.name === columnName);
  return Boolean(
    busy.value
    || executingId.value
    || activeInlineRowEditor.value
    || activeQueryConnection.value?.readOnly
    || !activeTableHasUniqueKey.value
    || !visibleResultRows.value.length
    || isGeneratedColumn(detail),
  );
}

async function selectColumnBatchEdit(columnName: string) {
  if (columnBatchEditDisabled(columnName)) return;
  if (activeWorkspaceTab.value) activeWorkspaceTab.value.selectedCell = null;
  closeColumnMenu();
  if (batchEditColumn.value !== columnName) {
    batchEditColumn.value = columnName;
    columnBatchEditValue.value = "";
    columnBatchEditNull.value = false;
    columnBatchEditError.value = "";
    columnDirectEditActive.value = false;
  }
  await nextTick();
  const input = document.querySelector<HTMLInputElement>(
    activeEditableTable.value ? ".column-direct-edit-input" : ".column-batch-edit-input",
  );
  input?.focus();
  input?.select();
}

async function submitColumnBatchEdit() {
  const column = activeBatchEditColumn.value;
  if (!column || !batchEditColumn.value) return;
  if (activeEditableTable.value && !columnDirectEditActive.value) return;
  columnBatchEditError.value = "";
  let value: CellValue;
  try {
    value = parseRowCell(column, undefined, {
      text: rowDraftValue(columnBatchEditValue.value, columnBatchEditInputType.value),
      isNull: columnBatchEditNull.value,
    });
  } catch (cause) {
    columnBatchEditError.value = cause instanceof Error ? cause.message : String(cause);
    if (activeEditableTable.value) error.value = columnBatchEditError.value;
    return;
  }
  const indexes = visibleResultRows.value.map((entry) => entry.rowIndex);
  if (await applyBatchEditToRows(batchEditColumn.value, value, indexes)) cancelColumnBatchEdit();
}

function handleDirectColumnEditInput() {
  columnDirectEditActive.value = true;
  columnBatchEditError.value = "";
  error.value = null;
}

function cancelDirectColumnEdit() {
  if (!columnDirectEditActive.value) {
    cancelColumnBatchEdit();
    return;
  }
  columnBatchEditValue.value = "";
  columnBatchEditNull.value = false;
  columnBatchEditError.value = "";
  columnDirectEditActive.value = false;
  error.value = null;
}

async function startDirectColumnEditWithText(text: string) {
  columnBatchEditValue.value = columnDirectEditActive.value ? `${columnBatchEditValue.value}${text}` : text;
  handleDirectColumnEditInput();
  await nextTick();
  const input = document.querySelector<HTMLInputElement>(".column-direct-edit-input");
  input?.focus();
  input?.setSelectionRange(input.value.length, input.value.length);
}

function selectDataCell(row: number, column: number) {
  const tab = activeWorkspaceTab.value;
  if (!tab) return;
  tab.selectedRowIndex = row;
  tab.selectedCell = { row, column };
}

function handleResultDataCellClick(row: number, column: number) {
  if (batchEditColumn.value) cancelColumnBatchEdit();
  selectDataCell(row, column);
}

function gridCellTabIndex(row: number, column: number) {
  const selected = activeWorkspaceTab.value?.selectedCell;
  if (selected) return selected.row === row && selected.column === column ? 0 : -1;
  return visibleResultRows.value[0]?.rowIndex === row && visibleColumnEntries.value[0]?.sourceIndex === column ? 0 : -1;
}

async function focusGridCell(row: number, column: number) {
  if (!ensureGridRowVisible(row)) return;
  await nextTick();
  const target = document.querySelector<HTMLElement>(`[data-grid-row="${row}"][data-grid-column="${column}"]`);
  target?.focus();
  target?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
}

async function handleGridCellKeydown(event: KeyboardEvent, row: number, column: number, columnName: string) {
  const target = event.target;
  if (target instanceof Element && target.closest("input, button, select, textarea")) return;
  const tab = activeWorkspaceTab.value;
  if (!tab) return;
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "c") {
    event.preventDefault();
    selectDataCell(row, column);
    await copyResult("cell");
    return;
  }
  if (activeEditableTable.value && batchEditColumn.value === columnName) {
    if (event.key === "Enter" || event.key === "F2") {
      event.preventDefault();
      if (event.key === "Enter" && columnDirectEditActive.value) await submitColumnBatchEdit();
      else await selectColumnBatchEdit(columnName);
      return;
    }
    if (!event.metaKey && !event.ctrlKey && !event.altKey && event.key.length === 1) {
      event.preventDefault();
      await startDirectColumnEditWithText(event.key);
      return;
    }
  }
  if (event.key === "Enter" || event.key === "F2") {
    event.preventDefault();
    selectDataCell(row, column);
    if (
      activeEditableTable.value
      && !event.altKey
      && !activeQueryConnection.value?.readOnly
      && activeTableHasUniqueKey.value
      && inlineColumnEditable(columnName)
    ) startInlineRowEdit(row, columnName);
    else openCellViewer(row, column);
    return;
  }
  const rows = visibleResultRows.value;
  const columns = visibleColumnEntries.value;
  const rowPosition = rows.findIndex((entry) => entry.rowIndex === row);
  const columnPosition = columns.findIndex((entry) => entry.sourceIndex === column);
  if (rowPosition < 0 || columnPosition < 0) return;
  let nextRowPosition = rowPosition;
  let nextColumnPosition = columnPosition;
  if (event.key === "ArrowUp") nextRowPosition = Math.max(0, rowPosition - 1);
  else if (event.key === "ArrowDown") nextRowPosition = Math.min(rows.length - 1, rowPosition + 1);
  else if (event.key === "ArrowLeft") nextColumnPosition = Math.max(0, columnPosition - 1);
  else if (event.key === "ArrowRight") nextColumnPosition = Math.min(columns.length - 1, columnPosition + 1);
  else if (event.key === "Home") {
    nextColumnPosition = 0;
    if (event.metaKey || event.ctrlKey) nextRowPosition = 0;
  } else if (event.key === "End") {
    nextColumnPosition = columns.length - 1;
    if (event.metaKey || event.ctrlKey) nextRowPosition = rows.length - 1;
  } else return;
  event.preventDefault();
  const nextRow = rows[nextRowPosition]?.rowIndex;
  const nextColumn = columns[nextColumnPosition]?.sourceIndex;
  if (nextRow == null || nextColumn == null) return;
  if (activeEditableTable.value && !await selectTableDataRow(nextRow)) return;
  selectDataCell(nextRow, nextColumn);
  await focusGridCell(nextRow, nextColumn);
}

function openCellViewer(row: number, column: number) {
  const page = displayedResult.value;
  const value = page?.rows[row]?.[column];
  const meta = page?.columns[column];
  if (value && meta) cellViewer.value = { column: meta.name, value };
}

function tsvCell(value: string) {
  return /[\t\r\n"]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value;
}

async function copyResult(scope: "cell" | "row" | "page") {
  const tab = activeWorkspaceTab.value;
  const page = displayedResult.value;
  if (!tab || !page?.columns.length) return;
  let value = "";
  if (scope === "cell") {
    const selected = tab.selectedCell;
    if (!selected) return;
    value = cellText(page.rows[selected.row]?.[selected.column] ?? { kind: "null" });
  } else if (scope === "row") {
    const rowIndex = tab.selectedRowIndex ?? tab.selectedCell?.row;
    if (rowIndex == null || !page.rows[rowIndex]) return;
    value = page.rows[rowIndex].map((cell) => tsvCell(cellText(cell))).join("\t");
  } else {
    const header = page.columns.map((column) => tsvCell(column.name)).join("\t");
    const rows = page.rows.map((row) => row.map((cell) => tsvCell(cellText(cell))).join("\t"));
    value = [header, ...rows].join("\n");
  }
  await copyText(value);
}

async function applyTableFilter() {
  const tab = activeWorkspaceTab.value;
  if (!tab || tab.kind !== "table") return;
  tab.appliedFilter = tab.filter?.trim() ?? "";
  if (tab.result) tab.result.rowOffset = 0;
  await reloadTableTab(tab);
}

async function clearTableFilter() {
  const tab = activeWorkspaceTab.value;
  if (!tab || tab.kind !== "table") return;
  tab.filter = "";
  tab.appliedFilter = "";
  if (tab.result) tab.result.rowOffset = 0;
  await reloadTableTab(tab);
}

async function applyTableSort(column: string, direction: "asc" | "desc") {
  const tab = activeWorkspaceTab.value;
  if (!tab || tab.kind !== "table") return;
  closeColumnMenu();
  tab.sortColumn = column;
  tab.sortDirection = direction;
  if (tab.result) tab.result.rowOffset = 0;
  await reloadTableTab(tab);
}

async function clearTableSort() {
  const tab = activeWorkspaceTab.value;
  if (!tab || tab.kind !== "table" || !tab.sortColumn) return;
  closeColumnMenu();
  tab.sortColumn = null;
  tab.sortDirection = "asc";
  if (tab.result) tab.result.rowOffset = 0;
  await reloadTableTab(tab);
}

async function addColumnFilter(column: string) {
  const tab = activeWorkspaceTab.value;
  if (!tab || tab.kind !== "table") return;
  const expression = `${quoteIdentifier(column, activeQueryConnection.value?.driverKind ?? "mysql")} = `;
  const currentFilter = tab.filter?.trim();
  tab.filter = currentFilter ? `${currentFilter} AND ${expression}` : expression;
  closeColumnMenu();
  await nextTick();
  const input = document.querySelector<HTMLInputElement>(".table-filter-input");
  input?.focus();
  input?.setSelectionRange(input.value.length, input.value.length);
}

function inlineColumnDetail(tab: WorkspaceTab, columnName: string) {
  return tab.tableDetail?.columns.find((column) => column.name === columnName);
}

function inlineEditorHasChanges(editor: InlineRowEditorState) {
  const tab = workspaceTabs.value.find((item) => item.id === editor.tabId);
  if (!tab) return false;
  if (editor.mode === "insert") {
    return editor.columns.some((column) => {
      const detail = inlineColumnDetail(tab, column.name);
      const draft = editor.draft[column.name]!;
      if (isGeneratedColumn(detail) || draft.useDefault) return false;
      const initiallyNull = columnIsNullable(column, detail) && !hasDatabaseDefault(detail);
      return draft.text !== "" || draft.isNull !== initiallyNull;
    });
  }
  return editor.columns.some((column, index) => {
    const detail = inlineColumnDetail(tab, column.name);
    return !isGeneratedColumn(detail)
      && rowCellChanged(editor.draft[column.name]!, editor.row[index] ?? { kind: "null" });
  });
}

function isInlineEditingRow(rowIndex: number) {
  return activeInlineRowEditor.value?.rowIndex === rowIndex;
}

function isInlineInsertingRow(rowIndex: number) {
  return activeInlineRowEditor.value?.mode === "insert" && isInlineEditingRow(rowIndex);
}

function inlineColumnEditable(columnName: string) {
  const tab = activeWorkspaceTab.value;
  return Boolean(tab?.tableDetail && !isGeneratedColumn(inlineColumnDetail(tab, columnName)));
}

function inlineColumnNullable(columnName: string) {
  const tab = activeWorkspaceTab.value;
  const column = activeInlineRowEditor.value?.columns.find((item) => item.name === columnName);
  return Boolean(tab && column && columnIsNullable(column, inlineColumnDetail(tab, columnName)));
}

function inlineColumnInputType(columnName: string) {
  const tab = activeWorkspaceTab.value;
  const column = activeInlineRowEditor.value?.columns.find((item) => item.name === columnName);
  return tab && column ? rowInputType(column, inlineColumnDetail(tab, columnName)) : "text";
}

async function focusInlineCell(columnName: string) {
  const editor = activeInlineRowEditor.value;
  if (editor && !ensureGridRowVisible(editor.rowIndex)) return;
  await nextTick();
  const input = Array.from(document.querySelectorAll<HTMLInputElement>(".inline-cell-input"))
    .find((item) => item.dataset.column === columnName || item.closest<HTMLElement>(".inline-date-picker")?.dataset.column === columnName);
  input?.focus();
  if (input?.type === "text") input.select();
}

function preferredInlineDatePickerPlacement(rowIndex: number, sourceIndex: number): InlineRowEditorState["datePickerPlacement"] {
  const cell = document.querySelector<HTMLElement>(`.editable-query-result td[data-grid-row="${rowIndex}"][data-grid-column="${sourceIndex}"]`)
    ?? document.querySelector<HTMLElement>(`.table-data-view td[data-grid-row="${rowIndex}"][data-grid-column="${sourceIndex}"]`);
  if (!cell) return "bottom-start";
  const rect = cell.getBoundingClientRect();
  const spaceBelow = window.innerHeight - rect.bottom;
  const spaceRight = window.innerWidth - rect.left;
  const vertical = spaceBelow < 350 && rect.top > spaceBelow ? "top" : "bottom";
  const horizontal = spaceRight < 280 && rect.right > spaceRight ? "end" : "start";
  return `${vertical}-${horizontal}`;
}

function activateInlineCell(rowIndex: number, sourceIndex: number, columnName: string) {
  selectDataCell(rowIndex, sourceIndex);
  if (activeInlineRowEditor.value) {
    activeInlineRowEditor.value.datePickerPlacement = preferredInlineDatePickerPlacement(rowIndex, sourceIndex);
    activeInlineRowEditor.value.activeColumn = columnName;
  }
}

function inlineDraftText(columnName: string) {
  const cell = activeInlineRowEditor.value?.draft[columnName];
  if (cell?.useDefault) return "DEFAULT";
  return cell?.isNull ? "NULL" : cell?.text ?? "";
}

function inlineCellInputValue(columnName: string) {
  const cell = activeInlineRowEditor.value?.draft[columnName];
  return rowInputValue(cell?.text ?? "", inlineColumnInputType(columnName));
}

function inlineCellPlaceholder(columnName: string) {
  const cell = activeInlineRowEditor.value?.draft[columnName];
  if (cell?.useDefault) return "DEFAULT";
  return cell?.isNull ? "NULL" : "";
}

function updateInlineCellValue(columnName: string, value: string) {
  const cell = activeInlineRowEditor.value?.draft[columnName];
  if (cell) {
    cell.text = rowDraftValue(value, inlineColumnInputType(columnName));
    cell.isNull = false;
    cell.useDefault = false;
    error.value = null;
  }
}

function updateInlineCellText(columnName: string, event: Event) {
  updateInlineCellValue(columnName, (event.currentTarget as HTMLInputElement).value);
}

function toggleInlineCellNull(columnName: string) {
  const editor = activeInlineRowEditor.value;
  if (!editor || !inlineColumnNullable(columnName)) return;
  const cell = editor.draft[columnName]!;
  cell.isNull = !cell.isNull;
  cell.useDefault = false;
  error.value = null;
}

function visibleInlineColumns() {
  return visibleColumnEntries.value.filter((entry) => inlineColumnEditable(entry.column.name));
}

async function handleInlineCellTab(event: KeyboardEvent, rowIndex: number, columnName: string) {
  if (event.shiftKey) return;
  event.preventDefault();
  const columns = visibleInlineColumns();
  const columnPosition = columns.findIndex((entry) => entry.column.name === columnName);
  if (columnPosition < 0) return;
  const nextColumn = columns[columnPosition + 1];
  if (nextColumn) {
    activateInlineCell(rowIndex, nextColumn.sourceIndex, nextColumn.column.name);
    await focusInlineCell(nextColumn.column.name);
    return;
  }

  const firstColumn = columns[0];
  if (!firstColumn) return;
  const rows = visibleResultRows.value;
  const rowPosition = rows.findIndex((entry) => entry.rowIndex === rowIndex);
  const nextRowIndex = rows[rowPosition + 1]?.rowIndex;
  const editor = activeInlineRowEditor.value;
  if (editor && !await finishInlineRowEdit(editor)) return;
  if (nextRowIndex != null) {
    startInlineRowEdit(nextRowIndex, firstColumn.column.name);
    return;
  }

  if (displayedResult.value?.hasMore) {
    const currentOffset = displayedResult.value.rowOffset ?? 0;
    await changeResultPage(1);
    if ((displayedResult.value?.rowOffset ?? 0) === currentOffset) return;
    const firstNextPageRow = visibleResultRows.value[0];
    const firstNextPageColumn = visibleInlineColumns()[0];
    if (firstNextPageRow && firstNextPageColumn) {
      startInlineRowEdit(firstNextPageRow.rowIndex, firstNextPageColumn.column.name);
    }
    return;
  }
  startInlineRowInsert();
}

function startInlineRowEdit(rowIndex?: number | null, columnName?: string) {
  const tab = activeWorkspaceTab.value;
  const page = displayedResult.value;
  const target = editableTableForTab(tab);
  const targetRowIndex = rowIndex ?? tab?.selectedRowIndex ?? null;
  if (!tab || !target || !tab.tableDetail || !page || targetRowIndex === null) {
    error.value = "请先选择要编辑的行";
    return;
  }
  if (activeQueryConnection.value?.readOnly) {
    error.value = "该连接处于只读模式";
    return;
  }
  if (!uniqueKeyColumns(tab.tableDetail).length) {
    error.value = "该表没有可用的主键或唯一索引，不能安全编辑行";
    return;
  }
  const editableColumns = page.columns.filter((column) => !isGeneratedColumn(inlineColumnDetail(tab, column.name)));
  if (!editableColumns.length) {
    error.value = "该表没有可编辑字段";
    return;
  }
  if (columnName && !editableColumns.some((column) => column.name === columnName)) {
    error.value = `${columnName} 是数据库生成字段，不能直接编辑`;
    return;
  }
  const existing = activeInlineRowEditor.value;
  if (existing) {
    if (existing.rowIndex !== targetRowIndex && inlineEditorHasChanges(existing)) {
      error.value = "请先保存或取消当前行的修改";
      return;
    }
    if (existing.rowIndex === targetRowIndex) {
      const targetColumn = columnName ?? existing.activeColumn;
      const sourceIndex = page.columns.findIndex((column) => column.name === targetColumn);
      activateInlineCell(targetRowIndex, sourceIndex, targetColumn);
      void focusInlineCell(targetColumn);
      return;
    }
  }
  const row = page.rows[targetRowIndex];
  if (!row) {
    error.value = "选中的数据行已不存在，请刷新后重试";
    return;
  }
  const targetColumn = columnName ?? editableColumns[0]!.name;
  const sourceIndex = page.columns.findIndex((column) => column.name === targetColumn);
  const draft = Object.fromEntries(page.columns.map((column, index) => {
    const value = row[index] ?? { kind: "null" } as CellValue;
    return [column.name, {
      text: value.kind === "null" ? "" : cellText(value),
      isNull: value.kind === "null",
    }];
  }));
  error.value = null;
  inlineRowEditor.value = {
    mode: "update",
    tabId: tab.id,
    rowIndex: targetRowIndex,
    columns: [...page.columns],
    row: [...row],
    draft,
    activeColumn: targetColumn,
    datePickerPlacement: preferredInlineDatePickerPlacement(targetRowIndex, sourceIndex),
  };
  selectDataCell(targetRowIndex, sourceIndex);
  void focusInlineCell(targetColumn);
}

async function selectTableDataRow(rowIndex: number) {
  const editor = activeInlineRowEditor.value;
  if (editor && editor.rowIndex !== rowIndex && !await finishInlineRowEdit(editor)) return false;
  selectDataRow(rowIndex);
  return true;
}

async function handleInlineRowFocusOut(event: FocusEvent, rowIndex: number) {
  const editor = activeInlineRowEditor.value;
  if (!editor || editor.rowIndex !== rowIndex) return;
  const selectedCell = activeWorkspaceTab.value?.selectedCell;
  const row = event.currentTarget instanceof HTMLElement ? event.currentTarget : null;
  await nextTick();
  const focused = document.activeElement;
  if (row?.contains(focused) || focused?.closest(".dp--menu")) return;
  if (!await finishInlineRowEdit(editor)) {
    void focusInlineCell(editor.activeColumn);
    return;
  }
  const tab = activeWorkspaceTab.value;
  if (tab?.id === editor.tabId && tab.selectedCell === selectedCell) tab.selectedCell = null;
}

async function handleTableDataCellClick(rowIndex: number, sourceIndex: number, columnName: string) {
  if (batchEditColumn.value) cancelColumnBatchEdit();
  const editor = activeInlineRowEditor.value;
  if (editor?.rowIndex === rowIndex) {
    if (inlineColumnEditable(columnName)) {
      activateInlineCell(rowIndex, sourceIndex, columnName);
      void focusInlineCell(columnName);
    } else selectDataCell(rowIndex, sourceIndex);
    return;
  }
  if (editor) {
    if (!await selectTableDataRow(rowIndex)) return;
  } else selectDataRow(rowIndex);
  selectDataCell(rowIndex, sourceIndex);
  startInlineRowEdit(rowIndex, columnName);
}

function cancelInlineRowEdit() {
  if (!busy.value && !inlineRowSavePromise) {
    inlineRowEditor.value = null;
    error.value = null;
  }
}

function saveInlineRowEdit() {
  if (inlineRowSavePromise) return inlineRowSavePromise;
  inlineRowSavePromise = performInlineRowSave().finally(() => { inlineRowSavePromise = null; });
  return inlineRowSavePromise;
}

async function finishInlineRowEdit(editor: InlineRowEditorState) {
  if (inlineRowEditor.value !== editor) return true;
  if (inlineEditorHasChanges(editor)) return saveInlineRowEdit();
  inlineRowEditor.value = null;
  error.value = null;
  return true;
}

async function performInlineRowSave(): Promise<boolean> {
  const editor = inlineRowEditor.value;
  const tab = editor ? workspaceTabs.value.find((item) => item.id === editor.tabId) : null;
  const target = editableTableForTab(tab);
  if (!editor) return true;
  if (!tab || !target || !tab.tableDetail || !tab.connectionId || busy.value) return false;
  let values: [string, CellValue][];
  try {
    values = editor.columns.flatMap((column, index) => {
      const detail = inlineColumnDetail(tab, column.name);
      const original = editor.row[index] ?? { kind: "null" } as CellValue;
      const draft = editor.draft[column.name]!;
      if (isGeneratedColumn(detail) || (editor.mode === "insert" && draft.useDefault)) return [];
      if (editor.mode === "update" && !rowCellChanged(draft, original)) return [];
      return [[
        column.name,
        parseRowCell(column, detail, draft, editor.mode === "update" ? original : undefined),
      ] as [string, CellValue]];
    });
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
    return false;
  }
  if (editor.mode === "update" && !values.length) {
    inlineRowEditor.value = null;
    return true;
  }
  if (!await applyQueryContext(tab)) return false;
  const originalValues = editor.mode === "update" ? rowPairs(editor.columns, editor.row) : [];
  const keyNames = editor.mode === "update" ? uniqueKeyColumns(tab.tableDetail) : [];
  const keyValues = originalValues.filter(([name]) => keyNames.includes(name));
  if (editor.mode === "update" && !keyValues.length) {
    error.value = "该表没有可用的主键或唯一索引，不能安全编辑行";
    return false;
  }
  const mutation = await store.mutateRow(tab.sessionId, {
    database: target.database,
    table: target.name,
    kind: editor.mode,
    values,
    keyValues,
    originalValues,
  });
  if (!mutation) return false;
  if (editor.mode === "update" && mutation.concurrentChange) {
    error.value = "该行已被其他会话修改，请刷新后重试";
    return false;
  }
  inlineRowEditor.value = null;
  await reloadBatchEditedTab(tab);
  return true;
}

function startInlineRowInsert() {
  const tab = activeWorkspaceTab.value;
  const page = displayedResult.value;
  if (!tab || !editableTableForTab(tab) || !tab.tableDetail || !page) return;
  if (activeQueryConnection.value?.readOnly) {
    error.value = "该连接处于只读模式";
    return;
  }
  const editableColumns = visibleInlineColumns();
  if (!editableColumns.length) {
    error.value = "该表没有可编辑字段";
    return;
  }
  cancelColumnBatchEdit();
  const row = page.columns.map(() => ({ kind: "null" } as CellValue));
  const draft = Object.fromEntries(page.columns.map((column) => {
    const detail = inlineColumnDetail(tab, column.name);
    const useDefault = hasDatabaseDefault(detail);
    return [column.name, {
      text: "",
      isNull: !useDefault && columnIsNullable(column, detail),
      useDefault,
    }];
  }));
  const firstColumn = editableColumns[0]!;
  const rowIndex = page.rows.length;
  error.value = null;
  inlineRowEditor.value = {
    mode: "insert",
    tabId: tab.id,
    rowIndex,
    columns: [...page.columns],
    row,
    draft,
    activeColumn: firstColumn.column.name,
    datePickerPlacement: "bottom-start",
  };
  selectDataCell(rowIndex, firstColumn.sourceIndex);
  void nextTick(() => {
    const editor = activeInlineRowEditor.value;
    if (!editor || editor.mode !== "insert" || editor.rowIndex !== rowIndex) return;
    editor.datePickerPlacement = preferredInlineDatePickerPlacement(rowIndex, firstColumn.sourceIndex);
    void focusInlineCell(firstColumn.column.name);
  });
}

function rowPairs(columns: ColumnMeta[], row: CellValue[]) {
  return columns.map((column, index) => [column.name, row[index] ?? { kind: "null" }] as [string, CellValue]);
}

function uniqueKeyColumns(detail: TableDetail) {
  const primary = detail.indexes.find((index) => index.primary && index.columns.length);
  if (primary) return primary.columns;
  const columns = new Map(detail.columns.map((column) => [column.name, column]));
  return detail.indexes.find((index) => index.unique && index.columns.length
    && index.columns.every((name) => columns.get(name)?.nullable === false))?.columns ?? [];
}

interface BatchTransactionScope { ownsTransaction: boolean; savepoint: string | null }

async function executeTransactionControl(tab: WorkspaceTab, sql: string): Promise<boolean> {
  if (!tab.connectionId) return false;
  try {
    await api.execute(tab.connectionId, tab.sessionId, {
      executionId: crypto.randomUUID(), sql, database: tab.database, allowWrite: true,
      pageSize: 1, rowOffset: 0,
    });
    return true;
  } catch (cause) {
    error.value = typeof cause === "object" && cause && "message" in cause ? String(cause.message) : String(cause);
    return false;
  }
}

async function beginBatchTransactionScope(tab: WorkspaceTab): Promise<BatchTransactionScope | null> {
  if (!await applyQueryContext(tab)) return null;
  if (!transactionSessions.value[tab.sessionId]) {
    return await store.beginTransaction(tab.sessionId) ? { ownsTransaction: true, savepoint: null } : null;
  }
  const savepoint = `cockpit_batch_${crypto.randomUUID().replace(/-/g, "")}`;
  const kind = connections.value.find((item) => item.id === tab.connectionId)?.driverKind ?? "mysql";
  return await executeTransactionControl(tab, `SAVEPOINT ${quoteIdentifier(savepoint, kind)}`)
    ? { ownsTransaction: false, savepoint }
    : null;
}

async function rollbackBatchTransactionScope(tab: WorkspaceTab, scope: BatchTransactionScope): Promise<boolean> {
  if (scope.ownsTransaction) return store.rollbackTransaction(tab.sessionId);
  if (!scope.savepoint) return false;
  const kind = connections.value.find((item) => item.id === tab.connectionId)?.driverKind ?? "mysql";
  const identifier = quoteIdentifier(scope.savepoint, kind);
  if (!await executeTransactionControl(tab, `ROLLBACK TO SAVEPOINT ${identifier}`)) return false;
  return executeTransactionControl(tab, `RELEASE SAVEPOINT ${identifier}`);
}

async function commitBatchTransactionScope(tab: WorkspaceTab, scope: BatchTransactionScope): Promise<boolean> {
  if (scope.ownsTransaction) return store.commitTransaction(tab.sessionId);
  if (!scope.savepoint) return false;
  const kind = connections.value.find((item) => item.id === tab.connectionId)?.driverKind ?? "mysql";
  return executeTransactionControl(tab, `RELEASE SAVEPOINT ${quoteIdentifier(scope.savepoint, kind)}`);
}

async function deleteSelectedRow() {
  const tab = activeWorkspaceTab.value;
  const page = displayedResult.value;
  const target = editableTableForTab(tab);
  if (!tab || !target || !tab.tableDetail || !page || tab.selectedRowIndex == null) {
    error.value = "请先选择要删除的行";
    return;
  }
  const row = page.rows[tab.selectedRowIndex];
  if (!row) return;
  const originalValues = rowPairs(page.columns, row);
  const keyNames = uniqueKeyColumns(tab.tableDetail);
  const keyValues = originalValues.filter(([name]) => keyNames.includes(name));
  if (!keyValues.length) {
    error.value = "该表没有可用的主键或唯一索引，不能安全删除行";
    return;
  }
  if (!await confirmDestructiveAction("删除选中的 1 行数据？")) return;
  const mutation = await store.mutateRow(tab.sessionId, {
    database: target.database, table: target.name, kind: "delete", values: [], keyValues, originalValues,
  });
  if (!mutation) return;
  if (mutation.concurrentChange) error.value = "该行已被其他会话修改，请刷新后重试";
  else await reloadBatchEditedTab(tab);
}

async function deleteSelectedRows() {
  const tab = activeWorkspaceTab.value;
  const page = displayedResult.value;
  const target = editableTableForTab(tab);
  const indexes = tab?.selectedRowIndexes ?? [];
  if (!tab || !target || !tab.tableDetail || !page || !indexes.length) return;
  const keyNames = uniqueKeyColumns(tab.tableDetail);
  if (!keyNames.length) { error.value = "该表没有可用的主键或唯一索引，不能安全批量删除"; return; }
  if (!await confirmDestructiveAction(
    `删除选中的 ${indexes.length} 行数据？`,
    "所有行都会进行原值并发校验；删除后无法恢复。",
  )) return;
  const transactionScope = await beginBatchTransactionScope(tab);
  if (!transactionScope) return;
  for (const index of indexes) {
    const row = page.rows[index];
    if (!row) continue;
    const originalValues = rowPairs(page.columns, row);
    const mutation = await store.mutateRow(tab.sessionId, {
      database: target.database, table: target.name, kind: "delete", values: [],
      keyValues: originalValues.filter(([name]) => keyNames.includes(name)), originalValues,
    });
    if (!mutation || mutation.concurrentChange) {
      const message = mutation?.concurrentChange
        ? `第 ${index + 1} 行已被其他会话修改`
        : error.value ?? `第 ${index + 1} 行删除失败`;
      const rolledBack = await rollbackBatchTransactionScope(tab, transactionScope);
      error.value = `${message}，${rolledBack ? "批量删除已回滚" : "回滚失败，事务状态需人工确认"}`;
      return;
    }
  }
  if (!await commitBatchTransactionScope(tab, transactionScope)) return;
  await reloadBatchEditedTab(tab);
}

async function applyBatchEditToRows(column: string, value: CellValue, indexes: number[]) {
  const tab = activeWorkspaceTab.value;
  const page = displayedResult.value;
  const target = activeEditableTable.value;
  if (!tab || !target || !tab.tableDetail || !page || !indexes.length) return false;
  if (!batchEditableColumns.value.some((item) => item.name === column)) {
    error.value = "该字段不能批量修改";
    return false;
  }
  const keyNames = uniqueKeyColumns(tab.tableDetail);
  if (!keyNames.length) { error.value = "该表没有可用的主键或唯一索引，不能安全批量修改"; return false; }
  const transactionScope = await beginBatchTransactionScope(tab);
  if (!transactionScope) return false;
  for (const index of indexes) {
    const row = page.rows[index];
    if (!row) continue;
    const originalValues = rowPairs(page.columns, row);
    const mutation = await store.mutateRow(tab.sessionId, {
      database: target.database, table: target.name, kind: "update", values: [[column, value]],
      keyValues: originalValues.filter(([name]) => keyNames.includes(name)), originalValues,
    });
    if (!mutation || mutation.concurrentChange) {
      const message = mutation?.concurrentChange
        ? `第 ${index + 1} 行已被其他会话修改`
        : error.value ?? `第 ${index + 1} 行修改失败`;
      const rolledBack = await rollbackBatchTransactionScope(tab, transactionScope);
      error.value = `${message}，${rolledBack ? "批量修改已回滚" : "回滚失败，事务状态需人工确认"}`;
      return false;
    }
  }
  if (!await commitBatchTransactionScope(tab, transactionScope)) return false;
  await reloadBatchEditedTab(tab);
  return true;
}

async function applyBatchEdit(column: string, value: CellValue) {
  if (await applyBatchEditToRows(column, value, batchEditRowIndexes.value)) closeBatchEditDialog();
}

function cellSqlLiteral(value: CellValue) {
  if (value.kind === "null") return "NULL";
  if (["signed", "unsigned", "decimal", "float"].includes(value.kind)) return "value" in value ? String(value.value) : "NULL";
  const text = cellText(value);
  return `'${text.replace(/'/g, "''")}'`;
}

async function openSelectedForeignKey() {
  const reference = selectedForeignKey.value;
  if (!reference?.value || !activeConnectionId.value) return;
  const table: TableInfo = {
    database: reference.foreignKey.referencedDatabase,
    name: reference.foreignKey.referencedTable,
    tableType: "BASE TABLE",
  };
  await previewTable(table);
  const tab = activeWorkspaceTab.value;
  if (tab?.kind === "table") {
    tab.filter = `${quoteIdentifier(reference.referencedColumn, activeQueryConnection.value?.driverKind ?? "mysql")} = ${cellSqlLiteral(reference.value)}`;
    tab.appliedFilter = tab.filter;
    await reloadTableTab(tab);
  }
}

async function reloadTableTab(tab: WorkspaceTab) {
  if (tab.kind !== "table" || !tab.database) return;
  if (!await applyQueryContext(tab)) return;
  const offset = tab.result?.rowOffset ?? 0;
  const pageSize = tab.pageSize ?? 100;
  const statement = selectTablePageSql(
    tab.database,
    tab.title,
    pageSize,
    offset,
    tab.appliedFilter,
    tab.sortColumn,
    tab.sortDirection,
    connections.value.find((item) => item.id === tab.connectionId)?.driverKind ?? "mysql",
  );
  const page = await store.execute(tab.sessionId, statement, false, 0, pageSize);
  if (page) {
    if (inlineRowEditor.value?.tabId === tab.id) inlineRowEditor.value = null;
    page.rowOffset = offset;
    tab.sql = statement;
    tab.result = page;
    tab.selectedRowIndex = null;
    tab.selectedRowIndexes = [];
    tab.selectedCell = null;
  }
}

async function reloadBatchEditedTab(tab: WorkspaceTab) {
  if (tab.kind === "table") {
    await reloadTableTab(tab);
    return;
  }
  if (tab.kind !== "console") return;
  const resultSql = tab.pagingSql ?? tab.resultSql;
  if (!resultSql) return;
  if (!await applyQueryContext(tab)) return;
  const offset = tab.result?.rowOffset ?? 0;
  const pageSize = tab.pageSize ?? 500;
  const pageable = Boolean(tab.pagingSql);
  const statement = pageable && !tab.pagingUsesDriverOffset ? selectQueryPageSql(
    resultSql,
    pageSize,
    offset,
    connections.value.find((item) => item.id === tab.connectionId)?.driverKind ?? "mysql",
  ) : resultSql;
  const page = await store.execute(tab.sessionId, statement, false, pageable && tab.pagingUsesDriverOffset ? offset : 0, pageSize);
  if (page) {
    page.rowOffset = offset;
    const resultSetIndex = tab.resultSetIndex ?? 0;
    tab.result = page;
    tab.resultSetIndex = Math.min(resultSetIndex, page.additionalResultSets?.length ?? 0);
    selectConsoleEditableResult(tab, tab.resultSetIndex);
    tab.selectedRowIndex = null;
    tab.selectedRowIndexes = [];
    tab.selectedCell = null;
  }
}

function startExportTask(title: string, outputPath: string, total?: number) {
  const taskId = crypto.randomUUID();
  activeExportTaskId.value = taskId;
  createTransferTask({
    taskId,
    kind: "export",
    title,
    phase: "准备",
    completed: 0,
    total,
    message: "正在准备导出文件",
    status: "running",
    cancellable: false,
    startedAt: new Date().toISOString(),
    outputPath,
  });
  return taskId;
}

function exportErrorMessage(cause: unknown) {
  return typeof cause === "object" && cause && "message" in cause ? String(cause.message) : String(cause);
}

function completeExportTask(taskId: UUID, rowsWritten: number) {
  finishTransferTask(taskId, "completed", {
    phase: "完成",
    completed: rowsWritten,
    total: rowsWritten,
    message: `已导出 ${rowsWritten} 行`,
  });
}

function failExportTask(taskId: UUID, cause: unknown) {
  const message = exportErrorMessage(cause);
  const cancelled = message.includes("取消");
  error.value = message;
  finishTransferTask(taskId, cancelled ? "cancelled" : "failed", {
    phase: cancelled ? "已取消" : "失败",
    message,
    error: message,
  });
}

async function revealExportOutput() {
  const outputPath = activeExportTask.value?.outputPath;
  if (!outputPath) return;
  try {
    await api.revealFile(outputPath);
  } catch (cause) {
    error.value = exportErrorMessage(cause);
  }
}

async function exportCurrentPage() {
  const page = displayedResult.value;
  if (!page) return;
  const format = EXPORT_FORMATS.find((item) => item.value === exportFormat.value)!;
  const tab = activeWorkspaceTab.value;
  const baseName = (tab?.kind === "table" ? tab.title : `${tab?.title ?? selectedDatabase.value ?? "query"}-result`)
    .replace(/[\\/:*?"<>|]/g, "-");
  const outputPath = await save({
    title: "导出结果",
    defaultPath: `${baseName}.${format.extension}`,
    filters: [{ name: format.label, extensions: [format.extension] }],
  });
  if (!outputPath) return;
  const normalizedPath = outputPath.match(/\.(txt|sql|csv|xlsx)$/i)
    ? outputPath.replace(/\.(txt|sql|csv|xlsx)$/i, `.${format.extension}`)
    : `${outputPath}.${format.extension}`;
  const taskId = startExportTask(`导出 ${baseName} 当前页`, normalizedPath, page.rows.length);
  busy.value = true;
  error.value = null;
  try {
    const summary = await api.exportResultPage(normalizedPath, page, {
      format: exportFormat.value,
      databaseName: tab?.kind === "table" ? tab.database : null,
      tableName: tab?.kind === "table" ? tab.title : "query_result",
      databaseKind: activeQueryConnection.value?.driverKind ?? "mysql",
    });
    completeExportTask(taskId, summary.rowsWritten);
  } catch (cause) {
    failExportTask(taskId, cause);
  } finally { busy.value = false; }
}

async function exportFullTable() {
  const tab = activeWorkspaceTab.value;
  if (!tab || tab.kind !== "table" || !tab.connectionId || !tab.database) return;
  const format = EXPORT_FORMATS.find((item) => item.value === exportFormat.value)!;
  const outputPath = await save({
    title: "导出整表",
    defaultPath: `${tab.title}.${format.extension}`,
    filters: [{ name: format.label, extensions: [format.extension] }],
  });
  if (!outputPath) return;
  const normalizedPath = outputPath.match(/\.(txt|sql|csv|xlsx)$/i)
    ? outputPath.replace(/\.(txt|sql|csv|xlsx)$/i, `.${format.extension}`)
    : `${outputPath}.${format.extension}`;
  const taskId = startExportTask(`导出 ${tab.database}.${tab.title} 整表`, normalizedPath);
  busy.value = true;
  error.value = null;
  try {
    const summary = await api.exportTable(tab.connectionId, tab.database, tab.title, normalizedPath, {
      format: exportFormat.value, databaseName: tab.database, tableName: tab.title,
      databaseKind: connections.value.find((item) => item.id === tab.connectionId)?.driverKind ?? "mysql",
    }, taskId);
    completeExportTask(taskId, summary.rowsWritten);
  } catch (cause) {
    failExportTask(taskId, cause);
  } finally { busy.value = false; }
}

async function exportFullQuery() {
  const tab = activeWorkspaceTab.value;
  if (!tab || tab.kind !== "console" || !tab.connectionId || !tab.pagingSql) return;
  const format = EXPORT_FORMATS.find((item) => item.value === exportFormat.value)!;
  const outputPath = await save({
    title: "导出完整查询结果",
    defaultPath: `${tab.title.replace(/[\\/:*?"<>|]/g, "-")}.${format.extension}`,
    filters: [{ name: format.label, extensions: [format.extension] }],
  });
  if (!outputPath) return;
  const normalizedPath = outputPath.match(/\.(txt|sql|csv|xlsx)$/i)
    ? outputPath.replace(/\.(txt|sql|csv|xlsx)$/i, `.${format.extension}`)
    : `${outputPath}.${format.extension}`;
  const taskId = startExportTask(`导出 ${tab.title} 全部结果`, normalizedPath);
  busy.value = true;
  error.value = null;
  try {
    const summary = await api.exportQuery(tab.connectionId, tab.database, tab.pagingSql, normalizedPath, {
      format: exportFormat.value, databaseName: tab.database, tableName: "query_result",
      databaseKind: connections.value.find((item) => item.id === tab.connectionId)?.driverKind ?? "mysql",
    }, taskId);
    completeExportTask(taskId, summary.rowsWritten);
  } catch (cause) {
    failExportTask(taskId, cause);
  } finally { busy.value = false; }
}

async function importIntoCurrentTable() {
  const tab = activeWorkspaceTab.value;
  if (!tab || tab.kind !== "table" || !tab.connectionId || !tab.database) return;
  showImportDialog.value = true;
}

async function finishTableImport() {
  const tab = activeWorkspaceTab.value;
  if (tab?.kind === "table") await reloadTableTab(tab);
}

async function importSqlFile(targetDatabase?: string) {
  const database = targetDatabase ?? selectedDatabase.value;
  if (!activeConnectionId.value || !database) {
    error.value = "请先连接并选择数据库";
    return;
  }
  const inputPath = await open({ multiple: false, directory: false, filters: [{ name: "SQL 备份", extensions: ["sql", "gz", "enc"] }] });
  if (!inputPath || Array.isArray(inputPath)) return;
  const encryptionPassword = inputPath.toLocaleLowerCase().endsWith(".enc")
    ? await promptAction({
      title: "解密数据库备份",
      message: "所选备份文件已加密，请输入创建备份时使用的密码。",
      inputLabel: "备份密码",
      inputType: "password",
      inputRequired: true,
      inputValidationMessage: "请输入备份密码",
      trimInput: false,
      confirmLabel: "解密并继续",
    })
    : null;
  if (inputPath.toLocaleLowerCase().endsWith(".enc") && encryptionPassword === null) return;
  if (!await confirmAction({
    title: "确认恢复数据库",
    message: `将在数据库“${database}”中执行所选备份文件。`,
    detail: "文件中的写入和结构变更会直接生效。建议确认目标数据库并提前完成备份。",
    tone: "danger",
    confirmLabel: "开始恢复",
  })) return;
  busy.value = true;
  error.value = null;
  const taskId = crypto.randomUUID();
  createTransferTask({
    taskId, kind: "restore", title: `恢复 ${database}`, phase: "准备", completed: 0,
    status: "running", startedAt: new Date().toISOString(), outputPath: inputPath,
  });
  try {
    if (selectedDatabase.value !== database) await openDatabase(database);
    const summary = await api.importData({ connectionId: activeConnectionId.value, database, inputPath, format: "sql", hasHeaders: false, taskId, encryptionPassword });
    await Promise.all([store.loadTables("", false), store.loadDatabaseObjects(database)]);
    await showNotice({
      title: "恢复完成",
      message: `已成功执行 ${summary.statementsExecuted} 条语句。`,
      detail: `目标数据库：${database}`,
      tone: "success",
      confirmLabel: "完成",
    });
  } catch (cause) {
    const message = typeof cause === "object" && cause && "message" in cause ? String(cause.message) : String(cause);
    error.value = message;
    finishTransferTask(taskId, message.includes("取消") ? "cancelled" : "failed", { error: message, phase: message.includes("取消") ? "已取消" : "失败" });
  } finally { busy.value = false; }
}
</script>

<template>
  <div class="app-shell" :class="{ 'is-navigation-resizing': isNavigationResizing, 'is-result-resizing': isResultResizing, 'is-column-resizing': isColumnResizing }" :style="{ '--navigation-width': `${navigationWidth}px` }" :aria-busy="busy || Boolean(executingId)">
    <AppToolbar
      :settings-active="showSettings"
      @new-query="createQuery"
      @add-connection="showDialog = true; editing = null"
      @open-sql="openSqlFile"
      @settings="showSettings = true"
    />

    <div v-if="error && !activeWorkspaceTab" class="global-error-notice" role="alert">
      <span class="global-error-icon" aria-hidden="true">!</span>
      <div><strong>操作未完成</strong><span>{{ error }}</span></div>
      <button type="button" class="icon-button" aria-label="关闭错误提示" @click="error = null"><X :size="14" /></button>
    </div>

    <div v-if="queryFileNotice" class="query-file-notice" :class="queryFileNotice.kind" role="status">
      <FileCode2 :size="15" />
      <span>{{ queryFileNotice.message }}</span>
      <button type="button" aria-label="关闭文件提示" @click="queryFileNotice = null"><X :size="13" /></button>
    </div>

    <div v-if="activeExportTask" class="export-progress-notice" :class="activeExportTask.status" role="status">
      <RefreshCw v-if="activeExportTask.status === 'running'" :size="16" class="loading-icon" />
      <Download v-else :size="16" />
      <div class="export-progress-copy">
        <strong>{{ activeExportStatusLabel }}</strong>
        <span>{{ activeExportTask.message || `${activeExportTask.completed} 行` }}<template v-if="activeExportPercent !== null"> · {{ activeExportPercent }}%</template></span>
        <progress
          v-if="activeExportTask.status === 'running' || activeExportTask.status === 'completed'"
          :value="activeExportPercent ?? undefined"
          max="100"
          aria-label="数据导出进度"
        />
      </div>
      <button v-if="activeExportTask.status === 'completed' && activeExportTask.outputPath" type="button" class="link" @click="revealExportOutput">在文件夹中显示</button>
      <button v-if="activeExportTask.status !== 'running'" type="button" class="export-progress-close" aria-label="关闭导出提示" @click="activeExportTaskId = null"><X :size="13" /></button>
    </div>

    <NavigationSidebar
      v-model:expanded-table-group="expandedTableGroup"
      v-model:expanded-view-group="expandedViewGroup"
      v-model:expanded-function-group="expandedFunctionGroup"
      v-model:expanded-trigger-group="expandedTriggerGroup"
      v-model:expanded-event-group="expandedEventGroup"
      :connections="connections"
      :connection-info="connectionInfo"
      :busy="busy"
      :selected-database="selectedDatabase"
      :active-connection-id="activeConnectionId"
      :connection-groups="connectionGroups"
      :redis-databases="redisDatabases"
      :redis-loading="redisLoading"
      :filtered-databases="filteredDatabases"
      :filtered-base-tables="filteredBaseTables"
      :filtered-views="filteredViews"
      :filtered-routines="filteredRoutines"
      :filtered-triggers="filteredTriggers"
      :filtered-events="filteredEvents"
      :table-has-more="tableHasMore"
      :selected-table="selectedTable"
      :expanded-connection-id="expandedConnectionId"
      :expanded-database="expandedDatabase"
      :navigation-width="navigationWidth"
      :min-width="MIN_NAVIGATION_WIDTH"
      :max-width="MAX_NAVIGATION_WIDTH"
      :runtime-stats="runtimeStats"
      :runtime-stats-state="runtimeStatsState"
      @add-connection="showDialog = true; editing = null"
      @load-more="loadMoreTables"
      @toggle-connection="toggleConnection"
      @edit-connection="editConnection"
      @open-redis-manager="openRedisManager"
      @open-redis-database="openRedisManager"
      @disconnect-connection="disconnectConnection"
      @toggle-database="toggleDatabase"
      @context-menu="openContextMenu"
      @highlight-table="store.highlightTable"
      @preview-table="previewTable"
      @open-database-object="openDatabaseObject"
      @resize-start="startNavigationResize"
      @resize-move="resizeNavigation"
      @resize-end="finishNavigationResize"
      @resize-cancel="finishNavigationResize"
      @resize-reset="resetNavigationWidth"
      @resize-key="resizeNavigationWithKeyboard"
    />

    <main v-if="connections.length || activeWorkspaceTab || redisManagerConnection" class="workspace">
      <RedisManager v-if="redisManagerConnection" :connection="redisManagerConnection" :initial-database="redisManagerDatabase ?? undefined" @close="closeRedisManager" />
      <template v-if="!redisManagerConnection">
      <WorkspaceEmpty v-if="!activeWorkspaceTab" />

      <WorkspaceTabs
        v-if="workspaceTabs.length"
        :tabs="workspaceTabs"
        :active-id="activeWorkspaceTabId"
        :dirty-ids="dirtyWorkspaceTabIds"
        @activate="activateWorkspaceTabById"
        @close="closeWorkspaceTab"
        @toggle-pin="toggleActiveTabPin"
      />

      <div v-if="activeWorkspaceTab?.kind === 'console'" ref="workspaceContent" class="workspace-content">
        <section class="editor-card">
          <div class="card-toolbar editor-toolbar">
            <div class="editor-toolbar-context">
              <div v-if="activeWorkspaceTitle || activeQueryConnection?.production || activeQueryConnection?.readOnly || activeTransaction" class="editor-toolbar-heading"><strong v-if="activeWorkspaceTitle">{{ activeWorkspaceTitle }}</strong><span v-if="activeQueryConnection?.production" class="connection-badge production">生产</span><span v-if="activeQueryConnection?.readOnly" class="connection-badge readonly">只读</span><span v-if="activeTransaction" class="connection-badge transaction">事务中</span></div>
              <div v-if="activeWorkspaceTab.closable" class="query-context-picker" aria-label="查询上下文">
                <div class="query-context-field">
                  <span class="query-context-label query-context-connection-label"><img class="query-context-database-icon" :class="{ connected: Boolean(activeQueryConnection && connectionInfo[activeQueryConnection.id]) }" :src="activeQueryConnectionIcon" alt="" aria-hidden="true" />连接</span>
                  <span class="query-context-control"><AppSelect :model-value="activeWorkspaceTab.connectionId ?? null" :options="connections.map((connection) => ({ value: connection.id, label: connection.name }))" label="查询连接" variant="context" :menu-min-width="190" :disabled="busy" @update:model-value="selectQueryConnection" /></span>
                </div>
                <ChevronRight class="query-context-path" :size="13" aria-hidden="true" />
                <div class="query-context-field" :class="{ disabled: busy || !activeWorkspaceTab.connectionId }">
                  <span class="query-context-label">数据库</span>
                  <span class="query-context-control"><AppSelect :model-value="activeWorkspaceTab.database ?? null" :options="databases.map((database) => ({ value: database.name, label: database.name }))" label="查询数据库" variant="context" :menu-min-width="190" :disabled="busy || !activeWorkspaceTab.connectionId" @update:model-value="selectQueryDatabase" /></span>
                </div>
              </div>
              <div class="toolbar-actions query-toolbar-actions" role="toolbar" aria-label="查询操作">
                <div class="query-toolbar-group" role="group" aria-label="查询编辑">
                  <button class="ghost compact" @click="saveCurrentQuery()"><img class="query-toolbar-icon" :src="saveQueryIcon" alt="" aria-hidden="true" /> 保存 <kbd>{{ shortcutModifier }}S</kbd></button>
                  <button class="ghost compact" @click="formatSql"><img class="query-toolbar-icon" :src="formatIcon" alt="" aria-hidden="true" /> 美化</button>
                </div>
                <div class="query-toolbar-group" role="group" aria-label="事务操作">
                  <template v-if="activeTransaction">
                    <button class="ghost compact" :disabled="busy" @click="commitTransaction"><img class="query-toolbar-icon" :src="commitIcon" alt="" aria-hidden="true" /> 提交</button>
                    <button class="danger compact" :disabled="busy" @click="rollbackTransaction"><img class="query-toolbar-icon" :src="rollbackIcon" alt="" aria-hidden="true" /> 回滚</button>
                  </template>
                  <button v-else class="ghost compact" :disabled="busy || activeQueryConnection?.readOnly" @click="beginTransaction"><img class="query-toolbar-icon" :src="transactionIcon" alt="" aria-hidden="true" /> 事务</button>
                </div>
                <div class="query-toolbar-group" role="group" aria-label="执行与导出">
                  <button v-if="executingId" class="danger compact" @click="store.cancel"><img class="query-toolbar-icon" :src="stopIcon" alt="" aria-hidden="true" /> 停止</button>
                  <button v-else class="window-tool execute-button" :disabled="!connected || !activeWorkspaceTab.connectionId" @click="execute()"><img class="query-toolbar-icon" :src="runIcon" alt="" aria-hidden="true" /> 执行 <kbd>{{ shortcutModifier }}↵</kbd></button>
                  <button type="button" class="ghost compact query-export-trigger" aria-haspopup="dialog" aria-controls="query-export-dialog" :disabled="busy || !displayedResult?.columns.length" :title="displayedResult?.columns.length ? '导出查询结果' : '查询完成后可导出结果'" @click="showQueryExportDialog = true"><img class="query-toolbar-icon" :src="exportIcon" alt="" aria-hidden="true" /> 导出</button>
                </div>
              </div>
            </div>
          </div>
          <SqlEditor
            :key="activeWorkspaceTab.id"
            ref="sqlEditor"
            :model-value="sql"
            :document-id="activeWorkspaceTab.id"
            :schema="editorSchema"
            :load-table-columns="loadEditorTableColumns"
            :database-kind="activeQueryConnection?.driverKind ?? 'mysql'"
            :font-size="settings.editorFontSize ?? 12"
            :tab-size="settings.editorTabSize ?? 2"
            @commit:value="commitWorkspaceTabSql"
            @execute="execute"
          />
        </section>

        <div
          v-if="activeQueryPanelVisible"
          class="result-resizer"
          role="separator"
          aria-label="调整查询结果高度"
          aria-orientation="horizontal"
          :aria-valuenow="Math.round(resultPanelRatio * 100)"
          aria-valuemin="0"
          aria-valuemax="100"
          :aria-valuetext="`结果区域 ${Math.round(resultPanelRatio * 100)}%`"
          tabindex="0"
          @pointerdown="startResultResize"
          @pointermove="resizeResult"
          @pointerup="finishResultResize"
          @pointercancel="finishResultResize"
          @dblclick="resetResultPanelHeight"
          @keydown="resizeResultWithKeyboard"
        />

        <section v-if="activeQueryPanelVisible" class="result-card" :class="{ 'editable-query-result': Boolean(activeEditableTable) }" :style="resultPanelStyle">
          <div class="card-toolbar result-toolbar">
            <nav class="result-view-tabs" role="tablist" aria-label="查询结果视图">
              <button id="query-result-tab" type="button" role="tab" aria-controls="query-result-panel" :aria-selected="activeResultView === 'result'" :class="{ active: activeResultView === 'result' }" @click="selectResultView('result')">结果</button>
              <button id="query-summary-tab" type="button" role="tab" aria-controls="query-summary-panel" :aria-selected="activeResultView === 'summary'" :class="{ active: activeResultView === 'summary' }" :disabled="Boolean(activeInlineRowEditor)" @click="selectResultView('summary')">摘要</button>
            </nav>
            <RefreshCw v-if="executingId" :size="13" class="loading-icon table-loading-indicator" aria-label="正在执行查询" />
            <div v-if="activeResultView === 'result'" class="toolbar-actions result-toolbar-actions">
              <span v-if="displayedResult" class="muted result-toolbar-summary">{{ visibleResultRows.length }} / {{ displayedResult.rows.length }} 行 · {{ displayedResult.executionTimeMs }} ms</span>
              <span v-if="displayedResult && !displayedResult.columns.length" class="success">影响 {{ displayedResult.affectedRows }} 行</span>
              <template v-if="displayedResult?.columns.length">
                <input v-model="activeWorkspaceTab.resultFilter" class="result-search" type="search" aria-label="在当前结果页中搜索" placeholder="页内搜索" />
                <button class="ghost compact" @click="showColumnManager = true">列</button>
                <button class="ghost compact" @click="showResultInsights = true"><ArrowUpDown :size="13" /> 分析</button>
                <button class="ghost compact icon-only" aria-label="复制选中单元格" :disabled="!activeWorkspaceTab.selectedCell" @click="copyResult('cell')"><Copy :size="13" /></button>
                <button class="ghost compact icon-only" aria-label="复制当前页" @click="copyResult('page')"><Clipboard :size="13" /></button>
                <div v-if="activeEditableTable" class="table-primary-actions query-edit-actions">
                  <button class="ghost compact" :disabled="busy || Boolean(executingId) || activeQueryConnection?.readOnly || Boolean(activeInlineRowEditor)" @click="startInlineRowInsert"><Plus :size="13" /> 新增</button>
                  <button class="danger compact" :disabled="busy || Boolean(executingId) || activeWorkspaceTab.selectedRowIndex == null || activeQueryConnection?.readOnly || Boolean(activeInlineRowEditor)" @click="deleteSelectedRow"><Trash2 :size="13" /> 删除</button>
                </div>
              </template>
              <div v-if="displayedResult?.columns.length" class="result-pagination">
                <button class="ghost compact icon-only" aria-label="上一页" :disabled="Boolean(executingId) || !activeWorkspaceTab.pageable || (displayedResult.rowOffset ?? 0) === 0 || (activeWorkspaceTab.resultSetIndex ?? 0) !== 0" @click="changeResultPage(-1)"><ChevronLeft :size="13" /></button>
                <span>{{ Math.floor((displayedResult.rowOffset ?? 0) / (displayedResult.pageSize ?? 500)) + 1 }}</span>
                <button class="ghost compact icon-only" aria-label="下一页" :disabled="Boolean(executingId) || !activeWorkspaceTab.pageable || !displayedResult.hasMore || (activeWorkspaceTab.resultSetIndex ?? 0) !== 0" @click="changeResultPage(1)"><ChevronRight :size="13" /></button>
              </div>
            </div>
            <button class="ghost compact icon-only result-panel-close" aria-label="关闭结果面板" :disabled="Boolean(activeInlineRowEditor)" @click="closeResultPanel"><X :size="13" /></button>
          </div>
          <div v-if="activeResultView === 'result'" id="query-result-panel" class="result-view-panel" role="tabpanel" aria-labelledby="query-result-tab">
          <nav v-if="allResultSets.length > 1" class="result-set-tabs" role="tablist" aria-label="查询结果集"><button v-for="set in allResultSets" :key="set.resultSetIndex" role="tab" :aria-selected="(activeWorkspaceTab.resultSetIndex ?? 0) === set.resultSetIndex" :class="{ active: (activeWorkspaceTab.resultSetIndex ?? 0) === set.resultSetIndex }" @click="selectResultSet(set.resultSetIndex)">结果 {{ set.resultSetIndex + 1 }} <small>{{ set.rows.length }}</small></button></nav>
          <input
            v-if="activeEditableTable && activeBatchEditColumn"
            v-model="columnBatchEditValue"
            class="column-direct-edit-input"
            type="text"
            tabindex="-1"
            :aria-label="`直接编辑 ${activeBatchEditColumn.name} 整列，Enter 保存，Escape 取消`"
            :disabled="busy"
            autocomplete="off"
            autocorrect="off"
            autocapitalize="none"
            spellcheck="false"
            @input="handleDirectColumnEditInput"
            @keydown.enter.prevent="submitColumnBatchEdit"
            @keydown.esc.stop.prevent="cancelDirectColumnEdit"
          />
          <form v-else-if="activeBatchEditColumn" class="column-batch-edit-bar" @submit.prevent="submitColumnBatchEdit">
            <Pencil :size="13" />
            <span>已选列</span><strong>{{ activeBatchEditColumn.name }}</strong>
            <span class="muted">当前页可见 {{ visibleResultRows.length }} 行统一改为</span>
            <input v-model="columnBatchEditValue" class="column-batch-edit-input" :type="columnBatchEditInputType" :step="columnBatchEditInputType === 'datetime-local' ? 'any' : undefined" :disabled="columnBatchEditNull || busy" :aria-label="`${activeBatchEditColumn.name} 列的新值`" placeholder="输入新值" autocomplete="off" spellcheck="false" @input="columnBatchEditError = ''" />
            <label class="column-batch-null"><input v-model="columnBatchEditNull" type="checkbox" :disabled="!activeBatchEditColumn.nullable || busy" />NULL</label>
            <span v-if="columnBatchEditError" class="column-batch-edit-error" role="alert">{{ columnBatchEditError }}</span>
            <button type="submit" class="primary compact" :disabled="busy || !visibleResultRows.length">更新 {{ visibleResultRows.length }} 行</button>
            <button type="button" class="ghost compact" :disabled="busy" @click="cancelColumnBatchEdit">取消</button>
          </form>
          <div v-if="error" class="error-banner dismissible-error-banner"><span>{{ error }}</span><button type="button" aria-label="关闭错误提示" @click="error = null"><X :size="13" /></button></div>
          <div v-if="!displayedResult" class="empty-result"><Play :size="24" /><p>执行 SQL 后在这里查看结果</p></div>
          <div ref="gridScroll" v-else-if="displayedResult.columns.length" class="grid-scroll" @scroll.passive="handleGridScroll">
            <table
              class="data-grid"
              role="grid"
              aria-label="查询结果数据"
              :aria-rowcount="displayedResult.rows.length + (activeInlineRowEditor?.mode === 'insert' ? 1 : 0)"
              :aria-colcount="visibleColumnEntries.length"
              :style="{ width: `${gridWidth(visibleColumnEntries)}px` }"
            >
              <colgroup><col v-if="activeEditableTable" class="selection-column"><col v-for="entry in visibleColumnEntries" :key="columnKey(entry.column, entry.sourceIndex)" class="data-column" :style="{ width: `${columnWidth(entry.column, entry.sourceIndex)}px` }"></colgroup>
              <thead><tr>
                <th v-if="activeEditableTable" class="row-selection"><input type="checkbox" aria-label="选择当前页全部行" :aria-checked="selectedVisibleRowCount > 0 && selectedVisibleRowCount < visibleResultRows.length ? 'mixed' : selectedVisibleRowCount === visibleResultRows.length && visibleResultRows.length > 0" :checked="visibleResultRows.length > 0 && selectedVisibleRowCount === visibleResultRows.length" :indeterminate="selectedVisibleRowCount > 0 && selectedVisibleRowCount < visibleResultRows.length" :disabled="Boolean(executingId) || Boolean(activeInlineRowEditor)" @change="toggleAllVisibleRows(($event.currentTarget as HTMLInputElement).checked)" /></th>
                <th
                  v-for="(entry, displayIndex) in visibleColumnEntries"
                  :key="columnKey(entry.column, entry.sourceIndex)"
                  class="data-column"
                  :class="{ 'selectable-column': Boolean(activeEditableTable), 'editable-result-column': Boolean(activeEditableTable), 'selected-column': batchEditColumn === entry.column.name, frozen: displayIndex < (activeWorkspaceTab.frozenColumnCount ?? 0) }"
                  :tabindex="activeEditableTable ? 0 : undefined"
                  :style="frozenColumnStyle(displayIndex, entry.column, entry.sourceIndex)"
                  @click="activeEditableTable && selectColumnBatchEdit(entry.column.name)"
                  @keydown.enter.self.prevent="activeEditableTable && selectColumnBatchEdit(entry.column.name)"
                  @keydown.space.self.prevent="activeEditableTable && selectColumnBatchEdit(entry.column.name)"
                ><span class="column-name">{{ entry.column.name }}</span><small>{{ entry.column.databaseType }}{{ entry.column.unsigned ? ' unsigned' : '' }}</small><span class="column-resizer" role="separator" aria-orientation="vertical" :aria-label="`调整 ${entry.column.name} 列宽`" :aria-valuenow="columnWidth(entry.column, entry.sourceIndex)" :aria-valuemin="MIN_DATA_COLUMN_WIDTH" :aria-valuemax="MAX_DATA_COLUMN_WIDTH" tabindex="0" @click.stop @pointerdown="startColumnResize($event, entry.column, entry.sourceIndex)" @pointermove="resizeColumn" @pointerup="finishColumnResize" @pointercancel="finishColumnResize" @dblclick.stop="resetColumnWidth(entry.column, entry.sourceIndex)" @keydown="resizeColumnWithKeyboard($event, entry.column, entry.sourceIndex)" /></th>
              </tr></thead>
              <tbody>
                <tr v-if="virtualResultPaddingTop" class="virtual-spacer-row" aria-hidden="true"><td class="virtual-spacer-cell" :colspan="visibleColumnEntries.length + (activeEditableTable ? 1 : 0)" :style="{ height: `${virtualResultPaddingTop}px` }"></td></tr>
                <tr v-for="entry in renderedResultRows" :key="entry.rowIndex" class="data-row" :aria-rowindex="entry.rowIndex + 2" :class="{ selected: activeWorkspaceTab.selectedRowIndex === entry.rowIndex, 'alternate-row': entry.visibleIndex % 2 === 1, 'inline-editing-row': isInlineEditingRow(entry.rowIndex), 'inline-insert-row': isInlineInsertingRow(entry.rowIndex) }" @click="activeEditableTable ? selectTableDataRow(entry.rowIndex) : selectDataRow(entry.rowIndex)" @focusout="activeEditableTable && handleInlineRowFocusOut($event, entry.rowIndex)">
                  <td v-if="activeEditableTable" class="row-selection" @click.stop><input type="checkbox" :aria-label="`选择第 ${(displayedResult.rowOffset ?? 0) + entry.rowIndex + 1} 行`" :checked="selectedRowIndexes.includes(entry.rowIndex)" :disabled="Boolean(executingId) || Boolean(activeInlineRowEditor)" @change="toggleDataRow(entry.rowIndex, ($event.currentTarget as HTMLInputElement).checked)" /></td>
                  <td
                    v-for="(columnEntry, displayIndex) in visibleColumnEntries"
                    :key="columnEntry.sourceIndex"
                    role="gridcell"
                    :data-grid-row="entry.rowIndex"
                    :data-grid-column="columnEntry.sourceIndex"
                    :tabindex="gridCellTabIndex(entry.rowIndex, columnEntry.sourceIndex)"
                    :aria-selected="activeWorkspaceTab.selectedCell?.row === entry.rowIndex && activeWorkspaceTab.selectedCell?.column === columnEntry.sourceIndex"
                    :class="{
                      null: columnDirectEditActive && batchEditColumn === columnEntry.column.name
                        ? false
                        : isInlineEditingRow(entry.rowIndex) && inlineColumnEditable(columnEntry.column.name)
                          ? activeInlineRowEditor?.draft[columnEntry.column.name]?.isNull
                          : entry.row[columnEntry.sourceIndex]?.kind === 'null',
                      'selected-cell': activeWorkspaceTab.selectedCell?.row === entry.rowIndex && activeWorkspaceTab.selectedCell?.column === columnEntry.sourceIndex,
                      'selected-column': batchEditColumn === columnEntry.column.name,
                      'column-direct-edit-preview': columnDirectEditActive && batchEditColumn === columnEntry.column.name,
                      'inline-editing-cell': isInlineEditingRow(entry.rowIndex) && inlineColumnEditable(columnEntry.column.name) && activeInlineRowEditor?.activeColumn === columnEntry.column.name,
                      frozen: displayIndex < (activeWorkspaceTab.frozenColumnCount ?? 0),
                    }"
                    :style="frozenColumnStyle(displayIndex, columnEntry.column, columnEntry.sourceIndex)"
                    @focus="selectDataCell(entry.rowIndex, columnEntry.sourceIndex)"
                    @click.stop="activeEditableTable ? handleTableDataCellClick(entry.rowIndex, columnEntry.sourceIndex, columnEntry.column.name) : handleResultDataCellClick(entry.rowIndex, columnEntry.sourceIndex)"
                    @dblclick.stop="!activeEditableTable && openCellViewer(entry.rowIndex, columnEntry.sourceIndex)"
                    @keydown="handleGridCellKeydown($event, entry.rowIndex, columnEntry.sourceIndex, columnEntry.column.name)"
                  >
                    <div v-if="activeEditableTable && isInlineEditingRow(entry.rowIndex) && inlineColumnEditable(columnEntry.column.name) && activeInlineRowEditor?.activeColumn === columnEntry.column.name" class="inline-cell-editor">
                      <InlineDatePicker
                        v-if="inlineColumnInputType(columnEntry.column.name) !== 'text'"
                        :key="`${columnEntry.column.name}-${activeInlineRowEditor!.datePickerPlacement}`"
                        :model-value="activeInlineRowEditor!.draft[columnEntry.column.name]!.text"
                        :kind="inlineColumnInputType(columnEntry.column.name)"
                        :column-name="columnEntry.column.name"
                        :input-label="isInlineInsertingRow(entry.rowIndex) ? `新增行的 ${columnEntry.column.name}` : `编辑第 ${(displayedResult.rowOffset ?? 0) + entry.rowIndex + 1} 行的 ${columnEntry.column.name}`"
                        :placement="activeInlineRowEditor!.datePickerPlacement"
                        :disabled="busy"
                        :placeholder="inlineCellPlaceholder(columnEntry.column.name)"
                        :is-null="activeInlineRowEditor!.draft[columnEntry.column.name]!.isNull"
                        :use-default="activeInlineRowEditor!.draft[columnEntry.column.name]!.useDefault"
                        @update:model-value="updateInlineCellValue(columnEntry.column.name, $event)"
                        @focus="activateInlineCell(entry.rowIndex, columnEntry.sourceIndex, columnEntry.column.name)"
                        @enter="saveInlineRowEdit"
                        @tab="handleInlineCellTab($event, entry.rowIndex, columnEntry.column.name)"
                        @escape="cancelInlineRowEdit"
                      />
                      <input
                        v-else
                        :value="inlineCellInputValue(columnEntry.column.name)"
                        type="text"
                        class="inline-cell-input"
                        :class="{ null: activeInlineRowEditor!.draft[columnEntry.column.name]!.isNull, default: activeInlineRowEditor!.draft[columnEntry.column.name]!.useDefault }"
                        :data-column="columnEntry.column.name"
                        :aria-label="isInlineInsertingRow(entry.rowIndex) ? `新增行的 ${columnEntry.column.name}` : `编辑第 ${(displayedResult.rowOffset ?? 0) + entry.rowIndex + 1} 行的 ${columnEntry.column.name}`"
                        :disabled="busy"
                        :placeholder="inlineCellPlaceholder(columnEntry.column.name)"
                        autocomplete="off"
                        autocorrect="off"
                        autocapitalize="none"
                        spellcheck="false"
                        data-gramm="false"
                        @click.stop
                        @focus="activateInlineCell(entry.rowIndex, columnEntry.sourceIndex, columnEntry.column.name)"
                        @input="updateInlineCellText(columnEntry.column.name, $event)"
                        @keydown.enter.prevent="saveInlineRowEdit"
                        @keydown.tab="handleInlineCellTab($event, entry.rowIndex, columnEntry.column.name)"
                        @keydown.esc.prevent="cancelInlineRowEdit"
                      />
                      <button v-if="inlineColumnNullable(columnEntry.column.name)" type="button" class="inline-null-button" tabindex="-1" :class="{ active: activeInlineRowEditor!.draft[columnEntry.column.name]!.isNull, 'with-picker': inlineColumnInputType(columnEntry.column.name) !== 'text' }" :disabled="busy" aria-label="切换 NULL" @mousedown.prevent @click.stop="toggleInlineCellNull(columnEntry.column.name)">NULL</button>
                    </div>
                    <template v-else>{{ columnDirectEditActive && batchEditColumn === columnEntry.column.name ? columnBatchEditValue : isInlineEditingRow(entry.rowIndex) && inlineColumnEditable(columnEntry.column.name) ? inlineDraftText(columnEntry.column.name) : cellText(entry.row[columnEntry.sourceIndex]!) }}</template>
                  </td>
                </tr>
                <tr v-if="virtualResultPaddingBottom" class="virtual-spacer-row" aria-hidden="true"><td class="virtual-spacer-cell" :colspan="visibleColumnEntries.length + (activeEditableTable ? 1 : 0)" :style="{ height: `${virtualResultPaddingBottom}px` }"></td></tr>
              </tbody>
            </table>
            <div v-if="!visibleResultRows.length" class="grid-empty-state"><Search :size="18" /><strong>{{ activeWorkspaceTab.resultFilter ? '没有匹配的数据' : '查询结果为空' }}</strong><span>{{ activeWorkspaceTab.resultFilter ? '请调整页内搜索条件' : '查询已成功执行，但没有返回数据行' }}</span></div>
          </div>
          <div v-if="displayedResult && !displayedResult.columns.length" class="result-success-state"><span aria-hidden="true">✓</span><strong>执行成功</strong><p>语句已完成，影响 {{ displayedResult.affectedRows }} 行</p><small>{{ displayedResult.executionTimeMs }} ms</small></div>
          </div>
          <div v-else-if="activeResultView === 'summary'" id="query-summary-panel" class="result-summary-panel" role="tabpanel" aria-labelledby="query-summary-tab">
            <template v-if="displayedResult">
              <div class="result-summary-grid">
                <article><span>状态</span><strong class="success">执行成功</strong></article>
                <article><span>执行耗时</span><strong>{{ displayedResult.executionTimeMs }} ms</strong></article>
                <article><span>结果集</span><strong>{{ allResultSets.length }}</strong></article>
                <article><span>返回行数</span><strong>{{ totalResultRows }}</strong></article>
                <article><span>当前列数</span><strong>{{ displayedResult.columns.length }}</strong></article>
                <article><span>影响行数</span><strong>{{ totalAffectedRows }}</strong></article>
              </div>
              <p v-if="resultHasMore" class="result-summary-note">当前仅显示已加载的数据，查询仍有更多结果。</p>
            </template>
            <div v-else class="result-panel-empty"><strong>暂无执行摘要</strong><span>查询完成后会在这里显示耗时和数据量</span></div>
          </div>
        </section>
      </div>

      <section v-else-if="activeWorkspaceTab?.kind === 'create-table' || activeWorkspaceTab?.kind === 'alter-table'" class="create-table-view">
        <CreateTableEditor
          v-if="activeWorkspaceTab.createTableDefinition"
          :key="activeWorkspaceTab.id"
          :database="activeWorkspaceTab.database || ''"
          :database-kind="activeCreateTableConnection?.driverKind ?? 'mysql'"
          :model-value="activeWorkspaceTab.createTableDefinition"
          :original-definition="activeWorkspaceTab.originalTableDefinition"
          :mode="activeWorkspaceTab.kind === 'alter-table' ? 'alter' : 'create'"
          :busy="busy || creatingTableTabId === activeWorkspaceTab.id"
          :read-only="activeCreateTableConnection?.readOnly ?? false"
          :error="error"
          @update:model-value="updateCreateTableDefinition(activeWorkspaceTab, $event)"
          @create="createTableFromTab(activeWorkspaceTab, $event)"
          @cancel="closeWorkspaceTab(activeWorkspaceTab.id)"
        />
      </section>

      <section v-else-if="activeWorkspaceTab?.kind === 'database-object'" class="database-object-view">
        <DatabaseObjectEditor
          v-if="activeWorkspaceTab.databaseObjectDraft"
          :key="activeWorkspaceTab.id"
          :database="activeWorkspaceTab.database || ''"
          :database-kind="activeQueryConnection?.driverKind ?? activeConnectionKind"
          :model-value="activeWorkspaceTab.databaseObjectDraft"
          :existing="Boolean(activeWorkspaceTab.databaseObjectOriginalName)"
          :busy="busy || savingDatabaseObjectTabId === activeWorkspaceTab.id"
          :error="error"
          @update:model-value="updateDatabaseObjectDraft(activeWorkspaceTab, $event)"
          @open-sql="openObjectSql(activeWorkspaceTab, $event)"
          @save="saveDatabaseObject(activeWorkspaceTab, $event)"
        />
      </section>

      <section v-else-if="activeWorkspaceTab" class="table-data-view">
        <form v-if="displayedResult?.columns.length" class="table-filter-bar" @submit.prevent="applyTableFilter">
          <div class="table-filter-field">
            <Search :size="13" aria-hidden="true" />
            <input v-model="activeWorkspaceTab.filter" class="table-filter-input" aria-label="筛选条件" :disabled="Boolean(executingId) || Boolean(activeInlineRowEditor)" placeholder="输入 WHERE 条件，例如 status = 1" spellcheck="false" />
          </div>
          <button type="submit" class="ghost compact" :disabled="Boolean(executingId) || Boolean(activeInlineRowEditor)">应用</button>
          <button v-if="activeWorkspaceTab.appliedFilter" type="button" class="ghost compact" :disabled="Boolean(executingId) || Boolean(activeInlineRowEditor)" @click="clearTableFilter">清除</button>
          <span v-if="activeWorkspaceTab.sortColumn" class="sort-status">排序：{{ activeWorkspaceTab.sortColumn }} {{ activeWorkspaceTab.sortDirection === 'desc' ? '↓' : '↑' }}</span>
        </form>
        <input
          v-if="activeBatchEditColumn"
          v-model="columnBatchEditValue"
          class="column-direct-edit-input"
          type="text"
          tabindex="-1"
          :aria-label="`直接编辑 ${activeBatchEditColumn.name} 整列，Enter 保存，Escape 取消`"
          :disabled="busy"
          autocomplete="off"
          autocorrect="off"
          autocapitalize="none"
          spellcheck="false"
          @input="handleDirectColumnEditInput"
          @keydown.enter.prevent="submitColumnBatchEdit"
          @keydown.esc.stop.prevent="cancelDirectColumnEdit"
        />
        <div v-if="error" class="error-banner dismissible-error-banner"><span>{{ error }}</span><button type="button" aria-label="关闭错误提示" @click="error = null"><X :size="13" /></button></div>
        <div v-if="executingId && !displayedResult" class="empty-result"><RefreshCw :size="22" class="loading-icon" /><p>正在加载数据…</p></div>
        <div v-else-if="!displayedResult" class="empty-result"><Table2 :size="24" /><p>暂无数据</p></div>
        <div ref="gridScroll" v-else class="grid-scroll table-grid-scroll" @scroll.passive="handleGridScroll">
          <table
            class="data-grid"
            role="grid"
            :aria-label="`${activeWorkspaceTab.title} 数据`"
            :aria-rowcount="displayedResult.rows.length + (activeInlineRowEditor?.mode === 'insert' ? 1 : 0)"
            :aria-colcount="visibleColumnEntries.length"
            :style="{ width: `${gridWidth(visibleColumnEntries)}px` }"
          >
            <colgroup><col class="row-number-column"><col class="selection-column"><col v-for="entry in visibleColumnEntries" :key="columnKey(entry.column, entry.sourceIndex)" class="data-column" :style="{ width: `${columnWidth(entry.column, entry.sourceIndex)}px` }"></colgroup>
            <thead><tr>
              <th class="row-number">#</th>
              <th class="row-selection"><input type="checkbox" aria-label="选择当前页全部行" :aria-checked="selectedVisibleRowCount > 0 && selectedVisibleRowCount < visibleResultRows.length ? 'mixed' : selectedVisibleRowCount === visibleResultRows.length && visibleResultRows.length > 0" :checked="visibleResultRows.length > 0 && selectedVisibleRowCount === visibleResultRows.length" :indeterminate="selectedVisibleRowCount > 0 && selectedVisibleRowCount < visibleResultRows.length" :disabled="Boolean(executingId) || Boolean(activeInlineRowEditor)" @change="toggleAllVisibleRows(($event.currentTarget as HTMLInputElement).checked)" /></th>
              <th
                v-for="(entry, displayIndex) in visibleColumnEntries"
                :key="columnKey(entry.column, entry.sourceIndex)"
                class="data-column selectable-column"
                tabindex="0"
                :aria-sort="activeWorkspaceTab.sortColumn === entry.column.name ? (activeWorkspaceTab.sortDirection === 'desc' ? 'descending' : 'ascending') : 'none'"
                :class="{ sorted: activeWorkspaceTab.sortColumn === entry.column.name, 'selected-column': batchEditColumn === entry.column.name, 'menu-open': columnMenuColumn === entry.column.name, frozen: displayIndex < (activeWorkspaceTab.frozenColumnCount ?? 0) }"
                :style="frozenColumnStyle(displayIndex, entry.column, entry.sourceIndex)"
                @click="selectColumnBatchEdit(entry.column.name)"
                @keydown.enter.self.prevent="selectColumnBatchEdit(entry.column.name)"
                @keydown.space.self.prevent="selectColumnBatchEdit(entry.column.name)"
              >
                <span class="column-name">{{ entry.column.name }}<b v-if="activeWorkspaceTab.sortColumn === entry.column.name">{{ activeWorkspaceTab.sortDirection === 'desc' ? ' ↓' : ' ↑' }}</b></span>
                <small>{{ entry.column.databaseType }}{{ entry.column.unsigned ? ' unsigned' : '' }}</small>
                <div class="column-header-menu" @click.stop @keydown.esc.stop.prevent="closeColumnMenu">
                  <button
                    type="button"
                    class="column-menu-button"
                    :aria-label="`${entry.column.name} 列操作`"
                    aria-haspopup="menu"
                    :aria-expanded="columnMenuColumn === entry.column.name"
                    :disabled="Boolean(executingId) || Boolean(activeInlineRowEditor)"
                    @click="toggleColumnMenu(entry.column.name)"
                  ><MoreHorizontal :size="16" /></button>
                  <div v-if="columnMenuColumn === entry.column.name" class="column-menu-panel" role="menu" :aria-label="`${entry.column.name} 列操作`">
                    <button type="button" role="menuitem" :class="{ active: activeWorkspaceTab.sortColumn === entry.column.name && activeWorkspaceTab.sortDirection === 'asc' }" @click="applyTableSort(entry.column.name, 'asc')"><ArrowUpNarrowWide :size="15" />升序排序</button>
                    <button type="button" role="menuitem" :class="{ active: activeWorkspaceTab.sortColumn === entry.column.name && activeWorkspaceTab.sortDirection === 'desc' }" @click="applyTableSort(entry.column.name, 'desc')"><ArrowDownWideNarrow :size="15" />降序排序</button>
                    <button type="button" role="menuitem" class="remove-sort" :disabled="!activeWorkspaceTab.sortColumn" @click="clearTableSort"><CircleMinus :size="15" />移除所有排序</button>
                    <span class="column-menu-divider" aria-hidden="true" />
                    <button type="button" role="menuitem" @click="addColumnFilter(entry.column.name)"><ListFilter :size="15" />添加筛选</button>
                  </div>
                </div>
                <span class="column-resizer" role="separator" aria-orientation="vertical" :aria-label="`调整 ${entry.column.name} 列宽`" :aria-valuenow="columnWidth(entry.column, entry.sourceIndex)" :aria-valuemin="MIN_DATA_COLUMN_WIDTH" :aria-valuemax="MAX_DATA_COLUMN_WIDTH" tabindex="0" @click.stop @pointerdown="startColumnResize($event, entry.column, entry.sourceIndex)" @pointermove="resizeColumn" @pointerup="finishColumnResize" @pointercancel="finishColumnResize" @dblclick.stop="resetColumnWidth(entry.column, entry.sourceIndex)" @keydown="resizeColumnWithKeyboard($event, entry.column, entry.sourceIndex)" />
              </th>
            </tr></thead>
          <tbody>
            <tr v-if="virtualResultPaddingTop" class="virtual-spacer-row" aria-hidden="true"><td class="virtual-spacer-cell" :colspan="visibleColumnEntries.length + 2" :style="{ height: `${virtualResultPaddingTop}px` }"></td></tr>
            <tr v-for="entry in renderedResultRows" :key="entry.rowIndex" class="data-row" :aria-rowindex="entry.rowIndex + 2" :class="{ selected: activeWorkspaceTab.selectedRowIndex === entry.rowIndex, 'alternate-row': entry.visibleIndex % 2 === 1, 'inline-editing-row': isInlineEditingRow(entry.rowIndex), 'inline-insert-row': isInlineInsertingRow(entry.rowIndex) }" @click="selectTableDataRow(entry.rowIndex)" @focusout="handleInlineRowFocusOut($event, entry.rowIndex)">
              <td class="row-number">{{ isInlineInsertingRow(entry.rowIndex) ? '新增' : (displayedResult.rowOffset ?? 0) + entry.rowIndex + 1 }}</td>
              <td class="row-selection" @click.stop><input type="checkbox" :aria-label="`选择第 ${(displayedResult.rowOffset ?? 0) + entry.rowIndex + 1} 行`" :checked="selectedRowIndexes.includes(entry.rowIndex)" :disabled="Boolean(executingId) || Boolean(activeInlineRowEditor)" @change="toggleDataRow(entry.rowIndex, ($event.currentTarget as HTMLInputElement).checked)" /></td>
              <td
                v-for="(columnEntry, displayIndex) in visibleColumnEntries"
                :key="columnEntry.sourceIndex"
                role="gridcell"
                :data-grid-row="entry.rowIndex"
                :data-grid-column="columnEntry.sourceIndex"
                :tabindex="gridCellTabIndex(entry.rowIndex, columnEntry.sourceIndex)"
                :aria-selected="activeWorkspaceTab.selectedCell?.row === entry.rowIndex && activeWorkspaceTab.selectedCell?.column === columnEntry.sourceIndex"
                :class="{
                  null: columnDirectEditActive && batchEditColumn === columnEntry.column.name
                    ? false
                    : isInlineEditingRow(entry.rowIndex) && inlineColumnEditable(columnEntry.column.name)
                      ? activeInlineRowEditor?.draft[columnEntry.column.name]?.isNull
                      : entry.row[columnEntry.sourceIndex]?.kind === 'null',
                  'selected-cell': activeWorkspaceTab.selectedCell?.row === entry.rowIndex && activeWorkspaceTab.selectedCell?.column === columnEntry.sourceIndex,
                  'selected-column': batchEditColumn === columnEntry.column.name,
                  'column-direct-edit-preview': columnDirectEditActive && batchEditColumn === columnEntry.column.name,
                  'inline-editing-cell': isInlineEditingRow(entry.rowIndex) && inlineColumnEditable(columnEntry.column.name) && activeInlineRowEditor?.activeColumn === columnEntry.column.name,
                  frozen: displayIndex < (activeWorkspaceTab.frozenColumnCount ?? 0),
                }"
                :style="frozenColumnStyle(displayIndex, columnEntry.column, columnEntry.sourceIndex)"
                @focus="selectDataCell(entry.rowIndex, columnEntry.sourceIndex)"
                @click.stop="handleTableDataCellClick(entry.rowIndex, columnEntry.sourceIndex, columnEntry.column.name)"
                @keydown="handleGridCellKeydown($event, entry.rowIndex, columnEntry.sourceIndex, columnEntry.column.name)"
              >
                <div v-if="isInlineEditingRow(entry.rowIndex) && inlineColumnEditable(columnEntry.column.name) && activeInlineRowEditor?.activeColumn === columnEntry.column.name" class="inline-cell-editor">
                  <InlineDatePicker
                    v-if="inlineColumnInputType(columnEntry.column.name) !== 'text'"
                    :key="`${columnEntry.column.name}-${activeInlineRowEditor!.datePickerPlacement}`"
                    :model-value="activeInlineRowEditor!.draft[columnEntry.column.name]!.text"
                    :kind="inlineColumnInputType(columnEntry.column.name)"
                    :column-name="columnEntry.column.name"
                    :input-label="isInlineInsertingRow(entry.rowIndex) ? `新增行的 ${columnEntry.column.name}` : `编辑第 ${(displayedResult.rowOffset ?? 0) + entry.rowIndex + 1} 行的 ${columnEntry.column.name}`"
                    :placement="activeInlineRowEditor!.datePickerPlacement"
                    :disabled="busy"
                    :placeholder="inlineCellPlaceholder(columnEntry.column.name)"
                    :is-null="activeInlineRowEditor!.draft[columnEntry.column.name]!.isNull"
                    :use-default="activeInlineRowEditor!.draft[columnEntry.column.name]!.useDefault"
                    @update:model-value="updateInlineCellValue(columnEntry.column.name, $event)"
                    @focus="activateInlineCell(entry.rowIndex, columnEntry.sourceIndex, columnEntry.column.name)"
                    @enter="saveInlineRowEdit"
                    @tab="handleInlineCellTab($event, entry.rowIndex, columnEntry.column.name)"
                    @escape="cancelInlineRowEdit"
                  />
                  <input
                    v-else
                    :value="inlineCellInputValue(columnEntry.column.name)"
                    type="text"
                    class="inline-cell-input"
                    :class="{ null: activeInlineRowEditor!.draft[columnEntry.column.name]!.isNull, default: activeInlineRowEditor!.draft[columnEntry.column.name]!.useDefault }"
                    :data-column="columnEntry.column.name"
                    :aria-label="isInlineInsertingRow(entry.rowIndex) ? `新增行的 ${columnEntry.column.name}` : `编辑第 ${(displayedResult.rowOffset ?? 0) + entry.rowIndex + 1} 行的 ${columnEntry.column.name}`"
                    :disabled="busy"
                    :placeholder="inlineCellPlaceholder(columnEntry.column.name)"
                    autocomplete="off"
                    autocorrect="off"
                    autocapitalize="none"
                    spellcheck="false"
                    data-gramm="false"
                    @click.stop
                    @focus="activateInlineCell(entry.rowIndex, columnEntry.sourceIndex, columnEntry.column.name)"
                    @input="updateInlineCellText(columnEntry.column.name, $event)"
                    @keydown.enter.prevent="saveInlineRowEdit"
                    @keydown.tab="handleInlineCellTab($event, entry.rowIndex, columnEntry.column.name)"
                    @keydown.esc.prevent="cancelInlineRowEdit"
                  />
                  <button v-if="inlineColumnNullable(columnEntry.column.name)" type="button" class="inline-null-button" tabindex="-1" :class="{ active: activeInlineRowEditor!.draft[columnEntry.column.name]!.isNull, 'with-picker': inlineColumnInputType(columnEntry.column.name) !== 'text' }" :disabled="busy" aria-label="切换 NULL" @mousedown.prevent @click.stop="toggleInlineCellNull(columnEntry.column.name)">NULL</button>
                </div>
                <template v-else>{{ columnDirectEditActive && batchEditColumn === columnEntry.column.name ? columnBatchEditValue : isInlineEditingRow(entry.rowIndex) && inlineColumnEditable(columnEntry.column.name) ? inlineDraftText(columnEntry.column.name) : cellText(entry.row[columnEntry.sourceIndex]!) }}</template>
              </td>
            </tr>
            <tr v-if="virtualResultPaddingBottom" class="virtual-spacer-row" aria-hidden="true"><td class="virtual-spacer-cell" :colspan="visibleColumnEntries.length + 2" :style="{ height: `${virtualResultPaddingBottom}px` }"></td></tr>
          </tbody></table>
          <div v-if="!visibleResultRows.length" class="grid-empty-state"><Search :size="18" /><strong>{{ activeWorkspaceTab.resultFilter ? '没有匹配的数据' : '当前数据页为空' }}</strong><span>{{ activeWorkspaceTab.resultFilter ? '请调整页内搜索条件' : '可以调整筛选条件或切换分页后重试' }}</span></div>
        </div>
        <div v-if="displayedResult?.columns.length" class="card-toolbar table-toolbar">
          <div class="table-toolbar-heading"><strong>{{ activeWorkspaceTab.title }}</strong><RefreshCw v-if="executingId" :size="13" class="loading-icon table-loading-indicator" aria-label="正在加载数据" /><span class="muted">{{ visibleResultRows.length - (activeInlineRowEditor?.mode === 'insert' ? 1 : 0) }} / {{ displayedResult.rows.length }} 行</span><span v-if="selectedRowIndexes.length" class="connection-badge">已选 {{ selectedRowIndexes.length }}</span><span v-if="activeTransaction" class="connection-badge transaction">事务中</span></div>
          <div class="toolbar-actions table-toolbar-actions">
            <input v-model="activeWorkspaceTab.resultFilter" class="result-search" type="search" aria-label="在当前数据页中搜索" placeholder="页内搜索" />
            <div class="table-primary-actions">
              <button class="ghost compact" :disabled="busy || Boolean(executingId) || activeQueryConnection?.readOnly || Boolean(activeInlineRowEditor)" @click="startInlineRowInsert"><Plus :size="13" /> 新增</button>
              <button class="ghost compact" :disabled="busy || Boolean(executingId) || Boolean(activeInlineRowEditor) || activeWorkspaceTab.selectedRowIndex == null || activeQueryConnection?.readOnly || !activeTableHasUniqueKey" @click="startInlineRowEdit()"><Pencil :size="13" /> 编辑</button>
              <button class="danger compact" :disabled="busy || Boolean(executingId) || activeWorkspaceTab.selectedRowIndex == null || activeQueryConnection?.readOnly || Boolean(activeInlineRowEditor)" @click="deleteSelectedRow"><Trash2 :size="13" /> 删除</button>
            </div>
            <div ref="tableActionsMenu" class="table-actions-menu">
              <button type="button" class="ghost compact table-actions-trigger" aria-haspopup="menu" aria-controls="table-actions-panel" :aria-expanded="tableActionsOpen" :disabled="busy || Boolean(executingId) || Boolean(activeInlineRowEditor)" @click="tableActionsOpen = !tableActionsOpen">更多 <ChevronDown :size="13" aria-hidden="true" /></button>
              <Transition name="table-actions-panel">
                <div id="table-actions-panel" v-show="tableActionsOpen" class="table-actions-panel" role="menu" aria-label="更多表格操作" @click="handleTableActionsMenuClick">
                  <span class="table-actions-label" aria-hidden="true">查看</span>
                  <button type="button" role="menuitem" @click="showColumnManager = true"><Columns3 :size="15" aria-hidden="true" />列设置</button>
                  <button type="button" role="menuitem" @click="showResultInsights = true"><ArrowUpDown :size="15" aria-hidden="true" />分析结果</button>
                  <span class="table-actions-label" aria-hidden="true">行操作</span>
                  <button type="button" role="menuitem" :disabled="!selectedRowIndexes.length || activeQueryConnection?.readOnly" @click="openSelectedRowsBatchEdit"><Pencil :size="15" aria-hidden="true" />批量修改</button>
                  <button type="button" role="menuitem" class="destructive" :disabled="!selectedRowIndexes.length || activeQueryConnection?.readOnly" @click="deleteSelectedRows"><Trash2 :size="15" aria-hidden="true" />批量删除</button>
                  <button type="button" role="menuitem" :disabled="!selectedForeignKey" @click="openSelectedForeignKey"><ExternalLink :size="15" aria-hidden="true" />外键跳转</button>
                  <button type="button" role="menuitem" :disabled="activeWorkspaceTab.selectedRowIndex == null" @click="copyResult('row')"><Copy :size="15" aria-hidden="true" />复制当前行</button>
                  <button type="button" role="menuitem" @click="copyResult('page')"><Clipboard :size="15" aria-hidden="true" />复制当前页</button>
                  <span class="table-actions-label" aria-hidden="true">数据</span>
                  <template v-if="activeTransaction">
                    <button type="button" role="menuitem" @click="commitTransaction"><CircleCheck :size="15" aria-hidden="true" />提交事务</button>
                    <button type="button" role="menuitem" class="destructive" @click="rollbackTransaction"><RotateCcw :size="15" aria-hidden="true" />回滚事务</button>
                  </template>
                  <button v-else type="button" role="menuitem" :disabled="activeQueryConnection?.readOnly" @click="beginTransaction"><Play :size="15" aria-hidden="true" />开始事务</button>
                  <button type="button" role="menuitem" :disabled="activeQueryConnection?.readOnly" @click="importIntoCurrentTable"><FileUp :size="15" aria-hidden="true" />导入数据</button>
                  <span class="table-actions-label" aria-hidden="true">导出</span>
                  <TableExportControl v-model="exportFormat" :options="EXPORT_FORMATS" :disabled="busy" full-label="整表" @export-page="exportCurrentPage" @export-full="exportFullTable" />
                </div>
              </Transition>
            </div>
            <div class="result-pagination"><button class="ghost compact icon-only" aria-label="上一页" :disabled="Boolean(executingId) || Boolean(activeInlineRowEditor) || (displayedResult.rowOffset ?? 0) === 0" @click="changeResultPage(-1)"><ChevronLeft :size="13" /></button><span>{{ Math.floor((displayedResult.rowOffset ?? 0) / (displayedResult.pageSize ?? 100)) + 1 }}</span><button class="ghost compact icon-only" aria-label="下一页" :disabled="Boolean(executingId) || Boolean(activeInlineRowEditor) || !displayedResult.hasMore" @click="changeResultPage(1)"><ChevronRight :size="13" /></button></div>
          </div>
        </div>
      </section>
      </template>
    </main>

    <ConnectionDialog v-if="showDialog" :initial="editing" @close="showDialog = false; editing = null" @save="saveConnection" />

    <QueryExportDialog v-if="showQueryExportDialog && displayedResult?.columns.length" v-model="exportFormat" :options="EXPORT_FORMATS" :busy="busy" :full-disabled="!activeWorkspaceTab?.pagingSql" @close="showQueryExportDialog = false" @export-page="showQueryExportDialog = false; exportCurrentPage()" @export-full="showQueryExportDialog = false; exportFullQuery()" />

    <SettingsDialog v-if="showSettings" :initial="settings" :version="appVersion" :checking-update="updateCheckPending" @close="showSettings = false" @save="saveSettings" @diagnostics="showDiagnostics = true" @check-update="checkForUpdates()" />
    <DiagnosticsDialog v-if="showDiagnostics" @close="showDiagnostics = false" />

    <ServerAdminPanel v-if="showServerAdmin && activeConnectionId" :connection-id="activeConnectionId" :database-kind="activeConnectionKind" @close="showServerAdmin = false" @open-sql="openAdminSql" />

    <TransferCenter
      v-if="showTransferCenter"
      :tasks="transferTasks"
      :connections="connections.filter((connection) => Boolean(connectionInfo[connection.id]))"
      :schedule="backupSchedule"
      @close="showTransferCenter = false"
      @cancel="cancelTransferTask"
      @clear="clearTransferTasks"
      @save-schedule="saveBackupSchedule"
      @run-now="runBackupScheduleNow"
    />

    <SchemaCompareDialog
      v-if="compareDatabase && activeConnectionId"
      :connections="connections.filter((connection) => Boolean(connectionInfo[connection.id]))"
      :initial-connection-id="activeConnectionId"
      :initial-database="compareDatabase"
      @close="compareDatabase = null"
      @open-sql="openAdminSql($event); compareDatabase = null"
    />

    <CellViewer v-if="cellViewer" :column="cellViewer.column" :value="cellViewer.value" @close="cellViewer = null" />

    <ImportDataDialog
      v-if="showImportDialog && activeWorkspaceTab?.kind === 'table' && activeWorkspaceTab.connectionId && activeWorkspaceTab.database && activeWorkspaceTab.tableDetail"
      :connection-id="activeWorkspaceTab.connectionId"
      :database="activeWorkspaceTab.database"
      :table="activeWorkspaceTab.title"
      :target-columns="activeWorkspaceTab.tableDetail.columns.map((column) => column.name)"
      @close="showImportDialog = false"
      @imported="finishTableImport"
    />

    <SqlParameterDialog v-if="parameterSql" :sql="parameterSql" @close="parameterSql = null" @execute="executeParameterizedSql" />

    <SnippetDialog v-if="showSnippetDialog" :snippets="snippets" :current-sql="activeWorkspaceTab?.sql ?? ''" @close="showSnippetDialog = false" @open="openSnippet" @save="saveSnippet" @remove="removeSnippet" />

    <ResultInsightsDialog v-if="showResultInsights && displayedResult?.columns.length" :columns="displayedResult.columns" :rows="visibleResultRows.map((entry) => entry.row)" @close="showResultInsights = false" />

    <BatchEditDialog v-if="showBatchEdit && batchEditableColumns.length" :columns="batchEditableColumns" :selected-count="batchEditRowIndexes.length" @close="closeBatchEditDialog" @apply="applyBatchEdit" />

    <ColumnManagerDialog v-if="showColumnManager && displayedResult?.columns.length" :columns="displayedResult.columns" :order="activeWorkspaceTab?.columnOrder ?? []" :hidden="activeWorkspaceTab?.hiddenColumns ?? []" :frozen-count="activeWorkspaceTab?.frozenColumnCount ?? 0" @close="showColumnManager = false" @apply="applyColumnConfiguration" />

    <ContextMenu v-if="contextMenu" :x="contextMenu.x" :y="contextMenu.y" @close="closeContextMenu">
      <template v-if="contextMenu.target.kind === 'connection'">
        <div class="context-menu-title">{{ contextMenu.target.connection.name }}</div>
        <button role="menuitem" @click="runConnectionContextAction('toggle')"><Database :size="14" />{{ expandedConnectionId === contextMenu.target.connection.id ? '收起连接' : '连接并展开' }}</button>
        <button v-if="contextMenu.target.connection.driverKind === 'redis'" role="menuitem" @click="runConnectionContextAction('redis')"><KeyRound :size="14" />打开 Redis 管理器</button>
        <button role="menuitem" @click="runConnectionContextAction('edit')"><Pencil :size="14" />编辑连接</button>
        <button v-if="connectionInfo[contextMenu.target.connection.id]" role="menuitem" @click="runConnectionContextAction('disconnect')"><Unplug :size="14" />断开连接</button>
        <div class="context-menu-separator" />
        <button class="destructive" role="menuitem" @click="runConnectionContextAction('remove')"><Trash2 :size="14" />删除连接</button>
      </template>

      <template v-else-if="contextMenu.target.kind === 'database'">
        <div class="context-menu-title">{{ contextMenu.target.database }}</div>
        <button role="menuitem" @click="runDatabaseContextAction('open')"><Database :size="14" />打开数据库</button>
        <button role="menuitem" @click="runDatabaseContextAction('refresh')"><RefreshCw :size="14" />刷新对象</button>
        <button role="menuitem" @click="runDatabaseContextAction('backup')"><Download :size="14" />备份数据库</button>
        <button role="menuitem" @click="runDatabaseContextAction('restore')"><FileUp :size="14" />恢复 SQL 备份</button>
        <button role="menuitem" @click="runDatabaseContextAction('compare')"><Braces :size="14" />结构对比</button>
        <div class="context-menu-separator" />
        <button role="menuitem" @click="runDatabaseContextAction('copy')"><Clipboard :size="14" />复制名称</button>
        <div class="context-menu-separator" />
        <button class="destructive" role="menuitem" @click="runDatabaseContextAction('drop')"><Trash2 :size="14" />删除数据库</button>
      </template>

      <template v-else-if="contextMenu.target.kind === 'table-group'">
        <div class="context-menu-title">{{ contextMenu.target.database }} · 表</div>
        <button role="menuitem" @click="runTableGroupContextAction('create')"><Plus :size="14" />新建表</button>
        <button role="menuitem" @click="runTableGroupContextAction('view')"><Eye :size="14" />新建视图</button>
        <button role="menuitem" @click="runTableGroupContextAction('query')"><FileCode2 :size="14" />新建查询</button>
        <div class="context-menu-separator" />
        <button role="menuitem" @click="runTableGroupContextAction('toggle')"><ChevronDown v-if="expandedTableGroup" :size="14" /><ChevronRight v-else :size="14" />{{ expandedTableGroup ? '收起表分组' : '展开表分组' }}</button>
        <button role="menuitem" @click="runTableGroupContextAction('refresh')"><RefreshCw :size="14" />刷新表列表</button>
      </template>

      <template v-else-if="contextMenu.target.kind === 'object-group'">
        <div class="context-menu-title">{{ contextMenu.target.database }} · {{ objectGroupLabel(contextMenu.target.group) }}</div>
        <button v-if="contextMenu.target.group === 'view'" role="menuitem" @click="runObjectGroupContextAction('create-view')"><Plus :size="14" />新建视图</button>
        <template v-else-if="contextMenu.target.group === 'routine'">
          <button role="menuitem" :disabled="activeConnectionKind === 'sqlite'" @click="runObjectGroupContextAction('create-function')"><Plus :size="14" />新建函数</button>
          <button role="menuitem" :disabled="activeConnectionKind === 'sqlite'" @click="runObjectGroupContextAction('create-procedure')"><Plus :size="14" />新建存储过程</button>
        </template>
        <button v-else-if="contextMenu.target.group === 'trigger'" role="menuitem" @click="runObjectGroupContextAction('create-trigger')"><Plus :size="14" />新建触发器</button>
        <button v-else role="menuitem" :disabled="activeConnectionKind !== 'mysql' && activeConnectionKind !== 'mariadb'" @click="runObjectGroupContextAction('create-event')"><Plus :size="14" />新建事件</button>
        <button role="menuitem" @click="runObjectGroupContextAction('new-query')"><FileCode2 :size="14" />新建查询</button>
        <div class="context-menu-separator" />
        <button role="menuitem" @click="runObjectGroupContextAction('toggle')"><ChevronDown v-if="objectGroupExpanded(contextMenu.target.group)" :size="14" /><ChevronRight v-else :size="14" />{{ objectGroupExpanded(contextMenu.target.group) ? `收起${objectGroupLabel(contextMenu.target.group)}` : `展开${objectGroupLabel(contextMenu.target.group)}` }}</button>
        <button role="menuitem" @click="runObjectGroupContextAction('refresh')"><RefreshCw :size="14" />刷新{{ objectGroupLabel(contextMenu.target.group) }}列表</button>
      </template>

      <template v-else-if="contextMenu.target.kind === 'object'">
        <div class="context-menu-title">{{ contextMenu.target.label }} · {{ contextMenu.target.name }}</div>
        <button role="menuitem" @click="runObjectContextAction('open')"><FileCode2 :size="14" />查看定义</button>
        <button v-if="contextMenu.target.objectKind === 'procedure' || contextMenu.target.objectKind === 'function'" role="menuitem" @click="runObjectContextAction('invoke')"><Play :size="14" />生成调用语句</button>
        <button v-if="contextMenu.target.objectKind === 'event'" role="menuitem" @click="runObjectContextAction('toggle')"><Play :size="14" />{{ contextMenu.target.status === 'ENABLED' ? '停用事件' : '启用事件' }}</button>
        <button role="menuitem" @click="runObjectContextAction('copy')"><Clipboard :size="14" />复制名称</button>
        <button role="menuitem" @click="runObjectContextAction('copy-qualified')"><Clipboard :size="14" />复制完整名称</button>
        <div class="context-menu-separator" />
        <button class="destructive" role="menuitem" @click="runObjectContextAction('drop')"><Trash2 :size="14" />删除{{ contextMenu.target.label }}</button>
      </template>

      <template v-else>
        <div class="context-menu-title">{{ contextMenu.target.table.name }}</div>
        <button role="menuitem" @click="runTableContextAction('preview')"><Play :size="14" />预览前 100 行</button>
        <button role="menuitem" @click="runTableContextAction('generate')"><FileCode2 :size="14" />生成 SELECT</button>
        <button role="menuitem" @click="runTableContextAction('design')"><Pencil :size="14" />{{ contextMenu.target.table.tableType.includes('VIEW') ? '查看视图定义' : '设计表结构' }}</button>
        <div class="context-menu-separator" />
        <button role="menuitem" @click="runTableContextAction('copy')"><Clipboard :size="14" />复制名称</button>
        <button role="menuitem" @click="runTableContextAction('copy-qualified')"><Clipboard :size="14" />复制完整名称</button>
        <div class="context-menu-separator" />
        <button v-if="!contextMenu.target.table.tableType.includes('VIEW')" class="destructive" role="menuitem" @click="runTableContextAction('truncate')"><Trash2 :size="14" />清空表</button>
        <button class="destructive" role="menuitem" @click="runTableContextAction('drop')"><Trash2 :size="14" />删除{{ contextMenu.target.table.tableType.includes('VIEW') ? '视图' : '表' }}</button>
      </template>
    </ContextMenu>

    <ActionDialog v-if="actionDialog" :key="actionDialog.id" :state="actionDialog" @confirm="acceptActionDialog" @cancel="cancelActionDialog" />
  </div>
</template>
