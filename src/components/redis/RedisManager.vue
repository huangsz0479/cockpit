<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { RefreshCw, Search, Terminal, Trash2, CircleGauge, X } from "lucide-vue-next";
import AppSelect from "@/components/AppSelect.vue";
import { api } from "@/lib/api";
import type {
  ConnectionProfile, RedisDatabaseInfo, RedisKeyInfo, RedisKeyType,
  RedisReply, RedisStringValue, RedisValue, ServerMetric,
} from "@/types";

const props = withDefaults(defineProps<{ connection: ConnectionProfile; initialDatabase?: number }>(), { initialDatabase: undefined });
const emit = defineEmits<{ close: [] }>();

const databases = ref<RedisDatabaseInfo[]>([]);
const selectedDatabase = ref(0);
const pattern = ref("");
const keys = ref<RedisKeyInfo[]>([]);
const cursor = ref(0);
const scanComplete = ref(false);
const scanning = ref(false);
const selectedKey = ref<RedisKeyInfo | null>(null);
const value = ref<RedisValue | null>(null);
const valueLoading = ref(false);
const error = ref("");

const activeSection = ref<"value" | "console" | "info">("value");
const commandText = ref("");
const commandReply = ref<RedisReply | null>(null);
const serverMetrics = ref<ServerMetric[]>([]);
const infoLoading = ref(false);
const stringEditor = ref("");

const totalKeys = computed(() => databases.value.find((db) => db.index === selectedDatabase.value)?.keyCount ?? null);
const keyOptions = computed(() => databases.value.map((db) => ({ value: db.index, label: `DB ${db.index}（${db.keyCount} 个 key）` })));

function messageOf(error: unknown) {
  return typeof error === "object" && error && "message" in error ? String((error as { message: unknown }).message) : String(error);
}

function utf8ToBase64(value: string) {
  const bytes = new TextEncoder().encode(value);
  let binary = "";
  bytes.forEach((byte) => { binary += String.fromCharCode(byte); });
  return btoa(binary);
}

function base64ToUtf8(value: string) {
  try {
    const binary = atob(value);
    const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
    return new TextDecoder().decode(bytes);
  } catch {
    return "";
  }
}

function stringText(item: RedisStringValue) {
  if (item.preview != null) return item.preview;
  const decoded = base64ToUtf8(item.valueBase64);
  return decoded || (item.length > 0 ? `<${item.length} 字节二进制>` : "");
}

function kindLabel(kind: RedisKeyType) {
  return ({ string: "String", hash: "Hash", list: "List", set: "Set", zset: "ZSet", stream: "Stream", none: "—" } as const)[kind];
}

function ttlLabel(ttlSecs: number) {
  if (ttlSecs === -1) return "永久";
  if (ttlSecs === -2) return "已过期";
  if (ttlSecs < 60) return `${ttlSecs}秒`;
  if (ttlSecs < 3600) return `${Math.floor(ttlSecs / 60)}分`;
  if (ttlSecs < 86400) return `${Math.floor(ttlSecs / 3600)}时`;
  return `${Math.floor(ttlSecs / 86400)}天`;
}

function collectionTruncated(item: RedisValue | null) {
  return item != null && item.kind !== "none" && item.kind !== "string" && item.truncated;
}

function collectionLength(item: RedisValue | null) {
  if (!item || item.kind === "none" || item.kind === "string") return 0;
  return item.length;
}

async function loadDatabases() {
  error.value = "";
  try {
    databases.value = await api.listRedisDatabases(props.connection.id);
    selectedDatabase.value = databases.value.find((db) => db.index === props.initialDatabase)?.index
      ?? databases.value[0]?.index
      ?? 0;
    await scanKeys(true);
  } catch (cause) {
    error.value = messageOf(cause);
  }
}

async function scanKeys(reset: boolean) {
  scanning.value = true;
  error.value = "";
  try {
    const page = await api.scanRedisKeys(
      props.connection.id,
      selectedDatabase.value,
      reset ? 0 : cursor.value,
      pattern.value.trim() || undefined,
      200,
    );
    keys.value = reset ? page.keys : [...keys.value, ...page.keys];
    cursor.value = page.cursor;
    scanComplete.value = page.complete;
  } catch (cause) {
    error.value = messageOf(cause);
  } finally {
    scanning.value = false;
  }
}

