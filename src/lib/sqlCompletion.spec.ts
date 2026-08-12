import { describe, expect, it } from "vitest";
import { isSqlTableNamePosition, sqlCompletionQualifier, sqlTableReferences } from "./sqlCompletion";

describe("SQL completion context", () => {
  it("finds the table that provides unqualified WHERE fields", () => {
    expect(sqlTableReferences("SELECT * FROM system_dept WHERE id = 1")).toEqual([
      { table: "system_dept", alias: undefined, database: undefined, depth: 0 },
    ]);
  });

  it("understands qualified names, quoted identifiers, and aliases", () => {
    expect(sqlTableReferences("SELECT * FROM `demo-db`.`system dept` AS d WHERE d.id = 1")).toEqual([
      { table: "system dept", alias: "d", database: "demo-db", depth: 0 },
    ]);
  });

  it("collects joined tables and ignores table-like text in strings and comments", () => {
    const sql = "SELECT 'FROM fake' FROM users u JOIN teams t ON t.id = u.team_id -- JOIN ignored\nWHERE u.id = 1";
    expect(sqlTableReferences(sql).map(({ table, alias }) => ({ table, alias }))).toEqual([
      { table: "users", alias: "u" },
      { table: "teams", alias: "t" },
    ]);
  });

  it("uses only the statement containing the cursor", () => {
    const sql = "SELECT * FROM users WHERE id = 1; SELECT * FROM system_dept WHERE na";
    expect(sqlTableReferences(sql).map((reference) => reference.table)).toEqual(["system_dept"]);
  });

  it("does not leak an inner subquery table into the outer scope", () => {
    const sql = "SELECT * FROM users WHERE id IN (SELECT user_id FROM audit) AND na";
    expect(sqlTableReferences(sql).map((reference) => reference.table)).toEqual(["users"]);
  });

  it("detects qualifiers and table-name positions", () => {
    const qualified = "SELECT * FROM system_dept d WHERE d.na";
    const tablePosition = "SELECT * FROM system_de";
    const fieldPosition = "SELECT * FROM system_dept WHERE id";
    expect(sqlCompletionQualifier(qualified, qualified.indexOf("na"))).toBe("d");
    expect(isSqlTableNamePosition(tablePosition, tablePosition.length)).toBe(true);
    expect(isSqlTableNamePosition(fieldPosition, fieldPosition.length)).toBe(false);
  });
});
