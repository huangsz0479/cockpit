import type { DatabaseKind, TableDetail } from "@/types";

export function quoteMysqlIdentifier(identifier: string): string {
  return `\`${identifier.replace(/`/g, "``")}\``;
}

export function quoteIdentifier(identifier: string, databaseKind: DatabaseKind = "mysql"): string {
  return databaseKind === "mysql" || databaseKind === "mariadb"
    ? quoteMysqlIdentifier(identifier)
    : `"${identifier.replace(/"/g, "\"\"")}"`;
}

// Elasticsearch SQL 的 FROM 只接受索引名，不带库名限定。
function qualifiedTableName(database: string, table: string, databaseKind: DatabaseKind): string {
  return databaseKind === "elasticsearch"
    ? quoteIdentifier(table, databaseKind)
    : `${quoteIdentifier(database, databaseKind)}.${quoteIdentifier(table, databaseKind)}`;
}

// Elasticsearch SQL 解析器不接受尾部分号。
function statementTerminator(databaseKind: DatabaseKind): string {
  return databaseKind === "elasticsearch" ? "" : ";";
}

export function selectPreviewSql(database: string, table: string, databaseKind: DatabaseKind = "mysql"): string {
  return `SELECT *\nFROM ${qualifiedTableName(database, table, databaseKind)}\nLIMIT 100${statementTerminator(databaseKind)}`;
}

export function selectTablePageSql(
  database: string,
  table: string,
  pageSize: number,
  offset: number,
  filter = "",
  sortColumn?: string | null,
  sortDirection: "asc" | "desc" = "asc",
  databaseKind: DatabaseKind = "mysql",
): string {
  const normalizedFilter = filter.trim().replace(/^WHERE\s+/i, "").replace(/;+\s*$/, "");
  const whereClause = normalizedFilter ? `\nWHERE ${normalizedFilter}` : "";
  const orderClause = sortColumn
    ? `\nORDER BY ${quoteIdentifier(sortColumn, databaseKind)} ${sortDirection.toUpperCase()}`
    : "";
  // Elasticsearch SQL 只有 LIMIT 没有 OFFSET，深分页由后端游标完成。
  const pageClause = databaseKind === "elasticsearch"
    ? ""
    : `\nLIMIT ${Math.max(1, pageSize) + 1} OFFSET ${Math.max(0, offset)}${statementTerminator(databaseKind)}`;
  return `SELECT *\nFROM ${qualifiedTableName(database, table, databaseKind)}${whereClause}${orderClause}${pageClause}`;
}

export function selectQueryPageSql(sql: string, pageSize: number, offset: number, databaseKind: DatabaseKind = "mysql"): string {
  void databaseKind;
  const statement = sql.trim().replace(/;+\s*$/, "");
  return `${statement}\nLIMIT ${Math.max(1, pageSize) + 1} OFFSET ${Math.max(0, offset)};`;
}

function topLevelSqlTokens(sql: string): string[] | null {
  const tokens: string[] = [];
  let token = "";
  let depth = 0;
  let quote: "'" | '"' | "`" | null = null;
  let lineComment = false;
  let blockComment = false;
  const push = () => {
    if (token) tokens.push(token.toUpperCase());
    token = "";
  };
  for (let index = 0; index < sql.length; index += 1) {
    const character = sql[index]!;
    const next = sql[index + 1];
    if (lineComment) {
      if (character === "\n") lineComment = false;
      continue;
    }
    if (blockComment) {
      if (character === "*" && next === "/") { blockComment = false; index += 1; }
      continue;
    }
    if (quote) {
      if (character === "\\") index += 1;
      else if (character === quote) {
        if (next === quote) index += 1;
        else quote = null;
      }
      continue;
    }
    if (character === "#" || (character === "-" && next === "-")) {
      push(); lineComment = true; if (character === "-") index += 1; continue;
    }
    if (character === "/" && next === "*") {
      push(); blockComment = true; index += 1; continue;
    }
    if (character === "'" || character === '"' || character === "`") {
      push(); quote = character; continue;
    }
    if (character === "(") { push(); depth += 1; continue; }
    if (character === ")") { push(); depth = Math.max(0, depth - 1); continue; }
    if (depth === 0 && character === ";") return null;
    if (depth === 0 && /[A-Za-z0-9_]/.test(character)) token += character;
    else push();
  }
  push();
  return quote || blockComment || depth !== 0 ? null : tokens;
}

function hasTokenSequence(tokens: string[], expected: string[]) {
  return tokens.some((_, index) => expected.every((token, offset) => tokens[index + offset] === token));
}

export function canPageSelectQuery(sql: string): boolean {
  const statement = sql.trim().replace(/;+\s*$/, "");
  const tokens = topLevelSqlTokens(statement);
  if (!tokens || tokens[0] !== "SELECT") return false;
  if (tokens.some((token) => ["INTO", "PROCEDURE"].includes(token))) return false;
  return !hasTokenSequence(tokens, ["FOR", "UPDATE"])
    && !hasTokenSequence(tokens, ["FOR", "SHARE"])
    && !hasTokenSequence(tokens, ["LOCK", "IN", "SHARE", "MODE"]);
}

