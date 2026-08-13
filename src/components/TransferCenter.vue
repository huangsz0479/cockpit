<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { Ban, CircleCheck, CircleX, LoaderCircle } from "lucide-vue-next";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "@/lib/api";
import AppDialog from "@/components/AppDialog.vue";
import AppSelect from "@/components/AppSelect.vue";
import type { BackupSchedule, ConnectionProfile, DatabaseInfo, TransferTask, UUID } from "@/types";

const props = defineProps<{
  tasks: TransferTask[];
  connections: ConnectionProfile[];
  schedule: BackupSchedule | null;
}>();
const emit = defineEmits<{
  close: [];
  cancel: [taskId: UUID];
  clear: [];
  saveSchedule: [schedule: BackupSchedule | null];
  runNow: [schedule: BackupSchedule];
}>();

const connectionId = ref(props.schedule?.connectionId ?? props.connections[0]?.id ?? "");
const database = ref(props.schedule?.database ?? "");
const directory = ref(props.schedule?.directory ?? "");
const intervalHours = ref(props.schedule?.intervalHours ?? 24);
const compression = ref<"none" | "gzip">(props.schedule?.compression ?? "gzip");
const includeData = ref(props.schedule?.includeData ?? true);
const enabled = ref(props.schedule?.enabled ?? false);
const databases = ref<DatabaseInfo[]>([]);
const error = ref("");

const validSchedule = computed(() => Boolean(connectionId.value && database.value && directory.value && intervalHours.value >= 1));
const activeTasks = computed(() => props.tasks.filter((task) => task.status === "running"));

function taskPercent(task: TransferTask) {
  if (task.total == null) return null;
  if (task.status === "completed") return 100;
  if (task.total === 0) return 0;
  return Math.min(99, Math.floor((task.completed / task.total) * 100));
}

async function loadDatabases() {
  if (!connectionId.value) return;
  try {
    databases.value = await api.listDatabases(connectionId.value);
    if (!databases.value.some((item) => item.name === database.value)) database.value = databases.value[0]?.name ?? "";
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
  }
}
watch(connectionId, () => void loadDatabases(), { immediate: true });

async function chooseDirectory() {
  const value = await open({ directory: true, multiple: false, title: "选择定时备份目录" });
  if (value && !Array.isArray(value)) directory.value = value;
}

function currentSchedule(): BackupSchedule {
  const existingNext = props.schedule?.nextRunAt;
  return {
    enabled: enabled.value,
    connectionId: connectionId.value,
    database: database.value,
    directory: directory.value,
    intervalHours: Math.max(1, intervalHours.value),
    nextRunAt: existingNext && new Date(existingNext).getTime() > Date.now()
      ? existingNext
      : new Date(Date.now() + Math.max(1, intervalHours.value) * 3_600_000).toISOString(),
    compression: compression.value,
    includeData: includeData.value,
  };
}

function saveSchedule() {
  emit("saveSchedule", enabled.value && validSchedule.value ? currentSchedule() : null);
}
</script>

<template>
  <AppDialog title="任务与备份" title-id="transfer-center-title" description="查看进度、取消任务并配置应用运行期间的定时备份" dialog-class="transfer-center-dialog" close-label="关闭任务与备份" @close="emit('close')">
      <div class="transfer-center-layout">
        <section>
          <div class="fields-card-header"><strong>任务记录</strong><span>{{ activeTasks.length }} 个运行中</span><button class="link" :disabled="tasks.some((task) => task.status === 'running')" @click="emit('clear')">清空记录</button></div>
          <div class="transfer-task-list" aria-live="polite">
            <article v-for="task in tasks" :key="task.taskId" :class="task.status">
              <div class="transfer-task-heading"><span class="transfer-task-icon"><LoaderCircle v-if="task.status === 'running'" :size="15" class="loading-icon" /><CircleCheck v-else-if="task.status === 'completed'" :size="15" /><CircleX v-else-if="task.status === 'failed'" :size="15" /><Ban v-else :size="15" /></span><span><strong>{{ task.title }}</strong><small>{{ new Date(task.startedAt).toLocaleString() }} · {{ task.phase }}</small></span></div>
              <progress v-if="taskPercent(task) !== null && (task.status === 'running' || task.status === 'completed')" :value="taskPercent(task) ?? undefined" max="100" :aria-label="`${task.title}进度`" />
              <span>{{ task.message || task.error || (task.status === 'completed' ? '已完成' : task.status === 'cancelled' ? '已取消' : task.status === 'failed' ? '失败' : '运行中') }}<template v-if="taskPercent(task) !== null"> · {{ taskPercent(task) }}%</template></span>
              <button v-if="task.status === 'running' && task.cancellable !== false" class="danger compact" @click="emit('cancel', task.taskId)">取消</button>
            </article>
            <p v-if="!tasks.length" class="empty-small">尚无导入、导出、备份或恢复任务</p>
          </div>
        </section>
        <section class="backup-schedule-form">
          <div class="fields-card-header"><strong>定时备份</strong><label class="check-row"><input v-model="enabled" type="checkbox" />启用</label></div>
          <p v-if="!connections.length" class="inline-empty-state">连接数据库后才能配置或立即运行备份。</p>
          <label>连接<AppSelect v-model="connectionId" :options="connections.map((connection) => ({ value: connection.id, label: connection.name }))" label="备份连接" :disabled="!connections.length" /></label>
          <label>数据库<AppSelect v-model="database" :options="databases.map((item) => ({ value: item.name, label: item.name }))" label="备份数据库" :disabled="!connectionId" /></label>
          <label>目录<div class="path-field"><input v-model="directory" readonly :disabled="!connections.length" /><button class="secondary compact" :disabled="!connections.length" @click="chooseDirectory">选择</button></div></label>
          <label>间隔（小时）<input v-model.number="intervalHours" type="number" min="1" max="8760" :disabled="!connections.length" /></label>
          <label>压缩<AppSelect v-model="compression" :options="[{ value: 'gzip', label: 'Gzip' }, { value: 'none', label: '不压缩' }]" label="备份压缩" :disabled="!connections.length" /></label>
          <label class="check-row"><input v-model="includeData" type="checkbox" :disabled="!connections.length" />包含表数据</label>
          <p v-if="schedule">下次运行：{{ new Date(schedule.nextRunAt).toLocaleString() }}</p>
          <p v-if="error" class="error-banner">{{ error }}</p>
          <div class="toolbar-actions"><button class="secondary" :disabled="!validSchedule" @click="emit('runNow', currentSchedule())">立即运行</button><button class="primary" :disabled="enabled && !validSchedule" @click="saveSchedule">保存计划</button></div>
        </section>
      </div>
      <template #footer><span /><button class="secondary" @click="emit('close')">关闭</button></template>
  </AppDialog>
</template>
