import { afterEach, describe, expect, it, vi } from "vitest";
import { installDialogAccessibility } from "./dialogAccessibility";

afterEach(() => { document.body.innerHTML = ""; });

describe("dialog accessibility", () => {
  it("moves focus into a modal, traps Tab, and restores focus on close", async () => {
    const opener = document.createElement("button");
    opener.textContent = "打开设置";
    document.body.append(opener);
    opener.focus();
    const cleanup = installDialogAccessibility(document);

    const dialog = document.createElement("section");
    dialog.setAttribute("aria-modal", "true");
    dialog.innerHTML = '<button class="icon-button">关闭</button><input autofocus><button>保存</button>';
    document.body.append(dialog);
    await vi.waitFor(() => expect(document.activeElement).toBe(dialog.querySelector("input")));

    const save = dialog.querySelectorAll("button")[1]!;
    save.focus();
    save.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(dialog.querySelector(".icon-button"));

    dialog.remove();
    await vi.waitFor(() => expect(document.activeElement).toBe(opener));
    cleanup();
  });
});