export function canAppendSelectQueryLimit(sql: string): boolean {
  if (!canPageSelectQuery(sql)) return false;
  const statement = sql.trim().replace(/;+\s*$/, "");
  const tokens = topLevelSqlTokens(statement);
  return Boolean(tokens && !tokens.some((token) => token === "LIMIT" || token === "OFFSET"));
}

export interface SingleTableSelectTarget {
  database: string;
  table: string;
}

const SQL_IDENTIFIER_PART = '(?:`(?:``|[^`])+`|"(?:""|[^"])+"|\\[(?:\\]\\]|[^\\]])+\\]|[\\p{L}_][\\p{L}\\p{N}_$]*)';
const SINGLE_TABLE_SELECT_ALL = new RegExp(
  String.raw`^SELECT\s+\*\s+FROM\s+(${SQL_IDENTIFIER_PART})(?:\s*\.\s*(${SQL_IDENTIFIER_PART}))?([\s\S]*)$`,
  "iu",
);
const SELECT_TAIL_CLAUSE = /^(?:WHERE\b|ORDER\s+BY\b|LIMIT\b|OFFSET\b|FETCH\b|FOR\s+(?:UPDATE|SHARE)\b)/iu;
const NON_ROW_PRESERVING_SELECT_TAIL = /\b(?:SELECT|JOIN|GROUP\s+BY|HAVING|UNION|INTERSECT|EXCEPT)\b/iu;

function unquoteSqlIdentifier(identifier: string) {
  if (identifier.startsWith("`") && identifier.endsWith("`")) return identifier.slice(1, -1).replace(/``/g, "`");
  if (identifier.startsWith('"') && identifier.endsWith('"')) return identifier.slice(1, -1).replace(/""/g, '"');
  if (identifier.startsWith("[") && identifier.endsWith("]")) return identifier.slice(1, -1).replace(/]]/g, "]");
  return identifier;
}

function sqlStatementsWithoutComments(sql: string): string[] | null {
  const statements: string[] = [];
  let current = "";
  let quote: "'" | '"' | "`" | "]" | null = null;
  let lineComment = false;
  let blockComment = false;
  const pushStatement = () => {
    const statement = current.trim();
    if (statement) statements.push(statement);
    current = "";
  };

  for (let index = 0; index < sql.length; index += 1) {
    const character = sql[index]!;
    const next = sql[index + 1];
    if (lineComment) {
      if (character === "\n" || character === "\r") {
        lineComment = false;
        current += character;
      }
      continue;
    }
    if (blockComment) {
      if (character === "*" && next === "/") {
        blockComment = false;
        current += " ";
        index += 1;
      }
      continue;
    }
    if (quote) {
      current += character;
      if (character === "\\" && quote !== "]" && next) {
        current += next;
        index += 1;
      } else if (character === quote) {
        if (next === quote) {
          current += next;
          index += 1;
        } else {
          quote = null;
        }
      }
      continue;
    }
    if (character === "#" || (character === "-" && next === "-")) {
      lineComment = true;
      current += " ";
      if (character === "-") index += 1;
      continue;
    }
    if (character === "/" && next === "*") {
      if (sql[index + 2] === "!" || (sql[index + 2]?.toUpperCase() === "M" && sql[index + 3] === "!")) return null;
      blockComment = true;
      current += " ";
      index += 1;
      continue;
    }
    if (character === ";") {
      pushStatement();
      continue;
    }
    if (character === "'" || character === '"' || character === "`") quote = character;
    else if (character === "[") quote = "]";
    current += character;
  }

  if (blockComment || quote) return null;
  pushStatement();
  return statements;
}

function singleTableSelectAllStatementTarget(statement: string, defaultDatabase?: string | null): SingleTableSelectTarget | null {
  const matched = statement.match(SINGLE_TABLE_SELECT_ALL);
  if (!matched) return null;

  let tail = (matched[3] ?? "").trim();
  if (tail && !SELECT_TAIL_CLAUSE.test(tail)) {
    const alias = tail.match(new RegExp(String.raw`^(?:AS\s+)?${SQL_IDENTIFIER_PART}(?:\s+|$)`, "iu"));
    if (!alias) return null;
    tail = tail.slice(alias[0].length).trim();
  }
  if (tail && !SELECT_TAIL_CLAUSE.test(tail)) return null;
  if (NON_ROW_PRESERVING_SELECT_TAIL.test(tail)) return null;

  const qualifiedDatabase = matched[2] ? unquoteSqlIdentifier(matched[1]!) : defaultDatabase?.trim();
  const table = unquoteSqlIdentifier(matched[2] ?? matched[1]!);
  return qualifiedDatabase && table ? { database: qualifiedDatabase, table } : null;
}

export function singleTableSelectAllTargets(sql: string, defaultDatabase?: string | null): (SingleTableSelectTarget | null)[] {
  const statements = sqlStatementsWithoutComments(sql);
  return statements?.map((statement) => singleTableSelectAllStatementTarget(statement, defaultDatabase)) ?? [];
}

