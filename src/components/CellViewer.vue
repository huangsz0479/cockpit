<script setup lang="ts">
import { computed, ref } from "vue";
import { Check, Copy, Download, X } from "lucide-vue-next";
import { save } from "@tauri-apps/plugin-dialog";
import { api } from "@/lib/api";
import { cellText } from "@/lib/cell";
import type { CellValue } from "@/types";

const props = defineProps<{ column: string; value: CellValue }>();
const emit = defineEmits<{ close: [] }>();
const status = ref("");
const statusKind = ref<"success" | "error">("success");
const busy = ref(false);
const kindLabel = computed(() => ({
  null: "NULL",
  bool: "布尔值",
  signed: "有符号整数",
  unsigned: "无符号整数",
  decimal: "精确小数",
  float: "浮点数",
  text: "文本",
  json: "JSON",
  bytes: "二进制数据",
  date: "日期",
  time: "时间",
  date_time: "日期时间",
  geometry: "空间数据",
})[props.value.kind] ?? props.value.kind);
const displayed = computed(() => {
  if (props.value.kind === "json") {
    try { return JSON.stringify(JSON.parse(props.value.value), null, 2); } catch { return props.value.value; }
  }
  if (props.value.kind === "bytes") return props.value.value.preview || props.value.value.base64;
  if (props.value.kind === "geometry") return `SRID: ${props.value.value.srid ?? "—"}\nWKB (Base64):\n${props.value.value.wkbBase64}`;
  return cellText(props.value);
});
async function copy() {
  try {
    await navigator.clipboard.writeText(displayed.value);
    statusKind.value = "success";
    status.value = "内容已复制";
  } catch {
    statusKind.value = "error";
    status.value = "复制失败，请检查剪贴板权限";
  }
}
async function saveBytes() {
  if (props.value.kind !== "bytes") return;
  const path = await save({ title: "保存二进制字段", defaultPath: `${props.column}.bin` });
  if (!path) return;
  busy.value = true;
  status.value = "";
  try {
    await api.writeBinaryFile(path, props.value.value.base64);
    statusKind.value = "success";
    status.value = "文件已保存";
  } catch (cause) {
    statusKind.value = "error";
    status.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="dialog-backdrop" @mousedown.self="emit('close')"><section class="dialog cell-viewer-dialog" role="dialog" aria-modal="true" aria-labelledby="cell-viewer-title"><header><div><h2 id="cell-viewer-title">{{ column }}</h2><p>{{ kindLabel }}<template v-if="value.kind === 'bytes'"> · {{ value.value.length.toLocaleString() }} 字节</template></p></div><button class="icon-button" aria-label="关闭字段查看器" @click="emit('close')"><X :size="15" /></button></header><pre>{{ displayed }}</pre><footer><span class="cell-viewer-status" :class="statusKind">{{ status }}</span><button v-if="value.kind === 'bytes'" class="secondary" :disabled="busy" @click="saveBytes"><Download :size="14" />{{ busy ? '保存中…' : '保存文件' }}</button><button class="secondary" @click="copy"><Check v-if="status === '内容已复制'" :size="14" /><Copy v-else :size="14" />复制</button><button class="primary" @click="emit('close')">关闭</button></footer></section></div>
</template>
