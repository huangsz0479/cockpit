import { createApp, h, nextTick } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ColumnMeta } from "@/types";
import ColumnManagerDialog from "./ColumnManagerDialog.vue";

afterEach(() => { document.body.innerHTML = ""; });

describe("ColumnManagerDialog", () => {
  it("normalizes an empty frozen count and drops stale hidden columns", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const apply = vi.fn();
    const columns: ColumnMeta[] = [
      { name: "id", databaseType: "BIGINT", nullable: false, unsigned: false, binary: false },
      { name: "name", databaseType: "VARCHAR", nullable: false, unsigned: false, binary: false },
    ];
    const app = createApp({ render: () => h(ColumnManagerDialog, {
      columns,
      order: ["id", "name"],
      hidden: ["removed_column", "name"],
      frozenCount: 1,
      onApply: apply,
    }) });
    app.mount(host);

    const frozen = host.querySelector<HTMLInputElement>('input[type="number"]')!;
    frozen.value = "";
    frozen.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    Array.from(host.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent === "应用")!.click();

    expect(apply).toHaveBeenCalledWith(["id", "name"], ["name"], 0);
    app.unmount();
  });
});
