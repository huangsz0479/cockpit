import { describe, expect, it } from "vitest";
import { findSqlParameters, renderSqlParameters } from "./sqlParameters";

describe("SQL parameters", () => {
  it("finds unique placeholders outside strings and comments", () => {
    const sql = "SELECT {{ id }}, '{{ignored}}' -- {{comment}}\nWHERE name={{name}} OR parent={{id}}";
    expect(findSqlParameters(sql)).toEqual(["id", "name"]);
  });

  it("ignores placeholders in PostgreSQL dollar-quoted bodies", () => {
    expect(findSqlParameters("DO $$ BEGIN RAISE NOTICE '{{ignored}}'; END $$; SELECT {{real}};")).toEqual(["real"]);
  });

  it("renders typed values safely", () => {
    expect(renderSqlParameters("SELECT {{text}}, {{number}}, {{nil}}, {{raw}}", {
      text: { mode: "text", value: "O'Reilly" },
      number: { mode: "number", value: "-1.25e2" },
      nil: { mode: "null", value: "" },
      raw: { mode: "raw", value: "CURRENT_TIMESTAMP" },
    })).toBe("SELECT 'O''Reilly', -1.25e2, NULL, CURRENT_TIMESTAMP");
  });

  it("rejects invalid numeric and empty raw values", () => {
    expect(() => renderSqlParameters("SELECT {{value}}", { value: { mode: "number", value: "1; DROP TABLE t" } })).toThrow("不是有效数值");
    expect(() => renderSqlParameters("SELECT {{value}}", { value: { mode: "raw", value: " " } })).toThrow("原始 SQL 参数不能为空");
  });
});
