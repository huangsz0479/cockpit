<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RefreshCw } from "lucide-vue-next";
import { api } from "@/lib/api";
import { cellText } from "@/lib/cell";
import { useActionDialog } from "@/lib/actionDialog";
import ActionDialog from "@/components/ActionDialog.vue";
import AppDialog from "@/components/AppDialog.vue";
import type { DatabaseKind, QueryResultPage, ServerLockInfo, ServerMetric, ServerProcessInfo, ServerVariable, UserAccount, UUID } from "@/types";

const props = withDefaults(defineProps<{ connectionId: UUID; databaseKind?: DatabaseKind }>(), { databaseKind: "mysql" });
const emit = defineEmits<{ close: []; openSql: [sql: string] }>();
const { actionDialog, confirmAction, acceptActionDialog, cancelActionDialog } = useActionDialog();
type AdminTab = "processes" | "status" | "variables" | "locks" | "replication" | "users";
const tab = ref<AdminTab>(props.databaseKind === "sqlite" || props.databaseKind === "elasticsearch" ? "status" : "processes");
const tabs = computed<AdminTab[]>(() => props.databaseKind === "sqlite" || props.databaseKind === "elasticsearch"
  ? ["status"]
  : props.databaseKind === "mysql" || props.databaseKind === "mariadb"
    ? ["processes", "status", "variables", "locks", "replication", "users"]
    : ["processes", "status", "variables", "locks", "users"]);
const processes = ref<ServerProcessInfo[]>([]);
const metrics = ref<ServerMetric[]>([]);
const users = ref<UserAccount[]>([]);
const variables = ref<ServerVariable[]>([]);
const locks = ref<ServerLockInfo[]>([]);
const replication = ref<QueryResultPage | null>(null);
const binaryLogs = ref<QueryResultPage | null>(null);
const grants = ref<string[]>([]);
const selectedUser = ref<UserAccount | null>(null);
const filter = ref("");
const busy = ref(false);
const error = ref("");
const filteredMetrics = computed(() => metrics.value.filter((item) => !filter.value || item.name.toLowerCase().includes(filter.value.toLowerCase())));
const filteredVariables = computed(() => variables.value.filter((item) => !filter.value || item.name.toLowerCase().includes(filter.value.toLowerCase())));
const summary = computed(() => {
  if (tab.value === "processes") return `${processes.value.length} 个会话`;
  if (tab.value === "status") return `${filteredMetrics.value.length} 个状态项`;
  if (tab.value === "variables") return `${filteredVariables.value.length} 个变量`;
  if (tab.value === "locks") return `${locks.value.length} 个锁等待`;
  if (tab.value === "replication") return `${replication.value?.rows.length ? "已连接副本" : "无副本状态"} · ${binaryLogs.value?.rows.length ?? 0} 个 Binlog`;
  return `${users.value.length} 个用户`;
});
function resultRows(page: QueryResultPage | null) {
  if (!page) return [];
  return page.rows.map((row) => Object.fromEntries(page.columns.map((column, index) => [column.name, cellText(row[index] ?? { kind: "null" })])));
}
async function executeAdminQuery(sql: string) {
  return api.execute(props.connectionId, null, { executionId: crypto.randomUUID(), sql, database: null, timeoutSecs: 15, allowWrite: false, pageSize: 500, rowOffset: 0 });
}
async function loadReplication() {
  replication.value = await executeAdminQuery("SHOW REPLICA STATUS");
  if (!replication.value.rows.length) replication.value = await executeAdminQuery("SHOW SLAVE STATUS");
  binaryLogs.value = await executeAdminQuery("SHOW BINARY LOGS");
}

