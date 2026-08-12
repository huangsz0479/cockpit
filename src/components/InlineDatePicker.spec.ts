import { createApp, nextTick } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import InlineDatePicker from "./InlineDatePicker.vue";

afterEach(() => {
  document.body.innerHTML = "";
});

describe("InlineDatePicker", () => {
  it("uses a teleported floating menu and preserves table keyboard navigation", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const onTab = vi.fn();
    const app = createApp(InlineDatePicker, {
      modelValue: "2026-08-01",
      kind: "date",
      columnName: "entry_date",
      inputLabel: "编辑 entry_date",
      placement: "top-end",
      onTab,
    });
    app.mount(host);
    await nextTick();

    const input = host.querySelector<HTMLInputElement>(".inline-cell-input")!;
    expect(input.type).toBe("text");
    expect(input.value).toBe("2026-08-01");
    expect(input.dataset.column).toBe("entry_date");
    expect(input.getAttribute("aria-label")).toBe("编辑 entry_date");
    expect(host.querySelector(".inline-date-picker")?.getAttribute("data-placement")).toBe("top-end");

    input.click();
    await vi.waitFor(() => expect(document.body.querySelector(".dp--menu-wrapper")).not.toBeNull());
    const floatingMenu = document.body.querySelector<HTMLElement>(".dp--menu-wrapper")!;
    expect(host.contains(floatingMenu)).toBe(false);
    expect(floatingMenu.style.position).toBe("fixed");

    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    expect(onTab).toHaveBeenCalledTimes(1);
    await vi.waitFor(() => expect(document.body.querySelector(".dp--menu-wrapper")).toBeNull());
    app.unmount();
  });

  it("emits the updated datetime when a time control is selected", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const onUpdateModelValue = vi.fn();
    const app = createApp(InlineDatePicker, {
      modelValue: "2026-08-01 10:20:30",
      kind: "datetime-local",
      columnName: "created_at",
      inputLabel: "编辑 created_at",
      "onUpdate:modelValue": onUpdateModelValue,
    });
    app.mount(host);
    await nextTick();

    host.querySelector<HTMLInputElement>(".inline-cell-input")!.click();
    await vi.waitFor(() => expect(document.body.querySelector(".dp--menu-wrapper")).not.toBeNull());
    document.body.querySelector<HTMLButtonElement>('[data-test-id="open-time-picker-btn"]')!.click();
    await vi.waitFor(() => expect(document.body.querySelector(".dp--inc-dec-button")).not.toBeNull());
    const timeControl = document.body.querySelector<HTMLElement>(".dp--inc-dec-button")!;
    timeControl.click();

    await vi.waitFor(() => expect(onUpdateModelValue).toHaveBeenCalled());
    expect(onUpdateModelValue.mock.lastCall?.[0]).toMatch(/^2026-08-01 \d{2}:\d{2}:\d{2}$/);
    expect(onUpdateModelValue.mock.lastCall?.[0]).not.toBe("2026-08-01 10:20:30");
    app.unmount();
  });

  it("emits the selected calendar date", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const onUpdateModelValue = vi.fn();
    const app = createApp(InlineDatePicker, {
      modelValue: "2026-08-01",
      kind: "date",
      columnName: "entry_date",
      inputLabel: "编辑 entry_date",
      "onUpdate:modelValue": onUpdateModelValue,
    });
    app.mount(host);
    await nextTick();

    host.querySelector<HTMLInputElement>(".inline-cell-input")!.click();
    await vi.waitFor(() => expect(document.body.querySelector(".dp--menu-wrapper")).not.toBeNull());
    const targetDate = Array.from(document.body.querySelectorAll<HTMLElement>(".dp--calendar-item"))
      .find((item) => item.textContent?.trim() === "11" && !item.querySelector(".dp--cell-offset"));
    expect(targetDate).toBeDefined();
    const pointerDown = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    targetDate!.dispatchEvent(pointerDown);
    expect(pointerDown.defaultPrevented).toBe(true);
    targetDate!.click();

    await vi.waitFor(() => expect(onUpdateModelValue).toHaveBeenCalledWith("2026-08-11"));
    app.unmount();
  });
});
