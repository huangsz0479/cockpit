import { createApp, h } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import sqlIcon from "../../../src-tauri/icons/database/sql.svg";
import WorkspaceTabs from "./WorkspaceTabs.vue";

afterEach(() => { document.body.innerHTML = ""; });

describe("WorkspaceTabs", () => {
  it("keeps close controls separate from tab activation buttons", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const close = vi.fn();
    const app = createApp({ render: () => h(WorkspaceTabs, {
      tabs: [{ id: "query-1", kind: "console", title: "订单查询", closable: true, pinned: false }],
      activeId: "query-1",
      dirtyIds: [],
      canShowTableDetail: false,
      onClose: close,
    }) });
    app.mount(host);

    expect(host.querySelector("button button, button [role='button']")).toBeNull();
    expect(host.querySelector<HTMLImageElement>(".workspace-tab-icon")?.getAttribute("src")).toBe(sqlIcon);
    const closeButton = host.querySelector<HTMLButtonElement>('button[aria-label="关闭标签页 订单查询"]')!;
    closeButton.click();
    expect(close).toHaveBeenCalledWith("query-1");
    app.unmount();
  });

  it("supports standard keyboard navigation between tabs", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const activate = vi.fn();
    const app = createApp({ render: () => h(WorkspaceTabs, {
      tabs: [
        { id: "query-1", kind: "console", title: "查询一", closable: true, pinned: false },
        { id: "query-2", kind: "console", title: "查询二", closable: true, pinned: false },
      ],
      activeId: "query-1",
      dirtyIds: [],
      canShowTableDetail: false,
      onActivate: activate,
    }) });
    app.mount(host);

    const tabs = host.querySelectorAll<HTMLButtonElement>('[role="tab"]');
    tabs[0]!.focus();
    tabs[0]!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    expect(activate).toHaveBeenCalledWith("query-2");
    expect(document.activeElement).toBe(tabs[1]);
    app.unmount();
  });
});
