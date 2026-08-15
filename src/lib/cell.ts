import type { CellValue } from "@/types";

export function cellText(cell: CellValue | undefined): string {
  if (!cell || cell.kind === "null") return "NULL";
  if (cell.kind === "bool") return cell.value ? "true" : "false";
  if (cell.kind === "float") return Number.isFinite(cell.value) ? String(cell.value) : "NULL";
  if (cell.kind === "bytes") return cell.value.preview ?? `<${cell.value.length} bytes>`;
  if (cell.kind === "geometry") return `<geometry${cell.value.srid == null ? "" : ` SRID ${cell.value.srid}`}>`;
  return cell.value;
}

