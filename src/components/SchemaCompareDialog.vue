<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import AppDialog from "@/components/AppDialog.vue";
import AppSelect from "@/components/AppSelect.vue";
import { api } from "@/lib/api";
import { alterTableSql, tableDetailToDefinition } from "@/lib/sql";
import type { ColumnInfo, ConnectionProfile, DatabaseInfo, DatabaseKind, DatabaseObjectKind, TableDetail, TableInfo, UUID } from "@/types";

const props = defineProps<{
  connections: ConnectionProfile[];
  initialConnectionId: UUID;
  initialDatabase?: string | null;
}>();
const emit = defineEmits<{ close: []; openSql: [sql: string] }>();

const sourceConnectionId = ref(props.initialConnectionId);
const targetConnectionId = ref(props.initialConnectionId);
const sourceDatabases = ref<DatabaseInfo[]>([]);
const targetDatabases = ref<DatabaseInfo[]>([]);
const source = ref(props.initialDatabase || "");
const target = ref("");
const report = ref("");
const migrationSql = ref("");
const rollbackSql = ref("");
const activeScript = ref<"migration" | "rollback">("migration");
const includeDrops = ref(false);
const busy = ref(false);
const error = ref("");

const sourceConnection = computed(() => props.connections.find((item) => item.id === sourceConnectionId.value));
const targetConnection = computed(() => props.connections.find((item) => item.id === targetConnectionId.value));
const sourceKind = computed(() => sourceConnection.value?.driverKind ?? "mysql");
const targetKind = computed(() => targetConnection.value?.driverKind ?? "mysql");
const script = computed(() => activeScript.value === "migration" ? migrationSql.value : rollbackSql.value);
const canCompare = computed(() => Boolean(source.value && target.value) && !busy.value);

