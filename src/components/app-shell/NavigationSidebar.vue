<script setup lang="ts">
import { computed, ref } from "vue";
import { Activity, Cable, ChevronDown, ChevronRight, Database, KeyRound, LockKeyhole, MemoryStick, Pencil, Plus, Unplug } from "lucide-vue-next";
import databaseIcon from "../../../src-tauri/icons/database/database.svg";
import databaseIconSvg from "../../../src-tauri/icons/database/database.svg?raw";
import eventIcon from "../../../src-tauri/icons/database/event.svg";
import functionIcon from "../../../src-tauri/icons/database/fn.svg";
import mysqlIconSvg from "../../../src-tauri/icons/database/mysql.svg?raw";
import postgresqlIconSvg from "../../../src-tauri/icons/database/pgsql.svg?raw";
import sqliteIconSvg from "../../../src-tauri/icons/database/sql-lite.svg?raw";
import tableIcon from "../../../src-tauri/icons/database/Table.svg";
import triggerIcon from "../../../src-tauri/icons/database/trigger.svg";
import viewIcon from "../../../src-tauri/icons/database/view.svg";
import type { ConnectionInfo, ConnectionProfile, DatabaseInfo, DatabaseKind, DatabaseObjectKind, EventInfo, RoutineInfo, RuntimeStats, TableInfo, TriggerInfo, UUID } from "@/types";
import type { NavigationContextTarget } from "./types";

const props = defineProps<{
  connections: ConnectionProfile[];
  connectionInfo: Partial<Record<UUID, ConnectionInfo>>;
  busy: boolean;
  selectedDatabase: string | null;
  activeConnectionId: UUID | null;
  connectionGroups: { name: string; connections: ConnectionProfile[] }[];
  filteredDatabases: DatabaseInfo[];
  filteredBaseTables: TableInfo[];
  filteredViews: TableInfo[];
  filteredRoutines: RoutineInfo[];
  filteredTriggers: TriggerInfo[];
  filteredEvents: EventInfo[];
  tableHasMore: boolean;
  selectedTable: TableInfo | null;
  expandedConnectionId: UUID | null;
  expandedDatabase: string | null;
  navigationWidth: number;
  minWidth: number;
  maxWidth: number;
  runtimeStats: RuntimeStats | null;
  runtimeStatsState: "loading" | "ready" | "unavailable";
}>();

const expandedTableGroup = defineModel<boolean>("expandedTableGroup", { required: true });
const expandedViewGroup = defineModel<boolean>("expandedViewGroup", { required: true });
const expandedFunctionGroup = defineModel<boolean>("expandedFunctionGroup", { required: true });
const expandedTriggerGroup = defineModel<boolean>("expandedTriggerGroup", { required: true });
const expandedEventGroup = defineModel<boolean>("expandedEventGroup", { required: true });
const collapsedConnectionGroups = ref(new Set<string>());

const emit = defineEmits<{
  "add-connection": [];
  "load-more": [];
  "toggle-connection": [connection: ConnectionProfile];
  "edit-connection": [connection: ConnectionProfile];
  "disconnect-connection": [id: UUID];
  "toggle-database": [database: string];
  "open-redis-manager": [connection: ConnectionProfile];
  "context-menu": [event: MouseEvent, target: NavigationContextTarget];
  "highlight-table": [table: TableInfo];
  "preview-table": [table: TableInfo];
  "open-database-object": [database: string, kind: DatabaseObjectKind, name: string];
  "resize-start": [event: PointerEvent];
  "resize-move": [event: PointerEvent];
  "resize-end": [event: PointerEvent];
  "resize-cancel": [event: PointerEvent];
  "resize-reset": [];
  "resize-key": [event: KeyboardEvent];
}>();

function routineKind(routine: RoutineInfo): DatabaseObjectKind {
  return routine.routineType.toUpperCase() === "FUNCTION" ? "function" : "procedure";
}

