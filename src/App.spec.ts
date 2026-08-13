import { createApp, defineComponent, h, nextTick } from "vue";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, describe, expect, it, vi } from "vitest";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { ConnectionProfile, QueryResultPage, TableInfo } from "@/types";
import { api } from "@/lib/api";
import { useAppStore } from "@/stores/app";
import mysqlIcon from "../src-tauri/icons/database/mysql.svg";
import App from "./App.vue";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn().mockResolvedValue("0.1.5") }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

vi.mock("@/components/SqlEditor.vue", () => ({
  default: defineComponent({
    props: { modelValue: { type: String, default: "" } },
    template: '<div data-testid="sql-editor" :data-model-value="modelValue" />',
  }),
}));

vi.mock("@/components/ConnectionDialog.vue", () => ({
  default: defineComponent({
    props: { initial: { type: Object, default: null } },
    emits: ["close"],
    setup(props, { emit }) {
      return () => h("div", { "data-testid": "connection-dialog" }, [
        (props.initial as ConnectionProfile | null)?.name ?? "new",
        h("button", { "data-testid": "close-dialog", onClick: () => emit("close") }, "close"),
      ]);
    },
  }),
}));

vi.mock("@/components/ContextMenu.vue", () => ({
  default: defineComponent({
    emits: ["close"],
    setup(_, { slots }) {
      return () => h("div", { "data-testid": "context-menu" }, slots.default?.());
    },
  }),
}));

afterEach(() => {
  document.body.innerHTML = "";
  localStorage.removeItem("cockpit:snippets");
  localStorage.removeItem("cockpit:transfer-tasks");
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
  vi.useRealTimers();
  vi.clearAllMocks();
});

function profile(): ConnectionProfile {
  const now = new Date().toISOString();
  return {
    id: crypto.randomUUID(),
    name: "本地 MySQL",
    host: "127.0.0.1",
    port: 3306,
    username: "root",
    database: null,
    tls: { mode: "disabled", caCertPath: null, clientCertPath: null, clientKeyPath: null },
    ssh: null,
    connectTimeoutSecs: 5,
    queryTimeoutSecs: 30,
    poolSize: 5,
    readOnly: false,
    production: false,
    color: "#16a085",
    createdAt: now,
    updatedAt: now,
  };
}

function sqliteProfile(): ConnectionProfile {
  return {
    ...profile(),
    driverKind: "sqlite",
    name: "本地 SQLite",
    host: "",
    port: 0,
    username: "",
    database: "/tmp/cockpit-test.sqlite",
  };
}

function mountApp() {
  const host = document.createElement("div");
  document.body.append(host);
  const pinia = createPinia();
  setActivePinia(pinia);
  const app = createApp(App).use(pinia);
  app.mount(host);
  const store = useAppStore();
  vi.spyOn(store, "openTabSession").mockResolvedValue(true);
  vi.spyOn(store, "closeTabSession").mockResolvedValue(true);
  return { app, host, store };
}

