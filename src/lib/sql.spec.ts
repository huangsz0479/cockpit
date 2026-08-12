import { describe, expect, it } from "vitest";
import { alterTableSql, canAppendSelectQueryLimit, canPageSelectQuery, createDefaultTableDefinition, createTableSql, quoteIdentifier, quoteMysqlIdentifier, selectPreviewSql, selectQueryPageSql, selectTablePageSql, singleTableSelectAllTarget, tableDetailToDefinition, validateCreateTableDefinition } from "./sql";

describe("MySQL identifier SQL helpers", () => {
  it("quotes identifiers and escapes embedded backticks", () => {
    expect(quoteMysqlIdentifier("odd`name")).toBe("`odd``name`");
  });

  it("builds a bounded preview query with qualified names", () => {
    expect(selectPreviewSql("demo-db", "order items")).toBe(
      "SELECT *\nFROM `demo-db`.`order items`\nLIMIT 100;",
    );
  });

  it("builds filtered and sorted table pages with escaped identifiers", () => {
    expect(selectTablePageSql("demo", "order`items", 100, 200, "WHERE status = 1;", "created`at", "desc")).toBe(
      "SELECT *\nFROM `demo`.`order``items`\nWHERE status = 1\nORDER BY `created``at` DESC\nLIMIT 101 OFFSET 200;",
    );
  });

  it("appends a top-level limit for database-side query pagination", () => {
    expect(selectQueryPageSql("SELECT id, title FROM posts;", 50, 100)).toBe(
      "SELECT id, title FROM posts\nLIMIT 51 OFFSET 100;",
    );
  });

  it("uses database-specific identifier quotes outside query pagination", () => {
    expect(quoteIdentifier('odd"name', "postgresql")).toBe('"odd""name"');
    expect(selectPreviewSql("public", "order items", "postgresql")).toContain('FROM "public"."order items"');
    expect(selectQueryPageSql("SELECT 1", 10, 0, "sqlite")).toBe("SELECT 1\nLIMIT 11 OFFSET 0;");
  });

  it("only paginates SELECT statements that can accept a top-level LIMIT", () => {
    expect(canPageSelectQuery("SELECT a.id, b.id FROM a JOIN b ON b.id = a.id ORDER BY a.id")).toBe(true);
    expect(canPageSelectQuery("SELECT * FROM users WHERE id IN (SELECT user_id FROM audit LIMIT 1)")).toBe(true);
    expect(canPageSelectQuery("SELECT * FROM users LIMIT 10")).toBe(true);
    expect(canAppendSelectQueryLimit("SELECT * FROM users LIMIT 10")).toBe(false);
    expect(canAppendSelectQueryLimit("SELECT * FROM users ORDER BY id")).toBe(true);
    expect(canPageSelectQuery("SELECT * FROM users FOR UPDATE")).toBe(false);
    expect(canPageSelectQuery("SELECT * INTO OUTFILE '/tmp/users' FROM users")).toBe(false);
  });

  it("recognizes editable SELECT * queries against one table", () => {
    expect(singleTableSelectAllTarget("SELECT * FROM users WHERE active = 1 ORDER BY id", "demo")).toEqual({
      database: "demo",
      table: "users",
    });
    expect(singleTableSelectAllTarget("SELECT * FROM `audit-db`.`entry log` AS e LIMIT 20", "demo")).toEqual({
      database: "audit-db",
      table: "entry log",
    });
    expect(singleTableSelectAllTarget('SELECT * FROM "public"."users" u OFFSET 10', "other")).toEqual({
      database: "public",
      table: "users",
    });
  });

  it("rejects query results that cannot safely map to one complete table row", () => {
    expect(singleTableSelectAllTarget("SELECT id, name FROM users", "demo")).toBeNull();
    expect(singleTableSelectAllTarget("SELECT users.* FROM users", "demo")).toBeNull();
    expect(singleTableSelectAllTarget("SELECT * FROM users JOIN teams ON teams.id = users.team_id", "demo")).toBeNull();
    expect(singleTableSelectAllTarget("SELECT * FROM users, teams", "demo")).toBeNull();
    expect(singleTableSelectAllTarget("SELECT * FROM users GROUP BY team_id", "demo")).toBeNull();
    expect(singleTableSelectAllTarget("SELECT * FROM users WHERE active = 1 GROUP BY team_id", "demo")).toBeNull();
    expect(singleTableSelectAllTarget("SELECT * FROM users WHERE team_id IN (SELECT id FROM teams)", "demo")).toBeNull();
    expect(singleTableSelectAllTarget("SELECT * FROM users; SELECT * FROM teams", "demo")).toBeNull();
  });

  it("builds an editable create-table template for the selected database", () => {
    expect(createTableSql("demo`db", {
      name: "order items",
      columns: [
        { name: "id", dataType: "BIGINT", size: "", unsigned: true, nullable: false, primaryKey: true, autoIncrement: true },
        { name: "title", dataType: "VARCHAR", size: "120", unsigned: false, nullable: false, primaryKey: false, autoIncrement: false },
        { name: "amount", dataType: "DECIMAL", size: "10,2", unsigned: true, nullable: true, primaryKey: false, autoIncrement: false },
      ],
    })).toBe(
      "CREATE TABLE `demo``db`.`order items` (\n  `id` BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,\n  `title` VARCHAR(120) NOT NULL,\n  `amount` DECIMAL(10,2) UNSIGNED NULL,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;",
    );
  });

  it("builds generated columns, advanced indexes and table options", () => {
    const sql = createTableSql("demo", {
      name: "places",
      columns: [
        { name: "id", dataType: "BIGINT", size: "", unsigned: true, nullable: false, primaryKey: true, autoIncrement: true },
        { name: "title", dataType: "VARCHAR", size: "120", unsigned: false, nullable: false, primaryKey: false, autoIncrement: false, collation: "utf8mb4_bin" },
        { name: "point", dataType: "POINT", size: "", unsigned: false, nullable: false, primaryKey: false, autoIncrement: false },
        { name: "title_length", dataType: "INT", size: "", unsigned: true, nullable: false, primaryKey: false, autoIncrement: false, generatedExpression: "char_length(`title`)", generatedStored: true },
      ],
      indexes: [{ id: "spatial", name: "idx_point", columns: ["point"], unique: false, indexType: "SPATIAL" }],
      checks: [{ id: "positive", name: "chk_id", expression: "id > 0", enforced: true }],
      engine: "InnoDB",
      charset: "utf8mb4",
      collation: "utf8mb4_0900_ai_ci",
      comment: "location data",
      partitionClause: "PARTITION BY HASH(id) PARTITIONS 4",
    });
    expect(sql).toContain("GENERATED ALWAYS AS (char_length(`title`)) STORED");
    expect(sql).toContain("SPATIAL KEY `idx_point` (`point`)");
    expect(sql).toContain("CONSTRAINT `chk_id` CHECK (id > 0)");
    expect(sql).toContain("COLLATE=utf8mb4_0900_ai_ci COMMENT='location data'");
    expect(sql).toContain("PARTITION BY HASH(id) PARTITIONS 4");
  });

  it("does not emit empty precision and preserves MySQL column attributes", () => {
    const definition = tableDetailToDefinition({
      table: { database: "demo", name: "events", tableType: "BASE TABLE" },
      columns: [{
        name: "updated_at", ordinal: 1, dataType: "datetime", fullType: "datetime", nullable: false,
        defaultValue: "CURRENT_TIMESTAMP", extra: "DEFAULT_GENERATED on update CURRENT_TIMESTAMP INVISIBLE",
        comment: null, key: null, generationExpression: null, collation: null,
      }],
      indexes: [], foreignKeys: [],
      ddl: "CREATE TABLE `events` (`updated_at` datetime NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP INVISIBLE) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
    });
    const sql = createTableSql("demo", definition);
    expect(sql).toContain("`updated_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP INVISIBLE");
    expect(sql).not.toContain("DATETIME()");
  });

  it("preserves empty-string and bit defaults", () => {
    const sql = createTableSql("demo", {
      name: "flags",
      columns: [
        { name: "label", dataType: "VARCHAR", size: "20", unsigned: false, nullable: false, primaryKey: false, autoIncrement: false, defaultValue: "" },
        { name: "enabled", dataType: "BIT", size: "1", unsigned: false, nullable: false, primaryKey: false, autoIncrement: false, defaultValue: "b'0'" },
      ],
    });
    expect(sql).toContain("`label` VARCHAR(20) NOT NULL DEFAULT ''");
    expect(sql).toContain("`enabled` BIT(1) NOT NULL DEFAULT b'0'");
  });

  it("only rebuilds the changed secondary index during ALTER TABLE", () => {
    const base = {
      name: "items", originalName: "items", engine: "InnoDB", charset: "utf8mb4",
      columns: [
        { name: "id", originalName: "id", dataType: "BIGINT" as const, size: "", unsigned: true, nullable: false, primaryKey: true, autoIncrement: true },
        { name: "title", originalName: "title", dataType: "VARCHAR" as const, size: "100", unsigned: false, nullable: false, primaryKey: false, autoIncrement: false },
      ],
      indexes: [
        { id: "title", originalName: "idx_title", name: "idx_title", columns: ["title"], unique: false },
        { id: "id-title", originalName: "idx_id_title", name: "idx_id_title", columns: ["id", "title"], unique: false },
      ],
      foreignKeys: [], checks: [],
    };
    const sql = alterTableSql("demo", base, {
      ...base,
      indexes: [base.indexes[0]!, { ...base.indexes[1]!, unique: true }],
    });
    expect(sql).not.toContain("DROP INDEX `idx_title`");
    expect(sql).toContain("DROP INDEX `idx_id_title`");
    expect(sql).toContain("ADD UNIQUE KEY `idx_id_title`");
  });

  it("preserves advanced index definitions during unrelated visual changes", () => {
    const original = tableDetailToDefinition({
      table: { database: "demo", name: "articles", tableType: "BASE TABLE" },
      columns: [
        { name: "id", ordinal: 1, dataType: "bigint", fullType: "bigint unsigned", nullable: false, defaultValue: null, extra: "auto_increment", key: "PRI" },
        { name: "title", ordinal: 2, dataType: "varchar", fullType: "varchar(100)", nullable: false, defaultValue: null, extra: "", key: "MUL", collation: "utf8mb4_bin" },
      ],
      indexes: [
        { name: "PRIMARY", columns: ["id"], unique: true, primary: true, indexType: "BTREE" },
        { name: "idx_title", columns: ["title"], unique: false, primary: false, indexType: "BTREE" },
      ],
      foreignKeys: [],
      ddl: "CREATE TABLE `articles` (\n  `id` bigint unsigned NOT NULL AUTO_INCREMENT,\n  `title` varchar(100) COLLATE utf8mb4_bin NOT NULL,\n  PRIMARY KEY (`id`),\n  KEY `idx_title` (`title`(12) DESC) INVISIBLE\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
    });
    expect(original.indexes?.[0]?.preserveRaw).toBe(true);
    const next = structuredClone(original);
    next.comment = "edited safely";
    const sql = alterTableSql("demo", original, next);
    expect(sql).toContain("COMMENT='edited safely'");
    expect(sql).not.toContain("idx_title");
  });
});

