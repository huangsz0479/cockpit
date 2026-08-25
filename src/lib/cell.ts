import type { CellValue } from "@/types";

export function cellText(cell: CellValue | undefined): string {
  if (!cell || cell.kind === "null") return "NULL";
  if (cell.kind === "bool") return cell.value ? "true" : "false";
  if (cell.kind === "float") return Number.isFinite(cell.value) ? String(cell.value) : "NULL";
  if (cell.kind === "bytes") return cell.value.preview ?? `<${cell.value.length} bytes>`;
  if (cell.kind === "geometry") return `<geometry${cell.value.srid == null ? "" : ` SRID ${cell.value.srid}`}>`;
  return cell.value;
}

/// 把单元格还原为 JSON 原生值：布尔/浮点保持类型，整数以字符串承载避免精度丢失，
/// JSON 单元格解析回对象，整行据此组装成文档 JSON 展示。
export function cellToJsValue(cell: CellValue | undefined): unknown {
  if (!cell || cell.kind === "null") return null;
  if (cell.kind === "bool") return cell.value;
  if (cell.kind === "float") return cell.value;
  if (cell.kind === "json") {
    try {
      return JSON.parse(cell.value);
    } catch {
      return cell.value;
    }
  }
  if (cell.kind === "bytes") return cell.value.preview ?? cell.value.base64;
  if (cell.kind === "geometry") return { srid: cell.value.srid ?? null, wkbBase64: cell.value.wkbBase64 };
  return cell.value;
}

