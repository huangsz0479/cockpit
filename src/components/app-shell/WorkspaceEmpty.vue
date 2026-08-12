<script setup lang="ts">
import { Braces, Database, FileCode2, Table2 } from "lucide-vue-next";
</script>

<template>
  <section class="workspace-empty" aria-labelledby="workspace-empty-title">
    <div class="workspace-empty-card">
      <div class="workspace-empty-visual" aria-hidden="true">
        <span class="workspace-empty-orbit workspace-empty-orbit-wide" />
        <span class="workspace-empty-orbit workspace-empty-orbit-tall" />
        <span class="workspace-empty-icon"><Braces :size="29" :stroke-width="1.8" /></span>
        <span class="workspace-empty-node workspace-empty-node-database"><Database :size="13" /></span>
        <span class="workspace-empty-node workspace-empty-node-table"><Table2 :size="13" /></span>
        <span class="workspace-empty-node workspace-empty-node-query"><FileCode2 :size="13" /></span>
      </div>
      <div class="workspace-empty-copy">
        <small><span /> 工作区已就绪</small>
        <p id="workspace-empty-title">让数据在这里展开</p>
        <span>新建 SQL 查询，或从左侧资源管理器打开数据表。</span>
      </div>
    </div>
  </section>
</template>
