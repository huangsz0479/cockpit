import { createApp, h, nextTick } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ColumnMeta } from "@/types";
import BatchEditDialog from "./BatchEditDialog.vue";

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

const columns: ColumnMeta[] = [
  { name: "birthday", databaseType: "DATE", nullable: false, unsigned: false, binary: false },
  { name: "last_seen", databaseType: "DATETIME", nullable: false, unsigned: false, binary: false },
];

function mountDialog() {
  const host = document.createElement("div");
  document.body.append(host);
  const applied = vi.fn();
  const app = createApp({
    render: () => h(BatchEditDialog, {
      columns,
      selectedCount: 2,
      onApply: applied,
    }),
  });
  app.mount(host);
  return { app, host, applied };
}

describe("BatchEditDialog", () => {
  it("uses date pickers and emits database datetime values", async () => {
    const { app, host, applied } = mountDialog();
    const select = host.querySelector<HTMLButtonElement>('button[aria-label="目标字段"]')!;
    const valueInput = () => host.querySelector<HTMLInputElement>('.settings-form label:nth-child(2) input')!;

    expect(valueInput().type).toBe("date");
    valueInput().value = "2026-08-10";
    valueInput().dispatchEvent(new Event("input", { bubbles: true }));
    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    expect(applied).toHaveBeenLastCalledWith("birthday", { kind: "date", value: "2026-08-10" });

    select.click();
    await nextTick();
    Array.from(document.querySelectorAll<HTMLElement>('[role="option"]'))
      .find((option) => option.textContent?.includes("last_seen"))!.click();
    await nextTick();
    expect(valueInput().type).toBe("datetime-local");
    expect(valueInput().value).toBe("");

    valueInput().value = "2026-08-10T14:30:45";
    valueInput().dispatchEvent(new Event("input", { bubbles: true }));
    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    expect(applied).toHaveBeenLastCalledWith("last_seen", {
      kind: "date_time",
      value: "2026-08-10 14:30:45",
    });
    app.unmount();
  });

  it("rejects an empty non-null date", async () => {
    const { app, host, applied } = mountDialog();

    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    await nextTick();

    expect(applied).not.toHaveBeenCalled();
    expect(host.querySelector(".error-banner")?.textContent).toContain("birthday 需要选择日期");
    app.unmount();
  });

});
