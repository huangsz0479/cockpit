<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { AlertCircle, Database, Plus, Table2, Trash2 } from "lucide-vue-next";
import ActionDialog from "@/components/ActionDialog.vue";
import AppSelect from "@/components/AppSelect.vue";
import { useActionDialog } from "@/lib/actionDialog";
import {
  MYSQL_COLUMN_TYPES, SQLITE_COLUMN_TYPES, alterTableSql, createTableSql, mysqlColumnTypeSupportsAutoIncrement,
  mysqlColumnTypeSupportsSize, mysqlColumnTypeSupportsUnsigned, validateCreateTableDefinition,
} from "@/lib/sql";
import type {
  CreateTableColumnDefinition, CreateTableDefinition, MysqlColumnType, TableForeignKeyDefinition,
  TableCheckDefinition, TableIndexDefinition,
} from "@/lib/sql";
import type { DatabaseKind } from "@/types";

interface EditableColumn extends CreateTableColumnDefinition { id: string }

const props = withDefaults(defineProps<{
  database: string;
  modelValue: CreateTableDefinition;
  databaseKind?: DatabaseKind;
  originalDefinition?: CreateTableDefinition | null;
  mode?: "create" | "alter";
  busy?: boolean;
  readOnly?: boolean;
  error?: string | null;
}>(), { databaseKind: "mysql", originalDefinition: null, mode: "create", busy: false, readOnly: false, error: null });
const emit = defineEmits<{
  cancel: [];
  create: [definition: CreateTableDefinition];
  "update:modelValue": [definition: CreateTableDefinition];
}>();
const { actionDialog, confirmAction, acceptActionDialog, cancelActionDialog } = useActionDialog();
const sqlite = props.databaseKind === "sqlite";
const columnTypes: readonly MysqlColumnType[] = sqlite ? SQLITE_COLUMN_TYPES : MYSQL_COLUMN_TYPES;
const indexTypeOptions = (sqlite ? ["INDEX"] : ["INDEX", "FULLTEXT", "SPATIAL"])
  .map((value) => ({ value, label: value }));

