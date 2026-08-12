<script setup lang="ts">
import AppSelect from "@/components/AppSelect.vue";
import { Download, FileDown } from "lucide-vue-next";
import type { ExportFormat } from "@/types";

interface ExportOption {
  value: ExportFormat;
  label: string;
  disabled?: boolean;
}

withDefaults(defineProps<{
  modelValue: ExportFormat;
  options: ExportOption[];
  disabled?: boolean;
  fullDisabled?: boolean;
  fullLabel?: string;
}>(), {
  disabled: false,
  fullDisabled: false,
  fullLabel: "全部",
});

const emit = defineEmits<{
  "update:modelValue": [value: ExportFormat];
  exportPage: [];
  exportFull: [];
}>();
</script>

<template>
  <div class="table-export-control" role="group" aria-label="导出选项">
    <AppSelect
      :model-value="modelValue"
      :options="options"
      label="导出格式"
      variant="compact"
      :menu-min-width="120"
      :disabled="disabled"
      @update:model-value="emit('update:modelValue', $event)"
    />
    <button type="button" role="menuitem" class="ghost compact" :disabled="disabled" @click="emit('exportPage')">
      <FileDown :size="13" aria-hidden="true" />当前页
    </button>
    <button type="button" role="menuitem" class="ghost compact" :disabled="disabled || fullDisabled" @click="emit('exportFull')">
      <Download :size="13" aria-hidden="true" />{{ fullLabel }}
    </button>
  </div>
</template>

<style scoped>
.table-export-control { min-width: 0; display: grid; grid-template-columns: minmax(76px, 1fr) auto auto; align-items: center; gap: 4px; margin: 2px; }
.table-export-control > button { min-width: 0; gap: 4px; padding-inline: 7px; white-space: nowrap; }
.table-export-control > button > svg { flex: 0 0 auto; color: var(--muted); stroke-width: 1.8; }
</style>
