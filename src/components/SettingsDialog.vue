<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import {
  Check,
  Code2,
  Database,
  DatabaseBackup,
  Info,
  ScrollText,
  Settings2,
  ShieldCheck,
  X,
} from "lucide-vue-next";
import AppSelect from "@/components/AppSelect.vue";
import type { AppSettings } from "@/types";

const props = withDefaults(defineProps<{ initial: AppSettings; version?: string }>(), { version: "—" });
const emit = defineEmits<{
  close: [];
  save: [settings: AppSettings];
  diagnostics: [];
  checkUpdate: [manifestUrl: string];
}>();

const draft = reactive<AppSettings>({ ...props.initial });
if (!draft.updateManifestUrl?.trim()) draft.autoCheckUpdates = false;
const activeSection = ref<"general" | "editor" | "backup" | "security" | "about">("general");
const updateManifestUrlError = computed(() => {
  const value = draft.updateManifestUrl?.trim() ?? "";
  if (!value) return draft.autoCheckUpdates ? "启用自动检查前，请填写更新清单地址" : "";
  try {
    const url = new URL(value);
    const local = url.hostname === "localhost" || url.hostname === "127.0.0.1" || url.hostname === "::1";
    return url.protocol === "https:" || (url.protocol === "http:" && local)
      ? ""
      : "更新地址必须使用 HTTPS";
  } catch {
    return "请输入有效的更新清单地址";
  }
});

function submit() {
  if (updateManifestUrlError.value) {
    activeSection.value = "security";
    return;
  }
  emit("save", { ...draft });
}
</script>