const tableName = ref(props.modelValue.name);
const engine = ref(props.modelValue.engine ?? "InnoDB");
const charset = ref(props.modelValue.charset ?? "utf8mb4");
const collation = ref(props.modelValue.collation ?? "");
const tableComment = ref(props.modelValue.comment ?? "");
const partitionClause = ref(props.modelValue.partitionClause ?? "");
const activeSection = ref<"fields" | "indexes" | "foreignKeys" | "checks" | "options" | "sql">("fields");
type EditorSection = typeof activeSection.value;
const sections: { id: EditorSection; label: string }[] = [
  { id: "fields", label: "字段" }, { id: "indexes", label: "索引" },
  { id: "foreignKeys", label: "外键" }, { id: "checks", label: "检查约束" },
  { id: "options", label: "选项" }, { id: "sql", label: "SQL 预览" },
].filter((section) => !sqlite || section.id !== "options") as { id: EditorSection; label: string }[];
interface FieldColumnLayout { key: string; label: string; defaultWidth: number; resizable: boolean }
const fieldColumnLayout: FieldColumnLayout[] = [
  { key: "name", label: "字段名", defaultWidth: 150, resizable: true },
  { key: "type", label: "类型", defaultWidth: 115, resizable: true },
  { key: "size", label: "长度 / 精度", defaultWidth: 110, resizable: true },
  { key: "default", label: "默认值", defaultWidth: 130, resizable: true },
  { key: "comment", label: "注释", defaultWidth: 150, resizable: true },
  { key: "collation", label: "排序规则", defaultWidth: 170, resizable: true },
  { key: "generated", label: "生成表达式", defaultWidth: 180, resizable: true },
  { key: "stored", label: "存储", defaultWidth: 60, resizable: false },
  { key: "unsigned", label: "UN", defaultWidth: 48, resizable: false },
  { key: "nullable", label: "NULL", defaultWidth: 48, resizable: false },
  { key: "primary", label: "PK", defaultWidth: 48, resizable: false },
  { key: "autoIncrement", label: "AI", defaultWidth: 48, resizable: false },
  { key: "actions", label: "", defaultWidth: 40, resizable: false },
].filter((column) => !sqlite || !["size", "comment", "unsigned"].includes(column.key));
const MIN_FIELD_COLUMN_WIDTH = 72;
const MAX_FIELD_COLUMN_WIDTH = 1200;
const FIELD_COLUMN_RESIZE_STEP = 12;
const fieldColumnWidths = ref(fieldColumnLayout.map((column) => column.defaultWidth));
const fieldTableWidth = computed(() => fieldColumnWidths.value.reduce((total, width) => total + width, 0));
const fieldTableStyle = computed(() => ({ width: `${fieldTableWidth.value}px`, minWidth: `${fieldTableWidth.value}px` }));
let resizingFieldColumnIndex: number | null = null;
let fieldColumnResizeStartX = 0;
let fieldColumnResizeStartWidth = 0;
const textInputAttributes = {
  autocomplete: "off", autocorrect: "off", autocapitalize: "none", spellcheck: "false", "data-gramm": "false",
} as const;
const columns = ref<EditableColumn[]>(props.modelValue.columns.map((column) => ({ ...column, id: crypto.randomUUID() })));
const indexes = ref<TableIndexDefinition[]>((props.modelValue.indexes ?? []).map((index) => ({
  ...index, columns: [...index.columns],
})));
const foreignKeys = ref<TableForeignKeyDefinition[]>((props.modelValue.foreignKeys ?? []).map((foreignKey) => ({
  ...foreignKey, columns: [...foreignKey.columns], referencedColumns: [...foreignKey.referencedColumns],
})));
const checks = ref<TableCheckDefinition[]>((props.modelValue.checks ?? []).map((check) => ({ ...check })));
const definition = computed<CreateTableDefinition>(() => ({
  name: tableName.value,
  originalName: props.modelValue.originalName,
  columns: columns.value.map(({ id: _id, ...column }) => ({ ...column })),
  indexes: indexes.value,
  foreignKeys: foreignKeys.value,
  checks: checks.value,
  ...(sqlite ? {} : {
    engine: engine.value,
    charset: charset.value,
    collation: collation.value,
    comment: tableComment.value,
    partitionClause: partitionClause.value,
  }),
}));
const validationMessage = computed(() => props.readOnly
  ? `当前为只读连接，不能${props.mode === "alter" ? "修改" : "创建"}表`
  : validateCreateTableDefinition(definition.value, props.databaseKind));
const displayedValidationMessage = computed(() => props.readOnly || tableName.value.trim() ? validationMessage.value : null);
const canCreate = computed(() => !props.busy && !validationMessage.value && Boolean(sqlPreview.value));
const sqlPreview = computed(() => {
  if (validationMessage.value) return "";
  return props.mode === "alter" && props.originalDefinition
    ? alterTableSql(props.database, props.originalDefinition, definition.value)
    : createTableSql(props.database, definition.value, props.databaseKind);
});
const primaryKeyCount = computed(() => columns.value.filter((column) => column.primaryKey).length);
const hasAutoIncrement = computed(() => columns.value.some((column) => column.autoIncrement));

function emitDefinition() { emit("update:modelValue", definition.value); }
watch([tableName, engine, charset, collation, tableComment, partitionClause, columns, indexes, foreignKeys, checks], emitDefinition, { deep: true, flush: "sync" });

