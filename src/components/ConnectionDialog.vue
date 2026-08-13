<script setup lang="ts">
import { computed, onMounted, reactive, ref, toRaw } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import {
  AlertCircle,
  Cable,
  CheckCircle2,
  ChevronRight,
  CircleCheck,
  CircleGauge,
  Eye,
  Network,
  ShieldAlert,
  SlidersHorizontal,
} from "lucide-vue-next";
import { api } from "@/lib/api";
import { useActionDialog } from "@/lib/actionDialog";
import ActionDialog from "@/components/ActionDialog.vue";
import AppDialog from "@/components/AppDialog.vue";
import AppSelect from "@/components/AppSelect.vue";
import type { ConnectionProfile, DatabaseKind } from "@/types";

const props = defineProps<{ initial?: ConnectionProfile | null }>();
const { actionDialog, confirmAction, acceptActionDialog, cancelActionDialog } = useActionDialog();
const emit = defineEmits<{ close: []; save: [profile: ConnectionProfile, password?: string] }>();
const now = new Date().toISOString();
const profile = reactive<ConnectionProfile>(props.initial ? structuredClone(toRaw(props.initial)) : {
  id: crypto.randomUUID(), driverKind: "mysql", group: null, name: "", host: "127.0.0.1", port: 3306, username: "root", database: null,
  tls: { mode: "disabled", caCertPath: null, clientCertPath: null, clientKeyPath: null }, ssh: null,
  connectTimeoutSecs: 5, queryTimeoutSecs: 30, poolSize: 5, readOnly: false, production: false,
  color: "#16a085", createdAt: now, updatedAt: now,
});
profile.driverKind ??= "mysql";
profile.group ??= null;
const password = ref("");
const passwordStored = ref<boolean | null>(props.initial ? null : false);
const testing = ref(false);
const testMessage = ref("");
const testState = ref<"success" | "error" | null>(null);
const needsPassword = computed(() => profile.driverKind !== "sqlite");
const valid = computed(() => profile.name.trim() && profile.host.trim() && (
  profile.driverKind === "sqlite" || (profile.username.trim() && profile.port > 0)
));
const canSave = computed(() => Boolean(valid.value) && (
  !needsPassword.value || !props.initial || (passwordStored.value !== null && (passwordStored.value || password.value.length > 0))
));
const passwordPlaceholder = computed(() => props.initial && passwordStored.value !== false ? "留空则保留原密码" : "请输入密码");

function messageOf(error: unknown) {
  return typeof error === "object" && error && "message" in error ? String(error.message) : String(error);
}

onMounted(async () => {
  if (!props.initial || !needsPassword.value) { if (!needsPassword.value) passwordStored.value = true; return; }
  try {
    passwordStored.value = await api.hasConnectionPassword(props.initial.id);
  } catch (error) {
    passwordStored.value = false;
    testState.value = "error";
    testMessage.value = `无法读取已保存密码：${messageOf(error)}`;
  }
});