async function load() {
  busy.value = true; error.value = "";
  try {
    if (tab.value === "processes") processes.value = await api.serverProcesses(props.connectionId);
    else if (tab.value === "status") metrics.value = await api.serverStatus(props.connectionId);
    else if (tab.value === "variables") variables.value = await api.serverVariables(props.connectionId, filter.value);
    else if (tab.value === "locks") locks.value = await api.serverLocks(props.connectionId);
    else if (tab.value === "replication") await loadReplication();
    else {
      users.value = await api.databaseUsers(props.connectionId);
      const previousSelection = selectedUser.value;
      const nextSelection = users.value.find((user) => user.user === previousSelection?.user && user.host === previousSelection?.host)
        ?? users.value[0]
        ?? null;
      selectedUser.value = null;
      grants.value = [];
      if (nextSelection) await selectUser(nextSelection);
    }
  } catch (cause) { error.value = cause instanceof Error ? cause.message : String(cause); }
  finally { busy.value = false; }
}
async function selectUser(user: UserAccount) {
  selectedUser.value = user; grants.value = [];
  try { grants.value = await api.userGrants(props.connectionId, user.user, user.host); }
  catch (cause) { error.value = cause instanceof Error ? cause.message : String(cause); }
}
async function kill(process: ServerProcessInfo) {
  if (!await confirmAction({
    title: "终止服务器会话？",
    message: `会话 ${process.id}（${process.user}@${process.host}）将被立即终止。`,
    detail: "该会话正在执行的语句会被中断，未提交事务通常会被回滚。",
    tone: "danger",
    confirmLabel: "终止会话",
  })) return;
  try { await api.killServerProcess(props.connectionId, process.id); await load(); }
  catch (cause) { error.value = cause instanceof Error ? cause.message : String(cause); }
}
function quote(value: string) { return `'${value.replace(/'/g, "''")}'`; }
function editUserSql(user: UserAccount) {
  if (props.databaseKind === "postgresql") {
    emit("openSql", `${grants.value.map((grant) => `-- ${grant}`).join("\n")}\n\n-- 使用 GRANT role_name TO ${quoteIdentifier(user.user)}; 或 GRANT ... ON ... TO ${quoteIdentifier(user.user)};`);
    return;
  }
  const account = `${quote(user.user)}@${quote(user.host)}`;
  emit("openSql", `${grants.value.map((grant) => `${grant};`).join("\n")}\n\n-- 在下方添加 GRANT、REVOKE 或 ALTER USER 语句\n-- GRANT SELECT ON database_name.* TO ${account};`);
}
function manageUserSql(user: UserAccount, action: "lock" | "drop") {
  if (props.databaseKind === "postgresql") {
    emit("openSql", action === "drop" ? `DROP ROLE ${quoteIdentifier(user.user)};` : `ALTER ROLE ${quoteIdentifier(user.user)} ${user.locked ? "LOGIN" : "NOLOGIN"};`);
    return;
  }
  const account = `${quote(user.user)}@${quote(user.host)}`;
  emit("openSql", action === "drop" ? `DROP USER ${account};` : `ALTER USER ${account} ACCOUNT ${user.locked ? "UNLOCK" : "LOCK"};`);
}
function newUserSql() {
  emit("openSql", props.databaseKind === "postgresql"
    ? "CREATE ROLE new_user LOGIN PASSWORD 'replace_password';\nGRANT CONNECT ON DATABASE database_name TO new_user;"
    : "CREATE USER 'new_user'@'%' IDENTIFIED BY 'replace_password';\nGRANT SELECT ON database_name.* TO 'new_user'@'%';");
}
function editVariable(variable: ServerVariable) {
  emit("openSql", props.databaseKind === "postgresql"
    ? `-- ALTER SYSTEM 需要相应权限\nALTER SYSTEM SET ${quoteIdentifier(variable.name)} = ${quote(variable.value)};\nSELECT pg_reload_conf();`
    : `-- 修改后执行；部分变量需要持久化到配置文件\nSET GLOBAL \`${variable.name.replace(/`/g, "``")}\` = ${quote(variable.value)};`);
}
function quoteIdentifier(value: string) { return `"${value.replace(/"/g, "\"\"")}"`; }
async function killBlocker(lock: ServerLockInfo) {
  if (!lock.blockingThreadId || !await confirmAction({
    title: "终止阻塞线程？",
    message: `线程 ${lock.blockingThreadId} 将被立即终止。`,
    detail: `当前正在阻塞线程 ${lock.waitingThreadId}${lock.objectName ? ` 对对象 ${lock.objectName} 的访问` : ""}。`,
    tone: "danger",
    confirmLabel: "终止线程",
  })) return;
  try { await api.killServerProcess(props.connectionId, lock.blockingThreadId); await load(); }
  catch (cause) { error.value = cause instanceof Error ? cause.message : String(cause); }
}
onMounted(load);
</script>

