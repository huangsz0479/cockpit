<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { basicSetup } from "codemirror";
import { acceptCompletion } from "@codemirror/autocomplete";
import { indentLess, insertTab } from "@codemirror/commands";
import { Compartment, EditorState, Prec } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { sql, MariaSQL, MySQL, PostgreSQL, SQLite } from "@codemirror/lang-sql";
import {
  codeMirrorExternalTextInput,
  codeMirrorTextInputSettled,
  filterCodeMirrorTextInputTransaction,
  flushCodeMirrorTextInput,
  isCodeMirrorTextInputPending,
} from "@/lib/textInput";
import type { DatabaseKind } from "@/types";

const props = withDefaults(defineProps<{
  modelValue: string;
  documentId?: string | null;
  schema?: Record<string, readonly string[]>;
  databaseKind?: DatabaseKind;
  fontSize?: number;
  tabSize?: number;
}>(), { schema: () => ({}), databaseKind: "mysql", fontSize: 12, tabSize: 2 });
const emit = defineEmits<{
  "update:modelValue": [value: string];
  "commit:value": [documentId: string | null, value: string];
  execute: [sql?: string];
}>();
const host = ref<HTMLElement | null>(null);
let view: EditorView | null = null;
let lastEmittedValue: string | null = null;
let pendingExternalValue: string | null = null;
const sqlLanguage = new Compartment();
const tabSizeConfig = new Compartment();
const themeConfig = new Compartment();

function dialectFor(kind: DatabaseKind) {
  if (kind === "mariadb") return MariaSQL;
  if (kind === "postgresql") return PostgreSQL;
  if (kind === "sqlite") return SQLite;
  return MySQL;
}

function editorTheme(fontSize: number) {
  return EditorView.theme({
    "&": { height: "100%", backgroundColor: "var(--surface)", color: "var(--text)" },
    ".cm-scroller": { fontFamily: "var(--font-mono)", fontSize: `${fontSize}px` },
    ".cm-content": { caretColor: "var(--accent)" },
    ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent)" },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": { backgroundColor: "var(--selection)" },
    ".cm-activeLine": { backgroundColor: "var(--active-line, rgba(226, 232, 240, 0.25))" },
    ".cm-activeLineGutter": { backgroundColor: "var(--surface-3)", color: "var(--text)" },
    ".cm-gutters": { backgroundColor: "var(--surface-2)", color: "var(--muted)", borderRight: "1px solid var(--border)" },
  });
}

function selectedStatement(editor: EditorView) {
  const selection = editor.state.selection.main;
  if (!selection.empty) return editor.state.sliceDoc(selection.from, selection.to).trim();
  const source = editor.state.doc.toString();
  const boundaries = [0];
  let state: "normal" | "single" | "double" | "backtick" | "line" | "block" = "normal";
  let dollarTag: string | null = null;
  for (let index = 0; index < source.length; index += 1) {
    const character = source[index]!;
    const next = source[index + 1];
    if (dollarTag) {
      if (source.startsWith(dollarTag, index)) { index += dollarTag.length - 1; dollarTag = null; }
      continue;
    }
    if (state === "line") { if (character === "\n") state = "normal"; continue; }
    if (state === "block") { if (character === "*" && next === "/") { state = "normal"; index += 1; } continue; }
    if (state !== "normal") {
      const delimiter = state === "single" ? "'" : state === "double" ? '"' : "`";
      if (character === "\\") { index += 1; continue; }
      if (character === delimiter) {
        if (next === delimiter) index += 1;
        else state = "normal";
      }
      continue;
    }
    if (character === "-" && next === "-") { state = "line"; index += 1; }
    else if (character === "#") state = "line";
    else if (character === "/" && next === "*") { state = "block"; index += 1; }
    else if (character === "'") state = "single";
    else if (character === '"') state = "double";
    else if (character === "`") state = "backtick";
    else if (character === "$") {
      const match = /^\$[A-Za-z0-9_]*\$/.exec(source.slice(index));
      if (match) { dollarTag = match[0]; index += match[0].length - 1; }
    }
    else if (character === ";") boundaries.push(index + 1);
  }
  let start = 0;
  for (let index = boundaries.length - 1; index >= 0; index -= 1) {
    if (boundaries[index]! <= selection.from) { start = boundaries[index]!; break; }
  }
  const end = boundaries.find((boundary) => boundary > selection.from) ?? source.length;
  return source.slice(start, end).trim();
}

