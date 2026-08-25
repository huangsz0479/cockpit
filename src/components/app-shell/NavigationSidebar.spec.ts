import { createApp, h } from "vue";
import { afterEach, describe, expect, it } from "vitest";
import type { ConnectionInfo, ConnectionProfile, DatabaseInfo, DatabaseKind, RuntimeStats, UUID } from "@/types";
import NavigationSidebar from "./NavigationSidebar.vue";

afterEach(() => { document.body.innerHTML = ""; });

function profile(id: UUID, name: string, driverKind: DatabaseKind = "mysql"): ConnectionProfile {
  return {
    id,
    driverKind,
    group: null,
    name,
    host: "127.0.0.1",
    port: 3306,
    username: "root",
    database: null,
    tls: { mode: "disabled" },
    ssh: null,
    connectTimeoutSecs: 5,
    queryTimeoutSecs: 30,
    poolSize: 5,
    readOnly: false,
    production: false,
    color: "#16a085",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  };
}

interface MountOptions {
  expandedConnectionId?: UUID | null;
  expandedDatabase?: string | null;
  filteredDatabases?: DatabaseInfo[];
}

function mountSidebar(
  connections: ConnectionProfile[],
  connectionInfo: Record<UUID, ConnectionInfo>,
  runtimeStats: RuntimeStats | null,
  runtimeStatsState: "loading" | "ready" | "unavailable" = "ready",
  options: MountOptions = {},
) {
  const host = document.createElement("div");
  document.body.append(host);
  const app = createApp({
    render: () => h(NavigationSidebar, {
      connections,
      connectionInfo,
      busy: false,
      selectedDatabase: null,
      activeConnectionId: connections[0]?.id ?? null,
      connectionGroups: connections.length ? [{ name: "默认", connections }] : [],
      filteredDatabases: options.filteredDatabases ?? [],
      filteredBaseTables: [],
      filteredViews: [],
      filteredRoutines: [],
      filteredTriggers: [],
      filteredEvents: [],
      tableHasMore: false,
      selectedTable: null,
      expandedConnectionId: options.expandedConnectionId ?? null,
      expandedDatabase: options.expandedDatabase ?? null,
      navigationWidth: 292,
      minWidth: 240,
      maxWidth: 480,
      runtimeStats,
      runtimeStatsState,
      expandedTableGroup: false,
      expandedViewGroup: false,
      expandedFunctionGroup: false,
      expandedTriggerGroup: false,
      expandedEventGroup: false,
    }),
  });
  app.mount(host);
  return { app, host };
}

describe("NavigationSidebar runtime status", () => {
  it("shows logical connections, tab sessions, and formatted application memory", () => {
    const connections = [profile("connection-1", "主库"), profile("connection-2", "分析库"), profile("connection-3", "归档库")];
    const { app, host } = mountSidebar(connections, {
      "connection-1": { serverVersion: "8.0", connectionId: 1 },
      "connection-2": { serverVersion: "8.0", connectionId: 2 },
    }, {
      openConnectionCount: 2,
      tabSessionCount: 4,
      memoryBytes: 128 * 1024 * 1024,
    });

    expect(host.querySelector('[data-metric="connections"] dd')?.textContent).toBe("2");
    expect(host.querySelector('[data-metric="sessions"] dd')?.textContent).toBe("4");
    expect(host.querySelector('[data-metric="memory"] dd')?.textContent).toBe("128 MB");
    expect(host.querySelector('[data-metric="memory"]')?.getAttribute("title")).toContain("WebView");
    expect(host.querySelector(".navigation-status-heading")?.textContent).toBe("运行状态");
    expect(host.querySelector(".navigation-status-heading small")).toBeNull();
    app.unmount();
  });

  it("keeps the connected count aligned with the visible online indicators", () => {
    const connection = profile("connection-1", "主库");
    const { app, host } = mountSidebar([connection], {}, {
      openConnectionCount: 1,
      tabSessionCount: 0,
      memoryBytes: 128 * 1024 * 1024,
    });

    expect(host.querySelectorAll(".connection-root .status-dot.online")).toHaveLength(0);
    expect(host.querySelector('[data-metric="connections"] dd')?.textContent).toBe("0");
    app.unmount();
  });

  it("uses safe placeholders before sampling and stays hidden in the onboarding view", () => {
    const connection = profile("connection-1", "主库");
    const sampled = mountSidebar([connection], {
      "connection-1": { serverVersion: "8.0", connectionId: 1 },
    }, null, "unavailable");

    expect(sampled.host.querySelector('[data-metric="connections"] dd')?.textContent).toBe("1");
    expect(sampled.host.querySelector('[data-metric="sessions"] dd')?.textContent).toBe("—");
    expect(sampled.host.querySelector('[data-metric="memory"] dd')?.textContent).toBe("—");
    expect(sampled.host.querySelector(".navigation-status")?.textContent).not.toMatch(/NaN|undefined/);
    sampled.app.unmount();

    const onboarding = mountSidebar([], {}, null, "unavailable");
    expect(onboarding.host.querySelector(".navigation-status")).toBeNull();
    onboarding.app.unmount();
  });
});

describe("NavigationSidebar object group visibility", () => {
  function visibleGroupTitles(host: HTMLElement) {
    return Array.from(host.querySelectorAll<HTMLElement>(".object-group-title span")).map((label) => label.textContent);
  }

  function mountExpandedDatabase(driverKind: DatabaseKind) {
    const connection = profile("connection-1", "主库", driverKind);
    return mountSidebar([connection], { "connection-1": { serverVersion: "8.0", connectionId: 1 } }, null, "ready", {
      expandedConnectionId: connection.id,
      expandedDatabase: "db",
      filteredDatabases: [{ name: "db" }],
    });
  }

  it("shows every object group for MySQL", () => {
    const { app, host } = mountExpandedDatabase("mysql");
    expect(visibleGroupTitles(host)).toEqual(["表", "视图", "函数", "事件", "触发器"]);
    app.unmount();
  });

  it("hides events for PostgreSQL", () => {
    const { app, host } = mountExpandedDatabase("postgresql");
    expect(visibleGroupTitles(host)).toEqual(["表", "视图", "函数", "触发器"]);
    app.unmount();
  });

  it("hides routines and events for SQLite", () => {
    const { app, host } = mountExpandedDatabase("sqlite");
    expect(visibleGroupTitles(host)).toEqual(["表", "视图", "触发器"]);
    app.unmount();
  });

  it("renders only the table group for Elasticsearch and labels it as indices", () => {
    const { app, host } = mountExpandedDatabase("elasticsearch");
    expect(visibleGroupTitles(host)).toEqual(["索引"]);
    expect(host.querySelector(".tree-row.object-group-title")?.textContent).toContain("索引");
    app.unmount();
  });
});
