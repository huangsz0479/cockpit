<script setup lang="ts">
import { onMounted, ref } from "vue";
import { Check, Copy, FileText, LoaderCircle, RefreshCw } from "lucide-vue-next";
import AppDialog from "@/components/AppDialog.vue";
import { api } from "@/lib/api";
import type { DiagnosticsInfo } from "@/types";

const emit = defineEmits<{ close: [] }>();
const info = ref<DiagnosticsInfo | null>(null);
const error = ref("");
const busy = ref(false);
const copyState = ref<"idle" | "success" | "error">("idle");

async function load() {
  busy.value = true;
  error.value = "";
  copyState.value = "idle";
  try { info.value = await api.diagnostics(); }
  catch (cause) { error.value = cause instanceof Error ? cause.message : String(cause); }
  finally { busy.value = false; }
}

async function copyLogs() {
  if (!info.value?.logs) return;
  try {
    await navigator.clipboard.writeText(info.value.logs);
    copyState.value = "success";
  } catch {
    copyState.value = "error";
  }
}

onMounted(load);
</script>

<template>
  <AppDialog title="诊断日志" title-id="diagnostics-title" :description="`Cockpit ${info?.version || '—'} · 日志中的密码和令牌字段会在读取时脱敏`" dialog-class="diagnostics-dialog" close-label="关闭诊断日志" @close="emit('close')">
      <template #icon><FileText :size="18" /></template>
      <div class="diagnostics-toolbar"><span>{{ info?.logPath || '暂无日志文件' }}</span><button class="ghost compact" :disabled="busy" @click="load"><RefreshCw :size="13" :class="{ 'loading-icon': busy }" />刷新</button><button class="ghost compact" :disabled="!info?.logs" @click="copyLogs"><Check v-if="copyState === 'success'" :size="13" /><Copy v-else :size="13" />{{ copyState === 'success' ? '已复制' : '复制' }}</button></div>
      <p v-if="error" class="error-banner">{{ error }}</p>
      <div v-if="busy && !info" class="dialog-empty-state" role="status"><LoaderCircle :size="26" class="loading-icon" /><strong>正在读取诊断日志</strong><span>正在收集最新的应用运行信息…</span></div>
      <pre v-else-if="info?.logs">{{ info.logs }}</pre>
      <div v-else class="dialog-empty-state"><FileText :size="26" /><strong>{{ error ? '无法读取诊断日志' : '尚无诊断日志' }}</strong><span>{{ error ? '请稍后重试；如果问题持续存在，请检查应用的数据目录权限。' : '应用产生运行记录后会显示在这里。' }}</span></div>
      <template #footer><span :class="{ 'copy-error': copyState === 'error' }">{{ copyState === 'error' ? '复制失败，请检查剪贴板权限' : '最多显示最新 512 KB' }}</span><button class="secondary" @click="emit('close')">关闭</button></template>
  </AppDialog>
</template>