function quote(name: string, kind: DatabaseKind = targetKind.value) { return kind === "mysql" || kind === "mariadb" ? `\`${name.replace(/`/g, "``")}\`` : `"${name.replace(/"/g, "\"\"")}"`; }
function qualified(database: string, name: string, kind: DatabaseKind = targetKind.value) { return `${quote(database, kind)}.${quote(name, kind)}`; }
function stripDefiner(ddl: string) { return ddl.replace(/\s+DEFINER\s*=\s*(?:`[^`]*`|'[^']*'|[^\s]+)@(?:`[^`]*`|'[^']*'|[^\s]+)\s*/gi, " "); }
function rewriteDatabase(ddl: string, from: string, to: string) {
  return stripDefiner(ddl)
    .split(`\`${from}\``).join(quote(to))
    .split(`"${from}"`).join(quote(to))
    .split(`${from}.`).join(`${to}.`);
}
function normalizedDdl(ddl: string, database: string, name: string) {
  return stripDefiner(ddl)
    .split(`\`${database}\``).join("`__database__`")
    .split(`\`${name}\``).join("`__object__`")
    .split(`"${database}"`).join("\"__database__\"")
    .split(`"${name}"`).join("\"__object__\"")
    .replace(/\s+/g, " ")
    .trim()
    .toLocaleLowerCase();
}
function dropObjectSql(kind: DatabaseObjectKind, database: string, name: string) {
  return `DROP ${kind.toUpperCase()} IF EXISTS ${qualified(database, name)};`;
}

function contextStatement(kind: DatabaseKind, database: string) {
  if (kind === "mysql" || kind === "mariadb") return `USE ${quote(database, kind)};`;
  if (kind === "postgresql") return `SET search_path TO ${quote(database, kind)}, public;`;
  return `-- SQLite 数据库：${database}`;
}

function renameTableSql(kind: DatabaseKind, database: string, from: string, to: string) {
  if (kind === "mysql" || kind === "mariadb") return `RENAME TABLE ${qualified(database, from, kind)} TO ${qualified(database, to, kind)};`;
  return `ALTER TABLE ${qualified(database, from, kind)} RENAME TO ${quote(to, kind)};`;
}

function columnDefinition(column: ColumnInfo, kind: DatabaseKind) {
  return [quote(column.name, kind), column.fullType || column.dataType, column.nullable ? "" : "NOT NULL", column.defaultValue ? `DEFAULT ${column.defaultValue}` : ""].filter(Boolean).join(" ");
}

async function loadSide(side: "source" | "target") {
  const connectionId = side === "source" ? sourceConnectionId.value : targetConnectionId.value;
  if (!connectionId) return;
  try {
    const databases = await api.listDatabases(connectionId);
    if (side === "source") {
      sourceDatabases.value = databases;
      if (!databases.some((item) => item.name === source.value)) source.value = databases[0]?.name ?? "";
    } else {
      targetDatabases.value = databases;
      if (!databases.some((item) => item.name === target.value)) {
        target.value = databases.find((item) => item.name !== source.value)?.name ?? databases[0]?.name ?? "";
      }
    }
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
  }
}

watch(sourceConnectionId, () => void loadSide("source"));
watch(targetConnectionId, () => void loadSide("target"));
onMounted(async () => { await Promise.all([loadSide("source"), loadSide("target")]); });

interface ObjectEntry { kind: DatabaseObjectKind; name: string; label: string }
async function listObjects(connectionId: UUID, database: string): Promise<ObjectEntry[]> {
  const [routines, triggers, events] = await Promise.all([
    api.listRoutines(connectionId, database),
    api.listTriggers(connectionId, database),
    api.listEvents(connectionId, database),
  ]);
  return [
    ...routines.map((item) => ({ kind: (item.routineType.toUpperCase() === "FUNCTION" ? "function" : "procedure") as DatabaseObjectKind, name: item.name, label: item.routineType.toUpperCase() === "FUNCTION" ? "函数" : "存储过程" })),
    ...triggers.map((item) => ({ kind: "trigger" as const, name: item.name, label: "触发器" })),
    ...events.map((item) => ({ kind: "event" as const, name: item.name, label: "事件" })),
  ];
}

async function compareTablePair(sourceTable: TableInfo, targetTable: TableInfo) {
  return Promise.all([
    api.tableDetail(sourceConnectionId.value, source.value, sourceTable.name),
    api.tableDetail(targetConnectionId.value, target.value, targetTable.name),
  ]);
}

function changedTableSql(sourceDetail: TableDetail, targetDetail: TableDetail) {
  const kind = targetKind.value;
  if (kind !== "mysql" && kind !== "mariadb") {
    const build = (desired: TableDetail, current: TableDetail) => {
      const desiredMap = new Map(desired.columns.map((column) => [column.name, column]));
      const currentMap = new Map(current.columns.map((column) => [column.name, column]));
      const statements: string[] = [];
      for (const column of desired.columns) {
        const existing = currentMap.get(column.name);
        if (!existing) statements.push(`ALTER TABLE ${qualified(target.value, desired.table.name, kind)} ADD COLUMN ${columnDefinition(column, kind)};`);
        else if (existing.fullType !== column.fullType || existing.nullable !== column.nullable || existing.defaultValue !== column.defaultValue) {
          if (kind === "postgresql") {
            if (existing.fullType !== column.fullType) statements.push(`ALTER TABLE ${qualified(target.value, desired.table.name, kind)} ALTER COLUMN ${quote(column.name, kind)} TYPE ${column.fullType};`);
            if (existing.nullable !== column.nullable) statements.push(`ALTER TABLE ${qualified(target.value, desired.table.name, kind)} ALTER COLUMN ${quote(column.name, kind)} ${column.nullable ? "DROP" : "SET"} NOT NULL;`);
            if (existing.defaultValue !== column.defaultValue) statements.push(`ALTER TABLE ${qualified(target.value, desired.table.name, kind)} ALTER COLUMN ${quote(column.name, kind)} ${column.defaultValue ? `SET DEFAULT ${column.defaultValue}` : "DROP DEFAULT"};`);
          } else statements.push(`-- SQLite 字段 ${column.name} 的类型/约束变化需要重建表，请人工审查`);
        }
      }
      for (const column of current.columns.filter((item) => !desiredMap.has(item.name))) {
        const drop = `ALTER TABLE ${qualified(target.value, desired.table.name, kind)} DROP COLUMN ${quote(column.name, kind)};`;
        statements.push(includeDrops.value ? drop : `-- 请确认后执行：${drop}`);
      }
      return statements.join("\n");
    };
    return { forward: build(sourceDetail, targetDetail), reverse: build(targetDetail, sourceDetail) };
  }
  const forward = alterTableSql(target.value, tableDetailToDefinition(targetDetail), tableDetailToDefinition(sourceDetail));
  const reverse = alterTableSql(target.value, tableDetailToDefinition(sourceDetail), tableDetailToDefinition(targetDetail));
  return { forward, reverse };
}

async function compare() {
  if (!source.value || !target.value) { error.value = "请选择源和目标数据库"; return; }
  if (sourceConnectionId.value === targetConnectionId.value && source.value === target.value) { error.value = "源和目标不能完全相同"; return; }
  busy.value = true;
  error.value = "";
  report.value = "";
  migrationSql.value = "";
  rollbackSql.value = "";
  try {
    const [sourceTables, targetTables, sourceObjects, targetObjects] = await Promise.all([
      api.listTables(sourceConnectionId.value, source.value, "", 100000, 0),
      api.listTables(targetConnectionId.value, target.value, "", 100000, 0),
      listObjects(sourceConnectionId.value, source.value),
      listObjects(targetConnectionId.value, target.value),
    ]);
    const targetMap = new Map(targetTables.map((table) => [table.name, table]));
    const sourceMap = new Map(sourceTables.map((table) => [table.name, table]));
    const lines: string[] = [];
    const sameDialect = sourceKind.value === targetKind.value || ([sourceKind.value, targetKind.value].every((kind) => kind === "mysql" || kind === "mariadb"));
    const statements: string[] = [`-- ${sourceConnection.value?.name ?? "源"}.${source.value} → ${targetConnection.value?.name ?? "目标"}.${target.value}`, contextStatement(targetKind.value, target.value)];
    const rollback: string[] = [`-- 回滚 ${targetConnection.value?.name ?? "目标"}.${target.value}`, contextStatement(targetKind.value, target.value)];
    if (!sameDialect) {
      lines.push(`数据库类型不同：${sourceKind.value} → ${targetKind.value}（仅生成审查注释）`);
      statements.push("-- 源和目标数据库类型不同：以下定义仅供人工转换与审查，不会生成可直接执行的跨方言迁移。");
      rollback.push("-- 跨数据库类型对比不自动生成回滚语句。");
    }

    const missing = sourceTables.filter((table) => !targetMap.has(table.name));
    const extras = targetTables.filter((table) => !sourceMap.has(table.name));
    const consumedExtras = new Set<string>();
    const consumedMissing = new Set<string>();

    for (const sourceTable of missing.filter((table) => !table.tableType.includes("VIEW"))) {
      const sourceDetail = await api.tableDetail(sourceConnectionId.value, source.value, sourceTable.name);
      for (const targetTable of extras.filter((table) => !table.tableType.includes("VIEW") && !consumedExtras.has(table.name))) {
        const targetDetail = await api.tableDetail(targetConnectionId.value, target.value, targetTable.name);
        if (normalizedDdl(sourceDetail.ddl, source.value, sourceTable.name) !== normalizedDdl(targetDetail.ddl, target.value, targetTable.name)) continue;
        lines.push(`可能重命名：${targetTable.name} → ${sourceTable.name}`);
        statements.push(sameDialect ? renameTableSql(targetKind.value, target.value, targetTable.name, sourceTable.name) : `-- 可能重命名：${targetTable.name} → ${sourceTable.name}`);
        if (sameDialect) rollback.unshift(renameTableSql(targetKind.value, target.value, sourceTable.name, targetTable.name));
        consumedExtras.add(targetTable.name);
        consumedMissing.add(sourceTable.name);
        break;
      }
    }

    for (const table of sourceTables) {
      const targetTable = targetMap.get(table.name);
      if (!targetTable && consumedMissing.has(table.name)) continue;
      if (!targetTable) {
        lines.push(`缺少${table.tableType.includes("VIEW") ? "视图" : "表"}：${table.name}`);
        const detail = await api.tableDetail(sourceConnectionId.value, source.value, table.name);
        statements.push(sameDialect ? `${rewriteDatabase(detail.ddl, source.value, target.value).trimEnd().replace(/;+$/, "")};` : `-- 源定义（需转换为目标方言）：\n${detail.ddl.split("\n").map((line) => `-- ${line}`).join("\n")}`);
        if (sameDialect) rollback.unshift(`DROP ${table.tableType.includes("VIEW") ? "VIEW" : "TABLE"} IF EXISTS ${qualified(target.value, table.name)};`);
        continue;
      }
      const [sourceDetail, targetDetail] = await compareTablePair(table, targetTable);
      if (normalizedDdl(sourceDetail.ddl, source.value, table.name) === normalizedDdl(targetDetail.ddl, target.value, table.name)) continue;
      lines.push(`结构不同：${table.name}`);
      if (!table.tableType.includes("VIEW")) {
        const change = sameDialect ? changedTableSql(sourceDetail, targetDetail) : { forward: "-- 跨方言表结构差异，请人工迁移", reverse: "" };
        if (change.forward) statements.push(change.forward);
        if (change.reverse) rollback.unshift(change.reverse);
      } else {
        statements.push(`DROP VIEW IF EXISTS ${qualified(target.value, table.name)};\n${rewriteDatabase(sourceDetail.ddl, source.value, target.value).replace(/;+$/, "")};`);
        rollback.unshift(`DROP VIEW IF EXISTS ${qualified(target.value, table.name)};\n${targetDetail.ddl.replace(/;+$/, "")};`);
      }
    }

    for (const table of extras) {
      if (consumedExtras.has(table.name)) continue;
      lines.push(`目标多出${table.tableType.includes("VIEW") ? "视图" : "表"}：${table.name}`);
      const drop = `DROP ${table.tableType.includes("VIEW") ? "VIEW" : "TABLE"} ${qualified(target.value, table.name)};`;
      statements.push(sameDialect && includeDrops.value ? drop : `-- 请确认后执行：${drop}`);
      if (sameDialect && includeDrops.value) {
        const detail = await api.tableDetail(targetConnectionId.value, target.value, table.name);
        rollback.unshift(`${detail.ddl.replace(/;+$/, "")};`);
      }
    }

    const targetObjectMap = new Map(targetObjects.map((item) => [`${item.kind}:${item.name}`, item]));
    const sourceObjectMap = new Map(sourceObjects.map((item) => [`${item.kind}:${item.name}`, item]));
    for (const object of sourceObjects) {
      const key = `${object.kind}:${object.name}`;
      const targetObject = targetObjectMap.get(key);
      const sourceDefinition = await api.objectDefinition(sourceConnectionId.value, source.value, object.kind, object.name);
      if (!targetObject) {
        lines.push(`缺少${object.label}：${object.name}`);
        statements.push(sameDialect ? `${rewriteDatabase(sourceDefinition.ddl, source.value, target.value).replace(/;+$/, "")};` : `-- 源${object.label}定义（需转换方言）：\n${sourceDefinition.ddl.split("\n").map((line) => `-- ${line}`).join("\n")}`);
        if (sameDialect) rollback.unshift(dropObjectSql(object.kind, target.value, object.name));
        continue;
      }
      const targetDefinition = await api.objectDefinition(targetConnectionId.value, target.value, object.kind, object.name);
      if (normalizedDdl(sourceDefinition.ddl, source.value, object.name) === normalizedDdl(targetDefinition.ddl, target.value, object.name)) continue;
      lines.push(`${object.label}不同：${object.name}`);
      statements.push(sameDialect ? `${dropObjectSql(object.kind, target.value, object.name)}\n${rewriteDatabase(sourceDefinition.ddl, source.value, target.value).replace(/;+$/, "")};` : `-- ${object.label} ${object.name} 的定义不同，需人工转换`);
      if (sameDialect) rollback.unshift(`${dropObjectSql(object.kind, target.value, object.name)}\n${targetDefinition.ddl.replace(/;+$/, "")};`);
    }
    for (const object of targetObjects) {
      if (sourceObjectMap.has(`${object.kind}:${object.name}`)) continue;
      lines.push(`目标多出${object.label}：${object.name}`);
      const drop = dropObjectSql(object.kind, target.value, object.name);
      statements.push(sameDialect && includeDrops.value ? drop : `-- 请确认后执行：${drop}`);
      if (sameDialect && includeDrops.value) {
        const definition = await api.objectDefinition(targetConnectionId.value, target.value, object.kind, object.name);
        rollback.unshift(`${definition.ddl.replace(/;+$/, "")};`);
      }
    }

    report.value = lines.length ? lines.join("\n") : "两个数据库的表、视图、过程、函数、触发器和事件结构一致";
    migrationSql.value = statements.join("\n\n");
    rollbackSql.value = rollback.join("\n\n");
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <AppDialog title="结构对比" title-id="schema-compare-title" description="跨连接比较全部数据库对象，并生成迁移与回滚脚本" dialog-class="schema-compare-dialog" close-label="关闭结构对比" @close="emit('close')">
      <div class="schema-compare-endpoints">
        <fieldset><legend>源</legend><AppSelect v-model="sourceConnectionId" :options="connections.map((connection) => ({ value: connection.id, label: connection.name }))" label="源连接" /><AppSelect v-model="source" :options="sourceDatabases.map((database) => ({ value: database.name, label: database.name }))" label="源数据库" /></fieldset>
        <span>→</span>
        <fieldset><legend>目标</legend><AppSelect v-model="targetConnectionId" :options="connections.map((connection) => ({ value: connection.id, label: connection.name }))" label="目标连接" /><AppSelect v-model="target" :options="targetDatabases.map((database) => ({ value: database.name, label: database.name }))" label="目标数据库" /></fieldset>
        <button class="primary compact" :disabled="!canCompare" @click="compare">{{ busy ? '比较中…' : '开始比较' }}</button>
      </div>
      <label class="schema-drop-option"><input v-model="includeDrops" type="checkbox" /><span><strong>迁移脚本包含删除目标多余对象 <b>高风险</b></strong><small>启用后会同时生成回滚定义；执行前仍应逐条审查迁移 SQL。</small></span></label>
      <p v-if="error" class="error-banner">{{ error }}</p>
      <div class="schema-compare-results"><pre>{{ report || '选择源和目标后开始比较' }}</pre><div><nav><button :class="{ active: activeScript === 'migration' }" @click="activeScript = 'migration'">迁移 SQL</button><button :class="{ active: activeScript === 'rollback' }" @click="activeScript = 'rollback'">回滚 SQL</button></nav><pre>{{ script || '-- 脚本将显示在这里' }}</pre></div></div>
      <template #footer><button class="secondary" @click="emit('close')">关闭</button><button class="primary" :disabled="!script" @click="emit('openSql', script)">在查询中审查</button></template>
  </AppDialog>
</template>
