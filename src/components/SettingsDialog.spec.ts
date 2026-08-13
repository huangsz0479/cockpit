import { createApp, h, nextTick } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import SettingsDialog from "./SettingsDialog.vue";

afterEach(() => {
  document.body.innerHTML = "";
});

describe("SettingsDialog", () => {
  it("returns the edited paging and workspace settings", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const save = vi.fn();
    const app = createApp({
      render: () => h(SettingsDialog, {
        initial: {
          queryPageSize: 500,
          tablePageSize: 100,
          showSystemDatabases: false,
          autoSaveWorkspace: true,
          backupIncludeData: true,
        },
        onSave: save,
      }),
    });
    app.mount(host);

    expect(host.textContent).not.toContain("界面主题");
    expect(host.textContent).not.toContain("深色");

    const labels = Array.from(host.querySelectorAll<HTMLLabelElement>("label"));
    const queryPageSize = host.querySelector<HTMLButtonElement>('button[aria-label="查询结果每页"]')!;
    queryPageSize.click();
    await nextTick();
    Array.from(document.querySelectorAll<HTMLElement>('[role="option"]'))
      .find((option) => option.textContent?.includes("1,000 行"))!.click();
    labels.find((label) => label.textContent?.includes("MySQL 系统数据库"))!
      .querySelector<HTMLInputElement>('input[type="checkbox"]')!
      .click();
    await nextTick();
    Array.from(host.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent === "保存设置")!.click();

    expect(save).toHaveBeenCalledWith(expect.objectContaining({
      queryPageSize: 1000,
      showSystemDatabases: true,
    }));
    app.unmount();
  });

  it("shows one settings category at a time", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const app = createApp({
      render: () => h(SettingsDialog, {
        initial: {
          queryPageSize: 500,
          tablePageSize: 100,
          showSystemDatabases: false,
          autoSaveWorkspace: true,
          backupIncludeData: true,
        },
      }),
    });
    app.mount(host);

    const general = host.querySelector<HTMLElement>("#settings-panel-general")!;
    const editor = host.querySelector<HTMLElement>("#settings-panel-editor")!;
    expect(general.style.display).not.toBe("none");
    expect(editor.style.display).toBe("none");

    Array.from(host.querySelectorAll<HTMLButtonElement>(".settings-navigation button"))
      .find((button) => button.textContent?.includes("编辑器"))!
      .click();
    await nextTick();

    expect(general.style.display).toBe("none");
    expect(editor.style.display).not.toBe("none");
    app.unmount();
  });

  it("shows product identity, version and license information", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const app = createApp({
      render: () => h(SettingsDialog, {
        version: "1.2.3",
        initial: {
          queryPageSize: 500,
          tablePageSize: 100,
          showSystemDatabases: false,
          autoSaveWorkspace: true,
          backupIncludeData: true,
        },
      }),
    });
    app.mount(host);

    Array.from(host.querySelectorAll<HTMLButtonElement>(".settings-navigation button"))
      .find((button) => button.textContent?.includes("关于"))!
      .click();
    await nextTick();

    expect(host.querySelector(".settings-about")?.textContent).toContain("Cockpit");
    expect(host.querySelector(".settings-about")?.textContent).toContain("版本 1.2.3");
    expect(host.querySelector(".settings-about")?.textContent).toContain("Apache License 2.0");
    app.unmount();
  });
});
