import type { DatabaseKind } from "@/types";

export type DatabaseObjectGroup = "view" | "routine" | "trigger" | "event";

// 必须与各 driver 的 list_views/list_routines/list_triggers/list_events 实现保持一致：
// driver 永远返回空列表的对象类型在此标记为 false，侧边树和右键菜单据此隐藏对应分组。
const supportedObjectGroups: Record<DatabaseKind, DatabaseObjectGroup[]> = {
  mysql: ["view", "routine", "trigger", "event"],
  mariadb: ["view", "routine", "trigger", "event"],
  postgresql: ["view", "routine", "trigger"],
  sqlite: ["view", "trigger"],
  elasticsearch: [],
};

export function driverSupportsObjectGroup(kind: DatabaseKind | undefined, group: DatabaseObjectGroup): boolean {
  return supportedObjectGroups[kind ?? "mysql"].includes(group);
}

// Elasticsearch 的"表"实际是索引（index），界面上按各引擎自己的术语称呼这一分组。
export function driverTableGroupLabel(kind: DatabaseKind | undefined): string {
  return kind === "elasticsearch" ? "索引" : "表";
}

// 与后端 crates/cockpit-elasticsearch 的 is_valid_index_name 保持一致：
// 索引名会拼进 URL 路径，仅允许小写字母、数字与 - _ .，且不能以 - _ + 开头。
export function isValidElasticsearchIndexName(name: string): boolean {
  return name.length > 0
    && !/^[-_+]/.test(name)
    && name !== "." && name !== ".."
    && /^[a-z0-9_.-]+$/.test(name);
}
