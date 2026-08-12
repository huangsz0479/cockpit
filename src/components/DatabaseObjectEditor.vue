<script setup lang="ts">
import { computed, reactive, watch } from "vue";
import AppSelect from "@/components/AppSelect.vue";
import type { DatabaseKind, DatabaseObjectDraft } from "@/types";

const props = withDefaults(defineProps<{
  database: string;
  databaseKind?: DatabaseKind;
  modelValue: DatabaseObjectDraft;
  existing?: boolean;
  busy?: boolean;
  error?: string | null;
}>(), { databaseKind: "mysql", existing: false, busy: false, error: null });
const emit = defineEmits<{ "update:modelValue": [draft: DatabaseObjectDraft]; openSql: [sql: string]; save: [sql: string] }>();
const draft = reactive<DatabaseObjectDraft>({ ...props.modelValue });
let syncingFromParent = false;

watch(() => props.modelValue, (value) => {
  if (JSON.stringify(value) === JSON.stringify(draft)) return;
  syncingFromParent = true;
  Object.assign(draft, value);
  syncingFromParent = false;
}, { deep: true });

watch(draft, (value) => {
  if (!syncingFromParent) emit("update:modelValue", { ...value });
}, { deep: true });

watch(() => draft.kind, (value) => {
  if (!props.existing) draft.name = `new_${value}`;
  draft.body = value === "view" ? "SELECT 1 AS value" : "BEGIN\n  -- SQL body\nEND";
});

const quote = (value: string) => props.databaseKind === "mysql" || props.databaseKind === "mariadb"
  ? `\`${value.replace(/`/g, "``")}\``
  : `"${value.replace(/"/g, "\"\"")}"`;
const qualified = computed(() => `${quote(props.database)}.${quote(draft.name.trim() || "unnamed")}`);
const generated = computed(() => {
  if (draft.kind === "view") return `${props.databaseKind === "sqlite" ? "CREATE VIEW" : "CREATE OR REPLACE VIEW"} ${qualified.value} AS\n${draft.body.trim()};`;
  if (props.databaseKind === "postgresql") {
    if (draft.kind === "procedure") return `CREATE OR REPLACE PROCEDURE ${qualified.value}(${draft.parameters.trim()})\nLANGUAGE plpgsql AS $$\n${draft.body.trim()}\n$$;`;
    if (draft.kind === "function") return `CREATE OR REPLACE FUNCTION ${qualified.value}(${draft.parameters.trim()}) RETURNS ${draft.returnType.trim()}\nLANGUAGE plpgsql AS $$\n${draft.body.trim()}\n$$;`;
    if (draft.kind === "trigger") {
      const functionName = `${draft.name.trim() || "new_trigger"}_fn`;
      return `CREATE OR REPLACE FUNCTION ${quote(props.database)}.${quote(functionName)}() RETURNS trigger\nLANGUAGE plpgsql AS $$\n${draft.body.trim()}\n$$;\n\nCREATE TRIGGER ${quote(draft.name.trim() || "new_trigger")} ${draft.timing} ${draft.event} ON ${quote(props.database)}.${quote(draft.table.trim() || "table_name")} FOR EACH ROW EXECUTE FUNCTION ${quote(props.database)}.${quote(functionName)}();`;
    }
    return "-- PostgreSQL 不支持 MySQL EVENT，请使用外部调度器或 pg_cron。";
  }
  if (props.databaseKind === "sqlite") {
    if (draft.kind === "trigger") return `CREATE TRIGGER ${quote(draft.name.trim() || "new_trigger")} ${draft.timing} ${draft.event} ON ${quote(draft.table.trim() || "table_name")} FOR EACH ROW\n${draft.body.trim()};`;
    return "-- SQLite 仅支持视图和触发器。";
  }
  if (draft.kind === "procedure") return `DROP PROCEDURE IF EXISTS ${qualified.value};\nCREATE PROCEDURE ${qualified.value}(${draft.parameters.trim()})\n${draft.body.trim()};`;
  if (draft.kind === "function") return `DROP FUNCTION IF EXISTS ${qualified.value};\nCREATE FUNCTION ${qualified.value}(${draft.parameters.trim()}) RETURNS ${draft.returnType.trim()}\nDETERMINISTIC\n${draft.body.trim()};`;
  if (draft.kind === "trigger") return `DROP TRIGGER IF EXISTS ${qualified.value};\nCREATE TRIGGER ${qualified.value} ${draft.timing} ${draft.event} ON ${quote(props.database)}.${quote(draft.table.trim() || "table_name")} FOR EACH ROW\n${draft.body.trim()};`;
  return `DROP EVENT IF EXISTS ${qualified.value};\nCREATE EVENT ${qualified.value} ON SCHEDULE ${draft.schedule.trim()} ENABLE DO\n${draft.body.trim()};`;
});
const output = computed(() => draft.mode === "ddl" ? draft.ddl : generated.value);
const canSave = computed(() => Boolean(
  draft.name.trim()
  && output.value.trim()
  && (draft.kind !== "trigger" || draft.table.trim())
  && (draft.kind !== "function" || draft.returnType.trim()),
));
function showDdl() {
  if (!draft.ddl) draft.ddl = generated.value;
  draft.mode = "ddl";
}
</script>

