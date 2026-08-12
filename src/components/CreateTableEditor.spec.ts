import { createApp, h, nextTick } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createDefaultTableDefinition } from "@/lib/sql";
import CreateTableEditor from "./CreateTableEditor.vue";

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

function mountEditor(props: Record<string, unknown> = {}) {
  const host = document.createElement("div");
  document.body.append(host);
  const created = vi.fn();
  const updated = vi.fn();
  const app = createApp({
    render: () => h(CreateTableEditor, {
      database: "demo",
      modelValue: createDefaultTableDefinition(),
      onCreate: created,
      "onUpdate:modelValue": updated,
      ...props,
    }),
  });
  app.mount(host);
  return { app, host, created, updated };
}

describe("CreateTableEditor", () => {
  it("creates a table definition from the tab editor", async () => {
    const { app, host, created, updated } = mountEditor();
    const createButton = Array.from(host.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent === "创建表")!;
    expect(createButton.disabled).toBe(true);

    const tableName = host.querySelector<HTMLInputElement>('input[aria-label="表名"]')!;
    const columnName = host.querySelector<HTMLInputElement>('input[aria-label="第 1 个字段名称"]')!;
    const size = host.querySelector<HTMLInputElement>('input[aria-label*="长度或精度"]')!;
    for (const input of [tableName, columnName, size]) {
      expect(input.autocomplete).toBe("off");
      expect(input.getAttribute("spellcheck")).toBe("false");
      expect(input.getAttribute("autocorrect")).toBe("off");
      expect(input.getAttribute("autocapitalize")).toBe("none");
      expect(input.getAttribute("data-gramm")).toBe("false");
    }
    tableName.value = "users";
    tableName.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();

    expect(updated).toHaveBeenLastCalledWith(expect.objectContaining({ name: "users" }));
    expect(createButton.disabled).toBe(false);
    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    expect(created).toHaveBeenCalledWith({
      name: "users",
      columns: [{
        name: "id",
        dataType: "BIGINT",
        size: "",
        unsigned: true,
        nullable: false,
        primaryKey: true,
        autoIncrement: true,
        defaultValue: null,
        comment: "",
      }],
      indexes: [],
      foreignKeys: [],
      checks: [],
      engine: "InnoDB",
      charset: "utf8mb4",
      collation: "",
      comment: "",
      originalName: undefined,
      partitionClause: "",
    });
    app.unmount();
  });

  it("adds fields and reports duplicate names without exposing SQL", async () => {
    const { app, host } = mountEditor();
    const tableName = host.querySelector<HTMLInputElement>('input[aria-label="表名"]')!;
    tableName.value = "users";
    tableName.dispatchEvent(new Event("input", { bubbles: true }));
    Array.from(host.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent?.includes("添加字段"))!.click();
    await nextTick();

    const names = host.querySelectorAll<HTMLInputElement>('input[placeholder="字段名"]');
    expect(names).toHaveLength(2);
    expect(names[1]?.value).toBe("column_1");
    expect(host.querySelectorAll("textarea, pre, code")).toHaveLength(0);

    names[1]!.value = "id";
    names[1]!.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    expect(host.querySelector(".create-table-validation")?.textContent).toContain("字段名不能重复");
    app.unmount();
  });

  it("asks for confirmation before removing a table-design item", async () => {
    const { app, host } = mountEditor();
    const removeButton = host.querySelector<HTMLButtonElement>(".remove-column")!;

    removeButton.click();
    await nextTick();
    expect(document.querySelector(".action-dialog")?.textContent).toContain("删除字段");
    expect(document.querySelector(".action-dialog")?.textContent).toContain("字段“id”");
    expect(host.querySelectorAll('input[placeholder="字段名"]')).toHaveLength(1);
    Array.from(document.querySelectorAll<HTMLButtonElement>(".action-dialog button"))
      .find((button) => button.textContent === "取消")!
      .click();
    await nextTick();

    removeButton.click();
    await nextTick();
    Array.from(document.querySelectorAll<HTMLButtonElement>(".action-dialog button"))
      .find((button) => button.textContent === "从设计中删除")!
      .click();
    await vi.waitFor(() => expect(host.querySelectorAll('input[placeholder="字段名"]')).toHaveLength(0));
    app.unmount();
  });

  it("switches between the fields tab and SQL preview", async () => {
    const { app, host } = mountEditor();
    const tabs = Array.from(host.querySelectorAll<HTMLButtonElement>('[role="tab"]'));
    expect(tabs.map((tab) => tab.textContent)).toEqual(["字段", "索引", "外键", "检查约束", "选项", "SQL 预览"]);
    expect(tabs[0]?.getAttribute("aria-selected")).toBe("true");

    const tableName = host.querySelector<HTMLInputElement>('input[aria-label="表名"]')!;
    tableName.value = "users";
    tableName.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();

    tabs[5]!.click();
    await nextTick();
    expect(tabs[5]?.getAttribute("aria-selected")).toBe("true");
    expect(host.querySelector(".sql-preview-code")?.textContent).toContain("CREATE TABLE `demo`.`users`");
    expect(host.querySelector(".create-table-columns")).toBeNull();

    tabs[0]!.click();
    await nextTick();
    expect(host.querySelector(".create-table-columns")).not.toBeNull();
    app.unmount();
  });

  it("shows SQLite-specific controls and previews SQLite DDL", async () => {
    const { app, host } = mountEditor({
      database: "main",
      databaseKind: "sqlite",
      modelValue: createDefaultTableDefinition("sqlite"),
    });
    const tabs = Array.from(host.querySelectorAll<HTMLButtonElement>('[role="tab"]'));
    expect(tabs.map((tab) => tab.textContent)).toEqual(["字段", "索引", "外键", "检查约束", "SQL 预览"]);

    const headers = Array.from(host.querySelectorAll(".create-table-columns th")).map((header) => header.textContent?.trim());
    expect(headers).not.toContain("长度 / 精度");
    expect(headers).not.toContain("注释");
    expect(headers).not.toContain("UN");
    expect(host.querySelector('[aria-label*="长度或精度"]')).toBeNull();
    expect(host.querySelector('[aria-label*="无符号"]')).toBeNull();

    const typeSelect = host.querySelector<HTMLButtonElement>('[aria-label="字段 id 的类型"]')!;
    expect(typeSelect.dataset.value).toBe("INTEGER");
    typeSelect.click();
    await nextTick();
    expect(Array.from(document.querySelectorAll('[role="option"]')).map((option) => option.textContent?.trim())).toEqual([
      "INTEGER", "REAL", "TEXT", "BLOB", "NUMERIC",
    ]);
    Array.from(document.querySelectorAll<HTMLElement>('[role="option"]'))
      .find((option) => option.textContent?.includes("INTEGER"))!
      .click();
    await nextTick();

    const tableName = host.querySelector<HTMLInputElement>('input[aria-label="表名"]')!;
    tableName.value = "audit_log";
    tableName.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    tabs.at(-1)!.click();
    await nextTick();

    expect(host.querySelector(".sql-preview-code")?.textContent).toBe(
      'CREATE TABLE "main"."audit_log" (\n  "id" INTEGER PRIMARY KEY AUTOINCREMENT\n);',
    );
    expect(host.querySelector(".create-table-editor-footer")?.textContent).toContain("SQLite");
    expect(host.querySelector(".create-table-editor-footer")?.textContent).not.toContain("InnoDB");
    app.unmount();
  });

  it("resizes field columns with the same keyboard controls as the data grid", async () => {
    const { app, host } = mountEditor();
    const nameColumn = host.querySelectorAll<HTMLTableColElement>(".create-table-columns col")[0]!;
    const nameResizer = host.querySelector<HTMLElement>('[aria-label="调整字段名列宽"]')!;
    expect(nameColumn.style.width).toBe("150px");

    nameResizer.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, cancelable: true }));
    await nextTick();
    expect(nameColumn.style.width).toBe("162px");

    nameResizer.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true, cancelable: true }));
    await nextTick();
    expect(nameColumn.style.width).toBe("72px");

    nameResizer.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true }));
    await nextTick();
    expect(nameColumn.style.width).toBe("150px");

    const nameHeader = nameResizer.closest("th")!;
    nameHeader.getBoundingClientRect = vi.fn(() => new DOMRect(0, 0, 150, 36));
    Object.defineProperties(nameResizer, {
      setPointerCapture: { value: vi.fn() },
      hasPointerCapture: { value: vi.fn(() => true) },
      releasePointerCapture: { value: vi.fn() },
    });
    nameResizer.dispatchEvent(new PointerEvent("pointerdown", { pointerId: 1, clientX: 100, bubbles: true, cancelable: true }));
    nameResizer.dispatchEvent(new PointerEvent("pointermove", { pointerId: 1, clientX: 160, bubbles: true }));
    nameResizer.dispatchEvent(new PointerEvent("pointerup", { pointerId: 1, clientX: 160, bubbles: true }));
    await nextTick();
    expect(nameColumn.style.width).toBe("210px");
    app.unmount();
  });

  it("disables creation for a read-only connection", () => {
    const { app, host } = mountEditor({ readOnly: true });
    expect(host.querySelector(".create-table-validation")?.textContent).toContain("只读连接");
    expect(Array.from(host.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent === "创建表")?.disabled).toBe(true);
    app.unmount();
  });
});