export const MYSQL_COLUMN_TYPES = [
  "TINYINT", "SMALLINT", "MEDIUMINT", "INT", "INTEGER", "BIGINT", "DECIMAL", "NUMERIC", "FLOAT", "DOUBLE", "REAL", "BIT",
  "VARCHAR", "CHAR", "TEXT", "TINYTEXT", "MEDIUMTEXT", "LONGTEXT", "BINARY", "VARBINARY",
  "BLOB", "TINYBLOB", "MEDIUMBLOB", "LONGBLOB", "DATE", "TIME", "DATETIME", "TIMESTAMP",
  "YEAR", "BOOLEAN", "JSON", "ENUM", "SET", "GEOMETRY", "POINT", "LINESTRING", "POLYGON",
  "MULTIPOINT", "MULTILINESTRING", "MULTIPOLYGON", "GEOMETRYCOLLECTION",
] as const;

export type MysqlColumnType = typeof MYSQL_COLUMN_TYPES[number];

export const SQLITE_COLUMN_TYPES = ["INTEGER", "REAL", "TEXT", "BLOB", "NUMERIC"] as const satisfies readonly MysqlColumnType[];

export interface CreateTableColumnDefinition {
  name: string;
  dataType: MysqlColumnType;
  size: string;
  unsigned: boolean;
  nullable: boolean;
  primaryKey: boolean;
  autoIncrement: boolean;
  defaultValue?: string | null;
  defaultExpression?: boolean;
  onUpdate?: string;
  invisible?: boolean;
  extraAttributes?: string;
  comment?: string;
  originalName?: string;
  generatedExpression?: string;
  generatedStored?: boolean;
  collation?: string;
}

export interface TableIndexDefinition {
  id: string;
  originalName?: string;
  rawDefinition?: string;
  preserveRaw?: boolean;
  name: string;
  columns: string[];
  unique: boolean;
  indexType?: "INDEX" | "FULLTEXT" | "SPATIAL";
}

export interface TableForeignKeyDefinition {
  id: string;
  originalName?: string;
  name: string;
  columns: string[];
  referencedDatabase: string;
  referencedTable: string;
  referencedColumns: string[];
  onUpdate: string;
  onDelete: string;
}

export interface TableCheckDefinition {
  id: string;
  originalName?: string;
  name: string;
  expression: string;
  enforced: boolean;
}

export interface CreateTableDefinition {
  name: string;
  columns: CreateTableColumnDefinition[];
  indexes?: TableIndexDefinition[];
  foreignKeys?: TableForeignKeyDefinition[];
  checks?: TableCheckDefinition[];
  engine?: string;
  charset?: string;
  originalName?: string;
  collation?: string;
  comment?: string;
  partitionClause?: string;
}

export function createDefaultTableDefinition(databaseKind: DatabaseKind = "mysql"): CreateTableDefinition {
  if (databaseKind === "sqlite") {
    return {
      name: "",
      columns: [{
        name: "id",
        dataType: "INTEGER",
        size: "",
        unsigned: false,
        nullable: false,
        primaryKey: true,
        autoIncrement: true,
        defaultValue: null,
      }],
      indexes: [],
      foreignKeys: [],
      checks: [],
    };
  }
  return {
    name: "",
    columns: [{
      name: "id",
      dataType: "BIGINT",
      size: "",
      unsigned: true,
      nullable: false,
      primaryKey: true,
      autoIncrement: true,
      defaultValue: null,
      comment: "",
    }],
    indexes: [],
    foreignKeys: [],
    checks: [],
    engine: "InnoDB",
    charset: "utf8mb4",
    collation: "",
    comment: "",
    partitionClause: "",
  };
}

const SIZED_COLUMN_TYPES = new Set<MysqlColumnType>(["VARCHAR", "CHAR", "VARBINARY", "BINARY", "DECIMAL", "NUMERIC", "BIT", "TIME", "DATETIME", "TIMESTAMP", "ENUM", "SET"]);
const UNSIGNED_COLUMN_TYPES = new Set<MysqlColumnType>(["TINYINT", "SMALLINT", "MEDIUMINT", "INT", "INTEGER", "BIGINT", "DECIMAL", "NUMERIC", "FLOAT", "DOUBLE", "REAL"]);
const AUTO_INCREMENT_COLUMN_TYPES = new Set<MysqlColumnType>(["TINYINT", "SMALLINT", "MEDIUMINT", "INT", "INTEGER", "BIGINT"]);

export function mysqlColumnTypeSupportsSize(dataType: MysqlColumnType): boolean {
  return SIZED_COLUMN_TYPES.has(dataType);
}

export function mysqlColumnTypeSupportsUnsigned(dataType: MysqlColumnType): boolean {
  return UNSIGNED_COLUMN_TYPES.has(dataType);
}

export function mysqlColumnTypeSupportsAutoIncrement(dataType: MysqlColumnType): boolean {
  return AUTO_INCREMENT_COLUMN_TYPES.has(dataType);
}

function validateIdentifier(identifier: string, label: string): string | null {
  if (!identifier.trim()) return `请输入${label}`;
  if (identifier.length > 64) return `${label}不能超过 64 个字符`;
  return null;
}

