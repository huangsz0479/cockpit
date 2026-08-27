import { cellText } from "@/lib/cell";
import type { CellValue, ColumnInfo, ColumnMeta } from "@/types";

export interface RowDraftCell {
  text: string;
  isNull: boolean;
  useDefault?: boolean;
}

export type RowInputType = "text" | "date" | "datetime-local";

const INTEGER_TYPES = new Set([
  "tinyint", "smallint", "mediumint", "int", "integer", "bigint", "int2", "int4", "int8",
  "smallserial", "serial", "bigserial", "year", "bit",
]);
const DECIMAL_TYPES = new Set(["decimal", "numeric", "money"]);
const FLOAT_TYPES = new Set(["float", "double", "double precision", "real", "float4", "float8"]);

export function columnTypeName(column: ColumnMeta, detail?: ColumnInfo) {
  return (detail?.dataType || column.databaseType).trim().toLocaleLowerCase().replace(/\(.*/, "");
}

export function rowInputType(column: ColumnMeta, detail?: ColumnInfo): RowInputType {
  const type = columnTypeName(column, detail);
  if (type === "date") return "date";
  if (type === "datetime" || type.startsWith("timestamp")) {
    const declaration = detail?.fullType || column.databaseType;
    const precision = declaration.match(/\((\d+)\)/)?.[1];
    if (precision && Number(precision) > 3) return "text";
    return "datetime-local";
  }
  return "text";
}

export function rowInputValue(value: string, inputType: RowInputType) {
  if (inputType !== "datetime-local") return value;
  const matched = value.match(/^(\d{4}-\d{2}-\d{2})[ T](\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?)/);
  if (!matched) return value;
  const browserCompatibleTime = matched[2]!.replace(/\.(\d{3})\d+$/, ".$1");
  return `${matched[1]}T${browserCompatibleTime}`;
}

export function rowDraftValue(value: string, inputType: RowInputType) {
  return inputType === "datetime-local"
    ? value.replace(/^(\d{4}-\d{2}-\d{2})T/, "$1 ")
    : value;
}

export function isGeneratedColumn(detail?: ColumnInfo) {
  return Boolean(detail?.generationExpression?.trim())
    || /(?:stored|virtual)\s+generated|generated\s+always/i.test(detail?.extra ?? "");
}

export function hasDatabaseDefault(detail?: ColumnInfo) {
  const extra = detail?.extra?.trim().toLocaleLowerCase() ?? "";
  return detail?.defaultValue != null
    || extra.includes("auto_increment")
    || extra === "always"
    || extra === "by default";
}

export function columnIsNullable(column: ColumnMeta, detail?: ColumnInfo) {
  return detail?.nullable ?? column.nullable;
}

export function rowCellChanged(draft: RowDraftCell, original: CellValue) {
  if (draft.isNull) return original.kind !== "null";
  if (original.kind === "null") return true;
  return draft.text !== cellText(original);
}

export function parseRowCell(
  column: ColumnMeta,
  detail: ColumnInfo | undefined,
  draft: RowDraftCell,
  original?: CellValue,
): CellValue {
  if (draft.isNull) {
    if (!columnIsNullable(column, detail)) throw new Error(`${column.name} 不能为空`);
    return { kind: "null" };
  }
  const value = draft.text;
  if (original && original.kind !== "null" && cellText(original) === value) return original;
  const type = columnTypeName(column, detail);
  if (type === "bool" || type === "boolean") {
    if (/^(?:true|1)$/i.test(value)) return { kind: "bool", value: true };
    if (/^(?:false|0)$/i.test(value)) return { kind: "bool", value: false };
    throw new Error(`${column.name} 需要填写 true、false、1 或 0`);
  }
  if (INTEGER_TYPES.has(type)) {
    if (!(column.unsigned ? /^\d+$/ : /^-?\d+$/).test(value)) throw new Error(`${column.name} 需要填写整数`);
    return { kind: column.unsigned ? "unsigned" : "signed", value };
  }
  if (DECIMAL_TYPES.has(type)) {
    if (!/^-?\d+(?:\.\d+)?$/.test(value)) throw new Error(`${column.name} 需要填写十进制数`);
    return { kind: "decimal", value };
  }
  if (FLOAT_TYPES.has(type)) {
    const number = Number(value);
    if (!Number.isFinite(number)) throw new Error(`${column.name} 需要填写有限数值`);
    return { kind: "float", value: number };
  }
  if (type === "json" || type === "jsonb") {
    try { JSON.parse(value); } catch { throw new Error(`${column.name} 不是有效的 JSON`); }
    return { kind: "json", value };
  }
  if (type === "date") {
    if (!value) throw new Error(`${column.name} 需要选择日期`);
    return { kind: "date", value };
  }
  if (type.startsWith("time") && !type.startsWith("timestamp")) {
    if (!value.trim()) throw new Error(`${column.name} 需要填写时间`);
    if (!/^\d{1,2}:\d{2}(:\d{2}(\.\d+)?)?$/.test(value.trim())) {
      throw new Error(`${column.name} 时间格式无效，应为 HH:MM 或 HH:MM:SS`);
    }
    return { kind: "time", value };
  }
  if (type === "datetime" || type.startsWith("timestamp")) {
    if (!value) throw new Error(`${column.name} 需要选择日期和时间`);
    return { kind: "date_time", value };
  }
  return { kind: "text", value };
}
