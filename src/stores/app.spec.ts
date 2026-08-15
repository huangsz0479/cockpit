import { createPinia, setActivePinia } from "pinia";
import { isReactive } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/api";
import { useAppStore } from "./app";
import type { QueryResultPage, RowMutationKind, RowMutationRequest } from "@/types";

vi.mock("@/lib/api", () => ({
  api: {
    beginTransaction: vi.fn(), cancel: vi.fn(), closeTabSession: vi.fn(), commitTransaction: vi.fn(),
    connect: vi.fn(), execute: vi.fn(), listDatabases: vi.fn(), listTables: vi.fn(), mutateRow: vi.fn(),
    openTabSession: vi.fn(), rollbackTransaction: vi.fn(),
  },
}));

describe("app store query execution", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it("requests a bounded result page", async () => {
    const store = useAppStore();
    const connectionId = crypto.randomUUID();
    const page: QueryResultPage = {
      executionId: crypto.randomUUID(),
      columns: [],
      rows: [],
      affectedRows: 0,
      executionTimeMs: 1,
      truncated: false,
      hasMore: false,
      resultSetIndex: 0,
      messages: [],
    };
    const sessionId = crypto.randomUUID();
    store.activeConnectionId = connectionId;
    vi.mocked(api.execute).mockResolvedValue(page);

    await store.execute(sessionId, "SELECT * FROM bus_card_info");

    expect(api.execute).toHaveBeenCalledWith(connectionId, sessionId, expect.any(Object));
    const request = vi.mocked(api.execute).mock.calls[0]![2];
    expect(request).toMatchObject({ pageSize: 500, rowOffset: 0 });
    expect(request).not.toHaveProperty("recordHistory");
  });

  it("keeps result row payloads out of deep Vue reactivity", async () => {
    const store = useAppStore();
    store.activeConnectionId = crypto.randomUUID();
    const page: QueryResultPage = {
      executionId: crypto.randomUUID(),
      columns: [{ name: "id", databaseType: "BIGINT", nullable: false, unsigned: true, binary: false }],
      rows: [[{ kind: "unsigned", value: "1" }]],
      affectedRows: 0,
      executionTimeMs: 1,
      truncated: false,
      hasMore: false,
      resultSetIndex: 0,
      messages: [],
      additionalResultSets: [{
        columns: [{ name: "id", databaseType: "BIGINT", nullable: false, unsigned: true, binary: false }],
        rows: [[{ kind: "unsigned", value: "2" }]],
        affectedRows: 0,
        truncated: false,
        hasMore: false,
        resultSetIndex: 1,
        rowOffset: 0,
        pageSize: 500,
      }],
    };
    vi.mocked(api.execute).mockResolvedValue(page);

    const returned = await store.execute(crypto.randomUUID(), "SELECT 1; SELECT 2");

    expect(returned).not.toBeUndefined();
    expect(isReactive(returned!.rows)).toBe(false);
    expect(isReactive(returned!.additionalResultSets![0]!.rows)).toBe(false);
  });

  it("clears the previous connection tree before loading the next connection", async () => {
    const store = useAppStore();
    const previousConnectionId = crypto.randomUUID();
    const nextConnectionId = crypto.randomUUID();
    let resolveConnection!: (value: { serverVersion: string; connectionId: number }) => void;
    const pendingConnection = new Promise<{ serverVersion: string; connectionId: number }>((resolve) => {
      resolveConnection = resolve;
    });
    store.activeConnectionId = previousConnectionId;
    store.databases = [{ name: "previous_database" }];
    store.selectedDatabase = "previous_database";
    store.tables = [{ database: "previous_database", name: "previous_table", tableType: "BASE TABLE" }];
    vi.mocked(api.connect).mockReturnValueOnce(pendingConnection);
    vi.mocked(api.listDatabases).mockResolvedValueOnce([{ name: "next_database" }]);

    const connecting = store.connect(nextConnectionId);

    expect(store.databases).toEqual([]);
    expect(store.selectedDatabase).toBeNull();
    expect(store.tables).toEqual([]);

    resolveConnection({ serverVersion: "8.0", connectionId: 2 });
    await connecting;

    expect(store.activeConnectionId).toBe(nextConnectionId);
    expect(store.databases).toEqual([{ name: "next_database" }]);
    expect(store.tables).toEqual([]);
  });

  it("ignores a database list returned by an older connection request", async () => {
    const store = useAppStore();
    const firstConnectionId = crypto.randomUUID();
    const secondConnectionId = crypto.randomUUID();
    let resolveFirstDatabases!: (value: { name: string }[]) => void;
    const pendingFirstDatabases = new Promise<{ name: string }[]>((resolve) => {
      resolveFirstDatabases = resolve;
    });
    vi.mocked(api.connect)
      .mockResolvedValueOnce({ serverVersion: "8.0", connectionId: 1 })
      .mockResolvedValueOnce({ serverVersion: "8.0", connectionId: 2 });
    vi.mocked(api.listDatabases)
      .mockReturnValueOnce(pendingFirstDatabases)
      .mockResolvedValueOnce([{ name: "second_database" }]);

    const firstConnection = store.connect(firstConnectionId);
    await vi.waitFor(() => expect(api.listDatabases).toHaveBeenCalledWith(firstConnectionId));
    const secondConnection = store.connect(secondConnectionId);
    await secondConnection;
    resolveFirstDatabases([{ name: "first_database" }]);
    await firstConnection;

    expect(store.activeConnectionId).toBe(secondConnectionId);
    expect(store.databases).toEqual([{ name: "second_database" }]);
  });

  it("does not restore tables from the previous connection after switching", async () => {
    const store = useAppStore();
    const previousConnectionId = crypto.randomUUID();
    const nextConnectionId = crypto.randomUUID();
    let resolvePreviousTables!: (value: { database: string; name: string; tableType: string }[]) => void;
    const pendingPreviousTables = new Promise<{ database: string; name: string; tableType: string }[]>((resolve) => {
      resolvePreviousTables = resolve;
    });
    store.activeConnectionId = previousConnectionId;
    store.selectedDatabase = "previous_database";
    vi.mocked(api.listTables).mockReturnValueOnce(pendingPreviousTables);
    vi.mocked(api.connect).mockResolvedValueOnce({ serverVersion: "8.0", connectionId: 2 });
    vi.mocked(api.listDatabases).mockResolvedValueOnce([{ name: "next_database" }]);

    const loadingPreviousTables = store.loadTables();
    const connecting = store.connect(nextConnectionId);
    resolvePreviousTables([{ database: "previous_database", name: "previous_table", tableType: "BASE TABLE" }]);
    await Promise.all([loadingPreviousTables, connecting]);

    expect(store.activeConnectionId).toBe(nextConnectionId);
    expect(store.databases).toEqual([{ name: "next_database" }]);
    expect(store.tables).toEqual([]);
  });

  it("ignores a database refresh returned for the previous connection", async () => {
    const store = useAppStore();
    const previousConnectionId = crypto.randomUUID();
    const nextConnectionId = crypto.randomUUID();
    let resolvePreviousDatabases!: (value: { name: string }[]) => void;
    let resolveNextConnection!: (value: { serverVersion: string; connectionId: number }) => void;
    const pendingPreviousDatabases = new Promise<{ name: string }[]>((resolve) => {
      resolvePreviousDatabases = resolve;
    });
    const pendingNextConnection = new Promise<{ serverVersion: string; connectionId: number }>((resolve) => {
      resolveNextConnection = resolve;
    });
    store.activeConnectionId = previousConnectionId;
    vi.mocked(api.listDatabases).mockReturnValueOnce(pendingPreviousDatabases);
    vi.mocked(api.connect).mockReturnValueOnce(pendingNextConnection);

    const refreshing = store.loadDatabases();
    const connecting = store.connect(nextConnectionId);
    resolvePreviousDatabases([{ name: "previous_database" }]);
    await refreshing;

    expect(store.databases).toEqual([]);

    vi.mocked(api.listDatabases).mockResolvedValueOnce([{ name: "next_database" }]);
    resolveNextConnection({ serverVersion: "8.0", connectionId: 2 });
    await connecting;
    expect(store.databases).toEqual([{ name: "next_database" }]);
  });

  it("cancels an in-flight query on the connection that started it", async () => {
    const store = useAppStore();
    const sourceConnectionId = crypto.randomUUID();
    const otherConnectionId = crypto.randomUUID();
    const sessionId = crypto.randomUUID();
    let resolveQuery!: (page: QueryResultPage) => void;
    const pendingQuery = new Promise<QueryResultPage>((resolve) => { resolveQuery = resolve; });
    vi.mocked(api.execute).mockReturnValueOnce(pendingQuery);
    vi.mocked(api.cancel).mockResolvedValueOnce(true);

    store.activeConnectionId = sourceConnectionId;
    const execution = store.execute(sessionId, "SELECT SLEEP(10)", false, 0, 100);
    const executionId = store.executingId!;
    expect(store.executingConnectionId).toBe(sourceConnectionId);

    store.activeConnectionId = otherConnectionId;
    await store.cancel();
    expect(api.cancel).toHaveBeenCalledWith(sourceConnectionId, sessionId, executionId);

    resolveQuery({
      executionId,
      columns: [],
      rows: [],
      affectedRows: 0,
      executionTimeMs: 1,
      truncated: false,
      hasMore: false,
      resultSetIndex: 0,
      messages: [],
    });
    await execution;
    expect(store.executingId).toBeNull();
    expect(store.executingConnectionId).toBeNull();
    expect(store.executingSessionId).toBeNull();
  });

  it("does not let a second query overwrite active execution state", async () => {
    const store = useAppStore();
    let resolveQuery!: (page: QueryResultPage) => void;
    const pendingQuery = new Promise<QueryResultPage>((resolve) => { resolveQuery = resolve; });
    store.activeConnectionId = crypto.randomUUID();
    const sessionId = crypto.randomUUID();
    vi.mocked(api.execute).mockReturnValueOnce(pendingQuery);

    const firstExecution = store.execute(sessionId, "SELECT SLEEP(10)", false, 0, 100);
    expect(await store.execute(crypto.randomUUID(), "SELECT 2", false, 0, 100)).toBeUndefined();
    expect(api.execute).toHaveBeenCalledTimes(1);
    expect(store.error).toBe("已有查询正在执行，请等待完成或先停止当前查询");

    resolveQuery({
      executionId: store.executingId!,
      columns: [],
      rows: [],
      affectedRows: 0,
      executionTimeMs: 1,
      truncated: false,
      hasMore: false,
      resultSetIndex: 0,
      messages: [],
    });
    await firstExecution;
  });

  it.each([
    ["insert", "新增失败"],
    ["update", "更新失败"],
    ["delete", "删除失败"],
  ] satisfies [RowMutationKind, string][])('labels a failed "%s" row mutation with its actual operation', async (kind, label) => {
    const store = useAppStore();
    store.activeConnectionId = crypto.randomUUID();
    const request: RowMutationRequest = {
      database: "demo",
      table: "card_cycle",
      kind,
      values: [],
      keyValues: [],
      originalValues: [],
    };
    vi.mocked(api.mutateRow).mockRejectedValueOnce({
      code: "QUERY_ERROR",
      message: "查询失败：Server error: Duplicate entry for key 'PRIMARY'",
    });

    expect(await store.mutateRow(crypto.randomUUID(), request)).toBeNull();
    expect(store.error).toBe(`${label}：Server error: Duplicate entry for key 'PRIMARY'`);
  });

  it("tracks transactions independently for each tab session", async () => {
    const store = useAppStore();
    const connectionId = crypto.randomUUID();
    const firstSessionId = crypto.randomUUID();
    const secondSessionId = crypto.randomUUID();
    store.activeConnectionId = connectionId;
    vi.mocked(api.beginTransaction).mockResolvedValue();
    vi.mocked(api.commitTransaction).mockResolvedValue();

    await store.beginTransaction(firstSessionId);
    await store.beginTransaction(secondSessionId);
    expect(store.transactionSessions[firstSessionId]).toBe(true);
    expect(store.transactionSessions[secondSessionId]).toBe(true);

    await store.commitTransaction(firstSessionId);
    expect(store.transactionSessions[firstSessionId]).toBeUndefined();
    expect(store.transactionSessions[secondSessionId]).toBe(true);
    expect(api.commitTransaction).toHaveBeenCalledWith(connectionId, firstSessionId);
  });
});
