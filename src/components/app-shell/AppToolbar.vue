<script setup lang="ts">
import { Cable, Database, FilePlus2, FolderOpen, Settings2 } from "lucide-vue-next";

defineProps<{
  settingsActive?: boolean;
}>();

defineEmits<{
  "new-query": [];
  "add-connection": [];
  "open-sql": [];
  settings: [];
}>();
</script>

<template>
  <header class="window-toolbar">
    <div class="window-leading-space" data-tauri-drag-region>
      <span class="window-brand" aria-label="Cockpit">
        <span class="window-brand-mark" aria-hidden="true"><Database :size="14" :stroke-width="1.9" /></span>
        <span class="window-brand-copy"><strong>Cockpit</strong><small>DATABASE STUDIO</small></span>
      </span>
    </div>
    <div class="window-primary-actions" role="toolbar" aria-label="全局操作">
      <div class="window-tool-group window-tool-group-primary" role="group" aria-label="创建与打开">
        <button type="button" class="window-tool window-tool-accent tool-connection" aria-label="新建连接" @mousedown.stop @click.stop="$emit('add-connection')">
          <span class="window-tool-symbol" aria-hidden="true"><Cable :size="14" /></span>
          <span>新建连接</span>
        </button>
        <button type="button" class="window-tool window-tool-accent tool-query" aria-label="新建查询" @mousedown.stop @click.stop="$emit('new-query')">
          <span class="window-tool-symbol" aria-hidden="true"><FilePlus2 :size="14" /></span>
          <span>新建查询</span>
        </button>
        <button type="button" class="window-tool window-tool-accent tool-file" aria-label="打开 SQL" @mousedown.stop @click.stop="$emit('open-sql')">
          <span class="window-tool-symbol" aria-hidden="true"><FolderOpen :size="14" /></span>
          <span>打开 SQL</span>
        </button>
      </div>
      <div class="window-trailing-space" data-tauri-drag-region />
      <span class="window-tool-divider" aria-hidden="true" />
      <div class="window-tool-group window-tool-group-utility" role="group" aria-label="应用设置">
        <button type="button" class="window-tool window-tool-accent tool-settings" :class="{ 'window-tool-active': settingsActive }" aria-label="设置" :aria-pressed="Boolean(settingsActive)" @mousedown.stop @click.stop="$emit('settings')">
          <span class="window-tool-symbol" aria-hidden="true"><Settings2 :size="14" /></span>
          <span>设置</span>
        </button>
      </div>
    </div>
  </header>
</template>
