<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import { X } from "lucide-vue-next";
import type { CellValue, ColumnInfo, ColumnMeta } from "@/types";
import { cellText } from "@/lib/cell";
import {
  columnIsNullable,
  columnTypeName,
  hasDatabaseDefault,
  isGeneratedColumn,
  parseRowCell,
  rowDraftValue,
  rowCellChanged,
  rowInputType,
  rowInputValue,
  type RowDraftCell,
} from "@/lib/rowEditing";

const props = defineProps<{
  mode: "insert" | "update";
  columns: ColumnMeta[];
  columnDetails: ColumnInfo[];
  row?: CellValue[] | null;
  busy?: boolean;
  error?: string | null;
}>();
const emit = defineEmits<{
  close: [];
  save: [values: [string, CellValue][]];
}>();

interface EditorColumn {
  meta: ColumnMeta;
  detail?: ColumnInfo;
  sourceIndex: number;
}

const validationError = ref("");

function columnDetail(column: ColumnMeta) {
  return props.columnDetails.find((item) => item.name === column.name);
}

function nullable(column: EditorColumn) {
  return columnIsNullable(column.meta, column.detail);
}

function typeName(column: EditorColumn) {
  return columnTypeName(column.meta, column.detail);
}

function displayedType(column: EditorColumn) {
  return column.detail?.fullType || column.detail?.dataType || column.meta.databaseType;
}

function inputType(column: EditorColumn) {
  return rowInputType(column.meta, column.detail);
}

function inputValue(column: EditorColumn) {
  return rowInputValue(draft[column.meta.name]!.text, inputType(column));
}

function updateDraftText(column: EditorColumn, event: Event) {
  draft[column.meta.name]!.text = rowDraftValue(
    (event.currentTarget as HTMLInputElement).value,
    inputType(column),
  );
}

const editorColumns = computed<EditorColumn[]>(() => props.columns
  .map((meta, sourceIndex) => ({ meta, detail: columnDetail(meta), sourceIndex }))
  .filter((column) => !isGeneratedColumn(column.detail)));

const generatedColumnCount = computed(() => props.columns.length - editorColumns.value.length);

const draft = reactive<Record<string, RowDraftCell>>(Object.fromEntries(props.columns.map((column, index) => {
  const detail = columnDetail(column);
  const value = props.row?.[index];
  const useDefault = props.mode === "insert" && hasDatabaseDefault(detail);
  return [column.name, {
    text: value && value.kind !== "null" ? cellText(value) : "",
    isNull: value?.kind === "null" || (props.mode === "insert" && !useDefault && (detail?.nullable ?? column.nullable)),
    useDefault,
  }];
})));

function originalValue(column: EditorColumn) {
  return props.row?.[column.sourceIndex] ?? { kind: "null" } as CellValue;
}

function changed(column: EditorColumn) {
  return rowCellChanged(draft[column.meta.name]!, originalValue(column));
}

const hasChanges = computed(() => editorColumns.value.some(changed));
const canSubmit = computed(() => !props.busy && (props.mode === "insert" || hasChanges.value));
const displayedError = computed(() => validationError.value || props.error || "");

function close() {
  if (!props.busy) emit("close");
}

function submit() {
  validationError.value = "";
  if (!canSubmit.value) return;
  try {
    const columns = props.mode === "update"
      ? editorColumns.value.filter(changed)
      : editorColumns.value.filter((column) => !draft[column.meta.name]!.useDefault);
    const values = columns.map((column) => [
      column.meta.name,
      parseRowCell(column.meta, column.detail, draft[column.meta.name]!, props.row?.[column.sourceIndex]),
    ] as [string, CellValue]);
    emit("save", values);
  } catch (cause) {
    validationError.value = cause instanceof Error ? cause.message : String(cause);
  }
}
</script>

<template>
  <div class="dialog-backdrop" @mousedown.self="close" @keydown.esc="close">
    <form class="dialog row-editor-dialog" role="dialog" aria-modal="true" :aria-labelledby="`row-editor-title-${mode}`" @submit.prevent="submit">
      <header>
        <div>
          <h2 :id="`row-editor-title-${mode}`">{{ mode === 'insert' ? '新增行' : '编辑行' }}</h2>
          <p>{{ mode === 'insert' ? '带默认值的字段会由数据库自动填写。' : '仅保存有变化的字段，并校验原始值以避免覆盖其他会话的修改。' }}</p>
        </div>
        <button type="button" class="icon-button" aria-label="关闭" :disabled="busy" @click="close"><X :size="15" /></button>
      </header>
      <div class="row-editor-fields">
        <div v-for="(column, index) in editorColumns" :key="column.meta.name" class="row-editor-field" :data-column="column.meta.name" :class="{ changed: mode === 'update' && changed(column) }">
          <label class="row-editor-field-label" :for="`row-editor-${mode}-${index}`">
            <strong>{{ column.meta.name }}</strong><small class="row-editor-type">{{ displayedType(column) }}</small>
            <small v-if="mode === 'insert' && hasDatabaseDefault(column.detail)" class="row-editor-default">默认：{{ column.detail?.defaultValue ?? '自动生成' }}</small>
          </label>
          <div class="row-value-input">
            <input
              :id="`row-editor-${mode}-${index}`"
              :value="inputValue(column)"
              :type="inputType(column)"
              :step="inputType(column) === 'datetime-local' ? 'any' : undefined"
              :aria-label="column.meta.name"
              :autofocus="index === 0"
              :disabled="draft[column.meta.name]!.isNull || draft[column.meta.name]!.useDefault"
              :placeholder="typeName(column) === 'bool' || typeName(column) === 'boolean' ? 'true / false' : ''"
              autocomplete="off"
              autocorrect="off"
              autocapitalize="none"
              spellcheck="false"
              data-gramm="false"
              @input="updateDraftText(column, $event)"
            />
            <label v-if="nullable(column)" class="null-toggle">
              <input v-model="draft[column.meta.name]!.isNull" type="checkbox" :disabled="draft[column.meta.name]!.useDefault" />NULL
            </label>
            <label v-if="mode === 'insert' && hasDatabaseDefault(column.detail)" class="null-toggle default-toggle">
              <input v-model="draft[column.meta.name]!.useDefault" type="checkbox" />默认
            </label>
          </div>
        </div>
        <p v-if="generatedColumnCount" class="row-editor-generated">{{ generatedColumnCount }} 个生成字段将由数据库自动计算。</p>
      </div>
      <p v-if="displayedError" class="error-banner" role="alert">{{ displayedError }}</p>
      <footer>
        <button type="button" class="secondary" :disabled="busy" @click="close">取消</button>
        <button class="primary" :disabled="!canSubmit">{{ busy ? '保存中…' : mode === 'update' && !hasChanges ? '没有修改' : '保存' }}</button>
      </footer>
    </form>
  </div>
</template>