function inlineSvg(source: string) {
  const svgStart = source.indexOf("<svg");
  return svgStart === -1 ? source : source.slice(svgStart);
}

const databaseTypeIcons = {
  generic: inlineSvg(databaseIconSvg),
  mysql: inlineSvg(mysqlIconSvg),
  postgresql: inlineSvg(postgresqlIconSvg),
  sqlite: inlineSvg(sqliteIconSvg),
  redis: inlineSvg(databaseIconSvg),
};
const connectedCount = computed(() => props.connections.filter((connection) => Boolean(props.connectionInfo[connection.id])).length);

function formatMemory(bytes: number | null | undefined) {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return "—";
  const gibibytes = bytes / 1024 ** 3;
  if (gibibytes >= 1) {
    const digits = gibibytes >= 10 ? 0 : 1;
    return `${gibibytes.toFixed(digits).replace(/\.0$/, "")} GB`;
  }
  return `${Math.round(bytes / 1024 ** 2)} MB`;
}

function connectionIcon(kind?: DatabaseKind) {
  if (kind === "postgresql") return databaseTypeIcons.postgresql;
  if (kind === "sqlite") return databaseTypeIcons.sqlite;
  if (kind === "redis") return databaseTypeIcons.redis;
  if (kind === "mysql" || !kind) return databaseTypeIcons.mysql;
  return databaseTypeIcons.generic;
}

function handleResourceScroll(event: Event) {
  const target = event.currentTarget as HTMLElement;
  if (props.tableHasMore && !props.busy && target.scrollHeight - target.scrollTop - target.clientHeight < 100) emit("load-more");
}

function toggleConnectionGroup(name: string) {
  const collapsedGroups = new Set(collapsedConnectionGroups.value);
  if (collapsedGroups.has(name)) collapsedGroups.delete(name);
  else collapsedGroups.add(name);
  collapsedConnectionGroups.value = collapsedGroups;
}

</script>

