import { describe, expect, it } from "vitest";
import { parseRowCell } from "./rowEditing";
import type { ColumnMeta } from "@/types";

function column(databaseType: string): ColumnMeta {
  return { name: "started_at", databaseType, nullable: false, unsigned: false, binary: false };
}

describe("parseRowCell TIME validation", () => {
  it("requires a time before writing an empty cell", () => {
    expect(() => parseRowCell(column("time"), undefined, { text: "", isNull: false }))
      .toThrow("started_at 需要填写时间");
    expect(() => parseRowCell(column("time"), undefined, { text: "   ", isNull: false }))
      .toThrow("started_at 需要填写时间");
  });

  it("rejects values that are not HH:MM or HH:MM:SS", () => {
    expect(() => parseRowCell(column("time"), undefined, { text: "abc", isNull: false }))
      .toThrow("started_at 时间格式无效，应为 HH:MM 或 HH:MM:SS");
    expect(() => parseRowCell(column("time"), undefined, { text: "1230", isNull: false }))
      .toThrow("started_at 时间格式无效，应为 HH:MM 或 HH:MM:SS");
    expect(() => parseRowCell(column("time"), undefined, { text: "12:3", isNull: false }))
      .toThrow("started_at 时间格式无效，应为 HH:MM 或 HH:MM:SS");
  });

  it("accepts plain and fractional seconds", () => {
    expect(parseRowCell(column("time"), undefined, { text: "9:05", isNull: false }))
      .toEqual({ kind: "time", value: "9:05" });
    expect(parseRowCell(column("time"), undefined, { text: "09:05:07", isNull: false }))
      .toEqual({ kind: "time", value: "09:05:07" });
    expect(parseRowCell(column("time"), undefined, { text: "09:05:07.125", isNull: false }))
      .toEqual({ kind: "time", value: "09:05:07.125" });
  });
});

describe("parseRowCell timestamp routing", () => {
  it("keeps timestamp columns on the datetime branch instead of the time branch", () => {
    expect(() => parseRowCell(column("timestamp"), undefined, { text: "", isNull: false }))
      .toThrow("started_at 需要选择日期和时间");
    expect(parseRowCell(column("timestamp"), undefined, { text: "2026-08-27 10:30:00", isNull: false }))
      .toEqual({ kind: "date_time", value: "2026-08-27 10:30:00" });
    expect(parseRowCell(column("datetime"), undefined, { text: "2026-08-27 10:30:00", isNull: false }))
      .toEqual({ kind: "date_time", value: "2026-08-27 10:30:00" });
  });

  it("keeps date columns on the date branch", () => {
    expect(() => parseRowCell(column("date"), undefined, { text: "", isNull: false }))
      .toThrow("started_at 需要选择日期");
    expect(parseRowCell(column("date"), undefined, { text: "2026-08-27", isNull: false }))
      .toEqual({ kind: "date", value: "2026-08-27" });
  });
});
