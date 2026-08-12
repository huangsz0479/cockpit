import { createApp, h, nextTick } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import ContextMenu from "./ContextMenu.vue";

afterEach(() => {
  document.body.innerHTML = "";
});

describe("ContextMenu", () => {
  it("renders menu content in a body teleport", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const app = createApp({
      render: () => h(ContextMenu, { x: 20, y: 30 }, { default: () => h("button", "打开") }),
    });

    app.mount(host);
    await nextTick();

    expect(document.body.querySelector("[role=menu]")?.textContent).toContain("打开");
    expect(document.activeElement?.textContent).toBe("打开");
    app.unmount();
  });

  it("closes when Escape is pressed", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    let closed = false;
    const app = createApp({
      render: () => h(ContextMenu, { x: 20, y: 30, onClose: () => { closed = true; } }),
    });

    app.mount(host);
    await nextTick();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    expect(closed).toBe(true);
    app.unmount();
  });

  it("supports standard menu arrow navigation", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const close = vi.fn();
    const app = createApp({
      render: () => h(ContextMenu, { x: 20, y: 30, onClose: close }, {
        default: () => [h("button", "打开"), h("button", "复制"), h("button", "删除")],
      }),
    });
    app.mount(host);
    await nextTick();

    const menu = document.body.querySelector<HTMLElement>("[role=menu]")!;
    menu.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    expect(document.activeElement?.textContent).toBe("复制");
    menu.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    expect(document.activeElement?.textContent).toBe("删除");
    menu.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
    expect(close).toHaveBeenCalled();
    app.unmount();
  });
});