describe("App connection actions", () => {
  it("closes stacked dialogs from the top with Escape", async () => {
    const { app, host } = mountApp();

    Array.from(host.querySelectorAll<HTMLButtonElement>(".window-tool"))
      .find((button) => button.textContent?.includes("设置"))!.click();
    await nextTick();
    Array.from(host.querySelectorAll<HTMLButtonElement>(".settings-dialog button"))
      .find((button) => button.textContent === "诊断日志")!.click();
    await nextTick();
    expect(host.querySelector(".diagnostics-dialog")).not.toBeNull();
    expect(host.querySelector(".settings-dialog")).not.toBeNull();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await nextTick();
    expect(host.querySelector(".diagnostics-dialog")).toBeNull();
    expect(host.querySelector(".settings-dialog")).not.toBeNull();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await nextTick();
    expect(host.querySelector(".settings-dialog")).toBeNull();
    app.unmount();
  });

  it("shows only the connection name in the sidebar", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    store.connections.push(connection);
    await nextTick();

    const label = host.querySelector(".connection-root .tree-label");
    expect(label?.textContent).toBe("本地 MySQL");
    expect(label?.querySelector("small")).toBeNull();
    app.unmount();
  });

  it("renders the database logo and marks it connected for icon coloring", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    store.connections.push(connection);
    await nextTick();

    const icon = host.querySelector<HTMLElement>(".connection-root .database-kind-icon")!;
    expect(icon.querySelector("svg path")).not.toBeNull();
    expect(icon.classList.contains("connected")).toBe(false);

    store.connectionInfo[connection.id] = { serverVersion: "8.0", connectionId: 1 };
    await nextTick();
    expect(icon.classList.contains("connected")).toBe(true);
    app.unmount();
  });

  it("uses a single onboarding area when there are no connections", () => {
    const { app, host } = mountApp();

    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(0);
    expect(host.querySelector(".workspace-tabs")).toBeNull();
    expect(host.querySelector(".workspace")).toBeNull();
    expect(host.querySelector(".navigation-empty")).not.toBeNull();
    expect(host.querySelector(".navigation-resizer")).toBeNull();
    expect(host.querySelector("[data-testid='sql-editor']")).toBeNull();
    app.unmount();
  });

  it("shows the full workspace decoration after a connection has been added", async () => {
    const { app, host, store } = mountApp();
    store.connections.push(profile());
    await nextTick();

    expect(host.querySelector(".workspace")).not.toBeNull();
    expect(host.querySelector(".workspace-empty-card")?.textContent).toContain("让数据在这里展开");
    app.unmount();
  });

  it("refreshes runtime statistics periodically and stops after unmount", async () => {
    vi.useFakeTimers();
    Object.defineProperty(window, "__TAURI_INTERNALS__", { configurable: true, value: {} });
    const runtimeStats = vi.spyOn(api, "runtimeStats").mockResolvedValue({
      openConnectionCount: 0,
      tabSessionCount: 0,
      memoryBytes: 128 * 1024 * 1024,
    });
    const loadWorkspaceState = vi.spyOn(api, "loadWorkspaceState").mockResolvedValue(null);
    const listConnections = vi.spyOn(api, "listConnections").mockResolvedValue([]);
    const { app } = mountApp();

    expect(runtimeStats).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(runtimeStats).toHaveBeenCalledTimes(2);

    app.unmount();
    await vi.advanceTimersByTimeAsync(20_000);
    expect(runtimeStats).toHaveBeenCalledTimes(2);

    runtimeStats.mockRestore();
    loadWorkspaceState.mockRestore();
    listConnections.mockRestore();
  });

  it("does not duplicate query execution status in the global toolbar", async () => {
    const { app, host, store } = mountApp();
    store.executingId = crypto.randomUUID();
    await nextTick();

    expect(host.querySelector(".window-execution-status")).toBeNull();
    expect(host.querySelector(".window-primary-actions")?.textContent).not.toContain("正在执行");

    app.unmount();
  });

  it("shows errors globally when no workspace can display them", async () => {
    const { app, host, store } = mountApp();
    store.error = "连接服务暂时不可用";
    await nextTick();

    const notice = host.querySelector<HTMLElement>(".global-error-notice")!;
    expect(notice.getAttribute("role")).toBe("alert");
    expect(notice.textContent).toContain("连接服务暂时不可用");
    notice.querySelector<HTMLButtonElement>("button")!.click();
    await nextTick();
    expect(host.querySelector(".global-error-notice")).toBeNull();
    app.unmount();
  });

  it("uses the query action icons and hides removed toolbar actions", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    store.connectionInfo[connection.id] = { serverVersion: "8.0", connectionId: 1, currentDatabase: "demo" };
    store.databases = [{ name: "demo" }];
    store.selectedDatabase = "demo";
    await nextTick();

    Array.from(host.querySelectorAll<HTMLButtonElement>(".window-tool"))
      .find((button) => button.textContent?.includes("新建查询"))!.click();
    await nextTick();

    const toolbar = host.querySelector<HTMLElement>(".editor-toolbar .toolbar-actions")!;
    expect(toolbar.textContent).toContain("保存");
    expect(toolbar.textContent).toContain("美化");
    expect(toolbar.textContent).toContain("事务");
    expect(toolbar.textContent).not.toContain("保存查询");
    expect(toolbar.textContent).not.toContain("格式化");
    expect(toolbar.textContent).not.toContain("开始事务");
    expect(toolbar.textContent).not.toContain("导出 SQL");
    expect(toolbar.textContent).not.toContain("片段");
    expect(toolbar.textContent).not.toContain("计划");
    expect(toolbar.getAttribute("aria-label")).toBe("查询操作");
    expect(toolbar.querySelectorAll(".query-toolbar-group")).toHaveLength(3);
    const actionIcons = Array.from(toolbar.querySelectorAll<HTMLImageElement>("img.query-toolbar-icon"));
    expect(actionIcons).toHaveLength(5);
    expect(actionIcons.every((icon) => Boolean(icon.getAttribute("src")))).toBe(true);
    const executeButton = toolbar.querySelector<HTMLButtonElement>(".execute-button")!;
    expect(executeButton.textContent).toContain("执行");
    expect(executeButton.classList.contains("window-tool")).toBe(true);
    expect(executeButton.classList.contains("primary")).toBe(false);
    expect(host.querySelector(".query-context-database-icon")?.classList.contains("connected")).toBe(true);

    store.executingId = crypto.randomUUID();
    await nextTick();
    const stopButton = Array.from(toolbar.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.includes("停止"))!;
    expect(stopButton.querySelector("img.query-toolbar-icon")?.getAttribute("src")).toBeTruthy();

    await vi.waitFor(() => expect(store.openTabSession).toHaveBeenCalled());
    const sessionId = vi.mocked(store.openTabSession).mock.calls.at(-1)![1];
    store.transactionSessions[sessionId] = true;
    await nextTick();
    const commitButton = Array.from(toolbar.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.includes("提交"))!;
    const rollbackButton = Array.from(toolbar.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.includes("回滚"))!;
    expect(commitButton.querySelector("img.query-toolbar-icon")?.getAttribute("src")).toBeTruthy();
    expect(rollbackButton.querySelector("img.query-toolbar-icon")?.getAttribute("src")).toBeTruthy();
    app.unmount();
  });

  it("keeps transactions isolated between query tabs on the same connection", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    store.connectionInfo[connection.id] = { serverVersion: "8.0", connectionId: 1, currentDatabase: "demo" };
    store.databases = [{ name: "demo" }];
    store.selectedDatabase = "demo";
    await nextTick();

    const newQuery = () => Array.from(host.querySelectorAll<HTMLButtonElement>(".window-tool"))
      .find((button) => button.textContent?.includes("新建查询"))!;
    newQuery().click();
    await vi.waitFor(() => expect(store.openTabSession).toHaveBeenCalledTimes(1));
    const firstSessionId = vi.mocked(store.openTabSession).mock.calls[0]![1];
    store.transactionSessions[firstSessionId] = true;
    await nextTick();
    expect(host.querySelector(".query-toolbar-actions")?.textContent).toContain("提交");

    newQuery().click();
    await vi.waitFor(() => expect(store.openTabSession).toHaveBeenCalledTimes(2));
    const secondSessionId = vi.mocked(store.openTabSession).mock.calls[1]![1];
    expect(secondSessionId).not.toBe(firstSessionId);
    expect(host.querySelector(".query-toolbar-actions")?.textContent).toContain("事务");
    expect(host.querySelector(".query-toolbar-actions")?.textContent).not.toContain("提交");

    host.querySelectorAll<HTMLButtonElement>(".workspace-tab")[0]!.click();
    await nextTick();
    expect(host.querySelector(".query-toolbar-actions")?.textContent).toContain("提交");
    app.unmount();
  });

  it("always confirms destructive SQL even when optional risk prompts are disabled", async () => {
    const assess = vi.spyOn(api, "assess").mockResolvedValue({
      statementKind: "DELETE", risk: "destructive", requiresConfirmation: true, reason: "该语句会删除数据",
    });
    const { app, host, store } = mountApp();
    const connection = profile();
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    store.connectionInfo[connection.id] = { serverVersion: "8.0", connectionId: 1, currentDatabase: "demo" };
    const execute = vi.spyOn(store, "execute").mockResolvedValue(undefined);
    await nextTick();

    Array.from(host.querySelectorAll<HTMLButtonElement>(".window-tool"))
      .find((button) => button.textContent?.includes("设置"))!.click();
    await nextTick();
    const riskSetting = Array.from(host.querySelectorAll<HTMLLabelElement>(".settings-form label"))
      .find((label) => label.textContent?.includes("高风险 SQL"))!;
    riskSetting.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click();
    Array.from(host.querySelectorAll<HTMLButtonElement>(".settings-dialog button"))
      .find((button) => button.textContent === "保存设置")!.click();
    await nextTick();

    Array.from(host.querySelectorAll<HTMLButtonElement>(".window-tool"))
      .find((button) => button.textContent?.includes("新建查询"))!.click();
    await nextTick();
    host.querySelector<HTMLButtonElement>(".execute-button")!.click();
    await vi.waitFor(() => expect(assess).toHaveBeenCalled());
    await nextTick();
    expect(host.querySelector(".action-dialog")?.textContent).toContain("该语句会删除数据");
    expect(execute).not.toHaveBeenCalled();
    Array.from(host.querySelectorAll<HTMLButtonElement>(".action-dialog button"))
      .find((button) => button.textContent === "取消")!.click();
    await nextTick();

    assess.mockRestore();
    app.unmount();
  });

  it("supports quick keyboard actions for queries and settings", async () => {
    const { app, host, store } = mountApp();
    store.connections.push(profile());
    await nextTick();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "n", metaKey: true, bubbles: true }));
    await nextTick();
    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(1);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: ",", metaKey: true, bubbles: true }));
    await nextTick();
    expect(host.querySelector(".settings-dialog")).not.toBeNull();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "n", metaKey: true, bubbles: true }));
    await nextTick();
    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(1);
    app.unmount();
  });

  it("keeps the center window toolbar area free for toolbar actions", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    await nextTick();

    expect(host.querySelector(".window-context")).toBeNull();
    expect(host.querySelector(".window-leading-space")).not.toBeNull();
    expect(host.querySelector(".window-toolbar")?.textContent).not.toContain("本地 MySQL");
    expect(host.querySelector(".window-toolbar")?.textContent).not.toContain("选择数据库");
    app.unmount();
  });

  it("opens a table tab and loads the first 100 rows on double click", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    const table: TableInfo = { database: "demo", name: "users", tableType: "BASE TABLE" };
    const page: QueryResultPage = {
      executionId: crypto.randomUUID(),
      columns: [
        { name: "id", databaseType: "BIGINT", nullable: false, unsigned: true, binary: false },
        { name: "entry_date", databaseType: "DATE", nullable: false, unsigned: false, binary: false },
      ],
      rows: [
        [{ kind: "unsigned", value: "1" }, { kind: "date", value: "2026-08-01" }],
        [{ kind: "unsigned", value: "2" }, { kind: "date", value: "2026-08-02" }],
      ],
      affectedRows: 0,
      executionTimeMs: 4,
      truncated: false,
      hasMore: true,
      resultSetIndex: 0,
      messages: [],
    };
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    store.connectionInfo[connection.id] = {
      serverVersion: "8.0",
      connectionId: 1,
      currentDatabase: "demo",
    };
    store.databases.push({ name: "demo" });
    vi.spyOn(store, "connect").mockResolvedValue();
    vi.spyOn(store, "selectDatabase").mockImplementation(async (database) => {
      store.selectedDatabase = database;
      store.tables = [table];
    });
    const execute = vi.spyOn(store, "execute").mockResolvedValue(page);
    await nextTick();

    host.querySelector<HTMLButtonElement>(".connection-toggle")!.click();
    await nextTick();
    host.querySelector<HTMLElement>(".database-node")!.click();
    await nextTick();
    host.querySelector<HTMLButtonElement>(".object-group-title")!.click();
    await nextTick();
    const objectNode = host.querySelector<HTMLButtonElement>(".object-node")!;
    const selectTable = vi.spyOn(store, "selectTable");
    objectNode.click();
    await nextTick();
    expect(store.selectedTable?.name).toBe("users");
    expect(selectTable).not.toHaveBeenCalled();
    store.tableDetail = {
      table,
      columns: [
        { name: "id", ordinal: 1, dataType: "bigint", fullType: "bigint unsigned", nullable: false, defaultValue: null, extra: "auto_increment", key: "PRI" },
        { name: "entry_date", ordinal: 2, dataType: "date", fullType: "date", nullable: false, defaultValue: null, extra: "", key: "" },
      ],
      indexes: [{ name: "PRIMARY", columns: ["id"], unique: true, primary: true }],
      foreignKeys: [],
      ddl: "CREATE TABLE users (id BIGINT UNSIGNED PRIMARY KEY, entry_date DATE NOT NULL)",
    };

    objectNode.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await vi.waitFor(() => expect(host.querySelector(".table-data-view .data-grid")).not.toBeNull());

    expect(execute).toHaveBeenCalledWith(expect.any(String), "SELECT *\nFROM `demo`.`users`\nLIMIT 101 OFFSET 0;", false, 0, 100);
    const sessionId = execute.mock.calls[0]![0]!;
    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("users");
    expect(host.querySelector(".editor-card")).toBeNull();
    expect(host.querySelector("[data-testid='sql-editor']")).toBeNull();
    expect(host.querySelector(".table-data-view .data-grid")?.textContent).toContain("BIGINT unsigned");
    expect(host.querySelector(".table-data-view .data-grid")?.textContent).toContain("1");
    expect(host.querySelector(".table-data-view [title]")).toBeNull();
    const tabCount = host.querySelectorAll(".workspace-tab").length;
    objectNode.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await vi.waitFor(() => expect(execute).toHaveBeenCalledTimes(2));
    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(tabCount);
    const dataGrid = host.querySelector<HTMLTableElement>(".table-data-view .data-grid")!;
    expect(dataGrid.getAttribute("role")).toBe("grid");
    expect(dataGrid.getAttribute("aria-label")).toBe("users 数据");
    const gridCells = dataGrid.querySelectorAll<HTMLElement>('tbody td[role="gridcell"]');
    expect(gridCells[0]!.tabIndex).toBe(0);
    expect(gridCells[1]!.tabIndex).toBe(-1);
    gridCells[0]!.focus();
    gridCells[0]!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    await vi.waitFor(() => expect(document.activeElement).toBe(gridCells[1]));
    expect(gridCells[1]!.getAttribute("aria-selected")).toBe("true");
    gridCells[1]!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    await vi.waitFor(() => expect(document.activeElement).toBe(gridCells[3]));
    gridCells[3]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await nextTick();
    expect(host.querySelector<HTMLInputElement>('.inline-cell-input[data-column="entry_date"]')).not.toBeNull();
    host.querySelector<HTMLInputElement>('.inline-cell-input[data-column="entry_date"]')!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await nextTick();
    expect(host.querySelector(".inline-cell-input")).toBeNull();
    const selectAllRows = dataGrid.querySelector<HTMLInputElement>('thead input[aria-label="选择当前页全部行"]')!;
    const firstRowSelection = dataGrid.querySelector<HTMLInputElement>('tbody input[aria-label="选择第 1 行"]')!;
    firstRowSelection.click();
    await nextTick();
    expect(selectAllRows.indeterminate).toBe(true);
    expect(selectAllRows.getAttribute("aria-checked")).toBe("mixed");
    firstRowSelection.click();
    await nextTick();
    expect(selectAllRows.indeterminate).toBe(false);
    const exportFormat = host.querySelector<HTMLButtonElement>('.table-data-view button[aria-label="导出格式"]')!;
    expect(exportFormat.dataset.value).toBe("excel");
    exportFormat.click();
    await nextTick();
    expect(Array.from(document.querySelectorAll<HTMLElement>('[role="option"]')).map((option) => option.textContent?.trim())).toEqual(["TXT", "SQL", "CSV", "Excel"]);
    exportFormat.click();
    await nextTick();
    const tableToolbar = host.querySelector<HTMLElement>(".table-toolbar")!;
    expect(tableToolbar.previousElementSibling).toBe(host.querySelector(".table-grid-scroll"));
    expect(Array.from(tableToolbar.querySelectorAll<HTMLButtonElement>(".table-primary-actions button"), (button) => button.textContent?.trim())).toEqual(["新增", "编辑", "删除"]);
    const moreButton = tableToolbar.querySelector<HTMLButtonElement>('.table-actions-menu > button[aria-haspopup="menu"]')!;
    const morePanel = tableToolbar.querySelector<HTMLElement>(".table-actions-panel")!;
    expect(morePanel.style.display).toBe("none");
    moreButton.click();
    await nextTick();
    expect(morePanel.style.display).not.toBe("none");
    expect(moreButton.getAttribute("aria-haspopup")).toBe("menu");
    expect(morePanel.getAttribute("role")).toBe("menu");
    expect(morePanel.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')).not.toHaveLength(0);
    expect(morePanel.textContent).toContain("列设置");
    expect(morePanel.textContent).toContain("批量删除");
    expect(morePanel.textContent).toContain("导出");
    document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    await vi.waitFor(() => expect(morePanel.style.display).toBe("none"));

    const mutateRow = vi.spyOn(store, "mutateRow").mockResolvedValue({ affectedRows: 1, concurrentChange: false });
    const beginTransaction = vi.spyOn(store, "beginTransaction").mockResolvedValue(true);
    const commitTransaction = vi.spyOn(store, "commitTransaction").mockResolvedValue(true);
    const entryDateHeader = Array.from(host.querySelectorAll<HTMLElement>(".table-data-view .data-grid th.data-column"))
      .find((header) => header.textContent?.includes("entry_date"))!;
    entryDateHeader.click();
    await nextTick();

    expect(host.querySelector(".batch-edit-dialog")).toBeNull();
    expect(host.querySelector(".column-batch-edit-bar")).toBeNull();
    expect(host.querySelector(".data-grid .selected-cell")).toBeNull();
    expect(host.querySelector("th.selected-column")?.textContent).toContain("entry_date");
    const directInput = host.querySelector<HTMLInputElement>(".column-direct-edit-input")!;
    expect(document.activeElement).toBe(directInput);
    directInput.value = "2026-08-10";
    directInput.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    const previewCells = Array.from(host.querySelectorAll<HTMLElement>("td.column-direct-edit-preview"));
    expect(previewCells).toHaveLength(2);
    expect(previewCells.map((cell) => cell.textContent)).toEqual(["2026-08-10", "2026-08-10"]);
    directInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(mutateRow).toHaveBeenCalledTimes(2));
    expect(beginTransaction).toHaveBeenCalledTimes(1);
    expect(commitTransaction).toHaveBeenCalledTimes(1);
    expect(beginTransaction).toHaveBeenCalledWith(sessionId);
    expect(commitTransaction).toHaveBeenCalledWith(sessionId);
    expect(mutateRow).toHaveBeenNthCalledWith(1, sessionId, {
      database: "demo", table: "users", kind: "update",
      values: [["entry_date", { kind: "date", value: "2026-08-10" }]],
      keyValues: [["id", { kind: "unsigned", value: "1" }]],
      originalValues: [
        ["id", { kind: "unsigned", value: "1" }],
        ["entry_date", { kind: "date", value: "2026-08-01" }],
      ],
    });
    expect(mutateRow).toHaveBeenNthCalledWith(2, sessionId, {
      database: "demo", table: "users", kind: "update",
      values: [["entry_date", { kind: "date", value: "2026-08-10" }]],
      keyValues: [["id", { kind: "unsigned", value: "2" }]],
      originalValues: [
        ["id", { kind: "unsigned", value: "2" }],
        ["entry_date", { kind: "date", value: "2026-08-02" }],
      ],
    });
    await vi.waitFor(() => expect(host.querySelector(".column-direct-edit-input")).toBeNull());

    store.transactionSessions[sessionId] = true;
    mutateRow.mockClear();
    beginTransaction.mockClear();
    commitTransaction.mockClear();
    const executeTransactionControl = vi.spyOn(api, "execute").mockResolvedValue(page);
    entryDateHeader.click();
    await nextTick();
    const savepointInput = host.querySelector<HTMLInputElement>(".column-direct-edit-input")!;
    savepointInput.value = "2026-08-11";
    savepointInput.dispatchEvent(new Event("input", { bubbles: true }));
    savepointInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(mutateRow).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(executeTransactionControl).toHaveBeenCalledTimes(2));
    expect(beginTransaction).not.toHaveBeenCalled();
    expect(commitTransaction).not.toHaveBeenCalled();
    const transactionSql = executeTransactionControl.mock.calls.map(([, , request]) => request.sql);
    expect(transactionSql[0]).toMatch(/^SAVEPOINT `cockpit_batch_[a-f0-9]+`$/);
    expect(transactionSql[1]).toMatch(/^RELEASE SAVEPOINT `cockpit_batch_[a-f0-9]+`$/);
    expect(transactionSql[1]!.slice("RELEASE ".length)).toBe(transactionSql[0]);
    delete store.transactionSessions[sessionId];

    await vi.waitFor(() => expect(host.querySelector(".column-direct-edit-input")).toBeNull());
    const columnHeader = host.querySelector<HTMLElement>(".table-data-view .data-grid th.data-column")!;
    expect(columnHeader.tabIndex).toBe(0);
    expect(columnHeader.getAttribute("aria-sort")).toBe("none");
    const openColumnMenu = () => columnHeader.querySelector<HTMLButtonElement>(".column-menu-button")!.click();
    openColumnMenu();
    await nextTick();
    expect(host.querySelector(".column-menu-panel")?.textContent).toContain("升序排序");
    expect(host.querySelector(".column-menu-panel")?.textContent).toContain("降序排序");
    expect(host.querySelector(".column-menu-panel")?.textContent).toContain("移除所有排序");
    expect(host.querySelector(".column-menu-panel")?.textContent).toContain("添加筛选");
    Array.from(host.querySelectorAll<HTMLButtonElement>(".column-menu-panel button"))
      .find((button) => button.textContent?.includes("升序排序"))!.click();
    await vi.waitFor(() => expect(columnHeader.getAttribute("aria-sort")).toBe("ascending"));
    openColumnMenu();
    await nextTick();
    Array.from(host.querySelectorAll<HTMLButtonElement>(".column-menu-panel button"))
      .find((button) => button.textContent?.includes("降序排序"))!.click();
    await vi.waitFor(() => expect(columnHeader.getAttribute("aria-sort")).toBe("descending"));
    openColumnMenu();
    await nextTick();
    Array.from(host.querySelectorAll<HTMLButtonElement>(".column-menu-panel button"))
      .find((button) => button.textContent?.includes("移除所有排序"))!.click();
    await vi.waitFor(() => expect(columnHeader.getAttribute("aria-sort")).toBe("none"));
    openColumnMenu();
    await nextTick();
    Array.from(host.querySelectorAll<HTMLButtonElement>(".column-menu-panel button"))
      .find((button) => button.textContent?.includes("添加筛选"))!.click();
    await nextTick();
    const tableFilterInput = host.querySelector<HTMLInputElement>(".table-filter-input")!;
    expect(tableFilterInput.value).toBe("`id` = ");
    expect(document.activeElement).toBe(tableFilterInput);
    const columnResizer = host.querySelector<HTMLElement>(".table-data-view .column-resizer")!;
    Object.defineProperty(columnHeader, "getBoundingClientRect", {
      configurable: true,
      value: vi.fn(() => ({ width: 160 })),
    });
    Object.defineProperties(columnResizer, {
      setPointerCapture: { configurable: true, value: vi.fn() },
      hasPointerCapture: { configurable: true, value: vi.fn(() => false) },
    });
    columnResizer.dispatchEvent(new PointerEvent("pointerdown", { clientX: 160, pointerId: 1, bubbles: true }));
    columnResizer.dispatchEvent(new PointerEvent("pointermove", { clientX: 220, pointerId: 1, bubbles: true }));
    columnResizer.dispatchEvent(new PointerEvent("pointerup", { clientX: 220, pointerId: 1, bubbles: true }));
    await nextTick();

    expect(host.querySelector<HTMLTableColElement>(".table-data-view col.data-column")?.style.width).toBe("220px");
    expect(host.querySelector(".app-shell")?.classList.contains("is-column-resizing")).toBe(false);

    entryDateHeader.click();
    await nextTick();
    expect(entryDateHeader.classList.contains("selected-column")).toBe(true);
    expect(host.querySelector(".column-direct-edit-input")).not.toBeNull();
    const outsideColumnTarget = host.querySelector<HTMLElement>(".result-search")!;
    outsideColumnTarget.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    await nextTick();
    expect(host.querySelector(".data-grid .selected-column")).toBeNull();
    expect(host.querySelector(".column-direct-edit-input")).toBeNull();

    entryDateHeader.click();
    await nextTick();
    expect(entryDateHeader.classList.contains("selected-column")).toBe(true);
    expect(host.querySelector(".column-direct-edit-input")).not.toBeNull();
    host.querySelector<HTMLElement>('.table-data-view tbody td[data-grid-row="0"][data-grid-column="1"]')!.click();
    await nextTick();
    expect(host.querySelector(".data-grid .selected-column")).toBeNull();
    expect(host.querySelector(".column-direct-edit-input")).toBeNull();
    const dateInputBeforeExternalFocus = host.querySelector<HTMLInputElement>('.inline-cell-input[data-column="entry_date"]')!;
    dateInputBeforeExternalFocus.click();
    await vi.waitFor(() => expect(document.body.querySelector(".dp--menu-wrapper")).not.toBeNull());
    const externalFocusTarget = host.querySelector<HTMLInputElement>(".result-search")!;
    externalFocusTarget.focus();
    await vi.waitFor(() => expect(host.querySelector(".inline-cell-input")).toBeNull());
    expect(document.activeElement).toBe(externalFocusTarget);
    expect(host.querySelector(".data-grid .selected-cell")).toBeNull();
    expect(document.body.querySelector(".dp--menu-wrapper")).toBeNull();

    mutateRow.mockReset()
      .mockImplementationOnce(async () => {
        store.error = "Duplicate entry '10' for key 'PRIMARY'";
        return null;
      })
      .mockResolvedValue({ affectedRows: 1, concurrentChange: false });
    host.querySelector<HTMLElement>(".table-data-view tbody td:not(.row-number):not(.row-selection)")!
      .click();
    await nextTick();

    expect(host.querySelector(".row-editor-dialog")).toBeNull();
    const inlineInput = host.querySelector<HTMLInputElement>('.inline-cell-input[data-column="id"]')!;
    expect(inlineInput.value).toBe("1");
    expect(tableToolbar.textContent).not.toContain("Tab 下个单元格");
    expect(tableToolbar.querySelector(".inline-editing-badge")).toBeNull();
    expect(Array.from(tableToolbar.querySelectorAll<HTMLButtonElement>(".table-primary-actions button"), (button) => button.textContent?.trim())).toEqual(["新增", "编辑", "删除"]);
    inlineInput.value = "10";
    inlineInput.dispatchEvent(new Event("input", { bubbles: true }));
    const edgeDateCell = host.querySelector<HTMLElement>('.table-data-view tbody td[data-grid-row="0"][data-grid-column="1"]')!;
    Object.defineProperty(edgeDateCell, "getBoundingClientRect", {
      configurable: true,
      value: () => new DOMRect(900, 700, 100, 29),
    });
    inlineInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    await nextTick();
    const nextInlineInput = host.querySelector<HTMLInputElement>('.inline-cell-input[data-column="entry_date"]')!;
    expect(nextInlineInput.value).toBe("2026-08-01");
    expect(host.querySelector(".inline-date-picker")?.getAttribute("data-placement")).toBe("top-end");
    expect(mutateRow).not.toHaveBeenCalled();
    nextInlineInput.click();
    await vi.waitFor(() => expect(document.body.querySelector(".dp--menu")).not.toBeNull());
    const calendarPointerDown = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    document.body.querySelector<HTMLElement>(".dp--calendar-item")!.dispatchEvent(calendarPointerDown);
    await nextTick();
    expect(calendarPointerDown.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(nextInlineInput);
    expect(host.querySelector<HTMLInputElement>('.inline-cell-input[data-column="entry_date"]')).not.toBeNull();
    expect(mutateRow).not.toHaveBeenCalled();
    nextInlineInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(mutateRow).toHaveBeenCalledTimes(1));
    await nextTick();

    expect(host.querySelector(".table-data-view .error-banner")?.textContent).toContain("Duplicate entry");
    expect(host.querySelector(".table-data-view .data-grid")).not.toBeNull();
    expect(host.querySelector<HTMLInputElement>('.inline-cell-input[data-column="entry_date"]')?.value).toBe("2026-08-01");
    expect(host.querySelector<HTMLButtonElement>('.table-data-view .error-banner button[aria-label="关闭错误提示"]')).not.toBeNull();
    host.querySelector<HTMLElement>('.table-data-view tbody td[data-grid-row="0"][data-grid-column="0"]')!.click();
    await nextTick();
    const retainedInput = host.querySelector<HTMLInputElement>('.inline-cell-input[data-column="id"]')!;
    expect(retainedInput.value).toBe("10");
    retainedInput.value = "11";
    retainedInput.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    expect(host.querySelector(".table-data-view .error-banner")).toBeNull();
    retainedInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    await nextTick();
    host.querySelector<HTMLInputElement>('.inline-cell-input[data-column="entry_date"]')!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(mutateRow).toHaveBeenCalledTimes(2));
    await nextTick();

    expect(mutateRow).toHaveBeenLastCalledWith(sessionId, {
      database: "demo",
      table: "users",
      kind: "update",
      values: [["id", { kind: "unsigned", value: "11" }]],
      keyValues: [["id", { kind: "unsigned", value: "1" }]],
      originalValues: [
        ["id", { kind: "unsigned", value: "1" }],
        ["entry_date", { kind: "date", value: "2026-08-01" }],
      ],
    });
    expect(host.querySelector(".table-data-view tbody tr:nth-child(2)")?.classList.contains("selected")).toBe(true);
    await vi.waitFor(() => expect(host.querySelector<HTMLInputElement>('.inline-cell-input[data-column="id"]')?.value).toBe("2"));

    const autosavedInput = host.querySelector<HTMLInputElement>(".inline-cell-input")!;
    autosavedInput.value = "3";
    autosavedInput.dispatchEvent(new Event("input", { bubbles: true }));
    host.querySelector<HTMLInputElement>(".result-search")!.focus();
    await vi.waitFor(() => expect(mutateRow).toHaveBeenCalledTimes(3));
    await vi.waitFor(() => expect(host.querySelector(".inline-cell-input")).toBeNull());
    expect(mutateRow).toHaveBeenLastCalledWith(sessionId, {
      database: "demo",
      table: "users",
      kind: "update",
      values: [["id", { kind: "unsigned", value: "3" }]],
      keyValues: [["id", { kind: "unsigned", value: "2" }]],
      originalValues: [
        ["id", { kind: "unsigned", value: "2" }],
        ["entry_date", { kind: "date", value: "2026-08-02" }],
      ],
    });

    host.querySelector<HTMLElement>('.table-data-view tbody td[role="gridcell"]')!.click();
    await nextTick();
    const canceledInput = host.querySelector<HTMLInputElement>(".inline-cell-input")!;
    canceledInput.value = "4";
    canceledInput.dispatchEvent(new Event("input", { bubbles: true }));
    canceledInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await nextTick();
    expect(host.querySelector(".inline-cell-input")).toBeNull();
    expect(mutateRow).toHaveBeenCalledTimes(3);

    host.querySelector<HTMLElement>('.table-data-view tbody td[role="gridcell"]')!.click();
    await nextTick();
    host.querySelector<HTMLInputElement>(".inline-cell-input")!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    host.querySelector<HTMLInputElement>('.table-data-view tbody input[aria-label="选择第 1 行"]')!.click();
    await nextTick();
    const nextPage: QueryResultPage = {
      ...page,
      executionId: crypto.randomUUID(),
      rows: [[{ kind: "unsigned", value: "101" }, { kind: "date", value: "2026-09-01" }]],
      hasMore: false,
    };
    execute.mockResolvedValueOnce(nextPage);
    const executeCountBeforePaging = execute.mock.calls.length;
    host.querySelector<HTMLElement>('.table-data-view tbody td[data-grid-row="1"][data-grid-column="1"]')!.click();
    await nextTick();
    host.querySelector<HTMLInputElement>('.inline-cell-input[data-column="entry_date"]')!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(execute).toHaveBeenCalledTimes(executeCountBeforePaging + 1));
    expect(execute).toHaveBeenLastCalledWith(
      sessionId,
      "SELECT *\nFROM `demo`.`users`\nLIMIT 101 OFFSET 100;",
      false, 0, 100,
    );
    await nextTick();
    expect(host.querySelector<HTMLInputElement>('.inline-cell-input[data-column="id"]')?.value).toBe("101");
    expect(host.querySelector<HTMLInputElement>('.table-data-view tbody input[aria-label="选择第 101 行"]')?.checked).toBe(false);
    expect(host.querySelector(".table-data-view .data-grid")?.textContent).toContain("101");

    host.querySelector<HTMLElement>('.table-data-view tbody td[data-grid-row="0"][data-grid-column="1"]')!.click();
    await nextTick();
    host.querySelector<HTMLInputElement>('.inline-cell-input[data-column="entry_date"]')!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(host.querySelector('.inline-insert-row .inline-cell-input[data-column="id"]')).not.toBeNull());
    expect(host.querySelector(".row-editor-dialog")).toBeNull();
    const insertedIdInput = host.querySelector<HTMLInputElement>('.inline-insert-row .inline-cell-input[data-column="id"]')!;
    expect(insertedIdInput.placeholder).toBe("DEFAULT");
    insertedIdInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    await nextTick();
    const insertedDateInput = host.querySelector<HTMLInputElement>('.inline-insert-row .inline-cell-input[data-column="entry_date"]')!;
    insertedDateInput.value = "2026-10-01";
    insertedDateInput.dispatchEvent(new Event("input", { bubbles: true }));
    insertedDateInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(mutateRow).toHaveBeenCalledTimes(4));
    expect(mutateRow).toHaveBeenLastCalledWith(sessionId, {
      database: "demo",
      table: "users",
      kind: "insert",
      values: [["entry_date", { kind: "date", value: "2026-10-01" }]],
      keyValues: [],
      originalValues: [],
    });
    await vi.waitFor(() => expect(host.querySelector(".inline-insert-row")).toBeNull());

    Array.from(host.querySelectorAll<HTMLButtonElement>(".table-primary-actions button"))
      .find((button) => button.textContent?.trim() === "新增")!.click();
    await nextTick();
    expect(host.querySelector('.inline-insert-row .inline-cell-input[data-column="id"]')).not.toBeNull();
    expect(host.querySelector(".row-editor-dialog")).toBeNull();
    host.querySelector<HTMLInputElement>(".inline-insert-row .inline-cell-input")!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    await nextTick();
    expect(host.querySelector(".inline-insert-row")).toBeNull();

    host.querySelector<HTMLButtonElement>('.window-primary-actions .window-tool[aria-label="新建查询"]')!.click();
    await nextTick();
    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(2);
    host.querySelector<HTMLButtonElement>(".connection-toggle")!.click();
    await vi.waitFor(() => expect(store.selectedDatabase).toBeNull());
    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(1);
    expect(host.querySelector(".workspace-tab")?.textContent).toContain("无标题@本地 MySQL");
    expect(host.querySelector(".workspace-tab")?.textContent).not.toContain("users");
    app.unmount();
  });

  it("shows export progress and a persistent success result", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    const page: QueryResultPage = {
      executionId: crypto.randomUUID(),
      columns: [{ name: "id", databaseType: "BIGINT", nullable: false, unsigned: true, binary: false }],
      rows: [[{ kind: "unsigned", value: "1" }], [{ kind: "unsigned", value: "2" }]],
      affectedRows: 0,
      executionTimeMs: 4,
      truncated: false,
      hasMore: false,
      resultSetIndex: 0,
      messages: [],
    };
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    store.connectionInfo[connection.id] = { serverVersion: "8.0", connectionId: 1, currentDatabase: "demo" };
    store.selectedDatabase = "demo";
    const assess = vi.spyOn(api, "assess").mockResolvedValue({
      statementKind: "SELECT", risk: "safe", requiresConfirmation: false,
    });
    const execute = vi.spyOn(store, "execute").mockResolvedValue(page);
    await nextTick();

    Array.from(host.querySelectorAll<HTMLButtonElement>(".window-tool"))
      .find((button) => button.textContent?.includes("新建查询"))!.click();
    await nextTick();
    const exportTrigger = host.querySelector<HTMLButtonElement>(".editor-toolbar .query-export-trigger")!;
    expect(exportTrigger.textContent?.trim()).toBe("导出");
    expect(exportTrigger.querySelector<HTMLImageElement>("img.query-toolbar-icon")?.getAttribute("src")).toBeTruthy();
    expect(exportTrigger.disabled).toBe(true);
    expect(exportTrigger.previousElementSibling?.classList.contains("execute-button")).toBe(true);
    host.querySelector<HTMLButtonElement>(".execute-button")!.click();
    await vi.waitFor(() => expect(execute).toHaveBeenCalled());
    await nextTick();
    expect(exportTrigger.disabled).toBe(false);
    exportTrigger.click();
    await nextTick();
    const exportDialog = host.querySelector<HTMLElement>("#query-export-dialog")!;
    expect(exportDialog.getAttribute("role")).toBe("dialog");
    expect(exportDialog.getAttribute("aria-modal")).toBe("true");
    expect(exportDialog.querySelector<HTMLButtonElement>('[aria-label="导出格式"]')?.textContent).toContain("Excel");
    expect(Array.from(exportDialog.querySelectorAll<HTMLButtonElement>(".query-export-scope-option")).map((button) => button.querySelector("strong")?.textContent)).toEqual(["当前页", "全部"]);

    vi.mocked(saveDialog).mockResolvedValueOnce("/tmp/query-result.xlsx");
    let completeExport!: () => void;
    const pendingExport = new Promise<{ outputPath: string; rowsWritten: number }>((resolve) => {
      completeExport = () => resolve({ outputPath: "/tmp/query-result.xlsx", rowsWritten: 2 });
    });
    const exportResultPage = vi.spyOn(api, "exportResultPage").mockReturnValue(pendingExport);
    expect(host.querySelector(".result-toolbar .export-controls")).toBeNull();
    Array.from(exportDialog.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.includes("当前页"))!.click();
    await nextTick();
    expect(host.querySelector("#query-export-dialog")).toBeNull();

    await vi.waitFor(() => expect(exportResultPage).toHaveBeenCalled());
    expect(host.querySelector(".export-progress-notice")?.textContent).toContain("正在导出");
    expect(host.querySelector(".export-progress-notice")?.textContent).toContain("0%");
    expect(host.querySelector(".export-progress-notice progress")).not.toBeNull();

    completeExport();
    await vi.waitFor(() => expect(host.querySelector(".export-progress-notice")?.textContent).toContain("导出成功"));
    expect(host.querySelector(".export-progress-notice")?.textContent).toContain("已导出 2 行");
    expect(host.querySelector(".export-progress-notice")?.textContent).toContain("100%");
    expect(host.querySelector<HTMLProgressElement>(".export-progress-notice progress")?.value).toBe(100);
    expect(host.querySelector<HTMLButtonElement>(".export-progress-notice .link")?.textContent).toContain("在文件夹中显示");

    assess.mockRestore();
    exportResultPage.mockRestore();
    app.unmount();
  });

  it("tracks editability independently for each result of a multi-statement query", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    const sql = "SELECT id FROM users;\nSELECT * FROM audit_log WHERE active = 1;";
    const page: QueryResultPage = {
      executionId: crypto.randomUUID(),
      columns: [{ name: "id", databaseType: "BIGINT", nullable: false, unsigned: true, binary: false }],
      rows: [[{ kind: "unsigned", value: "1" }]],
      affectedRows: 0,
      executionTimeMs: 4,
      truncated: false,
      hasMore: false,
      resultSetIndex: 0,
      messages: [],
      additionalResultSets: [{
        columns: [
          { name: "id", databaseType: "BIGINT", nullable: false, unsigned: true, binary: false },
          { name: "active", databaseType: "TINYINT", nullable: false, unsigned: false, binary: false },
        ],
        rows: [[{ kind: "unsigned", value: "8" }, { kind: "signed", value: "1" }]],
        affectedRows: 0,
        truncated: false,
        hasMore: false,
        resultSetIndex: 1,
        rowOffset: 0,
        pageSize: 500,
      }],
    };
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    store.connectionInfo[connection.id] = { serverVersion: "8.0", connectionId: 1, currentDatabase: "demo" };
    store.databases = [{ name: "demo" }];
    store.selectedDatabase = "demo";
    vi.mocked(openDialog).mockResolvedValueOnce("/tmp/multi-result.sql");
    vi.spyOn(api, "readTextFile").mockResolvedValue({ path: "/tmp/multi-result.sql", contents: sql });
    const assess = vi.spyOn(api, "assess").mockResolvedValue({
      statementKind: "MULTI_STATEMENT", risk: "safe", requiresConfirmation: false,
    });
    const tableDetail = vi.spyOn(api, "tableDetail").mockResolvedValue({
      table: { database: "demo", name: "audit_log", tableType: "BASE TABLE" },
      columns: [
        { name: "id", ordinal: 1, dataType: "bigint", fullType: "bigint unsigned", nullable: false, defaultValue: null, extra: "auto_increment", key: "PRI" },
        { name: "active", ordinal: 2, dataType: "tinyint", fullType: "tinyint", nullable: false, defaultValue: null, extra: "", key: "" },
      ],
      indexes: [{ name: "PRIMARY", columns: ["id"], unique: true, primary: true }],
      foreignKeys: [],
      ddl: "CREATE TABLE audit_log (id BIGINT PRIMARY KEY, active TINYINT NOT NULL)",
    });
    const execute = vi.spyOn(store, "execute").mockResolvedValue(page);
    const mutateRow = vi.spyOn(store, "mutateRow").mockResolvedValue({ affectedRows: 1, concurrentChange: false });
    await nextTick();

    host.querySelector<HTMLButtonElement>('.window-primary-actions .window-tool[aria-label="打开 SQL"]')!.click();
    await vi.waitFor(() => expect(host.querySelector("[data-testid='sql-editor']")).not.toBeNull());
    host.querySelector<HTMLButtonElement>(".execute-button")!.click();
    await vi.waitFor(() => expect(host.querySelectorAll(".result-set-tabs button")).toHaveLength(2));
    await vi.waitFor(() => expect(tableDetail).toHaveBeenCalledWith(connection.id, "demo", "audit_log"));
    expect(host.querySelector(".result-card.editable-query-result")).toBeNull();

    host.querySelectorAll<HTMLButtonElement>(".result-set-tabs button")[1]!.click();
    await nextTick();
    expect(host.querySelector(".result-card.editable-query-result")).not.toBeNull();
    expect(host.querySelector(".query-edit-actions")).not.toBeNull();

    host.querySelector<HTMLElement>('.editable-query-result tbody td[data-grid-row="0"][data-grid-column="1"]')!.click();
    await nextTick();
    const input = host.querySelector<HTMLInputElement>('.inline-cell-input[data-column="active"]')!;
    input.value = "0";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(mutateRow).toHaveBeenCalledTimes(1));
    await vi.waitFor(() => expect(execute).toHaveBeenCalledTimes(2));
    expect(execute).toHaveBeenLastCalledWith(expect.any(String), sql, false, 0, 500);
    expect(host.querySelectorAll<HTMLButtonElement>(".result-set-tabs button")[1]!.getAttribute("aria-selected")).toBe("true");
    expect(host.querySelector(".result-card.editable-query-result")).not.toBeNull();

    host.querySelectorAll<HTMLButtonElement>(".result-set-tabs button")[0]!.click();
    await nextTick();
    expect(host.querySelector(".result-card.editable-query-result")).toBeNull();

    assess.mockRestore();
    tableDetail.mockRestore();
    app.unmount();
  });

  it("keeps single-table SELECT * editable in the standard query result layout", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    const page: QueryResultPage = {
      executionId: crypto.randomUUID(),
      columns: [
        { name: "id", databaseType: "BIGINT", nullable: false, unsigned: true, binary: false },
        { name: "entry_date", databaseType: "DATE", nullable: false, unsigned: false, binary: false },
      ],
      rows: [
        [{ kind: "unsigned", value: "1" }, { kind: "date", value: "2026-08-01" }],
        [{ kind: "unsigned", value: "2" }, { kind: "date", value: "2026-08-02" }],
      ],
      affectedRows: 0,
      executionTimeMs: 4,
      truncated: false,
      hasMore: false,
      resultSetIndex: 0,
      messages: [],
    };
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    store.connectionInfo[connection.id] = { serverVersion: "8.0", connectionId: 1, currentDatabase: "demo" };
    store.databases = [{ name: "demo" }];
    store.selectedDatabase = "demo";
    vi.mocked(openDialog).mockResolvedValueOnce("/tmp/users.sql");
    vi.spyOn(api, "readTextFile").mockResolvedValue({ path: "/tmp/users.sql", contents: "SELECT * FROM users" });
    const assess = vi.spyOn(api, "assess").mockResolvedValue({
      statementKind: "SELECT", risk: "safe", requiresConfirmation: false,
    });
    const tableDetail = vi.spyOn(api, "tableDetail").mockResolvedValue({
      table: { database: "demo", name: "users", tableType: "BASE TABLE" },
      columns: [
        { name: "id", ordinal: 1, dataType: "bigint", fullType: "bigint unsigned", nullable: false, defaultValue: null, extra: "auto_increment", key: "PRI" },
        { name: "entry_date", ordinal: 2, dataType: "date", fullType: "date", nullable: false, defaultValue: null, extra: "", key: "" },
      ],
      indexes: [{ name: "PRIMARY", columns: ["id"], unique: true, primary: true }],
      foreignKeys: [],
      ddl: "CREATE TABLE users (id BIGINT UNSIGNED PRIMARY KEY, entry_date DATE NOT NULL)",
    });
    const execute = vi.spyOn(store, "execute").mockResolvedValue(page);
    const mutateRow = vi.spyOn(store, "mutateRow").mockResolvedValue({ affectedRows: 1, concurrentChange: false });
    vi.spyOn(store, "beginTransaction").mockResolvedValue(true);
    vi.spyOn(store, "commitTransaction").mockResolvedValue(true);
    await nextTick();

    host.querySelector<HTMLButtonElement>('.window-primary-actions .window-tool[aria-label="打开 SQL"]')!.click();
    await vi.waitFor(() => expect(host.querySelector("[data-testid='sql-editor']")).not.toBeNull());
    expect(host.querySelector("[data-testid='sql-editor']")?.getAttribute("data-model-value")).toBe("SELECT * FROM users");

    host.querySelector<HTMLButtonElement>(".execute-button")!.click();
    await vi.waitFor(() => expect(host.querySelector("#query-result-panel .data-grid")).not.toBeNull());
    await vi.waitFor(() => expect(tableDetail).toHaveBeenCalledWith(connection.id, "demo", "users"));
    await vi.waitFor(() => expect(host.querySelector(".result-card.editable-query-result")).not.toBeNull());
    expect(execute).toHaveBeenCalledWith(
      expect.any(String),
      "SELECT * FROM users\nLIMIT 501 OFFSET 0;",
      false, 0, 500,
    );
    expect(host.querySelector("[data-testid='sql-editor']")).not.toBeNull();
    expect(host.querySelector(".result-card")?.classList.contains("table-data-view")).toBe(false);
    expect(host.querySelector("#query-result-panel .data-grid")?.getAttribute("aria-label")).toBe("查询结果数据");
    expect(host.querySelector(".result-toolbar-actions")?.textContent).toContain("2 / 2 行 · 4 ms");
    expect(host.querySelector('.result-card input[aria-label="选择第 1 行"]')).not.toBeNull();
    expect(host.querySelector(".result-card .table-toolbar")).toBeNull();
    expect(Array.from(host.querySelectorAll<HTMLButtonElement>(".result-card .query-edit-actions button"), (button) => button.textContent?.trim())).toEqual(["新增", "删除"]);

    Array.from(host.querySelectorAll<HTMLElement>(".result-card.editable-query-result th.data-column"))
      .find((header) => header.textContent?.includes("entry_date"))!.click();
    await nextTick();
    const input = host.querySelector<HTMLInputElement>(".result-card .column-direct-edit-input")!;
    input.value = "2026-08-10";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));

    await vi.waitFor(() => expect(mutateRow).toHaveBeenCalledTimes(2));
    const sessionId = execute.mock.calls[0]![0]!;
    expect(mutateRow).toHaveBeenNthCalledWith(1, sessionId, {
      database: "demo",
      table: "users",
      kind: "update",
      values: [["entry_date", { kind: "date", value: "2026-08-10" }]],
      keyValues: [["id", { kind: "unsigned", value: "1" }]],
      originalValues: [
        ["id", { kind: "unsigned", value: "1" }],
        ["entry_date", { kind: "date", value: "2026-08-01" }],
      ],
    });
    await vi.waitFor(() => expect(host.querySelector(".column-direct-edit-input")).toBeNull());

    host.querySelector<HTMLElement>('.result-card.editable-query-result tbody td[data-grid-row="0"][data-grid-column="1"]')!.click();
    await nextTick();
    expect(host.querySelector<HTMLInputElement>('.result-card .inline-cell-input[data-column="entry_date"]')).not.toBeNull();

    assess.mockRestore();
    tableDetail.mockRestore();
    app.unmount();
  });

  it("offers table group actions from the context menu", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    store.connectionInfo[connection.id] = { serverVersion: "8.0", connectionId: 1, currentDatabase: "demo" };
    store.databases = [{ name: "demo" }];
    store.selectedDatabase = "demo";
    store.tables = [{ database: "demo", name: "users", tableType: "BASE TABLE" }];
    vi.spyOn(store, "connect").mockResolvedValue();
    vi.spyOn(store, "selectDatabase").mockImplementation(async (database) => {
      store.selectedDatabase = database;
    });
    const loadTables = vi.spyOn(store, "loadTables").mockResolvedValue();
    const execute = vi.spyOn(store, "execute").mockResolvedValue({
      executionId: crypto.randomUUID(),
      columns: [],
      rows: [],
      affectedRows: 0,
      executionTimeMs: 2,
      truncated: false,
      hasMore: false,
      resultSetIndex: 0,
      messages: [],
    });
    await nextTick();

    host.querySelector<HTMLButtonElement>(".connection-toggle")!.click();
    await nextTick();
    host.querySelector<HTMLElement>(".database-node")!.click();
    await nextTick();

    const tableGroup = host.querySelector<HTMLButtonElement>(".object-group-title")!;
    tableGroup.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 30, clientY: 40 }));
    await nextTick();

    let menu = host.querySelector<HTMLElement>('[data-testid="context-menu"]')!;
    expect(menu.querySelector(".context-menu-title")?.textContent).toBe("demo · 表");
    expect(Array.from(menu.querySelectorAll("button")).map((button) => button.textContent?.trim())).toEqual([
      "新建表",
      "新建视图",
      "新建查询",
      "展开表分组",
      "刷新表列表",
    ]);

    Array.from(menu.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent?.includes("刷新表列表"))!.click();
    await nextTick();
    expect(loadTables).toHaveBeenCalledWith("", false);
    expect(host.querySelectorAll(".object-node")).toHaveLength(1);

    tableGroup.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await nextTick();
    menu = host.querySelector<HTMLElement>('[data-testid="context-menu"]')!;
    Array.from(menu.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent?.includes("新建表"))!.click();
    await nextTick();

    expect(host.querySelector(".create-table-editor")).not.toBeNull();
    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(1);
    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("新建表");
    expect(host.querySelector('[data-testid="sql-editor"]')).toBeNull();
    const tableName = host.querySelector<HTMLInputElement>('input[aria-label="表名"]')!;
    tableName.value = "audit_log";
    tableName.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();

    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("audit_log");
    tableGroup.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await nextTick();
    menu = host.querySelector<HTMLElement>('[data-testid="context-menu"]')!;
    Array.from(menu.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent?.includes("新建查询"))!.click();
    await nextTick();
    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(2);
    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("无标题@本地 MySQL");

    Array.from(host.querySelectorAll<HTMLButtonElement>(".workspace-tab")).find((tab) => tab.textContent?.includes("audit_log"))!.click();
    await nextTick();
    expect(host.querySelector<HTMLInputElement>('input[aria-label="表名"]')?.value).toBe("audit_log");

    host.querySelector<HTMLFormElement>(".create-table-editor")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    await Promise.resolve();
    await nextTick();
    await Promise.resolve();
    await nextTick();

    expect(execute).toHaveBeenCalledWith(
      expect.any(String),
      "CREATE TABLE `demo`.`audit_log` (\n  `id` BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;",
      true,
    );
    await vi.waitFor(() => expect(host.querySelector(".create-table-editor")).toBeNull());
    expect(loadTables).toHaveBeenLastCalledWith("", false);
    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(1);
    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("无标题@本地 MySQL");
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="查询数据库"]')?.dataset.value).toBe("demo");
    app.unmount();
  });

  it("opens the visual table editor for SQLite and submits SQLite DDL", async () => {
    const { app, host, store } = mountApp();
    const connection = sqliteProfile();
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    store.connectionInfo[connection.id] = { serverVersion: "3.46", connectionId: 1, currentDatabase: "main" };
    store.databases = [{ name: "main" }];
    store.selectedDatabase = "main";
    vi.spyOn(store, "connect").mockResolvedValue();
    vi.spyOn(store, "selectDatabase").mockImplementation(async (database) => {
      store.selectedDatabase = database;
    });
    const loadTables = vi.spyOn(store, "loadTables").mockResolvedValue();
    const beginTransaction = vi.spyOn(store, "beginTransaction").mockResolvedValue(true);
    const commitTransaction = vi.spyOn(store, "commitTransaction").mockResolvedValue(true);
    const rollbackTransaction = vi.spyOn(store, "rollbackTransaction").mockResolvedValue(true);
    const execute = vi.spyOn(store, "execute").mockResolvedValue({
      executionId: crypto.randomUUID(),
      columns: [],
      rows: [],
      affectedRows: 0,
      executionTimeMs: 2,
      truncated: false,
      hasMore: false,
      resultSetIndex: 0,
      messages: [],
    });
    await nextTick();

    host.querySelector<HTMLButtonElement>(".connection-toggle")!.click();
    await nextTick();
    host.querySelector<HTMLElement>(".database-node")!.click();
    await nextTick();

    const tableGroup = host.querySelector<HTMLButtonElement>(".object-group-title")!;
    tableGroup.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await nextTick();
    const menu = host.querySelector<HTMLElement>('[data-testid="context-menu"]')!;
    Array.from(menu.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.includes("新建表"))!
      .click();
    await nextTick();

    expect(host.querySelector(".create-table-editor")).not.toBeNull();
    expect(host.querySelector('[data-testid="sql-editor"]')).toBeNull();
    expect(host.querySelector<HTMLButtonElement>('[aria-label="字段 id 的类型"]')?.dataset.value).toBe("INTEGER");
    const tableName = host.querySelector<HTMLInputElement>('input[aria-label="表名"]')!;
    tableName.value = "audit_log";
    tableName.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    Array.from(host.querySelectorAll<HTMLButtonElement>('[role="tab"]'))
      .find((tab) => tab.textContent === "索引")!
      .click();
    await nextTick();
    Array.from(host.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.includes("添加索引"))!
      .click();
    await nextTick();

    host.querySelector<HTMLFormElement>(".create-table-editor")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(execute).toHaveBeenCalledWith(
      expect.any(String),
      'CREATE TABLE "main"."audit_log" (\n  "id" INTEGER PRIMARY KEY AUTOINCREMENT\n);\nCREATE INDEX "main"."idx_audit_log_1" ON "audit_log" ("id");',
      true,
    ));
    expect(beginTransaction).toHaveBeenCalledWith(expect.any(String));
    expect(commitTransaction).toHaveBeenCalledWith(expect.any(String));
    expect(rollbackTransaction).not.toHaveBeenCalled();
    await vi.waitFor(() => expect(host.querySelector(".create-table-editor")).toBeNull());
    expect(loadTables).toHaveBeenLastCalledWith("", false);
    app.unmount();
  });

  it("offers complete context menus for every database object group", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    store.connectionInfo[connection.id] = { serverVersion: "8.0", connectionId: 1, currentDatabase: "demo" };
    store.databases = [{ name: "demo" }];
    store.selectedDatabase = "demo";
    vi.spyOn(store, "connect").mockResolvedValue();
    vi.spyOn(store, "selectDatabase").mockImplementation(async (database) => { store.selectedDatabase = database; });
    const loadTables = vi.spyOn(store, "loadTables").mockResolvedValue();
    const loadDatabaseObjects = vi.spyOn(store, "loadDatabaseObjects").mockResolvedValue();
    const execute = vi.spyOn(store, "execute").mockResolvedValue({
      executionId: crypto.randomUUID(), columns: [], rows: [], affectedRows: 0, executionTimeMs: 2,
      truncated: false, hasMore: false, resultSetIndex: 0, messages: [],
    });
    await nextTick();

    host.querySelector<HTMLButtonElement>(".connection-toggle")!.click();
    await nextTick();
    host.querySelector<HTMLElement>(".database-node")!.click();
    await nextTick();

    const groupTitles = Array.from(host.querySelectorAll<HTMLButtonElement>(".object-group-title"));
    expect(groupTitles.map((button) => button.querySelector("span")?.textContent)).toEqual(["表", "视图", "函数", "事件", "触发器"]);
    groupTitles.forEach((button) => button.click());
    await nextTick();
    expect(host.querySelector(".group-empty")).toBeNull();
    expect(host.querySelector(".database-children")?.textContent).not.toContain("暂无");
    groupTitles.forEach((button) => button.click());
    await nextTick();

    const group = (label: string) => Array.from(host.querySelectorAll<HTMLButtonElement>(".object-group-title"))
      .find((button) => button.querySelector("span")?.textContent === label)!;
    const openGroupMenu = async (label: string) => {
      group(label).dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
      await nextTick();
      return host.querySelector<HTMLElement>('[data-testid="context-menu"]')!;
    };
    const buttonLabels = (menu: HTMLElement) => Array.from(menu.querySelectorAll("button")).map((button) => button.textContent?.trim());

    expect(buttonLabels(await openGroupMenu("视图"))).toEqual(["新建视图", "新建查询", "展开视图", "刷新视图列表"]);
    expect(buttonLabels(await openGroupMenu("函数"))).toEqual(["新建函数", "新建存储过程", "新建查询", "展开函数", "刷新函数列表"]);
    expect(buttonLabels(await openGroupMenu("触发器"))).toEqual(["新建触发器", "新建查询", "展开触发器", "刷新触发器列表"]);
    expect(buttonLabels(await openGroupMenu("事件"))).toEqual(["新建事件", "新建查询", "展开事件", "刷新事件列表"]);

    let menu = await openGroupMenu("视图");
    Array.from(menu.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent?.includes("新建视图"))!.click();
    await nextTick();
    expect(host.querySelector(".dialog-backdrop")).toBeNull();
    expect(host.querySelector(".database-object-view > .database-object-editor")).not.toBeNull();
    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("新建视图");
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="对象类型"]')?.dataset.value).toBe("view");
    const objectName = host.querySelector<HTMLInputElement>(".object-editor-form input")!;
    objectName.value = "active_users";
    objectName.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();

    menu = await openGroupMenu("视图");
    Array.from(menu.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent?.includes("新建视图"))!.click();
    await nextTick();
    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(1);
    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("active_users");
    expect(host.querySelector<HTMLInputElement>(".object-editor-form input")?.value).toBe("active_users");

    Array.from(host.querySelectorAll<HTMLButtonElement>(".database-object-editor footer button"))
      .find((button) => button.textContent?.includes("创建对象"))!.click();
    await vi.waitFor(() => expect(execute).toHaveBeenCalledTimes(1));
    expect(execute).toHaveBeenCalledWith(expect.any(String), expect.stringContaining("CREATE OR REPLACE VIEW `demo`.`active_users`"), true);
    expect(loadTables).toHaveBeenCalledWith("", false);
    expect(loadDatabaseObjects).toHaveBeenCalledWith("demo");
    await vi.waitFor(() => expect(host.querySelector(".workspace-tab.active")?.textContent?.startsWith("新建视图")).toBe(false));
    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("视图 · active_users");

    host.querySelector<HTMLButtonElement>('.window-primary-actions .window-tool[aria-label="新建查询"]')!.click();
    await nextTick();
    Array.from(host.querySelectorAll<HTMLButtonElement>(".workspace-tab"))
      .find((tab) => tab.textContent?.includes("active_users"))!.click();
    await nextTick();
    expect(host.querySelector<HTMLInputElement>(".object-editor-form input")?.value).toBe("active_users");
    host.querySelector<HTMLElement>(".workspace-tab-shell.active .tab-close")!.click();
    await nextTick();

    menu = await openGroupMenu("函数");
    Array.from(menu.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent?.includes("新建函数"))!.click();
    await nextTick();
    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("新建函数");
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="对象类型"]')?.dataset.value).toBe("function");
    host.querySelector<HTMLElement>(".workspace-tab-shell.active .tab-close")!.click();
    await nextTick();

    menu = await openGroupMenu("触发器");
    Array.from(menu.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent?.includes("新建触发器"))!.click();
    await nextTick();
    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("新建触发器");
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="对象类型"]')?.dataset.value).toBe("trigger");
    host.querySelector<HTMLElement>(".workspace-tab-shell.active .tab-close")!.click();
    await nextTick();

    menu = await openGroupMenu("事件");
    Array.from(menu.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent?.includes("新建事件"))!.click();
    await nextTick();
    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("新建事件");
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="对象类型"]')?.dataset.value).toBe("event");
    host.querySelector<HTMLElement>(".workspace-tab-shell.active .tab-close")!.click();
    await nextTick();

    menu = await openGroupMenu("视图");
    Array.from(menu.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent?.includes("刷新视图列表"))!.click();
    await nextTick();
    expect(loadTables).toHaveBeenCalledWith("", false);

    menu = await openGroupMenu("触发器");
    Array.from(menu.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent?.includes("刷新触发器列表"))!.click();
    await nextTick();
    expect(loadDatabaseObjects).toHaveBeenCalledWith("demo");
    app.unmount();
  });

  it("opens an existing database object in a reusable workspace tab", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    store.connectionInfo[connection.id] = { serverVersion: "8.0", connectionId: 1, currentDatabase: "demo" };
    store.databases = [{ name: "demo" }];
    store.selectedDatabase = "demo";
    store.routines = [{ database: "demo", name: "active_user_count", routineType: "FUNCTION", dataType: "BIGINT" }];
    vi.spyOn(store, "connect").mockResolvedValue();
    vi.spyOn(store, "selectDatabase").mockImplementation(async (database) => { store.selectedDatabase = database; });
    const objectDefinition = vi.spyOn(api, "objectDefinition").mockResolvedValue({
      database: "demo",
      name: "active_user_count",
      kind: "function",
      ddl: "CREATE FUNCTION `demo`.`active_user_count`() RETURNS BIGINT RETURN 1",
    });
    await nextTick();

    host.querySelector<HTMLButtonElement>(".connection-toggle")!.click();
    await nextTick();
    host.querySelector<HTMLElement>(".database-node")!.click();
    await nextTick();
    Array.from(host.querySelectorAll<HTMLButtonElement>(".object-group-title"))
      .find((button) => button.querySelector("span")?.textContent === "函数")!.click();
    await nextTick();

    const routine = host.querySelector<HTMLButtonElement>(".metadata-node")!;
    routine.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await nextTick();
    let menu = host.querySelector<HTMLElement>('[data-testid="context-menu"]')!;
    Array.from(menu.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.includes("查看定义"))!.click();
    await vi.waitFor(() => expect(objectDefinition).toHaveBeenCalledTimes(1));
    await nextTick();

    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(1);
    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("函数 · active_user_count");
    expect(host.querySelector(".dialog-backdrop")).toBeNull();
    expect(host.querySelector<HTMLTextAreaElement>(".object-ddl-editor")?.value).toContain("CREATE FUNCTION");

    routine.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await nextTick();
    menu = host.querySelector<HTMLElement>('[data-testid="context-menu"]')!;
    Array.from(menu.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent?.includes("查看定义"))!.click();
    await nextTick();
    expect(objectDefinition).toHaveBeenCalledTimes(1);
    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(1);

    Array.from(host.querySelectorAll<HTMLButtonElement>(".database-object-editor footer button"))
      .find((button) => button.textContent?.includes("在查询 Tab 中审查"))!.click();
    await nextTick();
    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(2);
    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("函数 SQL · active_user_count");
    expect(host.querySelector("[data-testid='sql-editor']")?.getAttribute("data-model-value")).toContain("CREATE FUNCTION");
    expect(Array.from(host.querySelectorAll(".workspace-tab")).some((tab) => tab.textContent?.includes("函数 · active_user_count"))).toBe(true);
    app.unmount();
  });

  it("opens the new connection dialog from the primary toolbar", async () => {
    const { app, host } = mountApp();
    const newButtons = Array.from(host.querySelectorAll<HTMLButtonElement>('button[aria-label="新建连接"]'));

    expect(newButtons).toHaveLength(1);
    expect(host.querySelector(".window-primary-actions")?.textContent).toContain("新建连接");
    for (const button of newButtons) {
      button.click();
      await nextTick();
      expect(host.querySelector('[data-testid="connection-dialog"]')?.textContent).toContain("new");
      host.querySelector<HTMLButtonElement>('[data-testid="close-dialog"]')!.click();
      await nextTick();
    }
    app.unmount();
  });

  it("creates a new query tab from the primary toolbar", async () => {
    const { app, host } = mountApp();
    const toolbarActions = host.querySelector(".window-primary-actions");
    const buttons = Array.from(toolbarActions!.querySelectorAll<HTMLButtonElement>("button"));

    expect(buttons.map((button) => button.textContent?.trim())).toEqual(["新建连接", "新建查询", "打开 SQL", "设置"]);
    expect(buttons.every((button) => button.classList.contains("window-tool-accent"))).toBe(true);
    expect(buttons.map((button) => Array.from(button.classList).find((name) => name.startsWith("tool-")))).toEqual([
      "tool-connection", "tool-query", "tool-file", "tool-settings",
    ]);
    host.querySelector<HTMLButtonElement>('.window-primary-actions .window-tool[aria-label="新建查询"]')!.click();
    await nextTick();

    const tabs = Array.from(host.querySelectorAll<HTMLElement>(".workspace-tab"));
    expect(tabs).toHaveLength(1);
    expect(tabs[0]?.textContent).toContain("无标题@未选择连接");
    expect(tabs[0]?.classList.contains("active")).toBe(true);
    expect(host.querySelector(".editor-card")).not.toBeNull();
    expect(host.querySelector(".editor-toolbar-heading")?.textContent ?? "").not.toContain("无标题查询");
    expect(host.querySelector(".editor-toolbar-context > .query-context-picker")).not.toBeNull();
    expect(host.querySelector('button[aria-label="查询连接"]')).not.toBeNull();
    expect(host.querySelector('button[aria-label="查询数据库"]')).not.toBeNull();
    expect(host.querySelector(".result-resizer")).toBeNull();
    expect(host.querySelector(".result-card")).toBeNull();
    app.unmount();
  });

  it("resizes the query result panel by dragging the horizontal separator", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    store.connectionInfo[connection.id] = { serverVersion: "8.0", connectionId: 1, currentDatabase: "demo" };
    vi.spyOn(api, "assess").mockResolvedValue({ statementKind: "SELECT", risk: "safe", requiresConfirmation: false });
    vi.spyOn(store, "openTabSession").mockResolvedValue(true);
    vi.spyOn(store, "execute").mockResolvedValue(undefined);
    await nextTick();
    host.querySelector<HTMLButtonElement>('.window-primary-actions .window-tool[aria-label="新建查询"]')!.click();
    await nextTick();
    expect(host.querySelector(".result-card")).toBeNull();

    host.querySelector<HTMLButtonElement>(".execute-button")!.click();
    await vi.waitFor(() => expect(host.querySelector(".result-card")).not.toBeNull());

    host.querySelector<HTMLButtonElement>('button[aria-label="关闭结果面板"]')!.click();
    await nextTick();
    expect(host.querySelector(".result-card")).toBeNull();
    expect(host.querySelector(".result-resizer")).toBeNull();

    host.querySelector<HTMLButtonElement>(".execute-button")!.click();
    await vi.waitFor(() => expect(host.querySelector(".result-card")).not.toBeNull());

    const workspace = host.querySelector<HTMLElement>(".workspace-content")!;
    const resizer = host.querySelector<HTMLElement>(".result-resizer")!;
    const resultPanel = host.querySelector<HTMLElement>(".result-card")!;
    Object.defineProperty(workspace, "clientHeight", { configurable: true, value: 807 });
    Object.defineProperty(resultPanel, "getBoundingClientRect", {
      configurable: true,
      value: vi.fn(() => ({ height: 448 })),
    });
    Object.defineProperties(resizer, {
      setPointerCapture: { configurable: true, value: vi.fn() },
      hasPointerCapture: { configurable: true, value: vi.fn(() => false) },
    });

    resizer.dispatchEvent(new PointerEvent("pointerdown", { clientY: 400, pointerId: 1, bubbles: true }));
    resizer.dispatchEvent(new PointerEvent("pointermove", { clientY: 300, pointerId: 1, bubbles: true }));
    resizer.dispatchEvent(new PointerEvent("pointerup", { clientY: 300, pointerId: 1, bubbles: true }));
    await nextTick();

    expect(Number(resizer.getAttribute("aria-valuenow"))).toBeGreaterThan(56);
    expect(resultPanel.style.flexBasis).toBe("548px");
    expect(host.querySelector(".app-shell")?.classList.contains("is-result-resizing")).toBe(false);
    app.unmount();
  });

  it("renders a bounded row window for large wide query results", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    const rowCount = 2_000;
    const columnCount = 80;
    const columns: QueryResultPage["columns"] = Array.from({ length: columnCount }, (_, index) => ({
      name: `column_${index}`,
      databaseType: "BIGINT",
      nullable: false,
      unsigned: true,
      binary: false,
    }));
    const rows: QueryResultPage["rows"] = Array.from({ length: rowCount }, (_, rowIndex) => (
      columns.map((_, columnIndex) => ({ kind: "unsigned", value: `${rowIndex}:${columnIndex}` }))
    ));
    const page: QueryResultPage = {
      executionId: crypto.randomUUID(),
      columns,
      rows,
      affectedRows: 0,
      executionTimeMs: 12,
      truncated: false,
      hasMore: false,
      resultSetIndex: 0,
      messages: [],
    };
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    store.connectionInfo[connection.id] = { serverVersion: "8.0", connectionId: 1, currentDatabase: "demo" };
    const assess = vi.spyOn(api, "assess").mockResolvedValue({ statementKind: "SELECT", risk: "safe", requiresConfirmation: false });
    const execute = vi.spyOn(store, "execute").mockResolvedValue(page);
    await nextTick();

    host.querySelector<HTMLButtonElement>('.window-primary-actions .window-tool[aria-label="新建查询"]')!.click();
    await nextTick();
    host.querySelector<HTMLButtonElement>(".execute-button")!.click();
    await vi.waitFor(() => expect(host.querySelector("#query-result-panel .data-grid")).not.toBeNull());

    const scroll = host.querySelector<HTMLElement>("#query-result-panel .grid-scroll")!;
    Object.defineProperty(scroll, "clientHeight", { configurable: true, value: 240 });
    scroll.dispatchEvent(new Event("scroll"));
    await nextTick();

    const grid = host.querySelector<HTMLTableElement>("#query-result-panel .data-grid")!;
    expect(grid.getAttribute("aria-rowcount")).toBe(String(rowCount));
    expect(grid.getAttribute("aria-colcount")).toBe(String(columnCount));
    expect(grid.querySelectorAll("tbody tr.data-row").length).toBeLessThan(30);
    expect(grid.querySelectorAll('tbody td[role="gridcell"]').length).toBeLessThan(columnCount * 30);

    const firstCell = grid.querySelector<HTMLElement>('td[data-grid-row="0"][data-grid-column="0"]')!;
    firstCell.focus();
    firstCell.dispatchEvent(new KeyboardEvent("keydown", { key: "End", ctrlKey: true, bubbles: true, cancelable: true }));
    await vi.waitFor(() => {
      const activeCell = document.activeElement as HTMLElement;
      expect(activeCell.dataset.gridRow).toBe(String(rowCount - 1));
      expect(activeCell.dataset.gridColumn).toBe(String(columnCount - 1));
    });

    expect(grid.querySelectorAll("tbody tr.data-row").length).toBeLessThan(30);
    expect(grid.querySelector('td[data-grid-row="0"]')).toBeNull();
    expect(grid.querySelector(`tr[aria-rowindex="${rowCount + 1}"] td[data-grid-row="${rowCount - 1}"]`)).not.toBeNull();
    expect(scroll.scrollTop).toBeGreaterThan(0);

    assess.mockRestore();
    execute.mockRestore();
    app.unmount();
  });

  it("switches between result and summary in a compact result panel", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    const page: QueryResultPage = {
      executionId: crypto.randomUUID(),
      columns: [{ name: "id", databaseType: "BIGINT", nullable: false, unsigned: true, binary: false }],
      rows: [[{ kind: "unsigned", value: "1" }], [{ kind: "unsigned", value: "2" }]],
      affectedRows: 0,
      executionTimeMs: 7,
      truncated: false,
      hasMore: false,
      resultSetIndex: 0,
      messages: [{ severity: "warning", code: "01000", message: "测试消息" }],
    };
    store.connections.push(connection);
    store.activeConnectionId = connection.id;
    store.connectionInfo[connection.id] = { serverVersion: "8.0", connectionId: 1, currentDatabase: "demo" };
    vi.spyOn(api, "assess").mockResolvedValue({ statementKind: "SELECT", risk: "safe", requiresConfirmation: false });
    vi.spyOn(store, "execute").mockResolvedValue(page);
    await nextTick();

    host.querySelector<HTMLButtonElement>('.window-primary-actions .window-tool[aria-label="新建查询"]')!.click();
    await nextTick();
    host.querySelector<HTMLButtonElement>(".execute-button")!.click();
    await vi.waitFor(() => expect(host.querySelector("#query-result-panel .data-grid")).not.toBeNull());

    const resultPanel = host.querySelector<HTMLElement>(".result-card")!;
    const viewTabs = Array.from(resultPanel.querySelectorAll<HTMLButtonElement>(".result-view-tabs button"));
    expect(viewTabs.map((button) => button.textContent?.trim())).toEqual(["结果", "摘要"]);
    expect(resultPanel.querySelector("#query-messages-tab")).toBeNull();
    expect(resultPanel.querySelector("#query-messages-panel")).toBeNull();
    expect(resultPanel.style.flexBasis).toContain("38%");

    viewTabs[1]!.click();
    await nextTick();
    expect(resultPanel.querySelector("#query-summary-panel")?.textContent).toContain("执行耗时7 ms");
    expect(resultPanel.querySelector("#query-summary-panel")?.textContent).toContain("返回行数2");
    expect(resultPanel.querySelector(".data-grid")).toBeNull();

    viewTabs[0]!.click();
    await nextTick();
    expect(resultPanel.querySelector("#query-result-panel .data-grid")).not.toBeNull();
    app.unmount();
  });

  it("returns to the workspace entry point after closing the last query", async () => {
    const { app, host } = mountApp();
    host.querySelector<HTMLButtonElement>('.window-primary-actions .window-tool[aria-label="新建查询"]')!.click();
    await nextTick();

    host.querySelector<HTMLElement>(".tab-close")!.click();
    await vi.waitFor(() => expect(host.querySelectorAll(".workspace-tab")).toHaveLength(0));
    expect(host.querySelector(".workspace-tabs")).toBeNull();
    expect(host.querySelector(".workspace")).toBeNull();
    expect(host.querySelector(".navigation-empty")).not.toBeNull();
    app.unmount();
  });

  it("selects the connection and database for a new query", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    store.connections.push(connection);
    const connect = vi.spyOn(store, "connect").mockImplementation(async (connectionId) => {
      store.activeConnectionId = connectionId;
      store.databases = [{ name: "demo" }, { name: "analytics" }];
    });
    const selectDatabase = vi.spyOn(store, "selectDatabase").mockImplementation(async (database) => {
      store.selectedDatabase = database;
    });
    await nextTick();

    host.querySelector<HTMLButtonElement>('.window-primary-actions .window-tool[aria-label="新建查询"]')!.click();
    await nextTick();

    const connectionSelect = host.querySelector<HTMLButtonElement>('button[aria-label="查询连接"]')!;
    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("无标题@未选择连接");
    connectionSelect.click();
    await nextTick();
    const connectionOption = Array.from(document.querySelectorAll<HTMLElement>('[role="option"]'))
      .find((option) => option.textContent?.includes("本地 MySQL"))!;
    expect(connectionOption).not.toBeNull();
    connectionOption.click();
    await nextTick();
    await nextTick();

    expect(connect).toHaveBeenCalledWith(connection.id);
    expect(host.querySelector(".workspace-tab.active")?.textContent).toContain("无标题@本地 MySQL");
    expect(host.querySelector<HTMLImageElement>(".query-context-database-icon")?.getAttribute("src")).toBe(mysqlIcon);
    const databaseSelect = host.querySelector<HTMLButtonElement>('button[aria-label="查询数据库"]')!;
    databaseSelect.click();
    await nextTick();
    const databaseOptions = Array.from(document.querySelectorAll<HTMLElement>('[role="option"]'));
    expect(databaseOptions.map((option) => option.textContent?.trim())).toContain("demo");
    databaseOptions.find((option) => option.textContent?.includes("demo"))!.click();
    await nextTick();

    expect(selectDatabase).toHaveBeenCalledWith("demo");
    expect(databaseSelect.dataset.value).toBe("demo");
    app.unmount();
  });

  it("opens the edit dialog for the selected connection", async () => {
    const { app, host, store } = mountApp();
    store.connections.push(profile());
    await nextTick();

    const editButton = host.querySelector<HTMLButtonElement>('button[aria-label="编辑连接"]');
    expect(editButton).not.toBeNull();
    editButton!.click();
    await nextTick();

    expect(host.querySelector('[data-testid="connection-dialog"]')?.textContent).toContain("本地 MySQL");
    app.unmount();
  });

  it("keeps workspace tab actions limited to the current tab", async () => {
    const { app, host } = mountApp();

    expect(host.querySelector(".workspace-tab-actions")).toBeNull();
    const newQuery = host.querySelector<HTMLButtonElement>('.window-primary-actions .window-tool[aria-label="新建查询"]')!;
    newQuery.click();
    await nextTick();

    const tabActions = host.querySelector<HTMLElement>(".workspace-tab-actions")!;
    expect(Array.from(tabActions.querySelectorAll("button")).map((button) => button.getAttribute("aria-label"))).toEqual([
      "固定或取消固定当前标签",
    ]);
    newQuery.click();
    await nextTick();

    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(2);
    expect(Array.from(host.querySelectorAll(".workspace-tab")).every((tab) => tab.textContent?.includes("无标题@未选择连接"))).toBe(true);
    app.unmount();
  });

  it("shows production and read-only connection badges", async () => {
    const { app, host, store } = mountApp();
    const connection = profile();
    connection.production = true;
    connection.readOnly = true;
    store.connections.push(connection);
    await nextTick();

    expect(Array.from(host.querySelectorAll(".connection-badge")).map((badge) => badge.textContent)).toEqual(["生产", "只读"]);
    expect(host.querySelector(".connection-root")?.classList.contains("production")).toBe(true);
    app.unmount();
  });

  it("does not expose a standalone table structure page", async () => {
    const { app, host, store } = mountApp();
    store.tableDetail = {
      table: { database: "demo", name: "users", tableType: "BASE TABLE", estimatedRows: 12 },
      columns: [{ name: "id", ordinal: 1, dataType: "bigint", fullType: "bigint unsigned", nullable: false, key: "PRI" }],
      indexes: [],
      foreignKeys: [],
      ddl: "CREATE TABLE users (id BIGINT PRIMARY KEY)",
    };
    await nextTick();

    expect(host.querySelector(".workspace-tabs")).toBeNull();
    host.querySelector<HTMLButtonElement>('.window-primary-actions .window-tool[aria-label="新建查询"]')!.click();
    await nextTick();

    expect(host.querySelector(".detail-toggle")).toBeNull();
    expect(host.querySelector(".table-structure-view")).toBeNull();
    expect(host.querySelectorAll(".workspace-tab")).toHaveLength(1);
    app.unmount();
  });
});
