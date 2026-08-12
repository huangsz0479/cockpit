<script setup lang="ts">
import AppSelect from "@/components/AppSelect.vue";
import { Download, FileDown, X } from "lucide-vue-next";
import type { ExportFormat } from "@/types";

interface ExportOption {
  value: ExportFormat;
  label: string;
  disabled?: boolean;
}

defineProps<{
  modelValue: ExportFormat;
  options: ExportOption[];
  busy?: boolean;
  fullDisabled?: boolean;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: ExportFormat];
  close: [];
  exportPage: [];
  exportFull: [];
}>();
</script>

<template>
  <div class="dialog-backdrop query-export-dialog-backdrop" @mousedown.self="emit('close')" @keydown.esc.stop.prevent="emit('close')">
    <section id="query-export-dialog" class="dialog query-export-dialog" role="dialog" aria-modal="true" aria-labelledby="query-export-title" aria-describedby="query-export-description">
      <header>
        <div>
          <h2 id="query-export-title">导出查询结果</h2>
          <p id="query-export-description">选择文件格式和导出范围</p>
        </div>
        <button type="button" class="icon-button" aria-label="关闭导出弹窗" @click="emit('close')"><X :size="16" /></button>
      </header>
      <div class="query-export-dialog-body">
        <label class="query-export-format">
          <span>文件格式</span>
          <AppSelect :model-value="modelValue" :options="options" label="导出格式" :disabled="busy" :menu-min-width="160" @update:model-value="emit('update:modelValue', $event)" />
        </label>
        <fieldset class="query-export-scope">
          <legend>导出范围</legend>
          <div class="query-export-scope-options">
            <button type="button" class="query-export-scope-option" :disabled="busy" @click="emit('exportPage')">
              <FileDown :size="18" aria-hidden="true" />
              <span><strong>当前页</strong><small>导出当前显示的结果页</small></span>
            </button>
            <button type="button" class="query-export-scope-option" :disabled="busy || fullDisabled" @click="emit('exportFull')">
              <Download :size="18" aria-hidden="true" />
              <span><strong>全部</strong><small>{{ fullDisabled ? '当前查询不支持完整导出' : '导出完整查询结果' }}</small></span>
            </button>
          </div>
        </fieldset>
      </div>
    </section>
  </div>
</template>

<style scoped>
.query-export-dialog-backdrop { z-index: 100; }
.query-export-dialog { width: min(420px, calc(100vw - 30px)); overflow: visible; }
.query-export-dialog > header { min-height: 58px; padding: 10px 14px; }
.query-export-dialog-body { display: grid; gap: 16px; padding: 16px; }
.query-export-format { display: grid; grid-template-columns: 76px minmax(0, 1fr); align-items: center; gap: 10px; color: var(--text); font-size: 10.5px; font-weight: 650; }
.query-export-scope { min-width: 0; margin: 0; padding: 0; border: 0; }
.query-export-scope legend { margin-bottom: 7px; color: var(--text); font-size: 10.5px; font-weight: 650; }
.query-export-scope-options { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; }
.query-export-scope-option { min-width: 0; min-height: 66px; justify-content: flex-start; gap: 10px; padding: 10px; border-color: var(--border); background: var(--surface-1); text-align: left; }
.query-export-scope-option:hover:not(:disabled) { border-color: var(--accent); background: var(--surface-hover); }
.query-export-scope-option > svg { flex: 0 0 auto; color: var(--accent); }
.query-export-scope-option > span { min-width: 0; display: grid; gap: 3px; }
.query-export-scope-option strong { font-size: 11px; }
.query-export-scope-option small { overflow-wrap: anywhere; color: var(--muted); font-size: 9px; font-weight: 500; line-height: 1.35; }
@media (max-width: 430px) {
  .query-export-format, .query-export-scope-options { grid-template-columns: 1fr; }
}
</style>