async function test() {
  if (props.initial && passwordStored.value === false && !password.value) {
    testState.value = "error";
    testMessage.value = "没有已保存的密码，请先重新输入密码";
    return;
  }
  testing.value = true;
  testState.value = null;
  testMessage.value = "";
  try {
    const info = await api.testConnection(profile, password.value || undefined);
    testState.value = "success";
    const label = profile.driverKind === "mariadb" ? "MariaDB" : profile.driverKind === "postgresql" ? "PostgreSQL" : profile.driverKind === "sqlite" ? "SQLite" : "MySQL";
    testMessage.value = `连接成功 · ${label} ${info.serverVersion}${info.tlsCipher ? ` · TLS ${info.tlsCipher}` : ""}`;
  } catch (error) {
    const message = messageOf(error);
    const confirmationMarker = "SSH_HOST_KEY_CONFIRM_REQUIRED|";
    const changedMarker = "SSH_HOST_KEY_CHANGED|";
    if (profile.ssh && (message.includes(confirmationMarker) || message.includes(changedMarker))) {
      const marker = message.includes(confirmationMarker) ? confirmationMarker : changedMarker;
      const fingerprint = message.slice(message.indexOf(marker) + marker.length).trim();
      if (await confirmAction({
        title: marker === confirmationMarker ? "信任 SSH 主机？" : "SSH 主机密钥已变化",
        message: marker === confirmationMarker
          ? "这是首次连接该 SSH 主机。"
          : "服务器返回的 SSH 主机密钥与之前保存的指纹不同。",
        detail: `${marker === changedMarker ? "仅在确认服务器确实更换过密钥时继续。\n\n" : "请与服务器管理员核对以下公钥指纹：\n\n"}${fingerprint}`,
        tone: marker === changedMarker ? "danger" : "warning",
        confirmLabel: marker === changedMarker ? "接受新密钥" : "信任并继续",
      })) {
        profile.ssh.hostFingerprint = fingerprint;
        await test();
        return;
      }
    }
    testState.value = "error";
    testMessage.value = message;
  } finally { testing.value = false; }
}

async function pickCertificate(field: "caCertPath" | "clientCertPath" | "clientKeyPath" | "privateKeyPath") {
  const path = await open({ multiple: false, directory: false });
  if (!path) return;
  if (field === "privateKeyPath" && profile.ssh) profile.ssh.privateKeyPath = path;
  else if (field !== "privateKeyPath") profile.tls[field] = path;
}

async function pickSqliteFile() {
  const path = await open({ multiple: false, directory: false, filters: [{ name: "SQLite", extensions: ["db", "sqlite", "sqlite3"] }] });
  if (path && !Array.isArray(path)) profile.host = path;
}

function changeDriver(kind: DatabaseKind) {
  profile.driverKind = kind;
  profile.ssh = kind === "mysql" || kind === "mariadb" ? profile.ssh : null;
  if (kind === "sqlite") {
    profile.host = profile.host === "127.0.0.1" ? "database.sqlite3" : profile.host;
    profile.port = 1;
    profile.username = "";
    profile.database = "main";
    passwordStored.value = true;
  } else {
    if (!profile.username) profile.username = kind === "postgresql" ? "postgres" : "root";
    if (profile.port <= 1) profile.port = kind === "postgresql" ? 5432 : 3306;
    if (kind !== "postgresql" && profile.port === 5432) profile.port = 3306;
    if (kind === "postgresql" && profile.port === 3306) profile.port = 5432;
    if (!props.initial) passwordStored.value = false;
  }
}

function onDriverChange(kind: unknown) {
  if (["mysql", "mariadb", "postgresql", "sqlite"].includes(String(kind))) changeDriver(kind as DatabaseKind);
}

function toggleSsh(enabled: boolean) {
  profile.ssh = enabled ? {
    host: "", port: 22, username: profile.username, authMethod: "agent", privateKeyPath: null, useAgent: true, hostFingerprint: null,
  } : null;
}

function onSshToggle(event: Event) {
  toggleSsh((event.currentTarget as HTMLInputElement).checked);
}
</script>