function validateColumnSize(column: CreateTableColumnDefinition, databaseKind: DatabaseKind): string | null {
  const size = column.size.trim();
  if (databaseKind === "sqlite") return size ? `SQLite 字段“${column.name}”无需填写长度或精度` : null;
  if (["VARCHAR", "CHAR", "VARBINARY", "BINARY"].includes(column.dataType)) {
    if (!/^\d+$/.test(size)) return `字段“${column.name}”需要填写有效长度`;
    const length = Number(size);
    const maximum = column.dataType === "CHAR" || column.dataType === "BINARY" ? 255 : 65_535;
    if (length < 1 || length > maximum) return `字段“${column.name}”长度必须在 1 到 ${maximum} 之间`;
  } else if (column.dataType === "DECIMAL" || column.dataType === "NUMERIC") {
    const match = /^(\d+),(\d+)$/.exec(size);
    if (!match) return `字段“${column.name}”精度格式应为“总位数,小数位数”`;
    const precision = Number(match[1]);
    const scale = Number(match[2]);
    if (precision < 1 || precision > 65 || scale > 30 || scale > precision) {
      return `字段“${column.name}”的 DECIMAL 精度无效`;
    }
  } else if (column.dataType === "BIT") {
    if (!/^\d+$/.test(size) || Number(size) < 1 || Number(size) > 64) return `字段“${column.name}”的 BIT 长度必须在 1 到 64 之间`;
  } else if (["TIME", "DATETIME", "TIMESTAMP"].includes(column.dataType)) {
    if (size && (!/^\d$/.test(size) || Number(size) > 6)) return `字段“${column.name}”的小数秒精度必须在 0 到 6 之间`;
  } else if (["ENUM", "SET"].includes(column.dataType)) {
    if (!/^\s*'(?:[^'\\]|\\.)+'(?:\s*,\s*'(?:[^'\\]|\\.)+')*\s*$/.test(size)) return `字段“${column.name}”需要填写如 'a','b' 的成员列表`;
  } else if (size) {
    return `字段“${column.name}”的类型不支持长度或精度`;
  }
  return null;
}

export function validateCreateTableDefinition(definition: CreateTableDefinition, databaseKind: DatabaseKind = "mysql"): string | null {
  const sqlite = databaseKind === "sqlite";
  const tableError = validateIdentifier(definition.name, "表名");
  if (tableError) return tableError;
  if (!definition.columns.length) return "至少需要一个字段";

  const names = new Set<string>();
  let autoIncrementCount = 0;
  for (const [index, column] of definition.columns.entries()) {
    const nameError = validateIdentifier(column.name, `第 ${index + 1} 个字段名称`);
    if (nameError) return nameError;
    const normalizedName = column.name.trim().toLocaleLowerCase();
    if (names.has(normalizedName)) return `字段名不能重复：${column.name.trim()}`;
    names.add(normalizedName);
    if (sqlite
      ? !(SQLITE_COLUMN_TYPES as readonly MysqlColumnType[]).includes(column.dataType)
      : !MYSQL_COLUMN_TYPES.includes(column.dataType)) return `字段“${column.name}”的数据类型无效`;
    const sizeError = validateColumnSize(column, databaseKind);
    if (sizeError) return sizeError;
    if (column.unsigned && (sqlite || !mysqlColumnTypeSupportsUnsigned(column.dataType))) {
      return `字段“${column.name}”的类型不支持无符号`;
    }
    if (column.autoIncrement) {
      autoIncrementCount += 1;
      if (sqlite ? column.dataType !== "INTEGER" : !mysqlColumnTypeSupportsAutoIncrement(column.dataType)) return `字段“${column.name}”的类型不支持自增`;
      if (!column.primaryKey) return `自增字段“${column.name}”必须是主键`;
    }
    if (column.generatedExpression?.trim() && (column.autoIncrement || column.defaultValue != null || column.onUpdate?.trim())) return `生成列“${column.name}”不能设置默认值、自增或自动更新时间`;
    if (sqlite && column.generatedExpression?.trim() && column.primaryKey) return `SQLite 生成列“${column.name}”不能作为主键`;
  }
  if (autoIncrementCount > 1) return "一张表只能有一个自增字段";
  if (sqlite && autoIncrementCount && definition.columns.filter((column) => column.primaryKey).length > 1) {
    return "SQLite 自增字段不能与其他字段组成联合主键";
  }
  for (const index of definition.indexes ?? []) {
    if (!index.name.trim()) return "索引名称不能为空";
    if ((!index.columns.length && !index.preserveRaw) || index.columns.some((name) => !names.has(name.toLocaleLowerCase()))) return `索引“${index.name}”包含无效字段`;
    if (sqlite && index.indexType && index.indexType !== "INDEX") return `SQLite 不支持 ${index.indexType} 索引`;
  }
  const indexNames = (definition.indexes ?? []).map((index) => index.name.trim().toLocaleLowerCase());
  if (new Set(indexNames).size !== indexNames.length) return "索引名称不能重复";
  for (const foreignKey of definition.foreignKeys ?? []) {
    if (!foreignKey.name.trim() || !foreignKey.referencedTable.trim()) return "外键名称和引用表不能为空";
    if (!foreignKey.columns.length || foreignKey.columns.length !== foreignKey.referencedColumns.length) return `外键“${foreignKey.name}”字段数量不匹配`;
    if (foreignKey.columns.some((name) => !names.has(name.toLocaleLowerCase()))) return `外键“${foreignKey.name}”包含无效的本表字段`;
    if (foreignKey.referencedColumns.some((name) => !name.trim())) return `外键“${foreignKey.name}”包含空的引用字段`;
  }
  const foreignKeyNames = (definition.foreignKeys ?? []).map((foreignKey) => foreignKey.name.trim().toLocaleLowerCase());
  if (new Set(foreignKeyNames).size !== foreignKeyNames.length) return "外键名称不能重复";
  for (const check of definition.checks ?? []) {
    if (!check.name.trim()) return "检查约束名称不能为空";
    if (!check.expression.trim()) return `检查约束“${check.name}”的表达式不能为空`;
  }
  const checkNames = (definition.checks ?? []).map((check) => check.name.trim().toLocaleLowerCase());
  if (new Set(checkNames).size !== checkNames.length) return "检查约束名称不能重复";
  return null;
}

