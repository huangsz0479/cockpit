<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { basicSetup } from "codemirror";
import { json } from "@codemirror/lang-json";
import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { Check, Copy, Save } from "lucide-vue-next";
import AppDialog from "@/components/AppDialog.vue";
import { cellToJsValue } from "@/lib/cell";
import type { CellValue, ColumnMeta } from "@/types";

const props = defineProps<{
  columns: ColumnMeta[];
  row: CellValue[];
  rowNumber: number;
  documentId?: string | null;
  onSave?: (document: Record<string, unknown>) => Promise<string | null>;
}>();
const emit = defineEmits<{ close: [] }>();
const status = ref("");
const statusKind = ref<"success" | "error">("success");
const busy = ref(false);
const dirty = ref(false);
const host = ref<HTMLElement | null>(null);
let view: EditorView | null = null;
const editable = computed(() => typeof props.onSave === "function" && Boolean(props.documentId));

const initialJson = computed(() =>
  JSON.stringify(
    Object.fromEntries(props.columns.map((column, index) => [column.name, cellToJsValue(props.row[index])])),
    null,
    2,
  ),
);

function editorTheme() {
  return EditorView.theme({
    "&": { height: "100%", backgroundColor: "var(--surface-1)" },
    ".cm-scroller": { fontFamily: "var(--font-mono)", fontSize: "11px", lineHeight: 1.6 },
    ".cm-content": { caretColor: "var(--accent)", paddingBottom: "12px" },
    ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent)" },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": { backgroundColor: "var(--selection)" },
    ".cm-gutters": { backgroundColor: "var(--surface-2)", color: "var(--muted)", borderRight: "1px solid var(--border)" },
  });
}

async function save() {
  if (!editable.value || busy.value) return;
  const text = view?.state.doc.toString() ?? initialJson.value;
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch (cause) {
    fail(`JSON 格式不正确：${cause instanceof Error ? cause.message : String(cause)}`);
    return;
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    fail("文档内容必须是 JSON 对象");
    return;
  }
  const document = { ...(parsed as Record<string, unknown>) };
  // _id 是文档寻址标识，改了会写成另一篇文档，必须拒绝
  if (document._id !== undefined && String(document._id) !== props.documentId) {
    fail("文档 _id 不能修改");
    return;
  }
  delete document._id;
  // _version 是 ES 维护的修改计数，仅展示用，不允许写进文档体
  delete document._version;
  busy.value = true;
  status.value = "";
  try {
    const error = (await props.onSave?.(document)) ?? null;
    if (error) fail(error);
    else {
      statusKind.value = "success";
      status.value = "已保存";
      dirty.value = false;
    }
  } finally {
    busy.value = false;
  }
}

function fail(message: string) {
  statusKind.value = "error";
  status.value = message;
}

async function copy() {
  try {
    await navigator.clipboard.writeText(view?.state.doc.toString() ?? initialJson.value);
    statusKind.value = "success";
    status.value = "内容已复制";
  } catch {
    fail("复制失败，请检查剪贴板权限");
  }
}

onMounted(() => {
  if (!host.value) return;
  const readOnlyConfig = new Compartment();
  view = new EditorView({
    parent: host.value,
    state: EditorState.create({
      doc: initialJson.value,
      extensions: [
        basicSetup,
        json(),
        EditorView.lineWrapping,
        editorTheme(),
        readOnlyConfig.of(
          editable.value
            ? []
            : [EditorState.readOnly.of(true), EditorView.editable.of(false)],
        ),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) dirty.value = true;
        }),
        keymap.of([
          {
            key: "Mod-s",
            preventDefault: true,
            run: () => {
              void save();
              return true;
            },
          },
        ]),
      ],
    }),
  });
});

onBeforeUnmount(() => {
  view?.destroy();
  view = null;
});
</script>

<template>
  <AppDialog title="行数据 JSON" title-id="row-json-viewer-title" :description="`第 ${rowNumber} 行 · ${columns.length} 列${editable ? ' · 可编辑' : ''}`" dialog-class="cell-viewer-dialog" close-label="关闭行 JSON 查看器" @close="emit('close')">
    <div ref="host" class="row-json-editor" :aria-readonly="!editable" :aria-label="editable ? '文档 JSON 编辑器' : '文档 JSON 内容'" />
    <template #footer>
      <span class="cell-viewer-status" :class="statusKind">{{ status }}</span>
      <button v-if="editable" class="secondary" :disabled="busy || !dirty" @click="save"><Save :size="14" />{{ busy ? "保存中…" : "保存" }}</button>
      <button class="secondary" @click="copy"><Check v-if="status === '内容已复制'" :size="14" /><Copy v-else :size="14" />复制</button>
      <button class="primary" @click="emit('close')">关闭</button>
    </template>
  </AppDialog>
</template>