<template>
  <AppDialog :title="initial ? '编辑连接' : '新建数据库连接'" title-id="connection-dialog-title" description="填写基础信息即可连接，更多选项可稍后设置。" as="form" dialog-class="connection-dialog" close-label="关闭连接窗口" @close="emit('close')" @submit="canSave && emit('save', profile, password || undefined)">
      <template #icon><Cable :size="18" :stroke-width="1.8" /></template>

      <div class="connection-dialog-body">
        <div class="form-grid connection-basics-grid">
          <label><span>数据库类型</span><AppSelect :model-value="profile.driverKind" :options="[{ value: 'mysql', label: 'MySQL' }, { value: 'mariadb', label: 'MariaDB' }, { value: 'postgresql', label: 'PostgreSQL' }, { value: 'sqlite', label: 'SQLite' }]" label="数据库类型" @update:model-value="onDriverChange" /></label>
          <label><span>连接分组</span><input v-model="profile.group" placeholder="例如：生产 / 测试" autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false" data-gramm="false" /></label>
          <label class="wide"><span>连接名称 <b>*</b></span><input v-model="profile.name" placeholder="请输入连接名称" autofocus required autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false" data-gramm="false" /></label>
          <label v-if="profile.driverKind === 'sqlite'" class="wide"><span>数据库文件 <b>*</b></span><div class="path-field"><input v-model="profile.host" required autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false" data-gramm="false" /><button type="button" class="secondary compact" @click="pickSqliteFile">选择</button></div></label>
          <template v-else><label class="wide"><span>主机 <b>*</b></span><input v-model="profile.host" placeholder="127.0.0.1" required autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false" data-gramm="false" /></label>
          <label><span>端口</span><input v-model.number="profile.port" type="number" min="1" max="65535" /></label>
          <label><span>默认数据库</span><input v-model="profile.database" placeholder="可选" autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false" data-gramm="false" /></label>
          <label><span>用户名 <b>*</b></span><input v-model="profile.username" autocomplete="username" required autocorrect="off" autocapitalize="none" spellcheck="false" data-gramm="false" /></label>
          <label><span>密码 <b v-if="!initial || passwordStored === false">*</b></span><input v-model="password" type="password" autocomplete="current-password" :required="!initial || passwordStored === false" :placeholder="passwordPlaceholder" autocorrect="off" autocapitalize="none" spellcheck="false" data-gramm="false" /><small v-if="initial && passwordStored === false && !password" class="password-warning">重启后未找到已保存密码，请重新输入</small></label>
          </template>
        </div>

        <div class="connection-flags">
          <label class="connection-flag connection-flag-readonly">
            <input v-model="profile.readOnly" type="checkbox" />
            <span class="connection-flag-icon" aria-hidden="true"><Eye :size="14" /></span>
            <span class="connection-flag-copy"><strong>只读连接</strong><small>阻止执行写入语句</small></span>
            <span class="connection-flag-switch" aria-hidden="true"><i /></span>
          </label>
          <label class="connection-flag connection-flag-production">
            <input v-model="profile.production" type="checkbox" />
            <span class="connection-flag-icon" aria-hidden="true"><ShieldAlert :size="14" /></span>
            <span class="connection-flag-copy"><strong>生产环境</strong><small>显示醒目标识</small></span>
            <span class="connection-flag-switch" aria-hidden="true"><i /></span>
          </label>
          <label v-if="profile.driverKind === 'mysql' || profile.driverKind === 'mariadb'" class="connection-flag connection-flag-ssh">
            <input :checked="Boolean(profile.ssh)" type="checkbox" @change="onSshToggle" />
            <span class="connection-flag-icon" aria-hidden="true"><Network :size="14" /></span>
            <span class="connection-flag-copy"><strong>SSH 隧道</strong><small>代理或私钥认证</small></span>
            <span class="connection-flag-switch" aria-hidden="true"><i /></span>
          </label>
        </div>

        <details class="connection-options" :open="Boolean(profile.ssh) || profile.tls.mode !== 'disabled'">
          <summary>
            <span class="connection-options-leading">
              <span class="connection-options-icon" aria-hidden="true"><SlidersHorizontal :size="14" /></span>
              <span class="connection-options-copy"><strong>高级连接设置</strong><small>TLS、超时、连接池与标识</small></span>
            </span>
            <ChevronRight class="connection-options-chevron" :size="15" aria-hidden="true" />
          </summary>
          <div class="form-grid connection-advanced">
          <label><span>TLS 模式</span><AppSelect v-model="profile.tls.mode" :options="[{ value: 'disabled', label: '关闭' }, { value: 'preferred', label: '优先' }, { value: 'required', label: '必须' }, { value: 'verify_ca', label: '校验 CA' }, { value: 'verify_identity', label: '校验主机名' }]" label="TLS 模式" /></label>
          <label v-if="profile.tls.mode === 'verify_ca' || profile.tls.mode === 'verify_identity'" class="wide"><span>CA 证书</span><div class="path-field"><input v-model="profile.tls.caCertPath" autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false" data-gramm="false" /><button type="button" class="secondary compact" @click="pickCertificate('caCertPath')">选择</button></div></label>
          <label v-if="profile.tls.mode !== 'disabled'" class="wide"><span>客户端证书（可选）</span><div class="path-field"><input v-model="profile.tls.clientCertPath" autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false" data-gramm="false" /><button type="button" class="secondary compact" @click="pickCertificate('clientCertPath')">选择</button></div></label>
          <label v-if="profile.tls.mode !== 'disabled'" class="wide"><span>客户端私钥（可选）</span><div class="path-field"><input v-model="profile.tls.clientKeyPath" autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false" data-gramm="false" /><button type="button" class="secondary compact" @click="pickCertificate('clientKeyPath')">选择</button></div></label>
          <label><span>连接超时（秒）</span><input v-model.number="profile.connectTimeoutSecs" type="number" min="1" max="300" /></label>
          <label><span>查询超时（秒）</span><input v-model.number="profile.queryTimeoutSecs" type="number" min="1" max="86400" /></label>
          <label><span>连接池上限</span><input v-model.number="profile.poolSize" type="number" min="1" max="32" /></label>
          <label><span>连接标识色</span><input v-model="profile.color" type="color" autocorrect="off" autocapitalize="none" spellcheck="false" data-gramm="false" /></label>
          </div>
          <div v-if="profile.ssh" class="form-grid connection-advanced ssh-options">
            <label class="wide"><span>SSH 主机</span><input v-model="profile.ssh.host" placeholder="bastion.example.com" autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false" data-gramm="false" /></label>
            <label><span>SSH 端口</span><input v-model.number="profile.ssh.port" type="number" min="1" max="65535" /></label>
            <label><span>SSH 用户名</span><input v-model="profile.ssh.username" autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false" data-gramm="false" /></label>
            <label><span>认证方式</span><AppSelect v-model="profile.ssh.authMethod" :options="[{ value: 'agent', label: 'SSH Agent' }, { value: 'private_key', label: '私钥' }]" label="认证方式" /></label>
            <label v-if="profile.ssh.authMethod === 'private_key'" class="wide"><span>私钥文件</span><div class="path-field"><input v-model="profile.ssh.privateKeyPath" autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false" data-gramm="false" /><button type="button" class="secondary compact" @click="pickCertificate('privateKeyPath')">选择</button></div></label>
          </div>
        </details>
        <p v-if="testMessage" class="test-message" :class="testState" :role="testState === 'error' ? 'alert' : 'status'"><CheckCircle2 v-if="testState === 'success'" :size="15" /><AlertCircle v-else :size="15" />{{ testMessage }}</p>
      </div>

      <template #footer>
        <button type="button" class="secondary connection-test-button" :disabled="testing || !valid" @click="test">
          <span class="connection-action-icon" aria-hidden="true"><CircleGauge :size="14" :stroke-width="1.8" /></span>
          <span class="connection-action-label">{{ testing ? "正在测试…" : "测试连接" }}</span>
        </button>
        <button class="primary connection-save-button" :disabled="!canSave">
          <span class="connection-action-icon" aria-hidden="true"><CircleCheck :size="14" :stroke-width="1.8" /></span>
          <span class="connection-action-label">保存连接</span>
        </button>
      </template>
  </AppDialog>
  <ActionDialog v-if="actionDialog" :key="actionDialog.id" :state="actionDialog" @confirm="acceptActionDialog" @cancel="cancelActionDialog" />
</template>
