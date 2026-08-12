import { describe, expect, it } from "vitest";
import { cellText } from "./cell";

describe("cellText", () => {
  it("preserves large integers as text", () => {
    expect(cellText({ kind: "unsigned", value: "18446744073709551615" })).toBe("18446744073709551615");
  });

  it("renders null distinctly", () => {
    expect(cellText({ kind: "null" })).toBe("NULL");
  });
});

