<script setup lang="ts">
import { Braces, FileCode2, Pin, Plus, Table2, X } from "lucide-vue-next";
import sqlIcon from "../../../src-tauri/icons/database/sql.svg";
import type { WorkspaceTabView } from "./types";

const props = defineProps<{
  tabs: WorkspaceTabView[];
  activeId: string | null;
  dirtyIds: readonly string[];
}>();

const emit = defineEmits<{
  activate: [id: string];
  close: [id: string];
  "toggle-pin": [];
}>();

function scrollTabs(event: WheelEvent) {
  const target = event.currentTarget as HTMLElement;
  if (target.scrollWidth <= target.clientWidth) return;
  const delta = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY;
  if (!delta) return;
  event.preventDefault();
  target.scrollLeft += delta;
}

function navigateTabs(event: KeyboardEvent, tabId: string) {
  if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key) || !props.tabs.length) return;
  event.preventDefault();
  const currentIndex = Math.max(0, props.tabs.findIndex((tab) => tab.id === tabId));
  const nextIndex = event.key === "Home"
    ? 0
    : event.key === "End"
      ? props.tabs.length - 1
      : (currentIndex + (event.key === "ArrowRight" ? 1 : -1) + props.tabs.length) % props.tabs.length;
  emit("activate", props.tabs[nextIndex]!.id);
  const buttons = (event.currentTarget as HTMLElement)
    .closest(".workspace-tab-list")
    ?.querySelectorAll<HTMLButtonElement>(".workspace-tab");
  buttons?.[nextIndex]?.focus();
}
</script>

<template>
  <nav class="workspace-tabs" aria-label="工作区">
    <div class="workspace-tab-list" role="tablist" aria-label="工作区标签页" @wheel="scrollTabs">
      <div v-for="tab in tabs" :key="tab.id" class="workspace-tab-shell" role="presentation" :class="{ active: tab.id === activeId }">
        <button type="button" class="workspace-tab" role="tab" :aria-selected="tab.id === activeId" :aria-label="tab.title" :tabindex="tab.id === activeId ? 0 : -1" :class="{ active: tab.id === activeId }" @click="$emit('activate', tab.id)" @keydown="navigateTabs($event, tab.id)">
          <Plus v-if="tab.kind === 'create-table'" :size="14" />
          <Braces v-else-if="tab.kind === 'database-object'" :size="14" />
          <Table2 v-else-if="tab.kind === 'table'" :size="14" />
          <img v-else-if="tab.kind === 'console'" class="workspace-tab-icon" :src="sqlIcon" alt="" aria-hidden="true" />
          <FileCode2 v-else :size="14" />
          <span class="tab-title">{{ tab.title }}</span>
          <span v-if="dirtyIds.includes(tab.id)" class="tab-dirty" aria-label="有未保存的更改" />
          <span v-if="tab.pinned" class="tab-pinned" aria-label="已固定"><Pin :size="11" /></span>
        </button>
        <button v-if="!tab.pinned && tab.closable" type="button" class="tab-close" :aria-label="`关闭标签页 ${tab.title}`" @click="$emit('close', tab.id)"><X :size="13" /></button>
      </div>
    </div>
    <div class="workspace-tab-actions">
      <button v-if="activeId" type="button" class="tab-action" aria-label="固定或取消固定当前标签" @click="$emit('toggle-pin')"><Pin :size="14" /></button>
    </div>
  </nav>
</template>