<template>
  <aside class="navigation-pane">
    <div v-if="connections.length === 0" class="navigation-empty">
      <div class="navigation-empty-visual" aria-hidden="true">
        <span class="navigation-empty-orbit navigation-empty-orbit-outer" />
        <span class="navigation-empty-orbit navigation-empty-orbit-inner" />
        <span class="empty-icon"><Database :size="25" :stroke-width="1.8" /></span>
        <span class="navigation-empty-status" />
      </div>

      <div class="navigation-empty-copy">
        <small class="navigation-empty-eyebrow"><span /> 新工作区</small>
        <strong>连接你的<br /><em>第一台数据库</em></strong>
        <p>添加连接，立即浏览数据结构、编写 SQL 并轻松管理数据。</p>
      </div>

      <div class="navigation-empty-databases" aria-label="支持 MySQL、PostgreSQL、SQLite 和 Redis">
        <span><i v-html="databaseTypeIcons.mysql" />MySQL</span>
        <span><i v-html="databaseTypeIcons.postgresql" />PostgreSQL</span>
        <span><i v-html="databaseTypeIcons.sqlite" />SQLite</span>
        <span><i v-html="databaseTypeIcons.redis" />Redis</span>
      </div>

      <button type="button" class="navigation-empty-action" @click="$emit('add-connection')">
        <Plus :size="15" :stroke-width="2" />
        <span>添加数据库连接</span>
      </button>
      <small class="navigation-empty-hint"><LockKeyhole :size="11" /> 所有连接信息仅保存在本机</small>
    </div>

    <div v-if="connections.length" class="resource-tree" @scroll.passive="handleResourceScroll">
      <template v-for="group in connectionGroups" :key="group.name">
        <button type="button" class="connection-group-title" :aria-expanded="!collapsedConnectionGroups.has(group.name)" @click="toggleConnectionGroup(group.name)">
          <ChevronDown v-if="!collapsedConnectionGroups.has(group.name)" class="tree-chevron" :size="12" />
          <ChevronRight v-else class="tree-chevron" :size="12" />
          <span class="connection-group-name">{{ group.name }}</span>
          <small>{{ group.connections.length }}</small>
        </button>
        <section v-for="connection in group.connections" v-if="!collapsedConnectionGroups.has(group.name)" :key="connection.id" class="connection-tree">
          <div class="tree-row connection-root" :class="{ active: activeConnectionId === connection.id, production: connection.production }" :style="{ '--connection-color': connection.color || '#16a085' }" @contextmenu.prevent="$emit('context-menu', $event, { kind: 'connection', connection })">
            <button type="button" class="connection-toggle" @click="$emit('toggle-connection', connection)">
              <ChevronDown v-if="expandedConnectionId === connection.id" class="tree-chevron" :size="12" />
              <ChevronRight v-else class="tree-chevron" :size="12" />
              <span
                class="database-kind-icon"
                :class="{ connected: Boolean(connectionInfo[connection.id]) }"
                aria-hidden="true"
                v-html="connectionIcon(connection.driverKind)"
              />
              <span class="tree-label"><strong>{{ connection.name }}</strong><span v-if="connection.production || connection.readOnly" class="connection-badges"><span v-if="connection.production" class="connection-badge production">生产</span><span v-if="connection.readOnly" class="connection-badge readonly">只读</span></span></span>
              <span class="connection-kind">{{ connection.driverKind === 'postgresql' ? 'PG' : connection.driverKind === 'sqlite' ? 'SQLite' : connection.driverKind === 'mariadb' ? 'MariaDB' : connection.driverKind === 'redis' ? 'Redis' : 'MySQL' }}</span>
            </button>
            <span class="tree-row-actions"><button v-if="connection.driverKind === 'redis'" type="button" class="tree-action" aria-label="打开 Redis 管理器" @mousedown.stop @click.stop="$emit('open-redis-manager', connection)"><KeyRound :size="13" /></button><button type="button" class="tree-action" aria-label="编辑连接" @mousedown.stop @click.stop="$emit('edit-connection', connection)"><Pencil :size="13" /></button><button v-if="connectionInfo[connection.id]" type="button" class="tree-action" aria-label="断开连接" @mousedown.stop @click.stop="$emit('disconnect-connection', connection.id)"><Unplug :size="13" /></button></span>
            <span class="status-dot" :class="{ online: connectionInfo[connection.id] }" />
          </div>

          <div v-if="expandedConnectionId === connection.id" class="tree-branch connection-children">
            <div v-if="connection.driverKind === 'redis'" class="tree-empty redis-hint">
              <button type="button" class="link" @click="$emit('open-redis-manager', connection)"><KeyRound :size="12" />打开 Redis 管理器</button>
            </div>
            <div v-else-if="busy && !connectionInfo[connection.id]" class="tree-loading">正在连接…</div>
            <template v-else-if="connectionInfo[connection.id]">
              <div v-for="database in filteredDatabases" :key="database.name" class="database-tree">
                <div class="tree-row database-node" :class="{ active: selectedDatabase === database.name }" role="button" tabindex="0" @click="$emit('toggle-database', database.name)" @keydown.enter="$emit('toggle-database', database.name)" @keydown.space.prevent="$emit('toggle-database', database.name)" @contextmenu.prevent="$emit('context-menu', $event, { kind: 'database', database: database.name })">
                  <ChevronDown v-if="expandedDatabase === database.name" class="tree-chevron" :size="13" />
                  <ChevronRight v-else class="tree-chevron" :size="13" />
                  <img class="database-symbol" :src="databaseIcon" alt="" aria-hidden="true" />
                  <span class="node-name">{{ database.name }}</span>
                </div>

                <div v-if="expandedDatabase === database.name" class="tree-branch database-children">
                  <button class="tree-row object-group-title" @click="expandedTableGroup = !expandedTableGroup" @contextmenu.prevent="$emit('context-menu', $event, { kind: 'table-group', database: database.name })"><ChevronDown v-if="expandedTableGroup" :size="12" /><ChevronRight v-else :size="12" /><img class="table-symbol" :src="tableIcon" alt="" aria-hidden="true" /><span>表</span><small>{{ filteredBaseTables.length }}</small></button>
                  <template v-if="expandedTableGroup">
                    <button v-for="table in filteredBaseTables" :key="table.name" class="tree-row object-node" :class="{ active: selectedTable?.name === table.name }" @click="$emit('highlight-table', table)" @dblclick="$emit('preview-table', table)" @keydown.enter.prevent="$emit('preview-table', table)" @contextmenu.prevent="$emit('context-menu', $event, { kind: 'table', table })"><img class="table-symbol" :src="tableIcon" alt="" aria-hidden="true" /><span class="node-name">{{ table.name }}</span></button>
                  </template>

                  <button class="tree-row object-group-title" @click="expandedViewGroup = !expandedViewGroup" @contextmenu.prevent="$emit('context-menu', $event, { kind: 'object-group', database: database.name, group: 'view' })"><ChevronDown v-if="expandedViewGroup" :size="12" /><ChevronRight v-else :size="12" /><img class="view-symbol" :src="viewIcon" alt="" aria-hidden="true" /><span>视图</span><small>{{ filteredViews.length }}</small></button>
                  <template v-if="expandedViewGroup">
                    <button v-for="table in filteredViews" :key="table.name" class="tree-row object-node" :class="{ active: selectedTable?.name === table.name }" @click="$emit('highlight-table', table)" @dblclick="$emit('preview-table', table)" @keydown.enter.prevent="$emit('preview-table', table)" @contextmenu.prevent="$emit('context-menu', $event, { kind: 'table', table })"><img class="view-symbol" :src="viewIcon" alt="" aria-hidden="true" /><span class="node-name">{{ table.name }}</span></button>
                  </template>

                  <button class="tree-row object-group-title" @click="expandedFunctionGroup = !expandedFunctionGroup" @contextmenu.prevent="$emit('context-menu', $event, { kind: 'object-group', database: database.name, group: 'routine' })"><ChevronDown v-if="expandedFunctionGroup" :size="12" /><ChevronRight v-else :size="12" /><img class="routine-symbol" :src="functionIcon" alt="" aria-hidden="true" /><span>函数</span><small>{{ filteredRoutines.length }}</small></button>
                  <template v-if="expandedFunctionGroup"><button v-for="routine in filteredRoutines" :key="routine.routineType + routine.name" class="tree-row object-node metadata-node" @dblclick="$emit('open-database-object', routine.database, routineKind(routine), routine.name)" @keydown.enter.prevent="$emit('open-database-object', routine.database, routineKind(routine), routine.name)" @keydown.space.prevent="$emit('open-database-object', routine.database, routineKind(routine), routine.name)" @contextmenu.prevent="$emit('context-menu', $event, { kind: 'object', database: routine.database, objectKind: routineKind(routine), name: routine.name, label: routine.routineType === 'FUNCTION' ? '函数' : '存储过程' })"><img class="routine-symbol" :src="functionIcon" alt="" aria-hidden="true" /><span class="node-name">{{ routine.name }}</span><small>{{ routine.routineType }}</small></button></template>

                  <button class="tree-row object-group-title" @click="expandedEventGroup = !expandedEventGroup" @contextmenu.prevent="$emit('context-menu', $event, { kind: 'object-group', database: database.name, group: 'event' })"><ChevronDown v-if="expandedEventGroup" :size="12" /><ChevronRight v-else :size="12" /><img class="event-symbol" :src="eventIcon" alt="" aria-hidden="true" /><span>事件</span><small>{{ filteredEvents.length }}</small></button>
                  <template v-if="expandedEventGroup"><button v-for="event in filteredEvents" :key="event.name" class="tree-row object-node metadata-node" @dblclick="$emit('open-database-object', event.database, 'event', event.name)" @keydown.enter.prevent="$emit('open-database-object', event.database, 'event', event.name)" @keydown.space.prevent="$emit('open-database-object', event.database, 'event', event.name)" @contextmenu.prevent="$emit('context-menu', $event, { kind: 'object', database: event.database, objectKind: 'event', name: event.name, label: '事件', status: event.status })"><img class="event-symbol" :src="eventIcon" alt="" aria-hidden="true" /><span class="node-name">{{ event.name }}</span><small>{{ event.status }}</small></button></template>

                  <button class="tree-row object-group-title" @click="expandedTriggerGroup = !expandedTriggerGroup" @contextmenu.prevent="$emit('context-menu', $event, { kind: 'object-group', database: database.name, group: 'trigger' })"><ChevronDown v-if="expandedTriggerGroup" :size="12" /><ChevronRight v-else :size="12" /><img class="trigger-symbol" :src="triggerIcon" alt="" aria-hidden="true" /><span>触发器</span><small>{{ filteredTriggers.length }}</small></button>
                  <template v-if="expandedTriggerGroup"><button v-for="trigger in filteredTriggers" :key="trigger.name" class="tree-row object-node metadata-node" @dblclick="$emit('open-database-object', trigger.database, 'trigger', trigger.name)" @keydown.enter.prevent="$emit('open-database-object', trigger.database, 'trigger', trigger.name)" @keydown.space.prevent="$emit('open-database-object', trigger.database, 'trigger', trigger.name)" @contextmenu.prevent="$emit('context-menu', $event, { kind: 'object', database: trigger.database, objectKind: 'trigger', name: trigger.name, label: '触发器' })"><img class="trigger-symbol" :src="triggerIcon" alt="" aria-hidden="true" /><span class="node-name">{{ trigger.name }}</span><small>{{ trigger.timing }} {{ trigger.event }}</small></button></template>
                  <div v-if="tableHasMore" class="load-more">继续滚动加载</div>
                </div>
              </div>
              <div v-if="!filteredDatabases.length && !busy" class="tree-empty">没有可访问的数据库</div>
            </template>
          </div>
        </section>
      </template>
    </div>

    <footer v-if="connections.length" class="navigation-status" aria-label="运行状态">
      <div class="navigation-status-heading">
        <span><i class="navigation-status-dot" :class="{ online: runtimeStatsState === 'ready', unavailable: runtimeStatsState === 'unavailable' }" aria-hidden="true" />运行状态</span>
      </div>
      <dl class="navigation-status-metrics">
        <div data-metric="connections" title="当前已打开的数据库连接数；同一连接的多个标签页只计一次">
          <dt><Cable :size="12" />已连接</dt>
          <dd>{{ connectedCount }}</dd>
        </div>
        <div data-metric="sessions" title="查询和数据标签页使用的独立数据库会话数">
          <dt><Activity :size="12" />标签会话</dt>
          <dd>{{ runtimeStats?.tabSessionCount ?? "—" }}</dd>
        </div>
        <div data-metric="memory" title="Cockpit 主进程及其 WebView、网页渲染等子进程当前占用的物理内存之和">
          <dt><MemoryStick :size="12" />内存</dt>
          <dd>{{ formatMemory(runtimeStats?.memoryBytes) }}</dd>
        </div>
      </dl>
    </footer>

    <div v-if="connections.length" class="navigation-resizer" role="separator" aria-label="调整连接面板宽度" aria-orientation="vertical" :aria-valuenow="navigationWidth" :aria-valuemin="minWidth" :aria-valuemax="maxWidth" tabindex="0" @pointerdown="$emit('resize-start', $event)" @pointermove="$emit('resize-move', $event)" @pointerup="$emit('resize-end', $event)" @pointercancel="$emit('resize-cancel', $event)" @dblclick="$emit('resize-reset')" @keydown="$emit('resize-key', $event)" />
  </aside>
</template>