describe("SQLite create-table helpers", () => {
  it("builds a SQLite default definition and dialect-specific DDL", () => {
    const definition = createDefaultTableDefinition("sqlite");
    expect(definition.columns[0]).toEqual(expect.objectContaining({
      name: "id",
      dataType: "INTEGER",
      size: "",
      unsigned: false,
      nullable: false,
      primaryKey: true,
      autoIncrement: true,
    }));
    expect(definition.engine).toBeUndefined();
    expect(definition.charset).toBeUndefined();

    definition.name = 'audit"log';
    definition.columns.push({
      name: "payload",
      dataType: "TEXT",
      size: "",
      unsigned: false,
      nullable: false,
      primaryKey: false,
      autoIncrement: false,
      defaultValue: "queued",
    });

    expect(validateCreateTableDefinition(definition, "sqlite")).toBeNull();
    const sql = createTableSql("main", definition, "sqlite");
    expect(sql).toBe(
      "CREATE TABLE \"main\".\"audit\"\"log\" (\n  \"id\" INTEGER PRIMARY KEY AUTOINCREMENT,\n  \"payload\" TEXT NOT NULL DEFAULT 'queued'\n);",
    );
    expect(sql).not.toMatch(/ENGINE|CHARSET|UNSIGNED|AUTO_INCREMENT/);
  });

  it("rejects MySQL-only field options for SQLite", () => {
    const definition = createDefaultTableDefinition("sqlite");
    definition.name = "events";
    definition.columns[0]!.dataType = "BIGINT";
    expect(validateCreateTableDefinition(definition, "sqlite")).toContain("数据类型无效");

    definition.columns[0]!.dataType = "INTEGER";
    definition.columns[0]!.unsigned = true;
    expect(validateCreateTableDefinition(definition, "sqlite")).toContain("不支持无符号");
  });
});