async function selectDatabase(index: number) {
  selectedDatabase.value = index;
  selectedKey.value = null;
  value.value = null;
  keys.value = [];
  cursor.value = 0;
  scanComplete.value = false;
  await scanKeys(true);
}

watch(
  () => props.initialDatabase,
  (database) => {
    if (database != null && database !== selectedDatabase.value) void selectDatabase(database);
  },
);

async function openKey(key: RedisKeyInfo) {
  selectedKey.value = key;
  valueLoading.value = true;
  error.value = "";
  value.value = null;
  try {
    value.value = await api.redisValue(props.connection.id, selectedDatabase.value, key.key, 500);
    if (value.value.kind === "string") stringEditor.value = base64ToUtf8(value.value.value.valueBase64);
  } catch (cause) {
    error.value = messageOf(cause);
  } finally {
    valueLoading.value = false;
  }
}

async function saveString() {
  if (!selectedKey.value || value.value?.kind !== "string") return;
  error.value = "";
  try {
    await api.setRedisString(
      props.connection.id,
      selectedDatabase.value,
      selectedKey.value.key,
      utf8ToBase64(stringEditor.value),
      value.value.ttlSecs > 0 ? value.value.ttlSecs : null,
    );
    await openKey(selectedKey.value);
    await scanKeys(true);
  } catch (cause) {
    error.value = messageOf(cause);
  }
}

function editString() {
  if (value.value?.kind === "string") stringEditor.value = base64ToUtf8(value.value.value.valueBase64);
}

async function deleteSelectedKey() {
  if (!selectedKey.value) return;
  await deleteKey(selectedKey.value);
}

async function deleteKey(key: RedisKeyInfo) {
  if (!window.confirm(`确定删除 key “${key.key}”？此操作不可恢复。`)) return;
  error.value = "";
  try {
    await api.deleteRedisKeys(props.connection.id, selectedDatabase.value, [key.key]);
    if (selectedKey.value?.key === key.key) {
      selectedKey.value = null;
      value.value = null;
    }
    await scanKeys(true);
  } catch (cause) {
    error.value = messageOf(cause);
  }
}

async function expireSelectedKey() {
  if (!selectedKey.value) return;
  const input = window.prompt("输入过期秒数（0 表示移除过期时间）", "300");
  if (input == null) return;
  const seconds = Number(input);
  if (!Number.isInteger(seconds) || seconds < 0) { error.value = "过期时间必须是非负整数"; return; }
  error.value = "";
  try {
    await api.expireRedisKey(props.connection.id, selectedDatabase.value, selectedKey.value.key, seconds);
    await openKey(selectedKey.value);
    await scanKeys(true);
  } catch (cause) {
    error.value = messageOf(cause);
  }
}

async function renameSelectedKey() {
  if (!selectedKey.value) return;
  const next = window.prompt("输入新的 key 名称", selectedKey.value.key);
  if (!next || next === selectedKey.value.key) return;
  error.value = "";
  try {
    await api.renameRedisKey(props.connection.id, selectedDatabase.value, selectedKey.value.key, next);
    selectedKey.value = null;
    value.value = null;
    await scanKeys(true);
  } catch (cause) {
    error.value = messageOf(cause);
  }
}

async function runCommand() {
  const args = commandText.value.trim().split(/\s+/).filter(Boolean);
  if (!args.length) return;
  error.value = "";
  try {
    commandReply.value = await api.runRedisCommand(props.connection.id, selectedDatabase.value, args, true);
  } catch (cause) {
    commandReply.value = { kind: "error", message: messageOf(cause) };
  }
}

async function loadServerInfo() {
  infoLoading.value = true;
  error.value = "";
  try {
    serverMetrics.value = await api.redisServerInfo(props.connection.id);
  } catch (cause) {
    error.value = messageOf(cause);
  } finally {
    infoLoading.value = false;
  }
}

function replyText(reply: RedisReply | null): string {
  if (!reply) return "";
  if (reply.kind === "nil") return "(nil)";
  if (reply.kind === "status") return reply.text;
  if (reply.kind === "integer") return String(reply.value);
  if (reply.kind === "error") return reply.message;
  if (reply.kind === "bulk_string") return reply.preview ?? base64ToUtf8(reply.base64) ?? `"${reply.base64}"`;
  return JSON.stringify(reply.items.map((item) => replyText(item)), null, 2);
}

onMounted(async () => {
  error.value = "";
  try {
    await api.connectRedis(props.connection.id);
    await loadDatabases();
  } catch (cause) {
    error.value = messageOf(cause);
  }
});

