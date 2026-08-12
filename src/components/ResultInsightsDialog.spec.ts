import { createApp, h, nextTick } from "vue";
import { afterEach, describe, expect, it } from "vitest";
import type { CellValue, ColumnMeta } from "@/types";
import ResultInsightsDialog from "./ResultInsightsDialog.vue";

afterEach(() => { document.body.innerHTML = ""; });

describe("ResultInsightsDialog", () => {
  it("does not render misleading zero bars before a numeric metric is selected", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const columns: ColumnMeta[] = [
      { name: "region", databaseType: "VARCHAR", nullable: false, unsigned: false, binary: false },
      { name: "revenue", databaseType: "DECIMAL", nullable: false, unsigned: false, binary: false },
    ];
    const rows: CellValue[][] = [
      [{ kind: "text", value: "华东" }, { kind: "decimal", value: "120.5" }],
      [{ kind: "text", value: "华北" }, { kind: "decimal", value: "80" }],
    ];
    const app = createApp({ render: () => h(ResultInsightsDialog, { columns, rows }) });
    app.mount(host);

    const aggregation = host.querySelector<HTMLButtonElement>('button[aria-label="统计方式"]')!;
    aggregation.click();
    await nextTick();
    Array.from(document.querySelectorAll<HTMLElement>('[role="option"]'))
      .find((option) => option.textContent?.includes("求和"))!.click();
    await nextTick();

    expect(host.querySelectorAll(".insight-bar-row")).toHaveLength(0);
    expect(host.querySelector(".dialog-empty-state span")?.textContent).toContain("请选择用于计算的数值字段");

    const metric = host.querySelector<HTMLButtonElement>('button[aria-label="数值字段"]')!;
    metric.click();
    await nextTick();
    Array.from(document.querySelectorAll<HTMLElement>('[role="option"]'))
      .find((option) => option.textContent?.includes("revenue"))!.click();
    await nextTick();
    expect(host.querySelectorAll(".insight-bar-row")).toHaveLength(2);
    app.unmount();
  });
});
