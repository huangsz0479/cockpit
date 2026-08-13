<script setup lang="ts">
import { ref } from "vue";
import { ArrowDown, ArrowUp } from "lucide-vue-next";
import AppDialog from "@/components/AppDialog.vue";
import type { ColumnMeta } from "@/types";

const props = defineProps<{
  columns: ColumnMeta[];
  order: string[];
  hidden: string[];
  frozenCount: number;
}>();
const emit = defineEmits<{ close: []; apply: [order: string[], hidden: string[], frozenCount: number] }>();
const order = ref([
  ...props.order.filter((name) => props.columns.some((column) => column.name === name)),
  ...props.columns.map((column) => column.name).filter((name) => !props.order.includes(name)),
]);
const hidden = ref(props.hidden.filter((name) => props.columns.some((column) => column.name === name)));
const frozenCount = ref(props.frozenCount);
function move(index: number, direction: -1 | 1) {
  const target = index + direction;
  if (target < 0 || target >= order.value.length) return;
  [order.value[index], order.value[target]] = [order.value[target]!, order.value[index]!];
}
function toggleHidden(name: string, checked: boolean) {
  hidden.value = checked ? hidden.value.filter((item) => item !== name) : [...hidden.value, name];
}
function apply() {
  const requested = Number(frozenCount.value);
  const normalized = Number.isFinite(requested) ? Math.round(requested) : 0;
  emit("apply", order.value, hidden.value, Math.max(0, Math.min(normalized, order.value.length)));
}
</script>

<template>
  <AppDialog title="列管理" title-id="column-manager-title" description="调整顺序、隐藏字段，并冻结左侧列" dialog-class="column-manager-dialog" close-label="关闭列管理" @close="emit('close')">
    <label>冻结前 <input v-model.number="frozenCount" type="number" min="0" :max="order.length" /> 列</label>
    <div class="column-manager-list"><div v-for="(name, index) in order" :key="name"><label><input type="checkbox" :checked="!hidden.includes(name)" :aria-label="`显示字段 ${name}`" @change="toggleHidden(name, ($event.currentTarget as HTMLInputElement).checked)" />{{ name }}</label><button type="button" class="ghost compact icon-only" :aria-label="`上移字段 ${name}`" :disabled="index === 0" @click="move(index, -1)"><ArrowUp :size="14" /></button><button type="button" class="ghost compact icon-only" :aria-label="`下移字段 ${name}`" :disabled="index === order.length - 1" @click="move(index, 1)"><ArrowDown :size="14" /></button></div></div>
    <template #footer><button class="secondary" @click="emit('close')">取消</button><button class="primary" @click="apply">应用</button></template>
  </AppDialog>
</template>