<template>
  <div class="dialog-backdrop" @mousedown.self="emit('close')">
    <section class="dialog settings-dialog" role="dialog" aria-modal="true" aria-labelledby="settings-title">
      <header class="settings-dialog-header">
        <div class="settings-dialog-heading">
          <span class="settings-dialog-heading-icon" aria-hidden="true"><Settings2 :size="18" :stroke-width="1.8" /></span>
          <div><h2 id="settings-title">应用设置</h2><p>按需调整 Cockpit 的使用偏好</p></div>
        </div>
        <button type="button" class="icon-button" aria-label="关闭设置" @click="emit('close')"><X :size="16" /></button>
      </header>
      <div class="settings-layout">
        <nav class="settings-navigation" role="tablist" aria-label="设置分类">
          <button id="settings-tab-general" type="button" role="tab" aria-controls="settings-panel-general" :aria-selected="activeSection === 'general'" :class="{ active: activeSection === 'general' }" @click="activeSection = 'general'">
            <span class="settings-navigation-icon" aria-hidden="true"><Settings2 :size="16" /></span><span class="settings-navigation-copy"><strong>常规</strong><small>界面与数据加载</small></span>
          </button>
          <button id="settings-tab-editor" type="button" role="tab" aria-controls="settings-panel-editor" :aria-selected="activeSection === 'editor'" :class="{ active: activeSection === 'editor' }" @click="activeSection = 'editor'">
            <span class="settings-navigation-icon" aria-hidden="true"><Code2 :size="16" /></span><span class="settings-navigation-copy"><strong>编辑器</strong><small>SQL 编写偏好</small></span>
          </button>
          <button id="settings-tab-backup" type="button" role="tab" aria-controls="settings-panel-backup" :aria-selected="activeSection === 'backup'" :class="{ active: activeSection === 'backup' }" @click="activeSection = 'backup'">
            <span class="settings-navigation-icon" aria-hidden="true"><DatabaseBackup :size="16" /></span><span class="settings-navigation-copy"><strong>备份与导出</strong><small>文件输出设置</small></span>
          </button>
          <button id="settings-tab-security" type="button" role="tab" aria-controls="settings-panel-security" :aria-selected="activeSection === 'security'" :class="{ active: activeSection === 'security' }" @click="activeSection = 'security'">
            <span class="settings-navigation-icon" aria-hidden="true"><ShieldCheck :size="16" /></span><span class="settings-navigation-copy"><strong>安全与更新</strong><small>确认和版本检查</small></span>
          </button>
          <button id="settings-tab-about" type="button" role="tab" aria-controls="settings-panel-about" :aria-selected="activeSection === 'about'" :class="{ active: activeSection === 'about' }" @click="activeSection = 'about'">
            <span class="settings-navigation-icon" aria-hidden="true"><Info :size="16" /></span><span class="settings-navigation-copy"><strong>关于</strong><small>版本与许可证</small></span>
          </button>
        </nav>

        <div class="settings-content">
          <div class="settings-form">
            <section id="settings-panel-general" v-show="activeSection === 'general'" class="settings-section" role="tabpanel" aria-labelledby="settings-tab-general settings-general-title">
              <div class="settings-section-heading"><span class="settings-section-icon" aria-hidden="true"><Settings2 :size="17" /></span><div><h3 id="settings-general-title">常规</h3><p>控制界面外观、分页数量和工作区行为。</p></div></div>
              <div class="settings-group">
                <h4>外观与数据</h4>
                <div class="settings-grid">
                  <label class="setting-field"><span>界面主题</span>
                    <AppSelect v-model="draft.theme" :options="[{ value: 'system', label: '跟随系统' }, { value: 'light', label: '浅色' }, { value: 'dark', label: '深色' }]" label="界面主题" />
                  </label>
                  <label class="setting-field"><span>查询结果每页</span>
                    <AppSelect v-model="draft.queryPageSize" :options="[100, 250, 500, 1000, 2000].map((value) => ({ value, label: `${value.toLocaleString()} 行` }))" label="查询结果每页" />
                  </label>
                  <label class="setting-field"><span>数据表每页</span>
                    <AppSelect v-model="draft.tablePageSize" :options="[50, 100, 250, 500].map((value) => ({ value, label: `${value.toLocaleString()} 行` }))" label="数据表每页" />
                  </label>
                </div>
              </div>
              <div class="settings-group">
                <h4>工作区</h4>
                <div class="settings-toggle-list">
                  <label class="setting-toggle"><input v-model="draft.autoSaveWorkspace" type="checkbox" /><span><strong>自动保存工作区</strong><small>下次启动时恢复已打开的查询和数据表</small></span></label>
                  <label class="setting-toggle"><input v-model="draft.showSystemDatabases" type="checkbox" /><span><strong>显示 MySQL 系统数据库</strong><small>在资源管理器中显示系统内置数据库</small></span></label>
                </div>
              </div>
            </section>

            <section id="settings-panel-editor" v-show="activeSection === 'editor'" class="settings-section" role="tabpanel" aria-labelledby="settings-tab-editor settings-editor-title">
              <div class="settings-section-heading"><span class="settings-section-icon" aria-hidden="true"><Code2 :size="17" /></span><div><h3 id="settings-editor-title">编辑器</h3><p>调整 SQL 编辑器的字号和缩进习惯。</p></div></div>
              <div class="settings-group">
                <h4>文本与缩进</h4>
                <div class="settings-grid">
                  <label class="setting-field"><span>编辑器字号</span>
                    <AppSelect v-model="draft.editorFontSize" :options="[11, 12, 13, 14, 16].map((value) => ({ value, label: `${value} px` }))" label="编辑器字号" />
                  </label>
                  <label class="setting-field"><span>Tab 宽度</span>
                    <AppSelect v-model="draft.editorTabSize" :options="[2, 4, 8].map((value) => ({ value, label: `${value} 空格` }))" label="Tab 宽度" />
                  </label>
                </div>
              </div>
            </section>

            <section id="settings-panel-backup" v-show="activeSection === 'backup'" class="settings-section" role="tabpanel" aria-labelledby="settings-tab-backup settings-backup-title">
              <div class="settings-section-heading"><span class="settings-section-icon" aria-hidden="true"><DatabaseBackup :size="17" /></span><div><h3 id="settings-backup-title">备份与导出</h3><p>设置备份内容、压缩方式和默认导出格式。</p></div></div>
              <div class="settings-group">
                <h4>备份选项</h4>
                <div class="settings-grid">
                  <label class="setting-field"><span>备份压缩</span>
                    <AppSelect v-model="draft.backupCompression" :options="[{ value: 'none', label: '不压缩' }, { value: 'gzip', label: 'Gzip' }]" label="备份压缩" />
                  </label>
                  <label class="setting-field"><span>默认导出格式</span>
                    <AppSelect v-model="draft.defaultExportFormat" :options="[{ value: 'excel', label: 'Excel' }, { value: 'csv', label: 'CSV' }, { value: 'sql', label: 'SQL' }, { value: 'txt', label: 'TXT' }]" label="默认导出格式" />
                  </label>
                </div>
                <div class="settings-toggle-list">
                  <label class="setting-toggle"><input v-model="draft.backupIncludeData" type="checkbox" /><span><strong>默认包含表数据</strong><small>整库备份时同时导出表结构和数据</small></span></label>
                  <label class="setting-toggle"><input v-model="draft.backupEncryption" type="checkbox" /><span><strong>手动备份加密</strong><small>备份时要求设置密码并加密输出文件</small></span></label>
                </div>
              </div>
            </section>

            <section id="settings-panel-security" v-show="activeSection === 'security'" class="settings-section" role="tabpanel" aria-labelledby="settings-tab-security settings-security-title">
              <div class="settings-section-heading"><span class="settings-section-icon" aria-hidden="true"><ShieldCheck :size="17" /></span><div><h3 id="settings-security-title">安全与更新</h3><p>管理高风险操作确认和应用更新检查。</p></div></div>
              <div class="settings-group">
                <h4>操作保护</h4>
                <div class="settings-toggle-list">
                  <label class="setting-toggle"><input v-model="draft.confirmDestructiveQueries" type="checkbox" /><span><strong>执行高风险 SQL 前要求确认</strong><small>删除操作无论此项是否开启都始终需要确认</small></span></label>
                </div>
              </div>
              <div class="settings-group">
                <h4>应用更新</h4>
                <div class="settings-toggle-list">
                  <label class="setting-toggle"><input v-model="draft.autoCheckUpdates" type="checkbox" /><span><strong>启动时检查更新</strong><small>应用启动后自动检查是否存在新版本</small></span></label>
                </div>
                <label class="setting-field setting-field-wide"><span>更新清单地址</span>
                  <span class="settings-inline-field"><input v-model="draft.updateManifestUrl" type="url" placeholder="https://…/latest.json" :aria-invalid="Boolean(updateManifestUrlError)" aria-describedby="settings-update-url-error" /><button type="button" class="ghost compact" :disabled="Boolean(updateManifestUrlError) || !draft.updateManifestUrl?.trim()" @click="emit('checkUpdate', draft.updateManifestUrl || '')">检查</button></span>
                  <small v-if="updateManifestUrlError" id="settings-update-url-error" class="settings-field-error" role="alert">{{ updateManifestUrlError }}</small>
                </label>
              </div>
            </section>

            <section id="settings-panel-about" v-show="activeSection === 'about'" class="settings-section settings-about" role="tabpanel" aria-labelledby="settings-tab-about settings-about-title">
              <div class="settings-about-product">
                <span class="settings-about-mark"><Database :size="24" /></span>
                <div><h3 id="settings-about-title">Cockpit</h3><p>轻量、安全、面向日常生产工作的桌面数据库工具。</p></div>
                <span class="settings-version">版本 {{ version }}</span>
              </div>
              <div class="settings-about-list">
                <div><strong>本地优先</strong><span>连接配置和工作区数据保存在本机；数据库密码交由系统凭据库保存。</span></div>
                <div><strong>安全边界</strong><span>只读限制、危险 SQL 确认和行级并发校验会在执行链路中生效。</span></div>
                <div><strong>开源许可</strong><span>Cockpit 根据 Apache License 2.0 授权发布。</span></div>
              </div>
              <p class="settings-about-note">诊断日志会对密码、令牌和连接字符串中的敏感字段进行脱敏。</p>
            </section>
          </div>
        </div>
      </div>
      <footer>
        <button type="button" class="ghost settings-diagnostics-button" @click="emit('diagnostics')"><ScrollText :size="14" aria-hidden="true" /><span>诊断日志</span></button>
        <span class="settings-footer-note">更改将在保存后生效</span>
        <button type="button" class="secondary settings-cancel-button" @click="emit('close')">取消</button>
        <button type="button" class="primary settings-save-button" @click="submit"><Check :size="14" aria-hidden="true" /><span>保存设置</span></button>
      </footer>
    </section>
  </div>
</template>