function columnTypeSql(column: CreateTableColumnDefinition): string {
  const size = column.size.trim();
  return mysqlColumnTypeSupportsSize(column.dataType) && size ? `${column.dataType}(${size})` : column.dataType;
}

function quoteMysqlString(value: string): string {
  return `'${value.replace(/\\/g, "\\\\").replace(/'/g, "''")}'`;
}

function defaultSql(value: string | null | undefined, expression = false, databaseKind: DatabaseKind = "mysql"): string {
  if (value == null) return "";
  const normalized = value.trim();
  const quoted = databaseKind === "sqlite"
    ? `'${value.replace(/'/g, "''")}'`
    : quoteMysqlString(value);
  if (!normalized) return `DEFAULT ${quoted}`;
  const keywordDefault = databaseKind === "sqlite"
    ? /^(NULL|TRUE|FALSE|CURRENT_TIMESTAMP|CURRENT_DATE|CURRENT_TIME)$/i
    : /^(NULL|CURRENT_TIMESTAMP(?:\(\d+\))?|CURRENT_DATE|CURRENT_TIME)$/i;
  if (keywordDefault.test(normalized)) return `DEFAULT ${normalized.toUpperCase()}`;
  if (/^-?\d+(?:\.\d+)?$/.test(normalized)) return `DEFAULT ${normalized}`;
  const binaryDefault = databaseKind === "sqlite"
    ? /^X'[0-9A-F]+'$/i
    : /^(?:B'[01]+'|0B[01]+|X'[0-9A-F]+'|0X[0-9A-F]+)$/i;
  if (binaryDefault.test(normalized)) return `DEFAULT ${normalized}`;
  if (expression) return `DEFAULT ${normalized.startsWith("(") && normalized.endsWith(")") ? normalized : `(${normalized})`}`;
  return `DEFAULT ${quoted}`;
}

function columnSql(column: CreateTableColumnDefinition): string {
  const generated = column.generatedExpression?.trim();
  return [
    quoteMysqlIdentifier(column.name.trim()),
    columnTypeSql(column),
    column.unsigned ? "UNSIGNED" : "",
    column.primaryKey || !column.nullable ? "NOT NULL" : "NULL",
    column.collation?.trim() ? `COLLATE ${column.collation.trim()}` : "",
    generated ? "" : defaultSql(column.defaultValue, column.defaultExpression),
    !generated && column.onUpdate?.trim() ? `ON UPDATE ${column.onUpdate.trim()}` : "",
    !generated && column.autoIncrement ? "AUTO_INCREMENT" : "",
    generated ? `GENERATED ALWAYS AS (${generated}) ${column.generatedStored ? "STORED" : "VIRTUAL"}` : "",
    column.comment?.trim() ? `COMMENT ${quoteMysqlString(column.comment.trim())}` : "",
    column.invisible ? "INVISIBLE" : "",
    column.extraAttributes?.trim() ?? "",
  ].filter(Boolean).join(" ");
}

function primaryKeySql(definition: CreateTableDefinition): string | null {
  const columns = definition.columns.filter((column) => column.primaryKey)
    .sort((left, right) => Number(right.autoIncrement) - Number(left.autoIncrement));
  return columns.length ? `PRIMARY KEY (${columns.map((column) => quoteMysqlIdentifier(column.name.trim())).join(", ")})` : null;
}

function indexSql(index: TableIndexDefinition): string {
  if (index.preserveRaw && index.rawDefinition?.trim()) return index.rawDefinition.trim().replace(/,$/, "");
  const prefix = index.indexType === "FULLTEXT" ? "FULLTEXT KEY" : index.indexType === "SPATIAL" ? "SPATIAL KEY" : index.unique ? "UNIQUE KEY" : "KEY";
  return `${prefix} ${quoteMysqlIdentifier(index.name.trim())} (${index.columns.map(quoteMysqlIdentifier).join(", ")})`;
}

function foreignKeySql(foreignKey: TableForeignKeyDefinition, fallbackDatabase: string): string {
  const referencedDatabase = foreignKey.referencedDatabase.trim() || fallbackDatabase;
  return [
    `CONSTRAINT ${quoteMysqlIdentifier(foreignKey.name.trim())}`,
    `FOREIGN KEY (${foreignKey.columns.map(quoteMysqlIdentifier).join(", ")})`,
    `REFERENCES ${quoteMysqlIdentifier(referencedDatabase)}.${quoteMysqlIdentifier(foreignKey.referencedTable.trim())} (${foreignKey.referencedColumns.map(quoteMysqlIdentifier).join(", ")})`,
    foreignKey.onUpdate ? `ON UPDATE ${foreignKey.onUpdate}` : "",
    foreignKey.onDelete ? `ON DELETE ${foreignKey.onDelete}` : "",
  ].filter(Boolean).join(" ");
}

