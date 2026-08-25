<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { basicSetup } from "codemirror";
import { acceptCompletion, type Completion, type CompletionContext } from "@codemirror/autocomplete";
import { indentLess, insertTab } from "@codemirror/commands";
import { syntaxTree } from "@codemirror/language";
import { Compartment, EditorState, Prec } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { sql, StandardSQL, MariaSQL, MySQL, PostgreSQL, SQLite } from "@codemirror/lang-sql";
import { isSqlTableNamePosition, sqlCompletionQualifier, sqlTableReferences, type SqlTableReference } from "@/lib/sqlCompletion";
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
  loadTableColumns?: (table: string, database?: string) => Promise<readonly string[]>;
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
  if (kind === "elasticsearch") return StandardSQL;
  return MySQL;
}

function schemaColumns(reference: SqlTableReference) {
  const candidates = reference.database ? [`${reference.database}.${reference.table}`] : [reference.table];
  for (const candidate of candidates) {
    const exact = props.schema[candidate];
    if (exact?.length) return exact;
    const matchedKey = Object.keys(props.schema).find((key) => key.toLocaleLowerCase() === candidate.toLocaleLowerCase());
    if (matchedKey && props.schema[matchedKey]?.length) return props.schema[matchedKey]!;
  }
  return null;
}

function referenceMatchesQualifier(reference: SqlTableReference, qualifier: string) {
  const normalized = qualifier.toLocaleLowerCase();
  return reference.alias?.toLocaleLowerCase() === normalized
    || reference.table.toLocaleLowerCase() === normalized
    || `${reference.database}.${reference.table}`.toLocaleLowerCase() === normalized;
}

function completionDetail(references: readonly SqlTableReference[]) {
  return references.map((reference) => reference.alias
    ? `${reference.alias} · ${reference.database ? `${reference.database}.` : ""}${reference.table}`
    : `${reference.database ? `${reference.database}.` : ""}${reference.table}`).join(", ");
}

async function contextualColumnCompletion(context: CompletionContext) {
  const nodeName = syntaxTree(context.state).resolveInner(context.pos, -1).name;
  if (["String", "LineComment", "BlockComment", "QuotedIdentifier"].includes(nodeName)) return null;
  const word = context.matchBefore(/[\p{L}\p{N}_$]*/u);
  if (!word) return null;
  const source = context.state.doc.toString();
  if (isSqlTableNamePosition(source, context.pos)) return null;
  const qualifier = sqlCompletionQualifier(source, word.from);
  if (!context.explicit && word.from === context.pos && !qualifier) return null;
  let references = sqlTableReferences(source, context.pos);
  if (qualifier) references = references.filter((reference) => referenceMatchesQualifier(reference, qualifier));
  if (!references.length) return null;

  const resolved = await Promise.all(references.map(async (reference) => ({
    reference,
    columns: schemaColumns(reference)
      ?? await props.loadTableColumns?.(reference.table, reference.database)
      ?? [],
  })));
  if (context.aborted) return null;
  const columns = new Map<string, { label: string; references: SqlTableReference[] }>();
  for (const { reference, columns: names } of resolved) {
    for (const label of names) {
      const key = label.toLocaleLowerCase();
      const existing = columns.get(key);
      if (existing) existing.references.push(reference);
      else columns.set(key, { label, references: [reference] });
    }
  }
  const options: Completion[] = [...columns.values()].map(({ label, references: owners }) => ({
    label,
    type: "property",
    detail: completionDetail(owners),
    boost: 50,
  }));
  if (!options.length) return null;
  return { from: word.from, options, validFor: /^(?:[\p{L}_][\p{L}\p{N}_$]*)?$/u };
}

function sqlSupport(databaseKind: DatabaseKind, schema: Record<string, readonly string[]>) {
  const dialect = dialectFor(databaseKind);
  return [
    sql({ dialect, upperCaseKeywords: true, schema }),
    dialect.language.data.of({ autocomplete: contextualColumnCompletion }),
  ];
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
    ".cm-tooltip.cm-tooltip-autocomplete": {
      overflow: "hidden",
      border: "1px solid var(--border-strong)",
      borderRadius: "var(--radius-md)",
      backgroundColor: "var(--surface-1)",
      color: "var(--text)",
      boxShadow: "var(--shadow-md)",
    },
    ".cm-tooltip-autocomplete > ul": {
      minWidth: "320px",
      padding: "3px 0",
      scrollbarColor: "var(--border-strong) var(--surface-2)",
      scrollbarWidth: "thin",
    },
    ".cm-tooltip-autocomplete > ul > li": {
      padding: "1px 8px",
      lineHeight: 1.2,
      color: "var(--text)",
    },
    ".cm-tooltip-autocomplete > ul > li:hover": { backgroundColor: "var(--surface-hover)" },
    ".cm-tooltip-autocomplete > ul > li[aria-selected]": {
      backgroundColor: "var(--accent-soft) !important",
      color: "var(--accent-strong) !important",
      boxShadow: "inset 2px 0 var(--accent)",
    },
    ".cm-completionIcon-property": { width: "10px", paddingRight: "6px", opacity: 1 },
    ".cm-completionIcon-property::after": {
      content: '""',
      display: "inline-block",
      width: "6px",
      height: "6px",
      borderRadius: "2px",
      backgroundColor: "var(--accent)",
      verticalAlign: "middle",
    },
    ".cm-completionLabel": { fontWeight: 520 },
    ".cm-completionMatchedText": { color: "var(--accent-strong)", fontWeight: 720, textDecoration: "none" },
    ".cm-completionDetail": { marginLeft: "8px", color: "var(--muted)", fontSize: ".82em", fontStyle: "normal", opacity: ".86" },
    ".cm-tooltip-autocomplete > ul::-webkit-scrollbar": { width: "8px", height: "8px" },
    ".cm-tooltip-autocomplete > ul::-webkit-scrollbar-track": { backgroundColor: "var(--surface-2)" },
    ".cm-tooltip-autocomplete > ul::-webkit-scrollbar-thumb": { border: "2px solid var(--surface-1)", borderRadius: "999px", backgroundColor: "var(--border-strong)" },
  });
}

function sqlForExecution(editor: EditorView) {
  const selection = editor.state.selection.main;
  if (!selection.empty) return editor.state.sliceDoc(selection.from, selection.to).trim();
  return editor.state.doc.toString().trim();
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
        sqlLanguage.of(sqlSupport(props.databaseKind, props.schema)),
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
          { key: "Mod-Enter", run: (editor) => { emit("execute", sqlForExecution(editor)); return true; } },
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
  view?.dispatch({ effects: sqlLanguage.reconfigure(sqlSupport(props.databaseKind, schema)) });
}, { deep: true });

watch(() => props.databaseKind, (databaseKind) => {
  view?.dispatch({ effects: sqlLanguage.reconfigure(sqlSupport(databaseKind, props.schema)) });
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
  sqlForExecution: () => view ? sqlForExecution(view) : props.modelValue.trim(),
  hasPendingTextInput: () => Boolean(view && isCodeMirrorTextInputPending(view)),
});
</script>

<template><div ref="host" class="sql-editor" /></template>
