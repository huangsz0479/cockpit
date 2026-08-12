import { createApp, h, nextTick } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import TableExportControl from "./TableExportControl.vue";

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("TableExportControl", () => {
  it("composes the shared compact select with the two export actions", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const update = vi.fn();
    const exportPage = vi.fn();
    const exportFull = vi.fn();
    const app = createApp({
      render: () => h(TableExportControl, {
        modelValue: "excel",
        options: [{ value: "csv", label: "CSV" }, { value: "excel", label: "Excel" }],
        fullLabel: "整表",
        "onUpdate:modelValue": update,
        onExportPage: exportPage,
        onExportFull: exportFull,
      }),
    });
    app.mount(host);

    expect(host.querySelector(".app-select-compact")).not.toBeNull();
    expect(host.querySelector(".app-select-segmented")).toBeNull();
    const buttons = Array.from(host.querySelectorAll<HTMLButtonElement>(".table-export-control > button"));
    expect(buttons.map((button) => button.textContent?.trim())).toEqual(["当前页", "整表"]);
    buttons[0]!.click();
    buttons[1]!.click();
    expect(exportPage).toHaveBeenCalledTimes(1);
    expect(exportFull).toHaveBeenCalledTimes(1);

    host.querySelector<HTMLButtonElement>('button[aria-label="导出格式"]')!.click();
    await nextTick();
    Array.from(document.querySelectorAll<HTMLElement>('[role="option"]'))
      .find((option) => option.textContent?.includes("CSV"))!.click();
    await nextTick();
    expect(update).toHaveBeenCalledWith("csv");
    app.unmount();
  });

  it("disables both actions while busy and only the full action when unavailable", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const app = createApp({
      render: () => h(TableExportControl, {
        modelValue: "excel",
        options: [{ value: "excel", label: "Excel" }],
        fullDisabled: true,
      }),
    });
    app.mount(host);

    const buttons = Array.from(host.querySelectorAll<HTMLButtonElement>(".table-export-control > button"));
    expect(buttons[0]!.disabled).toBe(false);
    expect(buttons[1]!.disabled).toBe(true);
    app.unmount();
  });
});