function checkSql(check: TableCheckDefinition): string {
  return `CONSTRAINT ${quoteMysqlIdentifier(check.name.trim())} CHECK (${check.expression.trim()})${check.enforced ? "" : " NOT ENFORCED"}`;
}

function sqliteColumnSql(column: CreateTableColumnDefinition): string {
  const generated = column.generatedExpression?.trim();
  const inlineAutoIncrement = column.autoIncrement;
  return [
    quoteIdentifier(column.name.trim(), "sqlite"),
    column.dataType,
    inlineAutoIncrement ? "PRIMARY KEY AUTOINCREMENT" : "",
    inlineAutoIncrement ? "" : column.primaryKey || !column.nullable ? "NOT NULL" : "NULL",
    column.collation?.trim() ? `COLLATE ${column.collation.trim()}` : "",
    generated ? "" : defaultSql(column.defaultValue, column.defaultExpression, "sqlite"),
    generated ? `GENERATED ALWAYS AS (${generated}) ${column.generatedStored ? "STORED" : "VIRTUAL"}` : "",
  ].filter(Boolean).join(" ");
}

function sqlitePrimaryKeySql(definition: CreateTableDefinition): string | null {
  const columns = definition.columns.filter((column) => column.primaryKey && !column.autoIncrement);
  return columns.length ? `PRIMARY KEY (${columns.map((column) => quoteIdentifier(column.name.trim(), "sqlite")).join(", ")})` : null;
}

function sqliteForeignKeySql(foreignKey: TableForeignKeyDefinition): string {
  return [
    `CONSTRAINT ${quoteIdentifier(foreignKey.name.trim(), "sqlite")}`,
    `FOREIGN KEY (${foreignKey.columns.map((column) => quoteIdentifier(column, "sqlite")).join(", ")})`,
    `REFERENCES ${quoteIdentifier(foreignKey.referencedTable.trim(), "sqlite")} (${foreignKey.referencedColumns.map((column) => quoteIdentifier(column, "sqlite")).join(", ")})`,
    foreignKey.onUpdate ? `ON UPDATE ${foreignKey.onUpdate}` : "",
    foreignKey.onDelete ? `ON DELETE ${foreignKey.onDelete}` : "",
  ].filter(Boolean).join(" ");
}

function sqliteCheckSql(check: TableCheckDefinition): string {
  return `CONSTRAINT ${quoteIdentifier(check.name.trim(), "sqlite")} CHECK (${check.expression.trim()})`;
}

function createSqliteTableSql(database: string, definition: CreateTableDefinition): string {
  const lines = definition.columns.map((column) => `  ${sqliteColumnSql(column)}`);
  const primary = sqlitePrimaryKeySql(definition);
  if (primary) lines.push(`  ${primary}`);
  lines.push(...(definition.foreignKeys ?? []).map((foreignKey) => `  ${sqliteForeignKeySql(foreignKey)}`));
  lines.push(...(definition.checks ?? []).map((check) => `  ${sqliteCheckSql(check)}`));
  const table = quoteIdentifier(definition.name.trim(), "sqlite");
  const create = `CREATE TABLE ${quoteIdentifier(database, "sqlite")}.${table} (\n${lines.join(",\n")}\n);`;
  const indexes = (definition.indexes ?? []).map((index) => [
    "CREATE",
    index.unique ? "UNIQUE" : "",
    "INDEX",
    `${quoteIdentifier(database, "sqlite")}.${quoteIdentifier(index.name.trim(), "sqlite")}`,
    "ON",
    table,
    `(${index.columns.map((column) => quoteIdentifier(column, "sqlite")).join(", ")});`,
  ].filter(Boolean).join(" "));
  return [create, ...indexes].join("\n");
}

export function createTableSql(database: string, definition: CreateTableDefinition, databaseKind: DatabaseKind = "mysql"): string {
  const validationError = validateCreateTableDefinition(definition, databaseKind);
  if (validationError) throw new Error(validationError);
  if (databaseKind === "sqlite") return createSqliteTableSql(database, definition);

  const lines = definition.columns.map((column) => `  ${columnSql(column)}`);
  const primary = primaryKeySql(definition);
  if (primary) lines.push(`  ${primary}`);
  lines.push(...(definition.indexes ?? []).map((index) => `  ${indexSql(index)}`));
  lines.push(...(definition.foreignKeys ?? []).map((foreignKey) => `  ${foreignKeySql(foreignKey, database)}`));
  lines.push(...(definition.checks ?? []).map((check) => `  ${checkSql(check)}`));
  const engine = definition.engine?.trim() || "InnoDB";
  const charset = definition.charset?.trim() || "utf8mb4";
  const collation = definition.collation?.trim() ? ` COLLATE=${definition.collation.trim()}` : "";
  const comment = definition.comment?.trim() ? ` COMMENT=${quoteMysqlString(definition.comment.trim())}` : "";
  const partition = definition.partitionClause?.trim() ? `\n${definition.partitionClause.trim()}` : "";
  return `CREATE TABLE ${quoteMysqlIdentifier(database)}.${quoteMysqlIdentifier(definition.name.trim())} (\n${lines.join(",\n")}\n) ENGINE=${engine} DEFAULT CHARSET=${charset}${collation}${comment}${partition};`;
}

