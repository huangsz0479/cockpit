<script setup lang="ts">
import { computed, ref, watch } from "vue";
import AppDialog from "@/components/AppDialog.vue";
import AppSelect from "@/components/AppSelect.vue";
import type { CellValue, ColumnMeta } from "@/types";
import { parseRowCell, rowDraftValue, rowInputType } from "@/lib/rowEditing";

const props = defineProps<{ columns: ColumnMeta[]; selectedCount: number }>();
const emit = defineEmits<{ close: []; apply: [column: string, value: CellValue] }>();
const column = ref(props.columns[0]?.name ?? "");
const value = ref("");
const isNull = ref(false);
const error = ref("");
const meta = computed(() => props.columns.find((item) => item.name === column.value));
const inputType = computed(() => meta.value ? rowInputType(meta.value) : "text");
watch(column, () => {
  value.value = "";
  isNull.value = false;
  error.value = "";
});
function submit() {
  error.value = "";
  if (!meta.value) return;
  try {
    const cell = parseRowCell(meta.value, undefined, {
      text: rowDraftValue(value.value, inputType.value),
      isNull: isNull.value,
    });
    emit("apply", column.value, cell);
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
  }
}
</script>

<template>
  <AppDialog title="批量修改" title-id="batch-edit-title" :description="`将同一个值写入选中的 ${selectedCount} 行，并逐行执行并发校验`" as="form" dialog-class="batch-edit-dialog" close-label="关闭批量修改" @close="emit('close')" @submit="submit">
    <div class="settings-form"><label>目标字段<AppSelect v-model="column" :options="columns.map((item) => ({ value: item.name, label: item.name }))" label="目标字段" /></label><label>新值<input v-model="value" :type="inputType" :step="inputType === 'datetime-local' ? 'any' : undefined" :disabled="isNull" autocomplete="off" spellcheck="false" /></label><label class="check-row"><input v-model="isNull" type="checkbox" :disabled="!meta?.nullable" />设置为 NULL</label></div>
    <p v-if="error" class="error-banner">{{ error }}</p>
    <template #footer><button type="button" class="secondary" @click="emit('close')">取消</button><button class="primary">应用到 {{ selectedCount }} 行</button></template>
  </AppDialog>
</template>
