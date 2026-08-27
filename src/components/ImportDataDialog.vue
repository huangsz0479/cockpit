<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { FileUp, LoaderCircle } from "lucide-vue-next";
import { open } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api } from "@/lib/api";
import AppDialog from "@/components/AppDialog.vue";
import AppSelect from "@/components/AppSelect.vue";
import type {
  ImportColumnMapping, ImportConflictStrategy, ImportFormat, ImportPreview, TransferProgress, UUID,
} from "@/types";

const props = defineProps<{
  connectionId: UUID;
  database: string;
  table: string;
  targetColumns: string[];
}>();
const emit = defineEmits<{ close: []; imported: [rows: number] }>();

const inputPath = ref("");
const format = ref<Exclude<ImportFormat, "sql">>("csv");
const hasHeaders = ref(true);
const delimiter = ref("auto");
const encoding = ref<"utf-8" | "utf-8-lossy" | "gb18030">("utf-8");
const sheetName = ref("");
const preview = ref<ImportPreview | null>(null);
const mappings = ref<ImportColumnMapping[]>([]);
const nullValues = ref("NULL,\\N");
const trimValues = ref(true);
const conflictStrategy = ref<ImportConflictStrategy>("error");
const batchSize = ref(250);
const continueOnError = ref(false);
const loadingPreview = ref(false);
const importing = ref(false);
const error = ref("");
const progress = ref<TransferProgress | null>(null);
const resultMessage = ref("");
let taskId: UUID | null = null;
let unlisten: UnlistenFn | null = null;
let latestPreviewRequestId = 0;

const canImport = computed(() => Boolean(
  inputPath.value && preview.value && mappings.value.some((mapping) => mapping.target) && !importing.value,
));
const progressPercent = computed(() => {
  const value = progress.value;
  if (!value?.total) return null;
  return Math.min(100, Math.round((value.completed / value.total) * 100));
});

async function chooseFile() {
  const path = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "数据文件", extensions: ["csv", "tsv", "txt", "xlsx", "xls", "xlsb"] }],
  });
  if (!path || Array.isArray(path)) return;
  inputPath.value = path;
  format.value = /\.(xlsx|xls|xlsb)$/i.test(path) ? "excel" : "csv";
  sheetName.value = "";
  await loadPreview();
}

async function loadPreview() {
  if (!inputPath.value) return;
  const requestId = ++latestPreviewRequestId;
  loadingPreview.value = true;
  error.value = "";
  resultMessage.value = "";
  try {
    const value = await api.previewImport({
      inputPath: inputPath.value,
      format: format.value,
      hasHeaders: hasHeaders.value,
      sheetName: sheetName.value || null,
      delimiter: format.value === "csv" ? delimiter.value : null,
      encoding: encoding.value,
      previewRows: 50,
    });
    if (requestId !== latestPreviewRequestId) return;
    preview.value = value;
    sheetName.value = value.selectedSheet ?? "";
    mappings.value = value.columns.map((source, index) => ({
      source,
      target: props.targetColumns.includes(source) ? source : props.targetColumns[index] ?? null,
    }));
    if (format.value === "csv" && delimiter.value === "auto" && value.detectedDelimiter) {
      delimiter.value = value.detectedDelimiter;
    }
  } catch (cause) {
    if (requestId !== latestPreviewRequestId) return;
    preview.value = null;
    error.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    if (requestId === latestPreviewRequestId) loadingPreview.value = false;
  }
}

watch([hasHeaders, encoding], () => { if (inputPath.value) void loadPreview(); });
watch(sheetName, (value, previous) => {
  if (value && previous && value !== previous && format.value === "excel") void loadPreview();
});

async function startImport() {
  if (!canImport.value) return;
  taskId = crypto.randomUUID();
  importing.value = true;
  error.value = "";
  resultMessage.value = "";
  progress.value = { taskId, kind: "import", phase: "准备", completed: 0 };
  unlisten?.();
  unlisten = await listen<TransferProgress>("transfer-progress", ({ payload }) => {
    if (payload.taskId === taskId) progress.value = payload;
  });
  try {
    const summary = await api.importData({
      connectionId: props.connectionId,
      database: props.database,
      table: props.table,
      inputPath: inputPath.value,
      format: format.value,
      hasHeaders: hasHeaders.value,
      taskId,
      sheetName: sheetName.value || null,
      delimiter: format.value === "csv" ? delimiter.value : null,
      encoding: encoding.value,
      mappings: mappings.value,
      nullValues: nullValues.value.split(",").map((value) => value.trim()).filter(Boolean),
      trimValues: trimValues.value,
      conflictStrategy: conflictStrategy.value,
      batchSize: batchSize.value,
      continueOnError: continueOnError.value,
    });
    const errors = summary.errors?.length ?? 0;
    resultMessage.value = `已导入 ${summary.rowsImported} 行${summary.rowsSkipped ? `，跳过 ${summary.rowsSkipped} 行` : ""}${errors ? `，记录 ${errors} 个错误` : ""}`;
    emit("imported", summary.rowsImported);
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    importing.value = false;
    taskId = null;
    unlisten?.();
    unlisten = null;
  }
}

