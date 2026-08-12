import { createApp, h, nextTick, ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import AppSelect, { type SelectValue } from "./AppSelect.vue";

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("AppSelect", () => {
  it("keeps ancestor action menus open when the trigger is clicked", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const parentClick = vi.fn();
    const app = createApp({
      render: () => h("div", { onClick: parentClick }, [
        h(AppSelect, {
          modelValue: "excel",
          options: [{ value: "csv", label: "CSV" }, { value: "excel", label: "Excel" }],
          label: "导出格式",
        }),
      ]),
    });
    app.mount(host);

    host.querySelector<HTMLButtonElement>('button[aria-label="导出格式"]')!.click();
    await nextTick();

    expect(parentClick).not.toHaveBeenCalled();
    expect(document.querySelector('[role="listbox"]')).not.toBeNull();
    app.unmount();
  });

  it("selects numeric and null values without rendering a native select", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const value = ref<SelectValue>(100);
    const changed = vi.fn();
    const app = createApp({
      render: () => h(AppSelect, {
        modelValue: value.value,
        options: [{ value: 100, label: "100 行" }, { value: 500, label: "500 行" }, { value: null, label: "忽略" }],
        label: "测试下拉",
        "onUpdate:modelValue": (nextValue) => { value.value = nextValue; },
        onChange: changed,
      }),
    });
    app.mount(host);

    expect(host.querySelector("select")).toBeNull();
    const button = host.querySelector<HTMLButtonElement>('button[aria-label="测试下拉"]')!;
    expect(button.dataset.value).toBe("100");
    button.click();
    await nextTick();
    expect(document.querySelector('[role="option"][title]')).toBeNull();
    Array.from(document.querySelectorAll<HTMLElement>('[role="option"]'))
      .find((option) => option.textContent?.includes("500 行"))!.click();
    await nextTick();
    expect(value.value).toBe(500);
    expect(changed).toHaveBeenLastCalledWith(500);

    button.click();
    await nextTick();
    Array.from(document.querySelectorAll<HTMLElement>('[role="option"]'))
      .find((option) => option.textContent?.includes("忽略"))!.click();
    await nextTick();
    expect(value.value).toBeNull();
    expect(button.textContent).toContain("忽略");
    app.unmount();
  });
});