function setFieldColumnWidth(index: number, width: number) {
  if (!fieldColumnLayout[index]?.resizable) return;
  fieldColumnWidths.value[index] = Math.round(Math.min(MAX_FIELD_COLUMN_WIDTH, Math.max(MIN_FIELD_COLUMN_WIDTH, width)));
}
function startFieldColumnResize(event: PointerEvent, index: number) {
  if (!fieldColumnLayout[index]?.resizable) return;
  event.preventDefault();
  event.stopPropagation();
  const target = event.currentTarget as HTMLElement;
  resizingFieldColumnIndex = index;
  fieldColumnResizeStartX = event.clientX;
  fieldColumnResizeStartWidth = target.closest("th")?.getBoundingClientRect().width ?? fieldColumnWidths.value[index]!;
  setFieldColumnWidth(index, fieldColumnResizeStartWidth);
  target.setPointerCapture(event.pointerId);
}
function resizeFieldColumn(event: PointerEvent) {
  if (resizingFieldColumnIndex === null) return;
  setFieldColumnWidth(resizingFieldColumnIndex, fieldColumnResizeStartWidth + event.clientX - fieldColumnResizeStartX);
}
function finishFieldColumnResize(event: PointerEvent) {
  if (resizingFieldColumnIndex === null) return;
  resizingFieldColumnIndex = null;
  const target = event.currentTarget as HTMLElement;
  if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
}
function resetFieldColumnWidth(index: number) {
  const column = fieldColumnLayout[index];
  if (column?.resizable) fieldColumnWidths.value[index] = column.defaultWidth;
}
function resizeFieldColumnWithKeyboard(event: KeyboardEvent, index: number) {
  if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key) || !fieldColumnLayout[index]?.resizable) return;
  event.preventDefault();
  if (event.key === "Home") setFieldColumnWidth(index, MIN_FIELD_COLUMN_WIDTH);
  else if (event.key === "End") setFieldColumnWidth(index, MAX_FIELD_COLUMN_WIDTH);
  else setFieldColumnWidth(index, fieldColumnWidths.value[index]! + (event.key === "ArrowLeft" ? -FIELD_COLUMN_RESIZE_STEP : FIELD_COLUMN_RESIZE_STEP));
}

