import { createApp, h, nextTick, reactive } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ConnectionProfile } from "@/types";
import ConnectionDialog from "./ConnectionDialog.vue";

const { hasConnectionPassword, testConnection } = vi.hoisted(() => ({
  hasConnectionPassword: vi.fn().mockResolvedValue(false),
  testConnection: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  api: {
    hasConnectionPassword,
    testConnection,
  },
}));

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
  hasConnectionPassword.mockResolvedValue(false);
  testConnection.mockReset();
});

describe("ConnectionDialog", () => {
  it("does not provide or rewrite a default connection name", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const app = createApp(ConnectionDialog);
    app.mount(host);

    const nameInput = host.querySelector<HTMLInputElement>('input[autofocus]')!;
    const saveButton = Array.from(host.querySelectorAll("button")).find((button) => button.textContent === "保存连接")!;
    expect(nameInput.value).toBe("");
    expect(nameInput.placeholder).toBe("请输入连接名称");
    expect(saveButton.disabled).toBe(true);

    host.querySelector<HTMLButtonElement>('button[aria-label="数据库类型"]')!.click();
    await nextTick();
    Array.from(document.querySelectorAll<HTMLElement>('[role="option"]'))
      .find((option) => option.textContent?.includes("SQLite"))!.click();
    await nextTick();

    expect(nameInput.value).toBe("");
    app.unmount();
  });

  it("disables smart text features for every free-text field", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const app = createApp(ConnectionDialog);
    app.mount(host);

    const inputs = host.querySelectorAll<HTMLInputElement>('input:not([type="number"]):not([type="checkbox"])');
    expect(inputs.length).toBeGreaterThan(0);
    for (const input of inputs) {
      expect(input.getAttribute("autocorrect")).toBe("off");
      expect(input.getAttribute("autocapitalize")).toBe("none");
      expect(input.getAttribute("spellcheck")).toBe("false");
      expect(input.getAttribute("data-gramm")).toBe("false");
    }
    expect(host.querySelector<HTMLInputElement>('input[autocomplete="username"]')).not.toBeNull();
    expect(host.querySelector<HTMLInputElement>('input[autocomplete="current-password"]')).not.toBeNull();
    app.unmount();
  });

  it("opens with a reactive connection profile from the store", async () => {
    const initial = reactive(profile());
    const host = document.createElement("div");
    document.body.append(host);
    const app = createApp({ render: () => h(ConnectionDialog, { initial }) });

    expect(() => app.mount(host)).not.toThrow();
    await Promise.resolve();
    await nextTick();

    expect(host.querySelector("h2")?.textContent).toBe("编辑连接");
    const nameInput = host.querySelector<HTMLInputElement>('input[autofocus]')!;
    expect(nameInput.value).toBe("本地 MySQL");
    nameInput.value = "只修改弹窗副本";
    nameInput.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    expect(initial.name).toBe("本地 MySQL");
    app.unmount();
  });

  it("keeps low-frequency connection settings collapsed by default", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const app = createApp(ConnectionDialog);
    app.mount(host);

    const options = host.querySelector<HTMLDetailsElement>(".connection-options")!;
    expect(options.open).toBe(false);
    host.querySelector<HTMLElement>(".connection-options summary")!.click();
    expect(options.open).toBe(true);
    app.unmount();
  });

  it("requires a new editable password when the stored credential is missing", async () => {
    const connection = profile();
    const host = document.createElement("div");
    document.body.append(host);
    const saved = vi.fn();
    const app = createApp({ render: () => h(ConnectionDialog, { initial: connection, onSave: saved }) });

    app.mount(host);
    await Promise.resolve();
    await nextTick();

    const passwordInput = host.querySelector<HTMLInputElement>('input[type="password"]')!;
    const saveButton = Array.from(host.querySelectorAll("button")).find((button) => button.textContent === "保存连接")!;
    expect(passwordInput.placeholder).toBe("请输入密码");
    expect(passwordInput.disabled).toBe(false);
    expect(saveButton.disabled).toBe(true);
    expect(host.textContent).toContain("重启后未找到已保存密码，请重新输入");

    passwordInput.value = "secret";
    passwordInput.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();

    expect(saveButton.disabled).toBe(false);
    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    expect(saved).toHaveBeenCalledWith(expect.objectContaining({ id: connection.id }), "secret");
    app.unmount();
  });

  it("uses distinct success and error states for connection tests", async () => {
    testConnection.mockResolvedValueOnce({ serverVersion: "8.0.36", tlsCipher: null });
    const host = document.createElement("div");
    document.body.append(host);
    const app = createApp(ConnectionDialog);
    app.mount(host);

    const name = host.querySelector<HTMLInputElement>('input[autofocus]')!;
    name.value = "测试连接";
    name.dispatchEvent(new Event("input", { bubbles: true }));
    const password = host.querySelector<HTMLInputElement>('input[type="password"]')!;
    password.value = "secret";
    password.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    const testButton = Array.from(host.querySelectorAll("button")).find((button) => button.textContent === "测试连接")!;
    testButton.click();
    await Promise.resolve();
    await nextTick();

    expect(host.querySelector(".test-message.success")?.textContent).toContain("连接成功");

    testConnection.mockRejectedValueOnce(new Error("连接被拒绝"));
    testButton.click();
    await Promise.resolve();
    await nextTick();

    expect(host.querySelector(".test-message.error")?.textContent).toContain("连接被拒绝");
    expect(host.querySelector(".test-message")?.getAttribute("role")).toBe("alert");
    app.unmount();
  });
});

function profile(): ConnectionProfile {
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
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
  };
}