onBeforeUnmount(() => {
  void api.disconnectRedis(props.connection.id).catch(() => {});
});
</script>

<template>
  <section class="redis-manager">
      <div class="redis-manager-body">
        <aside class="redis-key-pane">
          <div class="redis-manager-toolbar">
            <AppSelect v-model="selectedDatabase" :options="keyOptions" label="逻辑库" variant="compact" @change="selectDatabase" />
            <button type="button" class="icon-button" aria-label="关闭 Redis 管理器" @click="emit('close')"><X :size="15" /></button>
          </div>
          <div class="redis-key-search">
            <Search :size="13" />
            <input v-model="pattern" type="search" placeholder="按 pattern 过滤，例如 user:*" @keyup.enter="scanKeys(true)" />
            <button class="ghost compact" :disabled="scanning" @click="scanKeys(true)"><RefreshCw :size="13" :class="{ 'loading-icon': scanning }" /></button>
          </div>
          <div class="redis-key-summary">
            <span>已加载 {{ keys.length }} 个 key<template v-if="totalKeys != null"> / 共 {{ totalKeys }} 个</template></span>
            <small v-if="!scanComplete">继续滚动加载更多</small>
          </div>
          <div class="redis-key-list" @scroll.passive="($event) => { const el = $event.currentTarget as HTMLElement; if (!scanComplete && !scanning && el.scrollHeight - el.scrollTop - el.clientHeight < 80) scanKeys(false); }">
            <div v-for="key in keys" :key="key.key" class="redis-key-item" :class="{ active: selectedKey?.key === key.key }" role="button" tabindex="0" @click="openKey(key)" @keydown.enter.prevent="openKey(key)" @keydown.space.prevent="openKey(key)">
              <span class="redis-key-name">{{ key.key }}</span>
              <span class="redis-key-badge">{{ kindLabel(key.kind) }}</span>
              <span v-if="key.ttlSecs !== -1" class="redis-key-ttl" :class="{ expired: key.ttlSecs === -2 }">{{ ttlLabel(key.ttlSecs) }}</span>
              <button type="button" class="redis-key-delete" :aria-label="`删除 key ${key.key}`" title="删除 key" @click.stop="deleteKey(key)"><Trash2 :size="13" /></button>
            </div>
            <p v-if="!keys.length && !scanning" class="empty-small">该逻辑库没有 key</p>
          </div>
        </aside>

        <section class="redis-detail-pane">
          <div class="redis-section-tabs">
            <button :class="{ active: activeSection === 'value' }" @click="activeSection = 'value'">值</button>
            <button :class="{ active: activeSection === 'console' }" @click="activeSection = 'console'"><Terminal :size="13" />控制台</button>
            <button :class="{ active: activeSection === 'info' }" @click="activeSection = 'info'; loadServerInfo()"><CircleGauge :size="13" />服务器信息</button>
          </div>

          <p v-if="error" class="error-banner">{{ error }}</p>

          <div v-if="activeSection === 'value'" class="redis-value-pane">
            <template v-if="!selectedKey"><div class="empty-small">选择一个 key 查看值</div></template>
            <template v-else>
              <div class="redis-key-heading">
                <div><strong>{{ selectedKey.key }}</strong><span>{{ kindLabel(selectedKey.kind) }} · {{ ttlLabel(selectedKey.ttlSecs) }}</span></div>
                <div class="toolbar-actions">
                  <button v-if="value?.kind === 'string'" class="secondary compact" @click="saveString">保存</button>
                  <button class="secondary compact" @click="expireSelectedKey">过期</button>
                  <button class="secondary compact" @click="renameSelectedKey">重命名</button>
                  <button class="danger compact" @click="deleteSelectedKey"><Trash2 :size="13" />删除</button>
                </div>
              </div>
              <div v-if="valueLoading" class="tree-loading">正在加载…</div>
              <div v-else-if="value" class="redis-value-content">
                <template v-if="value.kind === 'string'">
                  <textarea v-model="stringEditor" class="redis-string-editor" spellcheck="false" @focus="editString" />
                  <small v-if="value.value.preview == null && value.value.length > 0">该值是二进制数据，直接编辑可能损坏原始内容。</small>
                </template>
                <table v-else-if="value.kind === 'hash'" class="data-table"><thead><tr><th>字段</th><th>值</th></tr></thead><tbody><tr v-for="field in value.fields" :key="field.field.valueBase64"><td>{{ stringText(field.field) }}</td><td>{{ stringText(field.value) }}</td></tr></tbody></table>
                <table v-else-if="value.kind === 'zset'" class="data-table"><thead><tr><th>成员</th><th>分数</th></tr></thead><tbody><tr v-for="member in value.members" :key="member.value.valueBase64"><td>{{ stringText(member.value) }}</td><td>{{ member.score }}</td></tr></tbody></table>
                <table v-else-if="value.kind === 'list' || value.kind === 'set'" class="data-table"><thead><tr><th>#</th><th>值</th></tr></thead><tbody><tr v-for="(item, index) in value.values" :key="index"><td>{{ index }}</td><td>{{ stringText(item) }}</td></tr></tbody></table>
                <div v-else-if="value.kind === 'stream'" class="redis-stream-entries"><article v-for="entry in value.entries" :key="entry.id"><strong>{{ entry.id }}</strong><table class="data-table"><tbody><tr v-for="field in entry.fields" :key="field.field.valueBase64"><th>{{ stringText(field.field) }}</th><td>{{ stringText(field.value) }}</td></tr></tbody></table></article></div>
                <p v-else-if="value.kind === 'none'" class="empty-small">该 key 不存在或已过期</p>
                <p v-if="collectionTruncated(value)" class="empty-small">结果已截断，仅显示前 {{ collectionLength(value) }} 项中的一部分。</p>
              </div>
            </template>
          </div>

          <div v-else-if="activeSection === 'console'" class="redis-console-pane">
            <div class="redis-console-input"><Terminal :size="13" /><input v-model="commandText" placeholder="例如 GET user:1" @keyup.enter="runCommand" /><button class="primary compact" @click="runCommand">执行</button></div>
            <pre v-if="commandReply" class="redis-console-output">{{ replyText(commandReply) }}</pre>
            <p v-else class="empty-small">输入 Redis 命令并执行，例如 <code>GET key</code>、<code>HGETALL key</code>。</p>
          </div>

          <div v-else class="redis-info-pane">
            <table v-if="serverMetrics.length" class="data-table"><tbody><tr v-for="metric in serverMetrics" :key="metric.name"><th>{{ metric.name }}</th><td>{{ metric.value }}</td></tr></tbody></table>
            <p v-else-if="!infoLoading" class="empty-small">暂无服务器信息</p>
          </div>
        </section>
      </div>

  </section>
