import { createApp, h } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import AppDialog from "./AppDialog.vue";

afterEach(() => { document.body.innerHTML = ""; });

describe("AppDialog", () => {
  it("provides the shared modal structure and form events", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const close = vi.fn();
    const submit = vi.fn();
    const app = createApp({
      render: () => h(AppDialog, {
        title: "统一弹窗",
        titleId: "shared-dialog-title",
        description: "统一的标题和操作区",
        as: "form",
        onClose: close,
        onSubmit: submit,
      }, {
        default: () => h("div", { class: "dialog-body" }, "内容"),
        footer: () => h("button", { class: "primary" }, "保存"),
      }),
    });
    app.mount(host);

    expect(host.querySelector('[role="dialog"]')?.getAttribute("aria-labelledby")).toBe("shared-dialog-title");
    expect(host.querySelector(".app-dialog-heading-icon")).not.toBeNull();
    expect(host.querySelector(".app-dialog-footer")?.textContent).toBe("保存");
    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    expect(submit).toHaveBeenCalledOnce();
    host.querySelector<HTMLElement>(".dialog-backdrop")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    expect(close).toHaveBeenCalledOnce();
    app.unmount();
  });

  it("does not close a locked dialog", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const close = vi.fn();
    const app = createApp({
      render: () => h(AppDialog, {
        title: "处理中",
        titleId: "locked-dialog-title",
        closeDisabled: true,
        onClose: close,
      }),
    });
    app.mount(host);

    host.querySelector<HTMLButtonElement>('.icon-button')!.click();
    host.querySelector<HTMLElement>(".dialog-backdrop")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    expect(close).not.toHaveBeenCalled();
    app.unmount();
  });
});
