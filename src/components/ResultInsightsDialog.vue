<script setup lang="ts">
import { computed, ref } from "vue";
import { BarChart3, X } from "lucide-vue-next";
import AppSelect from "@/components/AppSelect.vue";
import { cellText } from "@/lib/cell";
import type { CellValue, ColumnMeta } from "@/types";

const props = defineProps<{ columns: ColumnMeta[]; rows: CellValue[][] }>();
const emit = defineEmits<{ close: [] }>();
const dimension = ref(props.columns[0]?.name ?? "");
const metric = ref("");
const aggregation = ref<"count" | "sum" | "avg">("count");
const numericColumns = computed(() => props.columns.filter((column, index) => props.rows.some((row) => ["signed", "unsigned", "decimal", "float"].includes(row[index]?.kind ?? ""))));
const chartRows = computed(() => {
  const dimensionIndex = props.columns.findIndex((column) => column.name === dimension.value);
  const metricIndex = props.columns.findIndex((column) => column.name === metric.value);
  if (dimensionIndex < 0) return [];
  if (aggregation.value !== "count" && metricIndex < 0) return [];
  const groups = new Map<string, { sum: number; count: number }>();
  for (const row of props.rows) {
    const key = cellText(row[dimensionIndex] ?? { kind: "null" });
    const current = groups.get(key) ?? { sum: 0, count: 0 };
    current.count += 1;
    if (metricIndex >= 0) {
      const value = row[metricIndex];
      if (value && value.kind !== "null" && value.kind !== "bytes" && value.kind !== "geometry" && value.kind !== "bool") current.sum += Number(value.value) || 0;
    }
    groups.set(key, current);
  }
  const values = [...groups].map(([label, value]) => ({
    label,
    value: aggregation.value === "count" ? value.count : aggregation.value === "sum" ? value.sum : value.count ? value.sum / value.count : 0,
  })).sort((left, right) => right.value - left.value).slice(0, 30);
  const max = Math.max(...values.map((item) => Math.abs(item.value)), 1);
  return values.map((item) => ({ ...item, percent: Math.abs(item.value) / max * 100 }));
});
const emptyMessage = computed(() => {
  if (!props.rows.length) return "当前结果页没有可分析的数据";
  if (aggregation.value !== "count" && !metric.value) return "请选择用于计算的数值字段";
  return "当前配置没有可分析数据";
});
</script>

<template>
  <div class="dialog-backdrop" @mousedown.self="emit('close')"><section class="dialog result-insights-dialog" role="dialog" aria-modal="true" aria-labelledby="result-insights-title"><header><div><h2 id="result-insights-title">结果洞察</h2><p>对当前页进行分组、计数、求和和平均值分析</p></div><button class="icon-button" aria-label="关闭结果洞察" @click="emit('close')"><X :size="15" /></button></header><div class="insight-controls"><label>分组字段<AppSelect v-model="dimension" :options="columns.map((column) => ({ value: column.name, label: column.name }))" label="分组字段" /></label><label>统计方式<AppSelect v-model="aggregation" :options="[{ value: 'count', label: '计数' }, { value: 'sum', label: '求和' }, { value: 'avg', label: '平均值' }]" label="统计方式" /></label><label v-if="aggregation !== 'count'">数值字段<AppSelect v-model="metric" :options="[{ value: '', label: '请选择' }, ...numericColumns.map((column) => ({ value: column.name, label: column.name }))]" label="数值字段" /></label></div><div class="insight-chart"><div v-for="row in chartRows" :key="row.label" class="insight-bar-row"><span>{{ row.label }}</span><div><i :style="{ width: `${row.percent}%` }" /></div><strong>{{ Number.isInteger(row.value) ? row.value.toLocaleString() : row.value.toLocaleString(undefined, { maximumFractionDigits: 3 }) }}</strong></div><div v-if="!chartRows.length" class="dialog-empty-state"><BarChart3 :size="26" /><strong>暂无图表</strong><span>{{ emptyMessage }}</span></div></div><footer><span>最多显示前 30 项</span><button class="secondary" @click="emit('close')">关闭</button></footer></section></div>
</template>
