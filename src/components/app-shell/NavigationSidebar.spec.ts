import { createApp, h } from "vue";
import { afterEach, describe, expect, it } from "vitest";
import type { ConnectionInfo, ConnectionProfile, RuntimeStats, UUID } from "@/types";
import NavigationSidebar from "./NavigationSidebar.vue";

afterEach(() => { document.body.innerHTML = ""; });

function profile(id: UUID, name: string): ConnectionProfile {
  return {
    id,
    driverKind: "mysql",
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

function mountSidebar(
  connections: ConnectionProfile[],
  connectionInfo: Record<UUID, ConnectionInfo>,
  runtimeStats: RuntimeStats | null,
  runtimeStatsState: "loading" | "ready" | "unavailable" = "ready",
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
      filteredDatabases: [],
      filteredBaseTables: [],
      filteredViews: [],
      filteredRoutines: [],
      filteredTriggers: [],
      filteredEvents: [],
      tableHasMore: false,
      selectedTable: null,
      expandedConnectionId: null,
      expandedDatabase: null,
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
    expect(host.querySelector(".navigation-status-heading")?.textContent).toContain("每 10 秒更新");
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