async function cancelImport() {
  if (taskId) await api.cancelTransfer(taskId);
}

onBeforeUnmount(() => {
  unlisten?.();
});
</script>

<template>
  <AppDialog title="导入数据" title-id="import-data-title" :description="`${database}.${table} · 预览、字段映射和冲突处理`" dialog-class="import-dialog" close-label="关闭导入数据" :close-disabled="importing" @close="emit('close')">
      <template #icon><FileUp :size="18" /></template>

      <div class="import-source-grid">
        <label class="wide">文件<div class="path-field"><input :value="inputPath" readonly placeholder="选择 CSV 或 Excel 文件" /><button class="secondary compact" :disabled="importing" @click="chooseFile">选择</button></div></label>
        <label><span>首行为字段名</span><input v-model="hasHeaders" type="checkbox" /></label>
        <label v-if="format === 'csv'">编码<AppSelect v-model="encoding" :options="[{ value: 'utf-8', label: 'UTF-8' }, { value: 'utf-8-lossy', label: 'UTF-8（替换错误字符）' }, { value: 'gb18030', label: 'GB18030' }]" label="文件编码" /></label>
        <label v-if="format === 'csv'">分隔符<AppSelect v-model="delimiter" :options="[{ value: 'auto', label: '自动检测' }, { value: ',', label: '逗号' }, { value: '\t', label: 'Tab' }, { value: ';', label: '分号' }, { value: '|', label: '竖线' }]" label="文件分隔符" @change="loadPreview" /></label>
        <label v-if="format === 'excel' && preview?.sheets.length">工作表<AppSelect v-model="sheetName" :options="preview.sheets.map((sheet) => ({ value: sheet, label: sheet }))" label="工作表" /></label>
      </div>

      <div v-if="loadingPreview" class="import-empty-state" role="status"><LoaderCircle :size="24" class="loading-icon" /><strong>正在读取预览</strong><span>正在识别字段和前 50 行数据…</span></div>
      <template v-else-if="preview">
        <div class="import-summary">{{ preview.totalRows }} 行 · {{ preview.columns.length }} 个来源字段</div>
        <div class="import-mapping-list">
          <label v-for="mapping in mappings" :key="mapping.source"><span>{{ mapping.source }}</span><b>→</b><AppSelect v-model="mapping.target" :options="[{ value: null, label: '忽略' }, ...targetColumns.map((column) => ({ value: column, label: column }))]" :label="`${mapping.source} 的目标字段`" variant="compact" /></label>
        </div>
        <div class="import-preview-table"><table><thead><tr><th v-for="column in preview.columns" :key="column">{{ column }}</th></tr></thead><tbody><tr v-for="(row, rowIndex) in preview.rows" :key="rowIndex"><td v-for="(value, columnIndex) in row" :key="columnIndex">{{ value }}</td></tr></tbody></table></div>
        <div class="import-options-grid">
          <label>冲突策略<AppSelect v-model="conflictStrategy" :options="[{ value: 'error', label: '遇错停止' }, { value: 'ignore', label: '忽略重复键' }, { value: 'replace', label: '替换整行' }, { value: 'upsert', label: '更新重复键' }]" label="冲突策略" /></label>
          <label>批次大小<input v-model.number="batchSize" type="number" min="1" max="2000" /></label>
          <label>NULL 标记<input v-model="nullValues" placeholder="NULL,\N" /></label>
          <label class="check-row"><input v-model="trimValues" type="checkbox" />去除首尾空白</label>
          <label class="check-row"><input v-model="continueOnError" type="checkbox" />跳过错误行并继续</label>
        </div>
      </template>
      <div v-else class="import-empty-state"><FileUp :size="26" /><strong>{{ error ? '无法读取文件预览' : '选择要导入的数据文件' }}</strong><span>{{ error ? '请检查文件格式、编码和访问权限。' : '支持 CSV、TSV、TXT、XLSX、XLS 和 XLSB。' }}</span></div>

      <div v-if="progress" class="transfer-progress" role="status"><progress v-if="progressPercent !== null" :value="progressPercent" max="100" aria-label="数据导入进度" /><span>{{ progress.phase }}<template v-if="progressPercent !== null"> · {{ progressPercent }}%</template><template v-if="progress.message"> · {{ progress.message }}</template></span></div>
      <p v-if="resultMessage" class="success">{{ resultMessage }}</p>
      <p v-if="error" class="error-banner" role="alert">{{ error }}</p>
      <template #footer><button class="secondary" :disabled="importing" @click="emit('close')">关闭</button><button v-if="importing" class="danger" @click="cancelImport">取消导入</button><button v-else class="primary" :disabled="!canImport" @click="startImport">开始导入</button></template>
  </AppDialog>
</template>