function nextColumnName() {
  const names = new Set(columns.value.map((column) => column.name.trim().toLocaleLowerCase()));
  let index = 1;
  while (names.has(`column_${index}`)) index += 1;
  return `column_${index}`;
}
function addColumn() {
  columns.value.push({
    id: crypto.randomUUID(), name: nextColumnName(), dataType: sqlite ? "TEXT" : "VARCHAR", size: sqlite ? "" : "255",
    unsigned: false, nullable: true, primaryKey: false, autoIncrement: false, defaultValue: null,
    ...(sqlite ? {} : { comment: "" }),
  });
}
function confirmDefinitionRemoval(label: string, name: string) {
  return confirmAction({
    title: `删除${label}？`,
    message: `从当前表设计中删除${label}“${name}”。`,
    detail: "该修改仅影响当前设计，应用表结构修改前不会改动数据库。",
    tone: "warning",
    confirmLabel: "从设计中删除",
  });
}
async function removeColumn(columnId: string) {
  const column = columns.value.find((item) => item.id === columnId);
  if (!column || !await confirmDefinitionRemoval("字段", column.name || "未命名字段")) return;
  columns.value = columns.value.filter((item) => item.id !== columnId);
}
function sizePlaceholder(dataType: MysqlColumnType) {
  if (dataType === "DECIMAL" || dataType === "NUMERIC") return "10,2";
  if (["CHAR", "BINARY"].includes(dataType)) return "64";
  if (["VARCHAR", "VARBINARY"].includes(dataType)) return "255";
  if (dataType === "BIT") return "1";
  if (["TIME", "DATETIME", "TIMESTAMP"].includes(dataType)) return "0-6，可留空";
  if (["ENUM", "SET"].includes(dataType)) return "'a','b'";
  return "—";
}
function updateDataType(column: EditableColumn) {
  if (sqlite) {
    column.size = "";
    column.unsigned = false;
    if (column.dataType !== "INTEGER") column.autoIncrement = false;
    return;
  }
  if (column.dataType === "DECIMAL" || column.dataType === "NUMERIC") column.size = "10,2";
  else if (["CHAR", "BINARY"].includes(column.dataType)) column.size = "64";
  else if (["VARCHAR", "VARBINARY"].includes(column.dataType)) column.size = "255";
  else if (column.dataType === "BIT") column.size = "1";
  else if (["ENUM", "SET"].includes(column.dataType)) column.size = "'a','b'";
  else column.size = "";
  if (!mysqlColumnTypeSupportsUnsigned(column.dataType)) column.unsigned = false;
  if (!mysqlColumnTypeSupportsAutoIncrement(column.dataType)) column.autoIncrement = false;
}
function columnTypeSupportsSize(dataType: MysqlColumnType) { return !sqlite && mysqlColumnTypeSupportsSize(dataType); }
function columnTypeSupportsUnsigned(dataType: MysqlColumnType) { return !sqlite && mysqlColumnTypeSupportsUnsigned(dataType); }
function columnTypeSupportsAutoIncrement(dataType: MysqlColumnType) {
  return sqlite ? dataType === "INTEGER" : mysqlColumnTypeSupportsAutoIncrement(dataType);
}
function updatePrimaryKey(column: EditableColumn) { if (column.primaryKey) column.nullable = false; else column.autoIncrement = false; }
function updateNullable(column: EditableColumn) { if (column.nullable) { column.primaryKey = false; column.autoIncrement = false; } }
function updateAutoIncrement(column: EditableColumn) {
  if (!column.autoIncrement) return;
  for (const item of columns.value) if (item.id !== column.id) item.autoIncrement = false;
  column.primaryKey = true;
  column.nullable = false;
}
function addIndex() { indexes.value.push({ id: crypto.randomUUID(), name: `idx_${tableName.value || "table"}_${indexes.value.length + 1}`, columns: columns.value[0]?.name ? [columns.value[0].name] : [], unique: false, indexType: "INDEX" }); }
function addForeignKey() { foreignKeys.value.push({ id: crypto.randomUUID(), name: `fk_${tableName.value || "table"}_${foreignKeys.value.length + 1}`, columns: columns.value[0]?.name ? [columns.value[0].name] : [], referencedDatabase: props.database, referencedTable: "", referencedColumns: ["id"], onUpdate: "RESTRICT", onDelete: "RESTRICT" }); }
function addCheck() { checks.value.push({ id: crypto.randomUUID(), name: `chk_${tableName.value || "table"}_${checks.value.length + 1}`, expression: "", enforced: true }); }
function setColumns(target: { columns: string[] }, value: string) { target.columns = value.split(",").map((item) => item.trim()).filter(Boolean); }
function setReferencedColumns(target: TableForeignKeyDefinition, value: string) { target.referencedColumns = value.split(",").map((item) => item.trim()).filter(Boolean); }
function setColumnsFromEvent(target: { columns: string[] }, event: Event) { setColumns(target, (event.currentTarget as HTMLInputElement).value); }
function setReferencedColumnsFromEvent(target: TableForeignKeyDefinition, event: Event) { setReferencedColumns(target, (event.currentTarget as HTMLInputElement).value); }
async function removeIndex(id: string) {
  const index = indexes.value.find((item) => item.id === id);
  if (!index || !await confirmDefinitionRemoval("索引", index.name || "未命名索引")) return;
  indexes.value = indexes.value.filter((item) => item.id !== id);
}
async function removeForeignKey(id: string) {
  const foreignKey = foreignKeys.value.find((item) => item.id === id);
  if (!foreignKey || !await confirmDefinitionRemoval("外键", foreignKey.name || "未命名外键")) return;
  foreignKeys.value = foreignKeys.value.filter((item) => item.id !== id);
}
async function removeCheck(id: string) {
  const check = checks.value.find((item) => item.id === id);
  if (!check || !await confirmDefinitionRemoval("检查约束", check.name || "未命名约束")) return;
  checks.value = checks.value.filter((item) => item.id !== id);
}
function create() { if (canCreate.value) emit("create", definition.value); }
</script>

