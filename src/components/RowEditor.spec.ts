import { createApp, h, nextTick } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CellValue, ColumnInfo, ColumnMeta } from "@/types";
import RowEditor from "./RowEditor.vue";

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

const columns: ColumnMeta[] = [
  { name: "id", databaseType: "BIGINT", nullable: false, unsigned: true, binary: false },
  { name: "name", databaseType: "VARCHAR", nullable: false, unsigned: false, binary: false },
  { name: "enabled", databaseType: "BOOLEAN", nullable: false, unsigned: false, binary: false },
  { name: "display_name", databaseType: "VARCHAR", nullable: true, unsigned: false, binary: false },
];

const details: ColumnInfo[] = [
  { name: "id", ordinal: 1, dataType: "bigint", fullType: "bigint unsigned", nullable: false, defaultValue: null, extra: "auto_increment" },
  { name: "name", ordinal: 2, dataType: "varchar", fullType: "varchar(100)", nullable: false, defaultValue: null },
  { name: "enabled", ordinal: 3, dataType: "boolean", fullType: "boolean", nullable: false, defaultValue: "true" },
  { name: "display_name", ordinal: 4, dataType: "varchar", fullType: "varchar(100)", nullable: true, defaultValue: null, generationExpression: "upper(name)" },
];

function mountEditor(props: { mode?: "insert" | "update"; columns?: ColumnMeta[]; details?: ColumnInfo[]; row?: CellValue[] | null; busy?: boolean; error?: string | null } = {}) {
  const host = document.createElement("div");
  document.body.append(host);
  const saved = vi.fn();
  const closed = vi.fn();
  const app = createApp({
    render: () => h(RowEditor, {
      mode: props.mode ?? "update",
      columns: props.columns ?? columns,
      columnDetails: props.details ?? details,
      row: props.row,
      busy: props.busy,
      error: props.error,
      onSave: saved,
      onClose: closed,
    }),
  });
  app.mount(host);
  return { app, host, saved, closed };
}

function input(host: HTMLElement, name: string) {
  return host.querySelector<HTMLInputElement>(`input[aria-label="${name}"]`)!;
}

describe("RowEditor", () => {
  it("only saves changed fields and leaves generated fields out", async () => {
    const { app, host, saved } = mountEditor({
      row: [
        { kind: "unsigned", value: "1" },
        { kind: "text", value: "Alice" },
        { kind: "bool", value: true },
        { kind: "text", value: "ALICE" },
      ],
    });

    expect(host.querySelector('[data-column="display_name"]')).toBeNull();
    expect(host.textContent).toContain("1 个生成字段将由数据库自动计算");
    expect(Array.from(host.querySelectorAll<HTMLInputElement>(".row-value-input > input"), (field) => field.getAttribute("aria-label"))).toEqual(["id", "name", "enabled"]);
    expect(Array.from(host.querySelectorAll<HTMLButtonElement>("footer button"))[1]!.disabled).toBe(true);
    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    expect(saved).not.toHaveBeenCalled();

    input(host, "name").value = "Bob";
    input(host, "name").dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));

    expect(saved).toHaveBeenCalledWith([["name", { kind: "text", value: "Bob" }]]);
    app.unmount();
  });

  it("validates typed values before saving", async () => {
    const { app, host, saved } = mountEditor({
      row: [
        { kind: "unsigned", value: "1" },
        { kind: "text", value: "Alice" },
        { kind: "bool", value: true },
        { kind: "text", value: "ALICE" },
      ],
    });

    input(host, "id").value = "not-a-number";
    input(host, "id").dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    await nextTick();

    expect(saved).not.toHaveBeenCalled();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("id 需要填写整数");
    app.unmount();
  });

  it("uses database defaults when inserting and parses boolean input", async () => {
    const { app, host, saved } = mountEditor({ mode: "insert", row: null });

    expect(host.querySelectorAll<HTMLInputElement>(".default-toggle input:checked")).toHaveLength(2);
    expect(input(host, "id").disabled).toBe(true);
    expect(input(host, "enabled").disabled).toBe(true);
    input(host, "name").value = "Alice";
    input(host, "name").dispatchEvent(new Event("input", { bubbles: true }));
    host.querySelector<HTMLInputElement>('[data-column="enabled"] .default-toggle input')!.click();
    await nextTick();
    input(host, "enabled").value = "false";
    input(host, "enabled").dispatchEvent(new Event("input", { bubbles: true }));
    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));

    expect(saved).toHaveBeenCalledWith([
      ["name", { kind: "text", value: "Alice" }],
      ["enabled", { kind: "bool", value: false }],
    ]);
    app.unmount();
  });

  it("keeps database errors visible and cannot close while saving", () => {
    const { app, host, closed } = mountEditor({ busy: true, error: "更新失败" });

    expect(host.querySelector('[role="alert"]')?.textContent).toBe("更新失败");
    host.querySelector<HTMLElement>(".dialog-backdrop")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    host.querySelector<HTMLButtonElement>('button[aria-label="关闭"]')!.click();
    expect(closed).not.toHaveBeenCalled();
    app.unmount();
  });

  it("uses date pickers and preserves database datetime formatting", async () => {
    const dateColumns: ColumnMeta[] = [
      { name: "birthday", databaseType: "DATE", nullable: false, unsigned: false, binary: false },
      { name: "last_seen", databaseType: "DATETIME", nullable: false, unsigned: false, binary: false },
      { name: "precise_seen", databaseType: "DATETIME(6)", nullable: false, unsigned: false, binary: false },
    ];
    const dateDetails: ColumnInfo[] = [
      { name: "birthday", ordinal: 1, dataType: "date", fullType: "date", nullable: false, defaultValue: null },
      { name: "last_seen", ordinal: 2, dataType: "datetime", fullType: "datetime", nullable: false, defaultValue: null },
      { name: "precise_seen", ordinal: 3, dataType: "datetime", fullType: "datetime(6)", nullable: false, defaultValue: null },
    ];
    const { app, host, saved } = mountEditor({
      columns: dateColumns,
      details: dateDetails,
      row: [
        { kind: "date", value: "2026-08-10" },
        { kind: "date_time", value: "2026-08-10 09:30:45" },
        { kind: "date_time", value: "2026-08-10 09:30:45.123456" },
      ],
    });

    expect(input(host, "birthday").type).toBe("date");
    expect(input(host, "birthday").value).toBe("2026-08-10");
    expect(input(host, "last_seen").type).toBe("datetime-local");
    expect(input(host, "last_seen").value).toBe("2026-08-10T09:30:45");
    expect(input(host, "precise_seen").type).toBe("text");
    expect(input(host, "precise_seen").value).toBe("2026-08-10 09:30:45.123456");

    input(host, "last_seen").value = "2026-08-11T10:45:30";
    input(host, "last_seen").dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));

    expect(saved).toHaveBeenCalledWith([
      ["last_seen", { kind: "date_time", value: "2026-08-11 10:45:30" }],
    ]);
    app.unmount();
  });
});
