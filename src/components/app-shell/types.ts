import type { ConnectionProfile, DatabaseObjectKind, TableInfo } from "@/types";

export type ObjectGroupKind = "view" | "routine" | "trigger" | "event";

export type NavigationContextTarget =
  | { kind: "connection"; connection: ConnectionProfile }
  | { kind: "database"; database: string }
  | { kind: "table-group"; database: string }
  | { kind: "object-group"; database: string; group: ObjectGroupKind }
  | { kind: "table"; table: TableInfo }
  | { kind: "object"; database: string; objectKind: DatabaseObjectKind; name: string; label: string; status?: string };

export interface WorkspaceTabView {
  id: string;
  kind: "console" | "table" | "table-detail" | "create-table" | "alter-table" | "database-object";
  title: string;
  closable: boolean;
  pinned?: boolean;
}
