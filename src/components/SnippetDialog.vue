<script setup lang="ts">
import { computed, ref } from "vue";
import { X } from "lucide-vue-next";
import type { QuerySnippet } from "@/types";

const props = defineProps<{ snippets: QuerySnippet[]; currentSql: string }>();
const emit = defineEmits<{
  close: [];
  open: [snippet: QuerySnippet];
  save: [snippet: QuerySnippet];
  remove: [snippet: QuerySnippet];
}>();
const filter = ref("");
const name = ref("");
const tags = ref("");
const filtered = computed(() => {
  const value = filter.value.trim().toLocaleLowerCase();
  return props.snippets.filter((item) => !value || item.name.toLocaleLowerCase().includes(value) || item.tags.some((tag) => tag.toLocaleLowerCase().includes(value)) || item.sql.toLocaleLowerCase().includes(value));
});
function saveCurrent() {
  if (!name.value.trim() || !props.currentSql.trim()) return;
  emit("save", { id: crypto.randomUUID(), name: name.value.trim(), sql: props.currentSql, tags: tags.value.split(",").map((item) => item.trim()).filter(Boolean) });
  name.value = "";
  tags.value = "";
}
</script>

<template>
  <div class="dialog-backdrop" @mousedown.self="emit('close')"><section class="dialog snippet-dialog" role="dialog" aria-modal="true" aria-labelledby="snippet-title"><header><div><h2 id="snippet-title">SQL 片段</h2><p>保存常用模板，并可配合查询参数重复使用</p></div><button class="icon-button" aria-label="关闭 SQL 片段" @click="emit('close')"><X :size="15" /></button></header><div class="snippet-create"><input v-model="name" aria-label="片段名称" placeholder="片段名称" /><input v-model="tags" aria-label="片段标签" placeholder="标签，逗号分隔" /><button class="primary compact" :disabled="!name.trim() || !currentSql.trim()" @click="saveCurrent">保存当前 SQL</button></div><input v-model="filter" type="search" aria-label="搜索 SQL 片段" placeholder="搜索名称、标签或 SQL" /><div class="snippet-list"><article v-for="snippet in filtered" :key="snippet.id"><div><strong>{{ snippet.name }}</strong><small>{{ snippet.tags.join(' · ') || '无标签' }}</small><pre>{{ snippet.sql }}</pre></div><button class="primary compact" @click="emit('open', snippet)">打开</button><button class="danger compact" @click="emit('remove', snippet)">删除</button></article><div v-if="!filtered.length" class="dialog-empty-state"><strong>{{ snippets.length ? '没有匹配的 SQL 片段' : '尚未保存 SQL 片段' }}</strong><span>{{ snippets.length ? '尝试使用更短的搜索词。' : '为当前查询填写名称后即可保存。' }}</span></div></div><footer><span>{{ filter.trim() ? `${filtered.length} / ${snippets.length} 个片段` : `${snippets.length} 个片段` }}</span><button class="secondary" @click="emit('close')">关闭</button></footer></section></div>
</template>