</template>

<style scoped>
.redis-manager { flex: 1; min-height: 0; display: flex; flex-direction: column; background: var(--surface-1); }
.redis-manager-body { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(340px, 400px) minmax(0, 1fr); }
.redis-key-pane { display: flex; flex-direction: column; min-width: 0; min-height: 0; border-right: 1px solid var(--border); background: var(--surface-sidebar, var(--surface-2)); }
.redis-manager-toolbar { display: flex; align-items: center; gap: 6px; padding: 8px; border-bottom: 1px solid var(--border-subtle); background: var(--surface-1); }
.redis-manager-toolbar .app-select { flex: 1; min-width: 0; }
.redis-manager-toolbar .icon-button { flex: 0 0 auto; }
.redis-key-search { display: flex; align-items: center; gap: 6px; padding: 8px; border-bottom: 1px solid var(--border-subtle); background: var(--surface-1); }
.redis-key-search svg { flex: 0 0 auto; color: var(--muted); }
.redis-key-search input { flex: 1; min-width: 0; height: 30px; min-height: 30px; padding: 0 8px; font-size: 11px; }
.redis-key-summary { display: flex; justify-content: space-between; gap: 8px; padding: 7px 10px; font-size: 10.5px; color: var(--muted); border-bottom: 1px solid var(--border-subtle); background: var(--surface-1); }
.redis-key-list { flex: 1; min-height: 0; overflow-y: auto; padding: 5px; }
.redis-key-item { width: 100%; min-height: 30px; display: flex; justify-content: flex-start; align-items: center; gap: 7px; padding: 4px 8px; border: 1px solid transparent; border-radius: var(--radius-sm); text-align: left; color: var(--text); cursor: pointer; }
.redis-key-item:hover { background: var(--surface-hover); }
.redis-key-item.active { border-color: color-mix(in srgb, var(--accent) 22%, var(--border-subtle)); background: var(--accent-soft); }
.redis-key-name { min-width: 0; flex: 1; font: 600 12px/1.35 var(--font-mono); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.redis-key-badge { height: 16px; flex: 0 0 auto; display: inline-flex; align-items: center; padding: 0 5px; border: 1px solid var(--border-subtle); border-radius: 999px; background: var(--surface-1); color: var(--muted); font-size: 9px; font-weight: 650; line-height: 1; }
.redis-key-item.active .redis-key-badge { border-color: color-mix(in srgb, var(--accent) 22%, var(--border-subtle)); background: color-mix(in srgb, var(--accent-soft) 72%, var(--surface-1)); color: var(--accent); }
.redis-key-ttl { flex: 0 0 auto; color: var(--muted); font-size: 10px; font-variant-numeric: tabular-nums; }
.redis-key-ttl.expired { color: var(--warning); }
.redis-key-delete { width: 22px; height: 22px; flex: 0 0 auto; display: inline-grid; place-items: center; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--muted); opacity: 0; transition: opacity .12s ease, background .12s ease, color .12s ease; }
.redis-key-item:hover .redis-key-delete, .redis-key-item:focus-within .redis-key-delete, .redis-key-delete:focus { opacity: 1; }
.redis-key-delete:hover { background: color-mix(in srgb, var(--danger, #d9534f) 12%, transparent); color: var(--danger, #d9534f); }
.redis-detail-pane { display: flex; flex-direction: column; min-width: 0; min-height: 0; background: var(--surface-1); }
.redis-section-tabs { display: flex; gap: 4px; padding: 7px 10px; border-bottom: 1px solid var(--border); background: var(--surface-2); }
.redis-section-tabs button { min-height: 28px; height: 28px; gap: 5px; padding: 0 11px; border: 1px solid transparent; border-radius: var(--radius-sm); color: var(--muted); font-size: 11px; font-weight: 600; }
.redis-section-tabs button:hover { background: var(--surface-hover); color: var(--text); }
.redis-section-tabs button.active { border-color: color-mix(in srgb, var(--accent) 24%, var(--border-subtle)); background: var(--surface-1); color: var(--accent); box-shadow: var(--shadow-xs); }
.redis-value-pane, .redis-console-pane, .redis-info-pane { flex: 1; min-height: 0; overflow: auto; padding: 12px; }
.redis-key-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 10px; padding: 8px 10px; border: 1px solid var(--border-subtle); border-radius: var(--radius-md); background: var(--surface-2); }
.redis-key-heading strong { font: 650 13px var(--font-mono); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.redis-key-heading span { color: var(--muted); font-size: 11px; margin-left: 8px; }
.redis-string-editor { width: 100%; min-height: 280px; resize: vertical; padding: 10px; border: 1px solid var(--border-strong); border-radius: var(--radius-md); background: var(--surface-1); color: var(--text); font: 12px/1.55 var(--font-mono); }
.redis-console-input { display: flex; align-items: center; gap: 7px; padding: 8px; border: 1px solid var(--border); border-radius: var(--radius-md); background: var(--surface-1); box-shadow: var(--shadow-xs); }
.redis-console-input svg { flex: 0 0 auto; color: var(--muted); }
.redis-console-input input { flex: 1; min-width: 0; height: 30px; border: 0; background: transparent; font: 12px var(--font-mono); }
.redis-console-output { margin-top: 10px; padding: 12px; border: 1px solid var(--border); border-radius: var(--radius-md); background: var(--surface-2); color: var(--text); white-space: pre-wrap; word-break: break-word; font: 12px/1.55 var(--font-mono); }
.redis-stream-entries { display: flex; flex-direction: column; gap: 10px; }
.redis-stream-entries article { padding: 8px 10px; border: 1px solid var(--border-subtle); border-radius: var(--radius-md); background: var(--surface-1); }
.redis-stream-entries article > strong { font: 650 12px var(--font-mono); }
.data-table { width: 100%; border-collapse: collapse; font-size: 12px; }
.data-table th, .data-table td { padding: 7px 9px; border-bottom: 1px solid var(--border-subtle); text-align: left; vertical-align: top; font: 12px var(--font-mono); overflow-wrap: anywhere; }
.data-table th { width: 180px; background: var(--surface-2); color: var(--muted); font-weight: 650; }
.data-table tbody tr:hover { background: var(--surface-hover); }
.data-table tr:last-child td { border-bottom: 0; }
</style>
