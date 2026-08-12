export interface SqlTableReference {
  database?: string;
  table: string;
  alias?: string;
  depth: number;
}

interface SqlToken {
  kind: "identifier" | "keyword" | "punctuation";
  text: string;
  upper: string;
  from: number;
  to: number;
  depth: number;
}

const SQL_CLAUSE_KEYWORDS = new Set([
  "AS", "CROSS", "EXCEPT", "FETCH", "FOR", "FULL", "GROUP", "HAVING", "INNER", "INTERSECT",
  "JOIN", "LEFT", "LIMIT", "NATURAL", "OFFSET", "ON", "ORDER", "OUTER", "RETURNING", "RIGHT",
  "SET", "STRAIGHT_JOIN", "UNION", "USING", "WHERE", "WINDOW",
]);

function isIdentifierStart(character: string) {
  return /[\p{L}_]/u.test(character);
}

function isIdentifierContinue(character: string) {
  return /[\p{L}\p{N}_$]/u.test(character);
}

function unquoteIdentifier(value: string) {
  if (value.startsWith("`") && value.endsWith("`")) return value.slice(1, -1).split("``").join("`");
  if (value.startsWith('"') && value.endsWith('"')) return value.slice(1, -1).split('""').join('"');
  if (value.startsWith("[") && value.endsWith("]")) return value.slice(1, -1).split("]]").join("]");
  return value;
}

function scanQuoted(source: string, start: number, opening: string, closing = opening) {
  let index = start + 1;
  while (index < source.length) {
    if (source[index] === closing) {
      if (source[index + 1] === closing) index += 2;
      else return index + 1;
    } else if (source[index] === "\\" && opening !== "[") index += 2;
    else index += 1;
  }
  return source.length;
}

function scanSqlTokens(source: string): SqlToken[] {
  const tokens: SqlToken[] = [];
  let depth = 0;
  let index = 0;
  while (index < source.length) {
    const character = source[index]!;
    const next = source[index + 1];
    if (/\s/u.test(character)) { index += 1; continue; }
    if ((character === "-" && next === "-") || character === "#") {
      const newline = source.indexOf("\n", index + (character === "#" ? 1 : 2));
      index = newline < 0 ? source.length : newline + 1;
      continue;
    }
    if (character === "/" && next === "*") {
      const end = source.indexOf("*/", index + 2);
      index = end < 0 ? source.length : end + 2;
      continue;
    }
    if (character === "'") { index = scanQuoted(source, index, "'"); continue; }
    if (character === "$" && /^\$[A-Za-z0-9_]*\$/.test(source.slice(index))) {
      const tag = /^\$[A-Za-z0-9_]*\$/.exec(source.slice(index))![0];
      const end = source.indexOf(tag, index + tag.length);
      index = end < 0 ? source.length : end + tag.length;
      continue;
    }
    if (character === "(" || character === ")" || character === "," || character === "." || character === ";") {
      if (character === ")") depth = Math.max(0, depth - 1);
      tokens.push({ kind: "punctuation", text: character, upper: character, from: index, to: index + 1, depth });
      if (character === "(") depth += 1;
      index += 1;
      continue;
    }
    if (character === "`" || character === '"' || character === "[") {
      const end = scanQuoted(source, index, character, character === "[" ? "]" : character);
      const text = unquoteIdentifier(source.slice(index, end));
      tokens.push({ kind: "identifier", text, upper: text.toLocaleUpperCase(), from: index, to: end, depth });
      index = end;
      continue;
    }
    if (isIdentifierStart(character)) {
      let end = index + 1;
      while (end < source.length && isIdentifierContinue(source[end]!)) end += 1;
      const text = source.slice(index, end);
      const upper = text.toLocaleUpperCase();
      tokens.push({ kind: SQL_CLAUSE_KEYWORDS.has(upper) || upper === "FROM" ? "keyword" : "identifier", text, upper, from: index, to: end, depth });
      index = end;
      continue;
    }
    index += 1;
  }
  return tokens;
}