function applyExternalValue(value: string) {
  if (!view || value === view.state.doc.toString()) return;
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: value },
    annotations: codeMirrorExternalTextInput.of(true),
  });
}

function currentValue() {
  return view?.state.doc.toString() ?? props.modelValue;
}

function applyPendingExternalValue() {
  if (!view || view.composing || pendingExternalValue === null) return;
  const value = pendingExternalValue;
  flushCodeMirrorTextInput(view);
  pendingExternalValue = null;
  applyExternalValue(value);
}

onMounted(() => {
  if (!host.value) return;
  view = new EditorView({
    parent: host.value,
    state: EditorState.create({
      doc: props.modelValue,
      extensions: [
        basicSetup,
        sqlLanguage.of(sql({ dialect: dialectFor(props.databaseKind), upperCaseKeywords: true, schema: props.schema })),
        tabSizeConfig.of(EditorState.tabSize.of(props.tabSize)),
        themeConfig.of(editorTheme(props.fontSize)),
        EditorState.transactionFilter.of((transaction) => filterCodeMirrorTextInputTransaction(view, transaction)),
        EditorView.lineWrapping,
        EditorView.contentAttributes.of({
          autocomplete: "off",
          autocorrect: "off",
          autocapitalize: "none",
          spellcheck: "false",
          "data-gramm": "false",
        }),
        EditorView.updateListener.of((update) => {
          const inputSettled = update.transactions.some((transaction) => transaction.annotation(codeMirrorTextInputSettled));
          if (inputSettled && pendingExternalValue !== null) return;
          if ((!update.docChanged && !inputSettled) || isCodeMirrorTextInputPending(update.view)) return;
          lastEmittedValue = update.state.doc.toString();
          emit("update:modelValue", lastEmittedValue);
          emit("commit:value", props.documentId ?? null, lastEmittedValue);
        }),
        EditorView.domEventHandlers({
          compositionend: () => {
            queueMicrotask(() => {
              applyPendingExternalValue();
            });
          },
        }),
        keymap.of([
          { key: "Tab", run: acceptCompletion },
          { key: "Tab", run: insertTab, shift: indentLess },
        ]),
        Prec.high(keymap.of([
          { key: "Mod-Enter", run: (editor) => { emit("execute", selectedStatement(editor)); return true; } },
        ])),
      ],
    }),
  });
});

watch(() => props.modelValue, (value) => {
  if (!view) return;
  if (value === lastEmittedValue) {
    lastEmittedValue = null;
    return;
  }
  if (view.composing || isCodeMirrorTextInputPending(view)) {
    pendingExternalValue = value;
    if (!view.composing) queueMicrotask(applyPendingExternalValue);
    return;
  }
  if (value === view.state.doc.toString()) {
    pendingExternalValue = null;
    return;
  }
  pendingExternalValue = null;
  applyExternalValue(value);
});

watch(() => props.schema, (schema) => {
  view?.dispatch({ effects: sqlLanguage.reconfigure(sql({ dialect: dialectFor(props.databaseKind), upperCaseKeywords: true, schema })) });
}, { deep: true });

watch(() => props.databaseKind, (databaseKind) => {
  view?.dispatch({ effects: sqlLanguage.reconfigure(sql({ dialect: dialectFor(databaseKind), upperCaseKeywords: true, schema: props.schema })) });
});

watch(() => props.tabSize, (tabSize) => {
  view?.dispatch({ effects: tabSizeConfig.reconfigure(EditorState.tabSize.of(tabSize)) });
});

watch(() => props.fontSize, (fontSize) => {
  view?.dispatch({ effects: themeConfig.reconfigure(editorTheme(fontSize)) });
});

onBeforeUnmount(() => {
  if (view) flushCodeMirrorTextInput(view);
  view?.destroy();
  view = null;
  pendingExternalValue = null;
});

defineExpose({
  flushTextInput: () => {
    if (view) flushCodeMirrorTextInput(view);
  },
  currentValue,
  hasPendingTextInput: () => Boolean(view && isCodeMirrorTextInputPending(view)),
});
</script>

<template><div ref="host" class="sql-editor" /></template>
