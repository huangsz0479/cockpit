<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import { X } from "lucide-vue-next";
import AppSelect from "@/components/AppSelect.vue";
import { findSqlParameters, renderSqlParameters, type SqlParameterMode, type SqlParameterValue } from "@/lib/sqlParameters";

const props = defineProps<{ sql: string }>();
const emit = defineEmits<{ close: []; execute: [sql: string] }>();
const names = findSqlParameters(props.sql);
const values = reactive<Record<string, SqlParameterValue>>(Object.fromEntries(names.map((name) => [name, { value: "", mode: "text" as SqlParameterMode }])));
const error = ref("");
const hasRawParameters = computed(() => names.some((name) => values[name]?.mode === "raw"));

function submit() {
  error.value = "";
  try { emit("execute", renderSqlParameters(props.sql, values)); }
  catch (cause) { error.value = cause instanceof Error ? cause.message : String(cause); }
}
</script>

<template>
  <div class="dialog-backdrop" @mousedown.self="emit('close')"><form class="dialog sql-parameter-dialog" role="dialog" aria-modal="true" aria-labelledby="sql-parameter-title" @submit.prevent="submit"><header><div><h2 id="sql-parameter-title">查询参数</h2><p>参数只替换字符串和注释之外的 <code v-pre>{{name}}</code> 占位符</p></div><button type="button" class="icon-button" aria-label="关闭查询参数" @click="emit('close')"><X :size="15" /></button></header><div class="parameter-fields"><label v-for="name in names" :key="name"><span>{{ name }}</span><AppSelect v-model="values[name]!.mode" :options="[{ value: 'text', label: '文本' }, { value: 'number', label: '数值' }, { value: 'null', label: 'NULL' }, { value: 'raw', label: '原始 SQL（不转义）' }]" :label="`${name} 的参数类型`" /><input v-model="values[name]!.value" :aria-label="`${name} 的参数值`" :disabled="values[name]!.mode === 'null'" autocomplete="off" spellcheck="false" /></label></div><p v-if="hasRawParameters" class="parameter-warning">原始 SQL 参数不会进行转义，请仅填写可信且已经审查的 SQL 片段。</p><p v-if="error" class="error-banner">{{ error }}</p><footer><button type="button" class="secondary" @click="emit('close')">取消</button><button class="primary">执行</button></footer></form></div>
</template>
