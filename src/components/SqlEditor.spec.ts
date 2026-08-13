import { completionStatus, currentCompletions, startCompletion } from "@codemirror/autocomplete";
import { EditorView, runScopeHandlers } from "@codemirror/view";
import { createApp, defineComponent, h, nextTick, ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import { installTextInputProtection } from "@/lib/textInput";
import SqlEditor from "./SqlEditor.vue";

const mountedApps: Array<ReturnType<typeof createApp>> = [];
const inputProtectionDisposers: Array<() => void> = [];

afterEach(() => {
  for (const app of mountedApps.splice(0)) app.unmount();
  for (const dispose of inputProtectionDisposers.splice(0)) dispose();
  document.body.innerHTML = "";
});

function mountEditor(initialValue = "", onExecute = vi.fn(), editorProps: {
  schema?: Record<string, readonly string[]>;
  loadTableColumns?: (table: string, database?: string) => Promise<readonly string[]>;
} = {}) {
  const modelValue = ref(initialValue);
  const modelUpdates: string[] = [];
  const host = document.createElement("div");
  document.body.append(host);
  inputProtectionDisposers.push(installTextInputProtection(host));
  const app = createApp(defineComponent({
    setup() {
      return () => h(SqlEditor, {
        modelValue: modelValue.value,
        "onUpdate:modelValue": (value: string) => {
          modelUpdates.push(value);
          modelValue.value = value;
        },
        onExecute,
        ...editorProps,
      });
    },
  }));
  mountedApps.push(app);
  app.mount(host);
  const content = host.querySelector<HTMLElement>(".cm-content");
  const view = content && EditorView.findFromDOM(content);
  if (!view) throw new Error("SQL editor did not mount");
  return { modelValue, modelUpdates, onExecute, view };
}

async function waitForCompositionSettlement() {
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
  await nextTick();
}

async function waitForEditorFallback() {
  await new Promise<void>((resolve) => setTimeout(resolve, 80));
  await nextTick();
}

function compositionEvent(type: "compositionend" | "compositionupdate", data: string) {
  const event = new CompositionEvent(type, { bubbles: true, data });
  if (event.data !== data) Object.defineProperty(event, "data", { value: data });
  return event;
}

describe("SqlEditor", () => {
  it("executes SQL with the platform Mod-Enter shortcut", () => {
    const { onExecute, view } = mountEditor("SELECT 1");
    const isMac = /Mac|iPhone|iPad|iPod/.test(navigator.platform);
    const event = new KeyboardEvent("keydown", {
      key: "Enter",
      code: "Enter",
      keyCode: 13,
      ctrlKey: !isMac,
      metaKey: isMac,
      bubbles: true,
      cancelable: true,
    });

    const handled = runScopeHandlers(view, event, "editor");

    expect(handled).toBe(true);
    expect(onExecute).toHaveBeenCalledOnce();
    expect(view.state.doc.toString()).toBe("SELECT 1");
  });

  it("executes all statements when there is no selection regardless of cursor position", () => {
    const sql = `SELECT
  *
FROM
  bus_card_info t
WHERE
  t.card_id = 1;

SELECT
  *
FROM
  bus_card_info
WHERE
  apn IS NOT NULL
LIMIT
  100;`;
    const { onExecute, view } = mountEditor(sql);
    const isMac = /Mac|iPhone|iPad|iPod/.test(navigator.platform);
    const secondSelect = sql.lastIndexOf("SELECT");
    for (const anchor of [0, sql.indexOf("card_id"), sql.indexOf(";"), secondSelect - 1, secondSelect]) {
      view.dispatch({ selection: { anchor } });
      runScopeHandlers(view, new KeyboardEvent("keydown", {
        key: "Enter", code: "Enter", ctrlKey: !isMac, metaKey: isMac, bubbles: true, cancelable: true,
      }), "editor");
    }
    expect(onExecute).toHaveBeenCalledTimes(5);
    for (const [executedSql] of onExecute.mock.calls) expect(executedSql).toBe(sql);
  });

  it("executes only the selected SQL when text is selected", () => {
    const sql = "SELECT 'a;b';\nSELECT 2;";
    const { onExecute, view } = mountEditor(sql);
    const selectedFrom = sql.indexOf("SELECT 2");
    view.dispatch({ selection: { anchor: selectedFrom, head: sql.length } });
    const isMac = /Mac|iPhone|iPad|iPod/.test(navigator.platform);
    runScopeHandlers(view, new KeyboardEvent("keydown", {
      key: "Enter", code: "Enter", ctrlKey: !isMac, metaKey: isMac, bubbles: true, cancelable: true,
    }), "editor");
    expect(onExecute).toHaveBeenCalledWith("SELECT 2;");
  });

  it("accepts the selected completion with Tab", async () => {
    const { view } = mountEditor("sel");
    view.dispatch({ selection: { anchor: view.state.doc.length } });
    view.focus();
    startCompletion(view);
    await vi.waitFor(() => expect(completionStatus(view.state)).toBe("active"));
    await new Promise((resolve) => setTimeout(resolve, 100));

    const event = new KeyboardEvent("keydown", {
      key: "Tab",
      code: "Tab",
      keyCode: 9,
      bubbles: true,
      cancelable: true,
    });
    const handled = runScopeHandlers(view, event, "editor");

    expect(handled).toBe(true);
    expect(view.state.doc.toString()).toBe("SELECT");
  });

  it("suggests fields from the FROM table in a WHERE expression", async () => {
    const loadTableColumns = vi.fn().mockResolvedValue(["id", "dept_name", "parent_id"]);
    const { view } = mountEditor("SELECT * FROM system_dept WHERE i", vi.fn(), {
      schema: { system_dept: [] },
      loadTableColumns,
    });
    view.dispatch({ selection: { anchor: view.state.doc.length } });
    view.focus();
    startCompletion(view);

    await vi.waitFor(() => expect(completionStatus(view.state)).toBe("active"));
    expect(currentCompletions(view.state).map((completion) => completion.label)).toContain("id");
    expect(loadTableColumns).toHaveBeenCalledWith("system_dept", undefined);
  });

  it("uses already-known fields without reloading the table", async () => {
    const loadTableColumns = vi.fn();
    const { view } = mountEditor("SELECT * FROM system_dept WHERE dept_", vi.fn(), {
      schema: { system_dept: ["id", "dept_name"] },
      loadTableColumns,
    });
    view.dispatch({ selection: { anchor: view.state.doc.length } });
    view.focus();
    startCompletion(view);

    await vi.waitFor(() => expect(completionStatus(view.state)).toBe("active"));
    expect(currentCompletions(view.state).map((completion) => completion.label)).toContain("dept_name");
    expect(loadTableColumns).not.toHaveBeenCalled();
  });

  it("limits qualified field suggestions to the matching table alias", async () => {
    const { view } = mountEditor(
      "SELECT * FROM users u JOIN teams t ON t.id = u.team_id WHERE t.team_",
      vi.fn(),
      { schema: { users: ["team_user_name"], teams: ["team_name"] } },
    );
    view.dispatch({ selection: { anchor: view.state.doc.length } });
    view.focus();
    startCompletion(view);

    await vi.waitFor(() => expect(completionStatus(view.state)).toBe("active"));
    const labels = currentCompletions(view.state).map((completion) => completion.label);
    expect(labels).toContain("team_name");
    expect(labels).not.toContain("team_user_name");
  });

  it("automatically opens field suggestions immediately after an alias dot", async () => {
    const loadTableColumns = vi.fn().mockResolvedValue(["id", "dict_label", "dict_value"]);
    const sql = "SELECT * FROM system_dict_data t WHERE t";
    const { view } = mountEditor(sql, vi.fn(), {
      schema: { system_dict_data: [] },
      loadTableColumns,
    });
    view.dispatch({ selection: { anchor: view.state.doc.length } });
    view.focus();
    view.dispatch({
      changes: { from: view.state.doc.length, insert: "." },
      selection: { anchor: view.state.doc.length + 1 },
      userEvent: "input.type",
    });

    await vi.waitFor(() => expect(completionStatus(view.state)).toBe("active"));
    expect(currentCompletions(view.state).map((completion) => completion.label)).toEqual(
      expect.arrayContaining(["id", "dict_label", "dict_value"]),
    );
    expect(loadTableColumns).toHaveBeenCalledWith("system_dict_data", undefined);
  });

  it("inserts a tab at the current cursor position", () => {
    const { view } = mountEditor("SELECT 1");
    view.dispatch({ selection: { anchor: "SELECT".length } });

    const handled = runScopeHandlers(view, new KeyboardEvent("keydown", {
      key: "Tab", code: "Tab", bubbles: true, cancelable: true,
    }), "editor");

    expect(handled).toBe(true);
    expect(view.state.doc.toString()).toBe("SELECT\t 1");
    expect(view.state.selection.main.head).toBe("SELECT\t".length);
  });

  it("uses Tab and Shift-Tab to indent and unindent SQL", () => {
    const sql = "SELECT *\nFROM users";
    const { view } = mountEditor(sql);
    view.dispatch({ selection: { anchor: 0, head: view.state.doc.length } });

    const tabHandled = runScopeHandlers(view, new KeyboardEvent("keydown", {
      key: "Tab", code: "Tab", bubbles: true, cancelable: true,
    }), "editor");

    expect(tabHandled).toBe(true);
    expect(view.state.doc.toString()).toBe("  SELECT *\n  FROM users");

    view.dispatch({ selection: { anchor: 0, head: view.state.doc.length } });
    const shiftTabHandled = runScopeHandlers(view, new KeyboardEvent("keydown", {
      key: "Tab", code: "Tab", shiftKey: true, bubbles: true, cancelable: true,
    }), "editor");

    expect(shiftTabHandled).toBe(true);
    expect(view.state.doc.toString()).toBe(sql);
  });

  it("uses a translucent active-line background so a single-line selection stays visible", () => {
    const { view } = mountEditor("SELECT * FROM users");
    const activeLine = view.contentDOM.querySelector<HTMLElement>(".cm-activeLine");
    const editorStyles = Array.from(document.querySelectorAll("style"), (style) => style.textContent).join("\n");

    expect(activeLine).not.toBeNull();
    expect(editorStyles).toContain("rgba(226, 232, 240, 0.25)");
  });

  it("styles the completion menu without changing its row height", () => {
    mountEditor("SELECT * FROM system_dict_data t WHERE t.", vi.fn(), {
      schema: { system_dict_data: ["id", "dict_label"] },
    });
    const editorStyles = Array.from(document.querySelectorAll("style"), (style) => style.textContent).join("\n");

    expect(editorStyles).toContain("border-radius: var(--radius-md)");
    expect(editorStyles).toContain("box-shadow: var(--shadow-md)");
    expect(editorStyles).toContain("padding: 1px 8px");
    expect(editorStyles).toContain("line-height: 1.2");
    expect(editorStyles).toContain("font-style: normal");
  });

  it("disables native smart text features on the editable content", () => {
    const { view } = mountEditor();

    expect(view.contentDOM.getAttribute("autocomplete")).toBe("off");
    expect(view.contentDOM.getAttribute("autocorrect")).toBe("off");
    expect(view.contentDOM.getAttribute("autocapitalize")).toBe("none");
    expect(view.contentDOM.getAttribute("spellcheck")).toBe("false");
    expect(view.contentDOM.getAttribute("data-gramm")).toBe("false");
  });

  it("keeps IME composition changes in sync with v-model", async () => {
    const { modelValue, view } = mountEditor();

    view.dispatch({ changes: { from: 0, insert: "select" }, userEvent: "input.type.compose" });
    await nextTick();

    expect(modelValue.value).toBe("select");
    expect(view.state.doc.toString()).toBe("select");
  });

  it("normalizes per-character spacing when an ASCII IME composition is committed", async () => {
    const { modelValue, view } = mountEditor("SELECT ");
    view.dispatch({ selection: { anchor: view.state.doc.length } });
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.dispatch({
      changes: { from: view.state.doc.length, insert: "a s d j a s" },
      selection: { anchor: view.state.doc.length + "a s d j a s".length },
      userEvent: "input.type.compose",
    });
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "a s d j a s"));
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("SELECT asdjas");
    expect(modelValue.value).toBe("SELECT asdjas");
  });

  it("normalizes short grouped spacing without removing explicitly typed SQL spaces", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.dispatch({
      changes: { from: 0, insert: "se le ct" },
      selection: { anchor: "se le ct".length },
      userEvent: "input.type.compose",
    });
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "se le ct"));
    await waitForCompositionSettlement();
    expect(view.state.doc.toString()).toBe("select");
    expect(modelValue.value).toBe("select");

    const { modelValue: spacedModelValue, view: spacedView } = mountEditor();
    spacedView.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    spacedView.contentDOM.dispatchEvent(new InputEvent("beforeinput", {
      bubbles: true, data: " ", inputType: "insertCompositionText", isComposing: true,
    }));
    spacedView.dispatch({
      changes: { from: 0, insert: "a s d" },
      selection: { anchor: "a s d".length },
      userEvent: "input.type.compose",
    });
    spacedView.contentDOM.dispatchEvent(compositionEvent("compositionend", "a s d"));
    await waitForCompositionSettlement();
    expect(spacedView.state.doc.toString()).toBe("a s d");
    expect(spacedModelValue.value).toBe("a s d");
  });

  it("repairs a delayed `sho w` commit after switching the IME to ASCII input", async () => {
    const prefix = "\n".repeat(4);
    const { modelValue, modelUpdates, view } = mountEditor(prefix);
    const from = view.state.doc.length;
    view.dispatch({ selection: { anchor: from } });
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho "));

    // CodeMirror publishes a pending DOM mutation after its compositionend
    // observer runs. Keep that ordering here so the regression is realistic.
    queueMicrotask(() => {
      view.dispatch({
        changes: { from, insert: "sho w" },
        selection: { anchor: from + "sho w".length },
        userEvent: "input.type.compose",
      });
    });
    await Promise.resolve();
    expect(view.state.doc.toString()).toBe(`${prefix}show`);
    expect(modelValue.value).toBe(prefix);
    expect(modelUpdates).toEqual([]);
    await waitForCompositionSettlement();

    expect(view.state.doc.line(5).text).toBe("show");
    expect(view.state.doc.toString()).toBe(`${prefix}show`);
    expect(modelValue.value).toBe(`${prefix}show`);
    expect(modelUpdates).toEqual([`${prefix}show`]);
    expect(view.state.selection.main.head).toBe(prefix.length + "show".length);
  });

  it("repairs a direct `sho w` CodeMirror commit after a Caps Lock switch", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionupdate", "sho"));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    view.dispatch({
      changes: { from: 0, insert: "sho w" },
      selection: { anchor: "sho w".length },
      userEvent: "input.type.compose",
    });
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho w"));
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("show");
    expect(modelValue.value).toBe("show");
    expect(view.state.selection.main.head).toBe("show".length);
  });

  it("repairs `sho w` when composing input events carry the full payload", async () => {
    const { modelValue, modelUpdates, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionupdate", "sho"));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    view.contentDOM.dispatchEvent(new InputEvent("beforeinput", {
      bubbles: true, data: "sho w", inputType: "insertCompositionText", isComposing: true,
    }));
    view.contentDOM.dispatchEvent(new InputEvent("input", {
      bubbles: true, data: "sho w", inputType: "insertCompositionText", isComposing: true,
    }));
    view.dispatch({
      changes: { from: 0, insert: "sho w" },
      selection: { anchor: "sho w".length },
      userEvent: "input.type.compose",
    });
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho w"));
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("show");
    expect(modelValue.value).toBe("show");
    expect(modelUpdates).toEqual(["show"]);
  });

  it("repairs the switch boundary without consuming later punctuation", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho "));
    queueMicrotask(() => {
      view.dispatch({
        changes: { from: 0, insert: "sho w;" },
        selection: { anchor: "sho w;".length },
        userEvent: "input.type.compose",
      });
    });
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("show;");
    expect(modelValue.value).toBe("show;");
    expect(view.state.selection.main.head).toBe("show;".length);
  });

  it("repairs a CodeMirror commit that arrives after the first settlement pass", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho "));
    await waitForCompositionSettlement();
    expect(view.state.doc.toString()).toBe("");

    view.dispatch({
      changes: { from: 0, insert: "sho w;" },
      selection: { anchor: "sho w;".length },
      userEvent: "input.type.compose",
    });
    await waitForEditorFallback();

    expect(view.state.doc.toString()).toBe("show;");
    expect(modelValue.value).toBe("show;");
  });

  it("ignores a legacy WebKit IME key reported after compositionend", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho w"));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: " ", code: "Space", keyCode: 229, isComposing: false, bubbles: true,
    }));
    queueMicrotask(() => {
      view.dispatch({
        changes: { from: 0, insert: "sho w" },
        selection: { anchor: "sho w".length },
        userEvent: "input.type.compose",
      });
    });
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("show");
    expect(modelValue.value).toBe("show");
  });

  it("does not mistake a late full composition payload for an intentional space", async () => {
    const { modelValue, modelUpdates, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho "));
    view.contentDOM.dispatchEvent(new InputEvent("beforeinput", {
      bubbles: true,
      data: "sho w",
      inputType: "insertText",
      isComposing: false,
    }));
    view.dispatch({
      changes: { from: 0, insert: "sho w" },
      selection: { anchor: "sho w".length },
      userEvent: "input.type.compose",
    });
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("show");
    expect(modelValue.value).toBe("show");
    expect(modelUpdates).toEqual(["show"]);
  });

  it("uses the last composition boundary when the browser hides the switch key", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionupdate", "sho "));
    view.dispatch({
      changes: { from: 0, insert: "sho w" },
      selection: { anchor: "sho w".length },
      userEvent: "input.type.compose",
    });
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho w"));
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("show");
    expect(modelValue.value).toBe("show");
  });

  it.each([
    { name: "compositionend only reports the last segment", inserted: "sho w;", finalData: "w" },
    { name: "compositionend uses NBSP", inserted: "sho w;", finalData: "sho\u00a0w" },
    { name: "the editor state uses NBSP", inserted: "sho\u00a0w;", finalData: "sho w" },
  ])("repairs IME spacing when $name", async ({ inserted, finalData }) => {
    const { modelValue, modelUpdates, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionupdate", "sho"));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    view.dispatch({
      changes: { from: 0, insert: inserted },
      selection: { anchor: inserted.length },
      userEvent: "input.type.compose",
    });
    await nextTick();
    expect(modelValue.value).toBe("");
    expect(modelUpdates).toEqual([]);

    view.contentDOM.dispatchEvent(compositionEvent("compositionend", finalData));
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("show;");
    expect(view.state.selection.main.head).toBe("show;".length);
    expect(modelValue.value).toBe("show;");
    expect(modelUpdates).toEqual(["show;"]);
  });

  it("recovers the switch boundary when WebKit reports only the last character", async () => {
    const { modelValue, modelUpdates, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    view.dispatch({
      changes: { from: 0, insert: "sho w" },
      selection: { anchor: "sho w".length },
      userEvent: "input.type.compose",
    });
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "w"));
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("show");
    expect(modelValue.value).toBe("show");
    expect(modelUpdates).toEqual(["show"]);
  });

  it("repairs a full split committed after compositionend only reports its tail", async () => {
    const { modelValue, modelUpdates, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "w"));
    queueMicrotask(() => {
      view.dispatch({
        changes: { from: 0, insert: "sho w" },
        selection: { anchor: "sho w".length },
        userEvent: "input.type.compose",
      });
    });
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("show");
    expect(modelValue.value).toBe("show");
    expect(modelUpdates).toEqual(["show"]);
  });

  it("keeps waiting when a late commit publishes the stem and gap separately", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "w"));
    queueMicrotask(() => {
      view.dispatch({
        changes: { from: 0, insert: "sho" },
        selection: { anchor: 3 },
        userEvent: "input.type.compose",
      });
    });
    await waitForCompositionSettlement();
    expect(view.state.doc.toString()).toBe("sho");

    view.dispatch({
      changes: { from: 3, insert: " w" },
      selection: { anchor: 5 },
      userEvent: "input.type.compose",
    });
    await waitForEditorFallback();

    expect(view.state.doc.toString()).toBe("show");
    expect(modelValue.value).toBe("show");
  });

  it("keeps the tail-only session alive for a commit from the next task", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "w"));
    await waitForCompositionSettlement();
    expect(view.state.doc.toString()).toBe("");

    view.dispatch({
      changes: { from: 0, insert: "sho w" },
      selection: { anchor: "sho w".length },
      userEvent: "input.type.compose",
    });
    await waitForEditorFallback();

    expect(view.state.doc.toString()).toBe("show");
    expect(modelValue.value).toBe("show");
  });

  it("waits for a late split when only the stem is initially committed", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionupdate", "sho"));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    view.dispatch({
      changes: { from: 0, insert: "sho" },
      selection: { anchor: 3 },
      userEvent: "input.type.compose",
    });
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "w"));
    await waitForCompositionSettlement();
    expect(view.state.doc.toString()).toBe("sho");

    view.dispatch({
      changes: { from: 3, insert: " w" },
      selection: { anchor: 5 },
      userEvent: "input.type.compose",
    });
    await waitForEditorFallback();

    expect(view.state.doc.toString()).toBe("show");
    expect(modelValue.value).toBe("show");
  });

  it("keeps a real post-composition space after an input-mode switch", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionupdate", "where"));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    view.dispatch({
      changes: { from: 0, insert: "where" },
      selection: { anchor: "where".length },
      userEvent: "input.type.compose",
    });
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "x"));
    await waitForCompositionSettlement();
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: " ", code: "Space", bubbles: true,
    }));
    view.dispatch({
      changes: { from: "where".length, insert: " x" },
      selection: { anchor: "where x".length },
      userEvent: "input.type",
    });
    await waitForEditorFallback();

    expect(view.state.doc.toString()).toBe("where x");
    expect(modelValue.value).toBe("where x");
  });

  it("keeps a real SQL space typed immediately after compositionend", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho "));
    await waitForCompositionSettlement();
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: " ", code: "Space", isComposing: false, bubbles: true,
    }));
    view.dispatch({
      changes: { from: 0, insert: "sho a" },
      selection: { anchor: "sho a".length },
      userEvent: "input.type",
    });
    await waitForEditorFallback();

    expect(view.state.doc.toString()).toBe("sho a");
    expect(modelValue.value).toBe("sho a");
  });

  it("repairs direct IME spacing while preserving a later real SQL space", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionupdate", "sho"));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho w"));
    await waitForCompositionSettlement();
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: " ", code: "Space", bubbles: true,
    }));
    view.dispatch({
      changes: { from: 0, insert: "sho w a" },
      selection: { anchor: "sho w a".length },
      userEvent: "input.type",
    });
    await waitForEditorFallback();

    expect(view.state.doc.toString()).toBe("show a");
    expect(modelValue.value).toBe("show a");
  });

  it("keeps a legal one-character SQL token after switching input modes", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionupdate", "select"));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: " ", code: "Space", isComposing: false, bubbles: true,
    }));
    view.contentDOM.dispatchEvent(new InputEvent("beforeinput", {
      bubbles: true, data: "select 1", inputType: "insertCompositionText", isComposing: true,
    }));
    view.dispatch({
      changes: { from: 0, insert: "select 1" },
      selection: { anchor: "select 1".length },
      userEvent: "input.type.compose",
    });
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "select 1"));
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("select 1");
    expect(modelValue.value).toBe("select 1");
  });

  it("keeps a legal full composition payload after a mode switch without a Space key event", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionupdate", "select"));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    view.contentDOM.dispatchEvent(new InputEvent("beforeinput", {
      bubbles: true, data: "select 1", inputType: "insertCompositionText", isComposing: true,
    }));
    view.dispatch({
      changes: { from: 0, insert: "select 1" },
      selection: { anchor: "select 1".length },
      userEvent: "input.type.compose",
    });
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "select 1"));
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("select 1");
    expect(modelValue.value).toBe("select 1");
  });

  it("keeps a literal SQL space even when the browser marks its key event as composing", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionupdate", "where"));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: " ", code: "Space", isComposing: true, bubbles: true,
    }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionupdate", "where "));
    view.dispatch({
      changes: { from: 0, insert: "where x" },
      selection: { anchor: "where x".length },
      userEvent: "input.type.compose",
    });
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "where x"));
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("where x");
    expect(modelValue.value).toBe("where x");
  });

  it("settles a previous IME boundary before a consecutive composition starts", async () => {
    const { modelValue, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.dispatch({
      changes: { from: 0, insert: "sho " },
      selection: { anchor: "sho ".length },
      userEvent: "input.type.compose",
    });
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho "));
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.dispatch({
      changes: { from: view.state.doc.length, insert: "w" },
      selection: { anchor: view.state.doc.length + 1 },
      userEvent: "input.type.compose",
    });
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "w"));
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("show");
    expect(modelValue.value).toBe("show");
  });

  it("repairs a previous commit that reaches state after the next composition starts", async () => {
    const { modelValue, modelUpdates, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho "));
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));

    queueMicrotask(() => {
      view.dispatch({
        changes: { from: 0, insert: "sho " },
        selection: { anchor: "sho ".length },
        userEvent: "input.type.compose",
      });
    });
    await Promise.resolve();
    await nextTick();
    expect(view.state.doc.toString()).toBe("sho");
    expect(modelValue.value).toBe("");
    expect(modelUpdates).toEqual([]);

    view.dispatch({
      changes: { from: view.state.doc.length, insert: "w" },
      selection: { anchor: view.state.doc.length + 1 },
      userEvent: "input.type.compose",
    });
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "w"));
    await waitForEditorFallback();

    expect(view.state.doc.toString()).toBe("show");
    expect(view.state.selection.main.head).toBe("show".length);
    expect(modelValue.value).toBe("show");
    expect(modelUpdates).toEqual(["show"]);
  });

  it("defers external document replacement until IME composition ends", async () => {
    const { modelValue, view } = mountEditor("select");
    let composing = true;
    Object.defineProperty(view, "composing", { configurable: true, get: () => composing });
    expect(view.composing).toBe(true);

    modelValue.value = "SELECT * FROM users";
    await nextTick();
    expect(view.state.doc.toString()).toBe("select");

    composing = false;
    view.contentDOM.dispatchEvent(new Event("compositionend"));
    await Promise.resolve();
    expect(view.state.doc.toString()).toBe("SELECT * FROM users");
  });

  it("does not normalize an external value applied while an IME session settles", async () => {
    const { modelValue, modelUpdates, view } = mountEditor();
    let composing = true;
    Object.defineProperty(view, "composing", { configurable: true, get: () => composing });
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    modelValue.value = "sho w";
    await nextTick();
    expect(view.state.doc.toString()).toBe("");

    composing = false;
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho "));
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("sho w");
    expect(modelValue.value).toBe("sho w");
    expect(modelUpdates).toEqual(["sho w"]);
  });

  it("keeps an external value set after compositionend but before settlement", async () => {
    const { modelValue, modelUpdates, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho "));
    modelValue.value = "sho w";
    await nextTick();
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("sho w");
    expect(modelValue.value).toBe("sho w");
    expect(modelUpdates).toEqual(["sho w"]);
  });

  it("keeps an external value that matches the pending editor document", async () => {
    const { modelValue, modelUpdates, view } = mountEditor();
    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    view.contentDOM.dispatchEvent(compositionEvent("compositionupdate", "sho"));
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "CapsLock", code: "CapsLock", bubbles: true,
    }));
    view.dispatch({
      changes: { from: 0, insert: "sho w" },
      selection: { anchor: "sho w".length },
      userEvent: "input.type.compose",
    });
    expect(modelUpdates).toEqual([]);

    modelValue.value = "sho w";
    await nextTick();
    view.contentDOM.dispatchEvent(compositionEvent("compositionend", "sho w"));
    await waitForCompositionSettlement();

    expect(view.state.doc.toString()).toBe("sho w");
    expect(modelValue.value).toBe("sho w");
    expect(modelUpdates).not.toContain("show");
  });
});
