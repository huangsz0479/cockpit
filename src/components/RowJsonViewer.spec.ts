import { EditorView } from "@codemirror/view";
import { createApp, defineComponent, h, nextTick } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CellValue, ColumnMeta } from "@/types";
import RowJsonViewer from "./RowJsonViewer.vue";

const columns: ColumnMeta[] = [
  { name: "_id", databaseType: "KEYWORD", nullable: false, unsigned: false, binary: false },
  { name: "_version", databaseType: "LONG", nullable: true, unsigned: false, binary: false },
  { name: "title", databaseType: "TEXT", nullable: true, unsigned: false, binary: false },
  { name: "price", databaseType: "INTEGER", nullable: true, unsigned: false, binary: false },
];
const row: CellValue[] = [
  { kind: "text", value: "doc-1" },
  { kind: "signed", value: "3" },
  { kind: "text", value: "hello" },
  { kind: "signed", value: "42" },
];

const mountedApps: Array<ReturnType<typeof createApp>> = [];

afterEach(() => {
  for (const app of mountedApps.splice(0)) app.unmount();
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

function mountDialog(options: { onSave?: (document: Record<string, unknown>) => Promise<string | null>; documentId?: string | null } = {}) {
  const host = document.createElement("div");
  document.body.append(host);
  const app = createApp(defineComponent({
    setup() {
      return () => h(RowJsonViewer, {
        columns,
        row,
        rowNumber: 3,
        documentId: options.documentId ?? null,
        onSave: options.onSave,
      });
    },
  }));
  mountedApps.push(app);
  app.mount(host);
  const content = host.querySelector<HTMLElement>(".cm-content");
  const view = content && EditorView.findFromDOM(content);
  if (!view) throw new Error("row json editor did not mount");
  const editorText = () => view.state.doc.toString();
  const replaceEditorText = async (text: string) => {
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
    // dirty 状态驱动按钮解除禁用，需要等 Vue 完成一次重渲染
    await nextTick();
  };
  const buttonByText = (label: string) => {
    const button = Array.from(host.querySelectorAll<HTMLButtonElement>("button"))
      .find((item) => item.textContent?.includes(label));
    if (!button) throw new Error(`button not found: ${label}`);
    return button;
  };
  const hasButton = (label: string) =>
    Array.from(host.querySelectorAll<HTMLButtonElement>("button"))
      .some((item) => item.textContent?.includes(label));
  return { host, view, editorText, replaceEditorText, buttonByText, hasButton };
}

const flush = async () => {
  await new Promise((resolve) => {
    setTimeout(resolve, 0);
  });
};

describe("RowJsonViewer", () => {
  it("renders the full row as highlighted JSON without a save button when read-only", () => {
    const { host, editorText, hasButton } = mountDialog();
    expect(JSON.parse(editorText())).toEqual({ _id: "doc-1", _version: "3", title: "hello", price: "42" });
    expect(hasButton("保存")).toBe(false);
    expect(host.textContent).toContain("第 3 行");
    expect(host.textContent).toContain("4 列");
    expect(host.textContent).not.toContain("可编辑");
  });

  it("marks the dialog editable when a save handler and document id are provided", () => {
    const { host, buttonByText } = mountDialog({ documentId: "doc-1", onSave: vi.fn() });
    expect(host.textContent).toContain("可编辑");
    expect(buttonByText("保存")).toBeTruthy();
  });

  it("saves the edited document without the _id and _version meta fields", async () => {
    const onSave = vi.fn().mockResolvedValue(null);
    const { replaceEditorText, buttonByText } = mountDialog({ documentId: "doc-1", onSave });
    await replaceEditorText(JSON.stringify({ _id: "doc-1", _version: "3", title: "edited", price: 42, tags: ["a"] }, null, 2));
    buttonByText("保存").click();
    await flush();
    expect(onSave).toHaveBeenCalledOnce();
    expect(onSave.mock.calls[0]![0]).toEqual({ title: "edited", price: 42, tags: ["a"] });
  });

  it("rejects invalid JSON without calling save", async () => {
    const onSave = vi.fn().mockResolvedValue(null);
    const { host, replaceEditorText, buttonByText } = mountDialog({ documentId: "doc-1", onSave });
    await replaceEditorText("{ not json");
    buttonByText("保存").click();
    await flush();
    expect(onSave).not.toHaveBeenCalled();
    expect(host.textContent).toContain("JSON 格式不正确");
  });

  it("rejects modifications to the document _id", async () => {
    const onSave = vi.fn().mockResolvedValue(null);
    const { host, replaceEditorText, buttonByText } = mountDialog({ documentId: "doc-1", onSave });
    await replaceEditorText(JSON.stringify({ _id: "doc-2", title: "x" }));
    buttonByText("保存").click();
    await flush();
    expect(onSave).not.toHaveBeenCalled();
    expect(host.textContent).toContain("文档 _id 不能修改");
  });

  it("surfaces save errors returned by the handler", async () => {
    const onSave = vi.fn().mockResolvedValue("集群拒绝写入");
    const { host, replaceEditorText, buttonByText } = mountDialog({ documentId: "doc-1", onSave });
    await replaceEditorText(JSON.stringify({ title: "x" }));
    buttonByText("保存").click();
    await flush();
    expect(host.textContent).toContain("集群拒绝写入");
    expect(host.textContent).not.toContain("已保存");
  });

  it("copies the editor text to the clipboard", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const { host, buttonByText } = mountDialog();
    buttonByText("复制").click();
    await flush();
    expect(writeText).toHaveBeenCalledOnce();
    expect(writeText.mock.calls[0]![0]).toContain("\"_id\": \"doc-1\"");
    expect(host.textContent).toContain("内容已复制");
  });
});