<template>
  <section class="dialog object-editor-dialog database-object-editor">
      <header><div><h2>数据库对象设计器</h2><p>{{ database }} · {{ databaseKind === 'sqlite' ? '视图和触发器' : databaseKind === 'postgresql' ? '视图、过程、函数和触发器' : '视图、过程、函数、触发器和事件' }}</p></div></header>
      <nav class="admin-tabs" role="tablist" aria-label="对象编辑模式"><button v-if="!existing" role="tab" :aria-selected="draft.mode === 'visual'" :class="{ active: draft.mode === 'visual' }" @click="draft.mode = 'visual'">可视化</button><button role="tab" :aria-selected="draft.mode === 'ddl'" :class="{ active: draft.mode === 'ddl' }" @click="showDdl">DDL</button></nav>
      <div v-if="draft.mode === 'visual'" class="object-editor-form" role="tabpanel">
        <label>对象类型<AppSelect v-model="draft.kind" :options="[{ value: 'view', label: '视图' }, ...databaseKind !== 'sqlite' ? [{ value: 'procedure', label: '存储过程' }, { value: 'function', label: '函数' }] : [], { value: 'trigger', label: '触发器' }, ...databaseKind === 'mysql' || databaseKind === 'mariadb' ? [{ value: 'event', label: '事件' }] : []]" label="对象类型" :disabled="existing" /></label>
        <label>名称<input v-model="draft.name" :readonly="existing" autocomplete="off" spellcheck="false" /></label>
        <label v-if="draft.kind === 'procedure' || draft.kind === 'function'">参数<input v-model="draft.parameters" placeholder="p_id BIGINT, OUT p_count INT" /></label>
        <label v-if="draft.kind === 'function'">返回类型<input v-model="draft.returnType" /></label>
        <template v-if="draft.kind === 'trigger'"><label>目标表<input v-model="draft.table" /></label><label>时机<AppSelect v-model="draft.timing" :options="['BEFORE', 'AFTER'].map((value) => ({ value, label: value }))" label="触发时机" /></label><label>事件<AppSelect v-model="draft.event" :options="['INSERT', 'UPDATE', 'DELETE'].map((value) => ({ value, label: value }))" label="触发事件" /></label></template>
        <label v-if="draft.kind === 'event'">计划<input v-model="draft.schedule" placeholder="EVERY 1 DAY 或 AT '2026-08-10 00:00:00'" /></label>
        <label class="wide">{{ draft.kind === 'view' ? 'SELECT 定义' : '对象主体' }}<textarea v-model="draft.body" rows="12" spellcheck="false" /></label>
        <pre>{{ generated }}</pre>
      </div>
      <textarea v-else v-model="draft.ddl" class="object-ddl-editor" role="tabpanel" aria-label="数据库对象 DDL" spellcheck="false" />
      <div v-if="error" class="error-banner"><span>{{ error }}</span></div>
      <footer><span>{{ existing ? '保存后将更新数据库中的对象' : '保存后将创建对象并刷新左侧列表' }}</span><button class="secondary" :disabled="busy || !output.trim()" @click="emit('openSql', output)">在查询 Tab 中审查</button><button class="primary" :disabled="busy || !canSave" @click="emit('save', output)">{{ busy ? '保存中…' : existing ? '保存修改' : '创建对象' }}</button></footer>
  </section>
</template>