<template>
  <form class="create-table-editor" @submit.prevent="create">
    <header class="create-table-editor-header">
      <div class="create-table-heading-icon"><Table2 :size="16" /></div>
      <div class="create-table-heading-copy">
        <span class="create-table-eyebrow">{{ mode === 'alter' ? '修改表结构' : '新建表' }}</span>
        <label class="table-name-field"><span class="database-name"><Database :size="12" />{{ database }}</span><input v-model="tableName" v-bind="textInputAttributes" aria-label="表名" autofocus maxlength="64" placeholder="输入表名" /></label>
      </div>
      <div class="create-table-header-actions"><button type="button" class="secondary compact" :disabled="busy" @click="emit('cancel')">关闭</button><button class="primary compact create-table-submit" :disabled="!canCreate"><Table2 :size="14" />{{ busy ? '执行中…' : mode === 'alter' ? '应用修改' : '创建表' }}</button></div>
    </header>

    <nav class="create-table-tabs" role="tablist" aria-label="表设计分区">
      <button v-for="tab in sections" :key="tab.id" type="button" role="tab" :class="{ active: activeSection === tab.id }" :aria-selected="activeSection === tab.id" @click="activeSection = tab.id">{{ tab.label }}</button>
    </nav>

    <fieldset class="create-table-body" :disabled="busy">
      <section v-if="activeSection === 'fields'" class="fields-panel" role="tabpanel">
        <div class="fields-card-header"><span class="field-count">{{ columns.length }} 个字段</span><button type="button" class="secondary compact add-column" @click="addColumn"><Plus :size="14" />添加字段</button></div>
        <div class="create-table-columns-scroll"><table class="create-table-columns data-grid" :style="fieldTableStyle"><colgroup><col v-for="(fieldColumn, fieldColumnIndex) in fieldColumnLayout" :key="fieldColumn.key" :style="{ width: `${fieldColumnWidths[fieldColumnIndex]}px` }"></colgroup><thead><tr><th v-for="(fieldColumn, fieldColumnIndex) in fieldColumnLayout" :key="fieldColumn.key" :class="{ 'resizable-column': fieldColumn.resizable }">{{ fieldColumn.label }}<span v-if="fieldColumn.resizable" class="column-resizer" role="separator" aria-orientation="vertical" :aria-label="`调整${fieldColumn.label}列宽`" :aria-valuenow="fieldColumnWidths[fieldColumnIndex]" :aria-valuemin="MIN_FIELD_COLUMN_WIDTH" :aria-valuemax="MAX_FIELD_COLUMN_WIDTH" tabindex="0" @pointerdown="startFieldColumnResize($event, fieldColumnIndex)" @pointermove="resizeFieldColumn" @pointerup="finishFieldColumnResize" @pointercancel="finishFieldColumnResize" @dblclick.stop="resetFieldColumnWidth(fieldColumnIndex)" @keydown="resizeFieldColumnWithKeyboard($event, fieldColumnIndex)" /></th></tr></thead><tbody>
          <tr v-for="(column, index) in columns" :key="column.id">
            <td><input v-model="column.name" v-bind="textInputAttributes" :aria-label="`第 ${index + 1} 个字段名称`" maxlength="64" placeholder="字段名" /></td>
            <td><AppSelect v-model="column.dataType" :options="columnTypes.map((dataType) => ({ value: dataType, label: dataType }))" :label="`字段 ${column.name || index + 1} 的类型`" variant="cell" @change="updateDataType(column)" /></td>
            <td v-if="!sqlite"><input v-model="column.size" v-bind="textInputAttributes" :aria-label="`字段 ${column.name || index + 1} 的长度或精度`" :disabled="!columnTypeSupportsSize(column.dataType)" :placeholder="sizePlaceholder(column.dataType)" /></td>
            <td><input v-model="column.defaultValue" v-bind="textInputAttributes" :aria-label="`字段 ${column.name || index + 1} 的默认值`" placeholder="无" /></td>
            <td v-if="!sqlite"><input v-model="column.comment" v-bind="textInputAttributes" :aria-label="`字段 ${column.name || index + 1} 的注释`" placeholder="可选" /></td>
            <td><input v-model="column.collation" v-bind="textInputAttributes" :aria-label="`字段 ${column.name || index + 1} 的排序规则`" placeholder="默认" /></td>
            <td><input v-model="column.generatedExpression" v-bind="textInputAttributes" :aria-label="`字段 ${column.name || index + 1} 的生成表达式`" placeholder="可选" /></td>
            <td class="column-flag"><input v-model="column.generatedStored" type="checkbox" :aria-label="`字段 ${column.name || index + 1} 的生成值采用存储方式`" :disabled="!column.generatedExpression?.trim()" /></td>
            <td v-if="!sqlite" class="column-flag"><input v-model="column.unsigned" type="checkbox" :aria-label="`字段 ${column.name || index + 1} 无符号`" :disabled="!columnTypeSupportsUnsigned(column.dataType)" /></td>
            <td class="column-flag"><input v-model="column.nullable" type="checkbox" :aria-label="`字段 ${column.name || index + 1} 允许为空`" @change="updateNullable(column)" /></td>
            <td class="column-flag"><input v-model="column.primaryKey" type="checkbox" :aria-label="`字段 ${column.name || index + 1} 设为主键`" @change="updatePrimaryKey(column)" /></td>
            <td class="column-flag"><input v-model="column.autoIncrement" type="checkbox" :aria-label="`字段 ${column.name || index + 1} 自动递增`" :disabled="!columnTypeSupportsAutoIncrement(column.dataType)" @change="updateAutoIncrement(column)" /></td>
            <td><button type="button" class="icon-button remove-column" :aria-label="`删除字段 ${column.name || index + 1}`" @click="removeColumn(column.id)"><Trash2 :size="14" /></button></td>
          </tr>
        </tbody></table></div>
      </section>

      <section v-else-if="activeSection === 'indexes'" class="definition-list-panel"><div class="fields-card-header"><span>{{ indexes.length }} 个索引</span><button type="button" class="secondary compact" @click="addIndex"><Plus :size="14" />添加索引</button></div><div v-for="index in indexes" :key="index.id" class="definition-row"><input v-model="index.name" v-bind="textInputAttributes" aria-label="索引名称" placeholder="索引名称" :disabled="index.preserveRaw" /><input :value="index.columns.join(', ')" v-bind="textInputAttributes" aria-label="索引字段" placeholder="字段，逗号分隔" :disabled="index.preserveRaw" @input="setColumnsFromEvent(index, $event)" /><AppSelect v-model="index.indexType" :options="indexTypeOptions" label="索引类型" variant="compact" :disabled="index.preserveRaw" /><label><input v-model="index.unique" type="checkbox" :disabled="index.preserveRaw || index.indexType !== 'INDEX'" />唯一</label><button type="button" class="icon-button" :aria-label="`删除索引 ${index.name || '未命名索引'}`" @click="removeIndex(index.id)"><Trash2 :size="14" /></button></div></section>
      <section v-else-if="activeSection === 'foreignKeys'" class="definition-list-panel"><div class="fields-card-header"><span>{{ foreignKeys.length }} 个外键</span><button type="button" class="secondary compact" @click="addForeignKey"><Plus :size="14" />添加外键</button></div><div v-for="foreignKey in foreignKeys" :key="foreignKey.id" class="definition-row foreign-key-row" :class="{ 'sqlite-definition-row': sqlite }"><input v-model="foreignKey.name" v-bind="textInputAttributes" aria-label="外键名称" placeholder="外键名称" /><input :value="foreignKey.columns.join(', ')" v-bind="textInputAttributes" aria-label="本表字段" placeholder="本表字段" @input="setColumnsFromEvent(foreignKey, $event)" /><input v-if="!sqlite" v-model="foreignKey.referencedDatabase" v-bind="textInputAttributes" aria-label="引用数据库" placeholder="数据库" /><input v-model="foreignKey.referencedTable" v-bind="textInputAttributes" aria-label="引用表" placeholder="引用表" /><input :value="foreignKey.referencedColumns.join(', ')" v-bind="textInputAttributes" aria-label="引用字段" placeholder="引用字段" @input="setReferencedColumnsFromEvent(foreignKey, $event)" /><AppSelect v-model="foreignKey.onDelete" :options="['RESTRICT', 'CASCADE', 'SET NULL', 'NO ACTION'].map((value) => ({ value, label: value }))" label="删除规则" variant="compact" /><AppSelect v-model="foreignKey.onUpdate" :options="['RESTRICT', 'CASCADE', 'SET NULL', 'NO ACTION'].map((value) => ({ value, label: value }))" label="更新规则" variant="compact" /><button type="button" class="icon-button" :aria-label="`删除外键 ${foreignKey.name || '未命名外键'}`" @click="removeForeignKey(foreignKey.id)"><Trash2 :size="14" /></button></div></section>
      <section v-else-if="activeSection === 'checks'" class="definition-list-panel"><div class="fields-card-header"><span>{{ checks.length }} 个检查约束</span><button type="button" class="secondary compact" @click="addCheck"><Plus :size="14" />添加约束</button></div><div v-for="check in checks" :key="check.id" class="definition-row check-row" :class="{ 'sqlite-definition-row': sqlite }"><input v-model="check.name" v-bind="textInputAttributes" aria-label="检查约束名称" placeholder="约束名称" /><input v-model="check.expression" v-bind="textInputAttributes" aria-label="检查约束表达式" placeholder="例如 amount &gt;= 0" /><label v-if="!sqlite"><input v-model="check.enforced" type="checkbox" />强制执行</label><button type="button" class="icon-button" :aria-label="`删除检查约束 ${check.name || '未命名约束'}`" @click="removeCheck(check.id)"><Trash2 :size="14" /></button></div></section>
      <section v-else-if="activeSection === 'options'" class="table-options-panel"><label><span>存储引擎</span><AppSelect v-model="engine" :options="['InnoDB', 'MyISAM', 'MEMORY'].map((value) => ({ value, label: value }))" label="存储引擎" /></label><label><span>字符集</span><AppSelect v-model="charset" :options="['utf8mb4', 'utf8', 'latin1', 'ascii'].map((value) => ({ value, label: value }))" label="字符集" /></label><label><span>表排序规则</span><input v-model="collation" v-bind="textInputAttributes" placeholder="例如 utf8mb4_0900_ai_ci" /></label><label><span>表注释</span><input v-model="tableComment" v-bind="textInputAttributes" /></label><label class="wide"><span>分区定义</span><textarea v-model="partitionClause" v-bind="textInputAttributes" rows="5" placeholder="PARTITION BY HASH(id) PARTITIONS 4" /></label></section>
      <section v-else class="sql-preview-panel" role="tabpanel"><pre v-if="sqlPreview" class="sql-preview-code">{{ sqlPreview }}</pre><div v-else class="sql-preview-empty"><AlertCircle :size="18" /><span>{{ mode === 'alter' ? '当前没有结构变更' : '完善表名和字段定义后即可预览 SQL' }}</span></div></section>

      <div v-if="displayedValidationMessage || error" class="create-table-messages"><p v-if="displayedValidationMessage" class="create-table-validation" role="status"><AlertCircle :size="13" />{{ displayedValidationMessage }}</p><p v-if="error" class="test-message error" role="alert">{{ error }}</p></div>
    </fieldset>
    <footer class="create-table-editor-footer"><div class="create-table-summary"><span><b>{{ columns.length }}</b> 字段</span><span><b>{{ primaryKeyCount }}</b> 主键字段</span><span><b>{{ indexes.length }}</b> 索引</span><span><b>{{ foreignKeys.length }}</b> 外键</span><span><b>{{ checks.length }}</b> 检查约束</span><span v-if="hasAutoIncrement">自增</span><span v-if="!sqlite">{{ engine }} · {{ charset }}</span><span v-else>SQLite</span></div><span>修改保留在当前标签页</span></footer>
    <Teleport to="body"><ActionDialog v-if="actionDialog" :key="actionDialog.id" :state="actionDialog" @confirm="acceptActionDialog" @cancel="cancelActionDialog" /></Teleport>
  </form>
</template>