function parseColumnType(fullType: string) {
  const match = /^([a-z0-9]+)(?:\(([^)]+)\))?/i.exec(fullType.trim());
  const dataType = (match?.[1] ?? "VARCHAR").toUpperCase() as MysqlColumnType;
  return { dataType: MYSQL_COLUMN_TYPES.includes(dataType) ? dataType : "VARCHAR" as MysqlColumnType, size: match?.[2] ?? (dataType === "VARCHAR" ? "255" : "") };
}

export function tableDetailToDefinition(detail: TableDetail): CreateTableDefinition {
  const primaryColumns = new Set(detail.indexes.find((index) => index.primary)?.columns ?? []);
  const engine = /ENGINE=([^\s]+)/i.exec(detail.ddl)?.[1] ?? "InnoDB";
  const charset = /(?:DEFAULT\s+)?CHARSET=([^\s;]+)/i.exec(detail.ddl)?.[1] ?? "utf8mb4";
  return {
    name: detail.table.name,
    originalName: detail.table.name,
    columns: detail.columns.map((column) => {
      const parsed = parseColumnType(column.fullType);
      const extra = column.extra?.toLocaleLowerCase() ?? "";
      const onUpdate = /\bon update\s+(.+?)(?=\s+(?:stored|virtual|generated|invisible|visible)\b|$)/i.exec(column.extra ?? "")?.[1]?.trim() ?? "";
      const extraAttributes = unsupportedColumnExtra(column.extra ?? "");
      return {
        name: column.name,
        originalName: column.name,
        dataType: parsed.dataType,
        size: parsed.size,
        unsigned: /\bunsigned\b/i.test(column.fullType),
        nullable: column.nullable,
        primaryKey: primaryColumns.has(column.name),
        autoIncrement: extra.includes("auto_increment"),
        defaultValue: column.defaultValue,
        defaultExpression: extra.includes("default_generated"),
        onUpdate,
        invisible: /\binvisible\b/i.test(extra),
        extraAttributes,
        comment: column.comment ?? "",
        generatedExpression: column.generationExpression ?? "",
        generatedStored: column.extra?.toLocaleLowerCase().includes("stored generated") ?? false,
        collation: column.collation ?? "",
      };
    }),
    indexes: detail.indexes.filter((index) => !index.primary).map((index) => {
      const definition: TableIndexDefinition = {
        id: crypto.randomUUID(), originalName: index.name, name: index.name, columns: [...index.columns], unique: index.unique,
        indexType: index.indexType?.toUpperCase() === "FULLTEXT" ? "FULLTEXT" : index.indexType?.toUpperCase() === "SPATIAL" ? "SPATIAL" : "INDEX",
      };
      const rawDefinition = mysqlNamedDefinitionLine(detail.ddl, index.name, /^(?:UNIQUE\s+|FULLTEXT\s+|SPATIAL\s+)?KEY\s+/i);
      if (rawDefinition && canonicalDefinition(rawDefinition) !== canonicalDefinition(indexSql(definition))) {
        definition.rawDefinition = rawDefinition;
        definition.preserveRaw = true;
      }
      return definition;
    }),
    foreignKeys: detail.foreignKeys.map((foreignKey) => ({
      id: crypto.randomUUID(), originalName: foreignKey.name, name: foreignKey.name, columns: [...foreignKey.columns],
      referencedDatabase: foreignKey.referencedDatabase, referencedTable: foreignKey.referencedTable,
      referencedColumns: [...foreignKey.referencedColumns], onUpdate: foreignKey.onUpdate ?? "RESTRICT",
      onDelete: foreignKey.onDelete ?? "RESTRICT",
    })),
    checks: detail.ddl.split("\n").flatMap((line) => {
      const match = /^\s*(?:CONSTRAINT\s+`((?:``|[^`])+)`\s+)?CHECK\s*\((.*)\)\s*(NOT\s+ENFORCED)?\s*,?\s*$/i.exec(line);
      if (!match) return [];
      const expression = match[2]!.replace(/^\((.*)\)$/, "$1");
      return [{
        id: crypto.randomUUID(),
        originalName: match[1]?.replace(/``/g, "`") ?? `check_${detail.table.name}`,
        name: match[1]?.replace(/``/g, "`") ?? `check_${detail.table.name}`,
        expression,
        enforced: !match[3],
      }];
    }),
    engine,
    charset,
    collation: /COLLATE=([^\s;]+)/i.exec(detail.ddl)?.[1] ?? "",
    comment: /COMMENT='((?:[^']|'')*)'/i.exec(detail.ddl)?.[1]?.replace(/''/g, "'") ?? "",
    partitionClause: /\n(PARTITION BY[\s\S]+)$/i.exec(detail.ddl)?.[1]?.replace(/;$/, "") ?? "",
  };
}

function unsupportedColumnExtra(extra: string): string {
  return extra
    .replace(/\bauto_increment\b/ig, "")
    .replace(/\bdefault_generated\b/ig, "")
    .replace(/\bon update\s+.+?(?=\s+(?:(?:stored|virtual)\s+generated|invisible|visible|auto_increment)\b|$)/ig, "")
    .replace(/\b(?:stored|virtual)\s+generated\b/ig, "")
    .replace(/\b(?:invisible|visible)\b/ig, "")
    .replace(/\s+/g, " ")
    .trim();
}

