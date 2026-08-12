import { computed, markRaw, ref } from "vue";
import { defineStore } from "pinia";
import { api } from "@/lib/api";
import type {
  CommandError, ConnectionInfo, ConnectionProfile, DatabaseInfo, EventInfo,
  QueryResultPage, RoutineInfo, RowMutationRequest, TableDetail, TableInfo, TriggerInfo, UUID,
} from "@/types";

function messageOf(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) return String((error as CommandError).message);
  return "发生未知错误";
}

function rowMutationError(request: RowMutationRequest, message: string) {
  const action = request.kind === "insert" ? "新增" : request.kind === "delete" ? "删除" : "更新";
  const detail = message.replace(/^查询失败\s*[：:]\s*/u, "");
  return detail.startsWith(`${action}失败`) ? detail : `${action}失败：${detail}`;
}

function keepResultRowsRaw(page: QueryResultPage) {
  markRaw(page.rows);
  for (const resultSet of page.additionalResultSets ?? []) markRaw(resultSet.rows);
  return page;
}

export const useAppStore = defineStore("app", () => {
  const connections = ref<ConnectionProfile[]>([]);
  const connectionInfo = ref<Record<UUID, ConnectionInfo>>({});
  const activeConnectionId = ref<UUID | null>(null);
  const databases = ref<DatabaseInfo[]>([]);
  const selectedDatabase = ref<string | null>(null);
  const tables = ref<TableInfo[]>([]);
  const tableHasMore = ref(false);
  const selectedTable = ref<TableInfo | null>(null);
  const tableDetail = ref<TableDetail | null>(null);
  const routines = ref<RoutineInfo[]>([]);
  const triggers = ref<TriggerInfo[]>([]);
  const events = ref<EventInfo[]>([]);
  const transactionSessions = ref<Record<UUID, boolean>>({});
  const result = ref<QueryResultPage | null>(null);
  const executingId = ref<UUID | null>(null);
  const executingConnectionId = ref<UUID | null>(null);
  const executingSessionId = ref<UUID | null>(null);
  const busy = ref(false);
  const error = ref<string | null>(null);
  const tabSessionConnections = new Map<UUID, UUID>();
  let connectionRequestSequence = 0;
  let tableRequestSequence = 0;
  let currentTableFilter = "";

  const activeConnection = computed(() => connections.value.find((item) => item.id === activeConnectionId.value) ?? null);
  const connected = computed(() => activeConnectionId.value != null && connectionInfo.value[activeConnectionId.value] != null);

  async function run<T>(operation: () => Promise<T>): Promise<T | undefined> {
    busy.value = true;
    error.value = null;
    try { return await operation(); }
    catch (cause) { error.value = messageOf(cause); }
    finally { busy.value = false; }
  }

  async function loadConnections() {
    const value = await run(api.listConnections);
    if (value) connections.value = value;
  }

  async function saveConnection(profile: ConnectionProfile, password?: string) {
    const value = await run(() => api.saveConnection(profile, password));
    if (!value) return false;
    const index = connections.value.findIndex((item) => item.id === value.profile.id);
    if (index >= 0) connections.value[index] = value.profile;
    else connections.value.push(value.profile);
    connections.value.sort((a, b) => a.name.localeCompare(b.name));
    if (!value.secretPersisted) error.value = "系统凭据库不可用，密码仅在本次运行期间有效。";
    return true;
  }

  async function removeConnection(connectionId: UUID) {
    const done = await run(() => api.deleteConnection(connectionId));
    if (done === undefined && error.value) return;
    forgetTabSessions(connectionId);
    connections.value = connections.value.filter((item) => item.id !== connectionId);
    delete connectionInfo.value[connectionId];
    if (activeConnectionId.value === connectionId) resetWorkspace();
  }

  async function connect(connectionId: UUID) {
    const requestSequence = ++connectionRequestSequence;
    tableRequestSequence++;
    databases.value = [];
    selectedDatabase.value = null;
    tables.value = [];
    tableHasMore.value = false;
    selectedTable.value = null;
    tableDetail.value = null;
    routines.value = [];
    triggers.value = [];
    events.value = [];
    const info = await run(() => api.connect(connectionId));
    if (!info) return;
    connectionInfo.value[connectionId] = info;
    if (requestSequence !== connectionRequestSequence) return;
    activeConnectionId.value = connectionId;
    const databasePage = await run(() => api.listDatabases(connectionId));
    if (requestSequence !== connectionRequestSequence) return;
    databases.value = databasePage ?? [];
  }

  async function disconnect(connectionId: UUID) {
    await run(() => api.disconnect(connectionId));
    if (!error.value) forgetTabSessions(connectionId);
    delete connectionInfo.value[connectionId];
    if (activeConnectionId.value === connectionId) resetWorkspace();
  }

  async function selectDatabase(database: string) {
    if (!activeConnectionId.value) return;
    selectedDatabase.value = database;
    selectedTable.value = null;
    tableDetail.value = null;
    routines.value = [];
    triggers.value = [];
    events.value = [];
    tables.value = [];
    tableHasMore.value = false;
    await Promise.all([loadTables("", false), loadDatabaseObjects(database)]);
  }

  function closeDatabase() {
    selectedDatabase.value = null;
    tables.value = [];
    tableHasMore.value = false;
    selectedTable.value = null;
    tableDetail.value = null;
    routines.value = [];
    triggers.value = [];
    events.value = [];
  }

  async function loadDatabases() {
    if (!activeConnectionId.value) return;
    const connectionId = activeConnectionId.value;
    const requestSequence = connectionRequestSequence;
    const page = await run(() => api.listDatabases(connectionId));
    if (page && requestSequence === connectionRequestSequence
      && activeConnectionId.value === connectionId) databases.value = page;
  }

  async function loadTables(filter = "", append = false) {
    if (!activeConnectionId.value || !selectedDatabase.value) return;
    const connectionId = activeConnectionId.value;
    const database = selectedDatabase.value;
    const requestSequence = ++tableRequestSequence;
    if (!append) currentTableFilter = filter;
    const effectiveFilter = append ? currentTableFilter : filter;
    const offset = append ? tables.value.length : 0;
    const page = await run(() => api.listTables(connectionId, database, effectiveFilter, 500, offset));
    if (!page || requestSequence !== tableRequestSequence
      || activeConnectionId.value !== connectionId || selectedDatabase.value !== database) return;
    tables.value = append ? [...tables.value, ...page] : page;
    tableHasMore.value = page.length === 500;
  }

  async function loadDatabaseObjects(database: string) {
    if (!activeConnectionId.value) return;
    const connectionId = activeConnectionId.value;
    try {
      const [routinePage, triggerPage, eventPage] = await Promise.all([
        api.listRoutines(connectionId, database),
        api.listTriggers(connectionId, database),
        api.listEvents(connectionId, database),
      ]);
      if (activeConnectionId.value !== connectionId || selectedDatabase.value !== database) return;
      routines.value = routinePage;
      triggers.value = triggerPage;
      events.value = eventPage;
    } catch (cause) {
      error.value = messageOf(cause);
    }
  }

  async function selectTable(table: TableInfo) {
    if (!activeConnectionId.value) return;
    selectedTable.value = table;
    tableDetail.value = (await run(() => api.tableDetail(activeConnectionId.value!, table.database, table.name))) ?? null;
  }

  function highlightTable(table: TableInfo) {
    selectedTable.value = table;
    if (tableDetail.value?.table.database !== table.database || tableDetail.value.table.name !== table.name) {
      tableDetail.value = null;
    }
  }

  async function openTabSession(connectionId: UUID, sessionId: UUID) {
    const existingConnectionId = tabSessionConnections.get(sessionId);
    if (existingConnectionId) {
      if (existingConnectionId === connectionId) return true;
      error.value = "标签页会话已绑定到其他连接";
      return false;
    }
    const done = await run(() => api.openTabSession(connectionId, sessionId));
    if (done === undefined && error.value) return false;
    tabSessionConnections.set(sessionId, connectionId);
    return true;
  }

  async function closeTabSession(sessionId: UUID) {
    const done = await run(() => api.closeTabSession(sessionId));
    tabSessionConnections.delete(sessionId);
    delete transactionSessions.value[sessionId];
    return !(done === undefined && error.value);
  }

  function forgetTabSessions(connectionId: UUID) {
    for (const [sessionId, ownerId] of tabSessionConnections) {
      if (ownerId !== connectionId) continue;
      tabSessionConnections.delete(sessionId);
      delete transactionSessions.value[sessionId];
    }
  }

  async function execute(sessionId: UUID | null, sql: string, allowWrite = false, rowOffset = 0, pageSize = 500) {
    if (!activeConnectionId.value) { error.value = "请先连接数据库"; return; }
    if (executingId.value) {
      error.value = "已有查询正在执行，请等待完成或先停止当前查询";
      return;
    }
    const connectionId = activeConnectionId.value;
    const database = selectedDatabase.value;
    const executionId = crypto.randomUUID();
    executingId.value = executionId;
    executingConnectionId.value = connectionId;
    executingSessionId.value = sessionId;
    error.value = null;
    try {
      const page = keepResultRowsRaw(await api.execute(connectionId, sessionId, {
        executionId, sql, database, allowWrite, rowOffset, pageSize,
      }));
      result.value = page;
      return page;
    } catch (cause) { error.value = messageOf(cause); }
    finally {
      if (executingId.value === executionId) {
        executingId.value = null;
        executingConnectionId.value = null;
        executingSessionId.value = null;
      }
    }
  }

  async function mutateRow(sessionId: UUID, request: RowMutationRequest) {
    if (!activeConnectionId.value) return null;
    const result = await run(() => api.mutateRow(activeConnectionId.value!, sessionId, request));
    if (!result && error.value) error.value = rowMutationError(request, error.value);
    return result ?? null;
  }

  async function beginTransaction(sessionId: UUID) {
    if (!activeConnectionId.value) return false;
    const connectionId = activeConnectionId.value;
    const done = await run(() => api.beginTransaction(connectionId, sessionId));
    if (done === undefined && error.value) return false;
    transactionSessions.value[sessionId] = true;
    return true;
  }

  async function commitTransaction(sessionId: UUID) {
    if (!activeConnectionId.value) return false;
    const connectionId = activeConnectionId.value;
    const done = await run(() => api.commitTransaction(connectionId, sessionId));
    if (done === undefined && error.value) return false;
    delete transactionSessions.value[sessionId];
    return true;
  }

  async function rollbackTransaction(sessionId: UUID) {
    if (!activeConnectionId.value) return false;
    const connectionId = activeConnectionId.value;
    const done = await run(() => api.rollbackTransaction(connectionId, sessionId));
    if (done === undefined && error.value) return false;
    delete transactionSessions.value[sessionId];
    return true;
  }

  async function cancel() {
    const connectionId = executingConnectionId.value;
    const sessionId = executingSessionId.value;
    const executionId = executingId.value;
    if (!connectionId || !executionId) return;
    await run(() => api.cancel(connectionId, sessionId, executionId));
  }

  function resetWorkspace() {
    activeConnectionId.value = null;
    databases.value = [];
    selectedDatabase.value = null;
    tables.value = [];
    tableHasMore.value = false;
    selectedTable.value = null;
    tableDetail.value = null;
    routines.value = [];
    triggers.value = [];
    events.value = [];
    result.value = null;
  }

  return {
    connections, connectionInfo, activeConnectionId, activeConnection, connected, databases, selectedDatabase,
    tables, tableHasMore, selectedTable, tableDetail, routines, triggers, events,
    transactionSessions, result, executingId, executingConnectionId, executingSessionId, busy, error,
    loadConnections, saveConnection, removeConnection, connect, disconnect, loadDatabases, selectDatabase, closeDatabase, loadTables,
    loadDatabaseObjects, highlightTable, selectTable, openTabSession, closeTabSession, execute, cancel,
    mutateRow, beginTransaction, commitTransaction, rollbackTransaction,
  };
});