<template>
  <AppDialog title="服务器管理" title-id="server-admin-title" description="会话、运行状态与用户权限" dialog-class="server-admin-dialog" close-label="关闭服务器管理" @close="emit('close')">
      <nav class="admin-tabs" role="tablist" aria-label="服务器管理分类"><button v-for="item in tabs" :key="item" role="tab" :aria-selected="tab === item" :class="{ active: tab === item }" @click="tab = item; filter = ''; load()">{{ item === 'processes' ? '会话' : item === 'status' ? '状态' : item === 'variables' ? '变量' : item === 'locks' ? '锁等待' : item === 'replication' ? '复制 / Binlog' : '用户' }}</button></nav>
      <div class="admin-toolbar"><input v-if="tab === 'status' || tab === 'variables'" v-model="filter" type="search" :aria-label="tab === 'variables' ? '筛选服务器变量' : '筛选服务器状态项'" :placeholder="tab === 'variables' ? '筛选服务器变量' : '筛选状态项'" @keyup.enter="load" /><span class="admin-toolbar-summary">{{ summary }}</span><button class="ghost compact" :disabled="busy" @click="load"><RefreshCw :size="13" :class="{ 'loading-icon': busy }" />刷新</button><button v-if="tab === 'users'" class="ghost compact" @click="newUserSql">新建用户 SQL</button></div>
      <p v-if="error" class="error-banner">{{ error }}</p>
      <div v-if="tab === 'processes'" class="admin-content"><table v-if="processes.length"><thead><tr><th>ID</th><th>用户</th><th>主机</th><th>数据库</th><th>时间</th><th>状态 / SQL</th><th /></tr></thead><tbody><tr v-for="process in processes" :key="process.id"><td>{{ process.id }}</td><td>{{ process.user }}</td><td>{{ process.host }}</td><td>{{ process.database || '—' }}</td><td>{{ process.timeSecs }}s</td><td><span>{{ process.state || process.command }}</span><small>{{ process.sql || '' }}</small></td><td><button class="danger compact" @click="kill(process)">终止</button></td></tr></tbody></table><div v-else-if="!busy" class="admin-empty"><strong>没有活动会话</strong><span>当前服务器没有可显示的连接。</span></div></div>
      <div v-else-if="tab === 'status'" class="admin-content"><table v-if="filteredMetrics.length"><tbody><tr v-for="metric in filteredMetrics" :key="metric.name"><th>{{ metric.name }}</th><td>{{ metric.value }}</td></tr></tbody></table><div v-else-if="!busy" class="admin-empty"><strong>没有匹配的状态项</strong><span>清除筛选条件后再试。</span></div></div>
      <div v-else-if="tab === 'variables'" class="admin-content"><table v-if="filteredVariables.length"><thead><tr><th>变量</th><th>值</th><th>属性</th><th /></tr></thead><tbody><tr v-for="variable in filteredVariables" :key="variable.name"><th>{{ variable.name }}</th><td>{{ variable.value }}</td><td><span class="admin-state-badge" :class="{ dynamic: variable.dynamic }">{{ variable.dynamic ? '动态' : '只读' }}</span></td><td><button v-if="variable.dynamic" class="ghost compact" @click="editVariable(variable)">生成修改 SQL</button></td></tr></tbody></table><div v-else-if="!busy" class="admin-empty"><strong>没有匹配的变量</strong><span>尝试使用更短的筛选词。</span></div></div>
      <div v-else-if="tab === 'locks'" class="admin-content"><table><thead><tr><th>等待线程</th><th>阻塞线程</th><th>对象</th><th>锁</th><th>等待 SQL</th><th /></tr></thead><tbody><tr v-for="lock in locks" :key="`${lock.waitingThreadId}:${lock.objectName}`"><td>{{ lock.waitingThreadId }}</td><td>{{ lock.blockingThreadId ?? '—' }}</td><td>{{ lock.objectName || '—' }}</td><td>{{ lock.lockType }} · {{ lock.lockMode }} · {{ lock.lockStatus }}</td><td>{{ lock.waitingSql || '—' }}</td><td><button v-if="lock.blockingThreadId" class="danger compact" @click="killBlocker(lock)">终止阻塞</button></td></tr></tbody></table><p v-if="!locks.length" class="empty-small">当前没有锁等待</p></div>
      <div v-else-if="tab === 'replication'" class="admin-content replication-content"><h3>复制状态</h3><table v-if="replication?.rows.length"><tbody><tr v-for="(value, name) in resultRows(replication)[0]" :key="name"><th>{{ name }}</th><td>{{ value }}</td></tr></tbody></table><p v-else class="empty-small">当前实例未返回副本状态</p><h3>Binary Logs</h3><table v-if="binaryLogs?.rows.length"><thead><tr><th v-for="column in binaryLogs.columns" :key="column.name">{{ column.name }}</th></tr></thead><tbody><tr v-for="(row, index) in resultRows(binaryLogs)" :key="index"><td v-for="column in binaryLogs.columns" :key="column.name">{{ row[column.name] }}</td></tr></tbody></table><p v-else class="empty-small">未启用 Binlog 或当前账号无权查看</p></div>
      <div v-else-if="users.length" class="admin-user-layout"><div class="admin-user-list"><button v-for="user in users" :key="user.user + '@' + user.host" :class="{ active: selectedUser === user }" @click="selectUser(user)"><strong>{{ user.user || '(anonymous)' }}@{{ user.host }}</strong><small>{{ user.plugin || 'default' }}{{ user.locked ? ' · LOCKED' : '' }}</small></button></div><div class="admin-grants"><template v-if="selectedUser"><div class="admin-grants-heading"><div><strong>{{ selectedUser.user || '(anonymous)' }}@{{ selectedUser.host }}</strong><span>{{ grants.length }} 条授权</span></div><div class="toolbar-actions"><button class="primary compact" @click="editUserSql(selectedUser)">编辑权限</button><button class="secondary compact" @click="manageUserSql(selectedUser, 'lock')">{{ selectedUser.locked ? '解锁 SQL' : '锁定 SQL' }}</button><button class="danger compact" @click="manageUserSql(selectedUser, 'drop')">删除用户 SQL</button></div></div><pre>{{ grants.join('\n') }}</pre></template><p v-else>选择用户查看权限</p></div></div>
      <div v-else-if="!busy" class="admin-empty"><strong>没有数据库用户</strong><span>当前账号可能没有查看系统用户的权限。</span></div>
      <template #footer><span>{{ busy ? '正在加载…' : '' }}</span><button class="secondary" @click="emit('close')">关闭</button></template>
  </AppDialog>
  <ActionDialog v-if="actionDialog" :key="actionDialog.id" :state="actionDialog" @confirm="acceptActionDialog" @cancel="cancelActionDialog" />
</template>