function canonicalDefinition(value: string) {
  return value.trim().replace(/,$/, "").replace(/\s+/g, " ").toLocaleUpperCase();
}

function mysqlNamedDefinitionLine(ddl: string, name: string, prefix: RegExp): string | undefined {
  const escapedName = name.replace(/`/g, "``");
  return ddl.split("\n").map((line) => line.trim()).find((line) => {
    if (!prefix.test(line)) return false;
    const match = /KEY\s+`((?:``|[^`])+)`/i.exec(line);
    return match?.[1] === escapedName;
  })?.replace(/,$/, "");
}

function comparableColumn(column: CreateTableColumnDefinition) {
  return { ...column, originalName: undefined, comment: column.comment ?? "", defaultValue: column.defaultValue ?? null };
}

export function alterTableSql(database: string, original: CreateTableDefinition, next: CreateTableDefinition): string {
  const validationError = validateCreateTableDefinition(next);
  if (validationError) throw new Error(validationError);
  const clauses: string[] = [];
  const nextByOriginal = new Map(next.columns.filter((column) => column.originalName).map((column) => [column.originalName!, column]));
  for (const column of original.columns) {
    const replacement = nextByOriginal.get(column.name);
    if (!replacement) clauses.push(`DROP COLUMN ${quoteMysqlIdentifier(column.name)}`);
    else if (replacement.name !== column.name) clauses.push(`CHANGE COLUMN ${quoteMysqlIdentifier(column.name)} ${columnSql(replacement)}`);
    else if (JSON.stringify(comparableColumn(column)) !== JSON.stringify(comparableColumn(replacement))) clauses.push(`MODIFY COLUMN ${columnSql(replacement)}`);
  }
  for (const column of next.columns.filter((column) => !column.originalName)) clauses.push(`ADD COLUMN ${columnSql(column)}`);

  const originalPrimary = original.columns.filter((column) => column.primaryKey).map((column) => column.name);
  const nextPrimary = next.columns.filter((column) => column.primaryKey).map((column) => column.name);
  if (JSON.stringify(originalPrimary) !== JSON.stringify(nextPrimary)) {
    if (originalPrimary.length) clauses.push("DROP PRIMARY KEY");
    const primary = primaryKeySql(next);
    if (primary) clauses.push(`ADD ${primary}`);
  }
  appendObjectChanges(clauses, original.indexes ?? [], next.indexes ?? [],
    (index) => `DROP INDEX ${quoteMysqlIdentifier(index.name)}`,
    (index) => `ADD ${indexSql(index)}`);
  appendObjectChanges(clauses, original.foreignKeys ?? [], next.foreignKeys ?? [],
    (foreignKey) => `DROP FOREIGN KEY ${quoteMysqlIdentifier(foreignKey.name)}`,
    (foreignKey) => `ADD ${foreignKeySql(foreignKey, database)}`);
  appendObjectChanges(clauses, original.checks ?? [], next.checks ?? [],
    (check) => `DROP CHECK ${quoteMysqlIdentifier(check.name)}`,
    (check) => `ADD ${checkSql(check)}`);
  if ((original.engine ?? "InnoDB") !== (next.engine ?? "InnoDB")) clauses.push(`ENGINE=${next.engine || "InnoDB"}`);
  if ((original.charset ?? "utf8mb4") !== (next.charset ?? "utf8mb4")) clauses.push(`DEFAULT CHARACTER SET ${next.charset || "utf8mb4"}`);
  if ((original.collation ?? "") !== (next.collation ?? "")) clauses.push(next.collation ? `COLLATE ${next.collation}` : `DEFAULT CHARACTER SET ${next.charset || "utf8mb4"}`);
  if ((original.comment ?? "") !== (next.comment ?? "")) clauses.push(`COMMENT=${quoteMysqlString(next.comment ?? "")}`);
  if ((original.partitionClause ?? "") !== (next.partitionClause ?? "")) clauses.push(next.partitionClause?.trim() || "REMOVE PARTITIONING");
  if (original.name !== next.name) clauses.push(`RENAME TO ${quoteMysqlIdentifier(database)}.${quoteMysqlIdentifier(next.name)}`);
  return clauses.length ? `ALTER TABLE ${quoteMysqlIdentifier(database)}.${quoteMysqlIdentifier(original.name)}\n  ${clauses.join(",\n  ")};` : "";
}

function comparableNamedObject<T extends { id: string; originalName?: string }>(value: T) {
  const { id: _id, originalName: _originalName, ...comparable } = value;
  return comparable;
}

function appendObjectChanges<T extends { id: string; name: string; originalName?: string }>(
  clauses: string[], original: T[], next: T[], dropSql: (value: T) => string, addSql: (value: T) => string,
) {
  const nextByOriginal = new Map(next.map((value) => [value.originalName ?? value.name, value]));
  for (const value of original) {
    const replacement = nextByOriginal.get(value.originalName ?? value.name);
    if (!replacement) clauses.push(dropSql(value));
    else if (JSON.stringify(comparableNamedObject(value)) !== JSON.stringify(comparableNamedObject(replacement))) {
      clauses.push(dropSql(value), addSql(replacement));
    }
  }
  const originalNames = new Set(original.map((value) => value.originalName ?? value.name));
  for (const value of next) {
    if (!originalNames.has(value.originalName ?? value.name)) clauses.push(addSql(value));
  }
}
