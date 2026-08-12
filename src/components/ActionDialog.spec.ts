import { createApp, h, nextTick } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import ActionDialog from "./ActionDialog.vue";

afterEach(() => { document.body.innerHTML = ""; });

describe("ActionDialog", () => {
  it("validates protected input before confirming", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const confirm = vi.fn();
    const app = createApp({
      render: () => h(ActionDialog, {
        state: {
          id: 1,
          kind: "prompt",
          tone: "warning",
          title: "加密备份",
          message: "设置本次备份使用的密码。",
          confirmLabel: "继续",
          cancelLabel: "取消",
          inputLabel: "备份密码",
          inputType: "password",
          inputRequired: true,
          inputMinLength: 8,
          inputValidationMessage: "密码至少需要 8 个字符",
        },
        onConfirm: confirm,
      }),
    });
    app.mount(host);

    const submit = Array.from(host.querySelectorAll<HTMLButtonElement>("button"))
      .find((button) => button.textContent === "继续")!;
    submit.click();
    await nextTick();
    expect(confirm).not.toHaveBeenCalled();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("至少需要 8 个字符");

    const input = host.querySelector<HTMLInputElement>("input")!;
    input.value = "secure-pass";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    submit.click();
    await nextTick();
    expect(confirm).toHaveBeenCalledWith("secure-pass");
    app.unmount();
  });

  it("uses an alert dialog and explicit destructive action", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const app = createApp({
      render: () => h(ActionDialog, {
        state: {
          id: 2,
          kind: "confirm",
          tone: "danger",
          title: "删除数据库？",
          message: "数据库及其中全部对象会被永久删除。",
          confirmLabel: "继续操作",
          cancelLabel: "取消",
        },
      }),
    });
    app.mount(host);

    expect(host.querySelector('[role="alertdialog"]')).not.toBeNull();
    expect(host.querySelector(".destructive-primary")?.textContent).toBe("继续操作");
    app.unmount();
  });
});