function currentStatementTokens(tokens: readonly SqlToken[], cursor: number) {
  let start = 0;
  let end = Number.POSITIVE_INFINITY;
  for (const token of tokens) {
    if (token.text !== ";") continue;
    if (token.to <= cursor) start = token.to;
    else { end = token.from; break; }
  }
  return tokens.filter((token) => token.from >= start && token.from < end && token.text !== ";");
}

function cursorDepth(tokens: readonly SqlToken[], cursor: number) {
  let depth = 0;
  for (const token of tokens) {
    if (token.from >= cursor) break;
    if (token.text === "(") depth += 1;
    else if (token.text === ")") depth = Math.max(0, depth - 1);
  }
  return depth;
}

function identifierAt(tokens: readonly SqlToken[], index: number) {
  const token = tokens[index];
  return token?.kind === "identifier" ? token : null;
}

function tableReferenceAfter(tokens: readonly SqlToken[], keywordIndex: number): SqlTableReference | null {
  const keyword = tokens[keywordIndex]!;
  let index = keywordIndex + 1;
  if (tokens[index]?.text === "(") return null;
  const parts: string[] = [];
  const first = identifierAt(tokens, index);
  if (!first || first.depth !== keyword.depth) return null;
  parts.push(first.text);
  index += 1;
  while (tokens[index]?.text === ".") {
    const part = identifierAt(tokens, index + 1);
    if (!part || part.depth !== keyword.depth) break;
    parts.push(part.text);
    index += 2;
  }
  let alias: string | undefined;
  if (tokens[index]?.upper === "AS") index += 1;
  const aliasToken = identifierAt(tokens, index);
  if (aliasToken && aliasToken.depth === keyword.depth && !SQL_CLAUSE_KEYWORDS.has(aliasToken.upper)) alias = aliasToken.text;
  return {
    database: parts.length > 1 ? parts.slice(0, -1).join(".") : undefined,
    table: parts.at(-1)!,
    alias,
    depth: keyword.depth,
  };
}

/** Finds FROM/JOIN table references visible at the cursor in its current SQL statement. */
export function sqlTableReferences(source: string, cursor = source.length): SqlTableReference[] {
  const allTokens = scanSqlTokens(source);
  const tokens = currentStatementTokens(allTokens, cursor);
  const visibleDepth = cursorDepth(tokens, cursor);
  const references: SqlTableReference[] = [];
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index]!;
    if ((token.upper !== "FROM" && token.upper !== "JOIN") || token.depth > visibleDepth) continue;
    const reference = tableReferenceAfter(tokens, index);
    if (!reference) continue;
    const key = `${reference.database?.toLocaleLowerCase() ?? ""}\0${reference.table.toLocaleLowerCase()}\0${reference.alias?.toLocaleLowerCase() ?? ""}`;
    if (!references.some((existing) => `${existing.database?.toLocaleLowerCase() ?? ""}\0${existing.table.toLocaleLowerCase()}\0${existing.alias?.toLocaleLowerCase() ?? ""}` === key)) {
      references.push(reference);
    }
  }
  return references;
}

/** Returns the qualifier immediately before the identifier being completed, such as `d` in `d.na`. */
export function sqlCompletionQualifier(source: string, from: number) {
  const prefix = source.slice(0, from);
  const match = /(?:[`"]([^`"]+)[`"]|\[([^\]]+)\]|([\p{L}_][\p{L}\p{N}_$]*))\s*\.\s*$/u.exec(prefix);
  return match?.[1] ?? match?.[2] ?? match?.[3];
}

/** Avoids mixing field suggestions into the position where a FROM/JOIN table name is still being typed. */
export function isSqlTableNamePosition(source: string, cursor: number) {
  const tokens = currentStatementTokens(scanSqlTokens(source), cursor).filter((token) => token.from < cursor);
  let index = tokens.length - 1;
  while (index >= 0 && tokens[index]!.text === ".") index -= 1;
  while (index >= 0 && tokens[index]!.kind === "identifier") {
    index -= 1;
    if (tokens[index]?.text !== ".") break;
    index -= 1;
  }
  return tokens[index]?.upper === "FROM" || tokens[index]?.upper === "JOIN";
}
