import { afterEach, describe, expect, it } from "vitest";
import { createApp, defineComponent, h, nextTick, ref, vModelText, withDirectives } from "vue";
import { installTextInputProtection, normalizeSpacedAsciiComposition } from "./textInput";

afterEach(() => {
  document.body.innerHTML = "";
});

function expectProtected(element: Element) {
  expect(element.getAttribute("autocorrect")).toBe("off");
  expect(element.getAttribute("autocapitalize")).toBe("none");
  expect(element.getAttribute("spellcheck")).toBe("false");
  expect(element.getAttribute("data-gramm")).toBe("false");
}

function waitForCompositionSettlement() {
  return new Promise<void>((resolve) => setTimeout(resolve, 0));
}

function compositionEvent(type: "compositionend" | "compositionupdate", data: string) {
  const event = new CompositionEvent(type, { bubbles: true, data });
  if (event.data !== data) Object.defineProperty(event, "data", { value: data });
  return event;
}

describe("text input protection", () => {
  it("removes short grouped spacing produced by an IME commit", () => {
    expect(normalizeSpacedAsciiComposition("a s d j a s")).toBe("asdjas");
    expect(normalizeSpacedAsciiComposition("a\u00a0s\u00a0d")).toBe("asd");
    expect(normalizeSpacedAsciiComposition("se le")).toBe("se le");
    expect(normalizeSpacedAsciiComposition("se le ct")).toBe("select");
    expect(normalizeSpacedAsciiComposition("sho w")).toBe("sho w");
    expect(normalizeSpacedAsciiComposition("sho\u00a0w")).toBe("sho\u00a0w");
    expect(normalizeSpacedAsciiComposition("sho ")).toBe("sho");
    expect(normalizeSpacedAsciiComposition(" show")).toBe("show");
    expect(normalizeSpacedAsciiComposition(" show tables ")).toBe("show tables");
    expect(normalizeSpacedAsciiComposition("hello world")).toBe("hello world");
    expect(normalizeSpacedAsciiComposition("show tables")).toBe("show tables");
    expect(normalizeSpacedAsciiComposition("select 1")).toBe("select 1");
    expect(normalizeSpacedAsciiComposition("id in")).toBe("id in");
    expect(normalizeSpacedAsciiComposition("as t")).toBe("as t");
    expect(normalizeSpacedAsciiComposition("正常 空格")).toBe("正常 空格");
    expect(normalizeSpacedAsciiComposition("id in", true)).toBe("id in");
  });

  it("protects existing text controls without replacing credential autocomplete", () => {
    const root = document.createElement("div");
    root.innerHTML = `
      <input type="text">
      <input type="search">
      <input type="password" autocomplete="current-password">
      <textarea></textarea>
      <div contenteditable="true"></div>
      <input type="number">
    `;
    document.body.append(root);

    const dispose = installTextInputProtection(root);
    const protectedElements = root.querySelectorAll("input[type=text], input[type=search], input[type=password], textarea, [contenteditable=true]");
    protectedElements.forEach(expectProtected);
    expect(root.querySelector("input[type=text]")?.getAttribute("autocomplete")).toBe("off");
    expect(root.querySelector("input[type=password]")?.getAttribute("autocomplete")).toBe("current-password");
    expect(root.querySelector("input[type=number]")?.hasAttribute("autocorrect")).toBe(false);
    dispose();
  });

  it("protects text controls added after startup before they receive input", () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    const input = document.createElement("input");
    input.type = "text";
    root.append(input);

    input.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));

    expectProtected(input);
    expect(input.autocomplete).toBe("off");
    dispose();
  });

  it("normalizes a spaced ASCII IME commit without touching surrounding text", async () => {
    const root = document.createElement("div");
    const input = document.createElement("input");
    input.value = "prefix--suffix";
    root.append(input);
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    input.setSelectionRange(8, 8);
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.value = "prefix--a s d j a ssuffix";
    input.dispatchEvent(new InputEvent("input", { bubbles: true, isComposing: true, data: "a s d j a s" }));
    input.dispatchEvent(compositionEvent("compositionend", "a s d j a s"));
    await waitForCompositionSettlement();

    expect(input.value).toBe("prefix--asdjassuffix");
    expect(input.selectionStart).toBe("prefix--asdjas".length);
    dispose();
  });

  it("normalizes a Safari-style final input emitted in a later task", async () => {
    const root = document.createElement("div");
    const input = document.createElement("input");
    root.append(input);
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.dispatchEvent(compositionEvent("compositionend", "a s d j a s"));
    await waitForCompositionSettlement();
    input.value = "a s d j a s";
    input.dispatchEvent(new InputEvent("input", { bubbles: true, data: "a s d j a s" }));

    expect(input.value).toBe("asdjas");
    dispose();
  });

  it("removes the boundary space left when an IME switches to ASCII input", async () => {
    const root = document.createElement("div");
    const input = document.createElement("input");
    root.append(input);
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.value = "sho ";
    input.setSelectionRange(input.value.length, input.value.length);
    input.dispatchEvent(compositionEvent("compositionend", "sho "));
    await waitForCompositionSettlement();

    expect(input.value).toBe("sho");
    expect(input.selectionStart).toBe(3);
    input.setRangeText("w", input.selectionStart ?? 3, input.selectionEnd ?? 3, "end");
    input.dispatchEvent(new InputEvent("input", { bubbles: true, data: "w", inputType: "insertText" }));

    expect(input.value).toBe("show");
    expect(input.selectionStart).toBe(4);
    dispose();
  });

  it("does not treat Ctrl-Space input-source switching as a literal space", async () => {
    const root = document.createElement("div");
    const input = document.createElement("input");
    root.append(input);
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.dispatchEvent(compositionEvent("compositionupdate", "sho"));
    input.dispatchEvent(new KeyboardEvent("keydown", {
      key: " ", code: "Space", ctrlKey: true, bubbles: true,
    }));
    input.value = "sho w";
    input.dispatchEvent(compositionEvent("compositionend", "sho w"));
    await waitForCompositionSettlement();

    expect(input.value).toBe("show");
    dispose();
  });

  it("does not treat an IME candidate Space key as inserted text", async () => {
    const root = document.createElement("div");
    const input = document.createElement("input");
    root.append(input);
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.dispatchEvent(compositionEvent("compositionupdate", "sho"));
    input.dispatchEvent(new KeyboardEvent("keydown", {
      key: " ", code: "Space", keyCode: 229, isComposing: true, bubbles: true,
    }));
    input.value = "sho w";
    input.dispatchEvent(compositionEvent("compositionend", "sho w"));
    await waitForCompositionSettlement();

    expect(input.value).toBe("show");
    dispose();
  });

  it("repairs a direct split when Caps Lock switches the IME to ASCII", async () => {
    const root = document.createElement("div");
    const input = document.createElement("input");
    root.append(input);
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.dispatchEvent(compositionEvent("compositionupdate", "sho"));
    input.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    input.value = "sho w";
    input.dispatchEvent(compositionEvent("compositionend", "sho w"));
    await waitForCompositionSettlement();

    expect(input.value).toBe("show");
    dispose();
  });

  it("keeps a legal single-letter word during normal composition evolution", async () => {
    const root = document.createElement("div");
    const input = document.createElement("input");
    root.append(input);
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.dispatchEvent(compositionEvent("compositionupdate", "table "));
    input.dispatchEvent(new InputEvent("beforeinput", {
      bubbles: true, data: "table t", inputType: "insertCompositionText", isComposing: true,
    }));
    input.value = "table t";
    input.dispatchEvent(compositionEvent("compositionend", "table t"));
    await waitForCompositionSettlement();

    expect(input.value).toBe("table t");
    dispose();
  });

  it("keeps literal composition spaces reported through beforeinput", async () => {
    const root = document.createElement("div");
    const input = document.createElement("input");
    root.append(input);
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", {
      key: " ", code: "Space", isComposing: true, bubbles: true,
    }));
    input.dispatchEvent(new InputEvent("beforeinput", {
      bubbles: true, data: " ", inputType: "insertCompositionText", isComposing: true,
    }));
    input.value = "a s d";
    input.dispatchEvent(compositionEvent("compositionend", "a s d"));
    await waitForCompositionSettlement();

    expect(input.value).toBe("a s d");
    dispose();
  });

  it("keeps an Option-Space non-breaking space typed during composition", async () => {
    const root = document.createElement("div");
    const input = document.createElement("input");
    root.append(input);
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", {
      key: " ", code: "Space", altKey: true, isComposing: true, bubbles: true,
    }));
    input.dispatchEvent(new InputEvent("beforeinput", {
      bubbles: true, data: "\u00a0", inputType: "insertCompositionText", isComposing: true,
    }));
    input.value = "id\u00a0in";
    input.dispatchEvent(compositionEvent("compositionend", "id\u00a0in"));
    await waitForCompositionSettlement();

    expect(input.value).toBe("id\u00a0in");
    dispose();
  });

  it("keeps a normal multi-word ASCII composition without relying on key events", async () => {
    const root = document.createElement("div");
    const input = document.createElement("input");
    root.append(input);
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.value = "show tables";
    input.dispatchEvent(compositionEvent("compositionend", "show tables"));
    await waitForCompositionSettlement();

    expect(input.value).toBe("show tables");
    dispose();
  });

  it("ignores Vue's synthetic input until the browser publishes its final composition value", () => {
    const root = document.createElement("div");
    const input = document.createElement("input");
    root.append(input);
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.dispatchEvent(compositionEvent("compositionend", "sho "));
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.value = "sho w";
    input.dispatchEvent(new InputEvent("input", { bubbles: true, data: "sho w" }));

    expect(input.value).toBe("show");
    dispose();
  });

  it("publishes only the corrected composition value through Vue v-model", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    const modelValue = ref("");
    const updates: string[] = [];
    const app = createApp(defineComponent({
      setup() {
        return () => withDirectives(h("input", {
          "onUpdate:modelValue": (value: string) => {
            modelValue.value = value;
            updates.push(value);
          },
        }), [[vModelText, modelValue.value]]);
      },
    }));
    app.mount(root);
    const input = root.querySelector("input");
    if (!input) throw new Error("Vue input did not mount");
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.value = "sho w";
    input.setSelectionRange(input.value.length, input.value.length);
    input.dispatchEvent(compositionEvent("compositionend", "sho "));
    await nextTick();

    expect(input.value).toBe("show");
    expect(modelValue.value).toBe("show");
    expect(updates).toEqual(["show"]);
    app.unmount();
    dispose();
  });

  it("corrects a late native commit before Vue v-model consumes it", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    const modelValue = ref("");
    const updates: string[] = [];
    const app = createApp(defineComponent({
      setup() {
        return () => withDirectives(h("input", {
          "onUpdate:modelValue": (value: string) => {
            modelValue.value = value;
            updates.push(value);
          },
        }), [[vModelText, modelValue.value]]);
      },
    }));
    app.mount(root);
    const input = root.querySelector("input");
    if (!input) throw new Error("Vue input did not mount");
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.dispatchEvent(compositionEvent("compositionend", "sho "));
    await waitForCompositionSettlement();
    input.value = "sho w";
    input.setSelectionRange(input.value.length, input.value.length);
    input.dispatchEvent(new InputEvent("input", {
      bubbles: true, data: "sho w", inputType: "insertCompositionText",
    }));
    await nextTick();

    expect(input.value).toBe("show");
    expect(modelValue.value).toBe("show");
    expect(updates).toEqual(["show"]);
    app.unmount();
    dispose();
  });

  it("does not publish a partial native composition before a late final input", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    const modelValue = ref("");
    const updates: string[] = [];
    const app = createApp(defineComponent({
      setup() {
        return () => withDirectives(h("input", {
          "onUpdate:modelValue": (value: string) => {
            modelValue.value = value;
            updates.push(value);
          },
        }), [[vModelText, modelValue.value]]);
      },
    }));
    app.mount(root);
    const input = root.querySelector("input");
    if (!input) throw new Error("Vue input did not mount");

    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.value = "sho";
    input.dispatchEvent(compositionEvent("compositionupdate", "sho"));
    input.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    input.dispatchEvent(new InputEvent("input", {
      bubbles: true, data: "sho", inputType: "insertCompositionText", isComposing: true,
    }));
    input.dispatchEvent(compositionEvent("compositionend", "sho "));
    await nextTick();
    expect(updates).toEqual([]);

    input.value = "sho w";
    input.setSelectionRange(input.value.length, input.value.length);
    input.dispatchEvent(new InputEvent("input", {
      bubbles: true, data: "sho w", inputType: "insertCompositionText",
    }));
    await nextTick();

    expect(input.value).toBe("show");
    expect(modelValue.value).toBe("show");
    expect(updates).toEqual(["show"]);
    app.unmount();
    dispose();
  });

  it("keeps intentional spaces entered outside an IME composition", () => {
    const root = document.createElement("div");
    const input = document.createElement("input");
    root.append(input);
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    input.value = "a s d j a s";
    input.dispatchEvent(new InputEvent("input", { bubbles: true, data: "a s d j a s" }));

    expect(input.value).toBe("a s d j a s");
    dispose();
  });

  it("keeps spaces explicitly typed during an IME composition", async () => {
    const root = document.createElement("div");
    const input = document.createElement("input");
    root.append(input);
    document.body.append(root);
    const dispose = installTextInputProtection(root);
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.dispatchEvent(new InputEvent("beforeinput", {
      bubbles: true, data: " ", inputType: "insertCompositionText", isComposing: true,
    }));
    input.value = "a s d";
    input.dispatchEvent(compositionEvent("compositionend", "a s d"));
    await waitForCompositionSettlement();

    expect(input.value).toBe("a s d");
    dispose();
  });
});
