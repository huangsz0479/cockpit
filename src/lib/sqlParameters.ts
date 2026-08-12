export type SqlParameterMode = "text" | "number" | "null" | "raw";
export interface SqlParameterValue { value: string; mode: SqlParameterMode }

interface Placeholder { name: string; from: number; to: number }

function placeholders(sql: string): Placeholder[] {
  const result: Placeholder[] = [];
  let state: "normal" | "single" | "double" | "backtick" | "line" | "block" = "normal";
  let dollarTag: string | null = null;
  for (let index = 0; index < sql.length; index += 1) {
    const character = sql[index]!;
    const next = sql[index + 1];
    if (dollarTag) {
      if (sql.startsWith(dollarTag, index)) { index += dollarTag.length - 1; dollarTag = null; }
      continue;
    }
    if (state === "line") { if (character === "\n") state = "normal"; continue; }
    if (state === "block") { if (character === "*" && next === "/") { state = "normal"; index += 1; } continue; }
    if (state !== "normal") {
      const delimiter = state === "single" ? "'" : state === "double" ? '"' : "`";
      if (character === "\\") { index += 1; continue; }
      if (character === delimiter) {
        if (next === delimiter) index += 1;
        else state = "normal";
      }
      continue;
    }
    if (character === "-" && next === "-") { state = "line"; index += 1; continue; }
    if (character === "#") { state = "line"; continue; }
    if (character === "/" && next === "*") { state = "block"; index += 1; continue; }
    if (character === "'") { state = "single"; continue; }
    if (character === '"') { state = "double"; continue; }
    if (character === "`") { state = "backtick"; continue; }
    if (character === "$") {
      const match = /^\$[A-Za-z0-9_]*\$/.exec(sql.slice(index));
      if (match) { dollarTag = match[0]; index += match[0].length - 1; continue; }
    }
    if (character !== "{" || next !== "{") continue;
    const end = sql.indexOf("}}", index + 2);
    if (end < 0) continue;
    const name = sql.slice(index + 2, end).trim();
    if (/^[A-Za-z_][A-Za-z0-9_.-]*$/.test(name)) {
      result.push({ name, from: index, to: end + 2 });
      index = end + 1;
    }
  }
  return result;
}

export function findSqlParameters(sql: string): string[] {
  return [...new Set(placeholders(sql).map((item) => item.name))];
}

function sqlLiteral(parameter: SqlParameterValue): string {
  if (parameter.mode === "null") return "NULL";
  if (parameter.mode === "raw") {
    if (!parameter.value.trim()) throw new Error("原始 SQL 参数不能为空");
    return parameter.value.trim();
  }
  if (parameter.mode === "number") {
    if (!/^-?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?$/i.test(parameter.value.trim())) throw new Error(`不是有效数值：${parameter.value}`);
    return parameter.value.trim();
  }
  return `'${parameter.value.replace(/\\/g, "\\\\").replace(/'/g, "''")}'`;
}

export function renderSqlParameters(sql: string, values: Record<string, SqlParameterValue>): string {
  const items = placeholders(sql);
  let rendered = sql;
  for (const item of items.reverse()) {
    const value = values[item.name];
    if (!value) throw new Error(`缺少参数：${item.name}`);
    rendered = `${rendered.slice(0, item.from)}${sqlLiteral(value)}${rendered.slice(item.to)}`;
  }
  return rendered;
}
