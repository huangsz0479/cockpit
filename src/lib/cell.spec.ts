import { describe, expect, it } from "vitest";
import { cellText, cellToJsValue } from "./cell";

describe("cellText", () => {
  it("preserves large integers as text", () => {
    expect(cellText({ kind: "unsigned", value: "18446744073709551615" })).toBe("18446744073709551615");
  });

  it("renders null distinctly", () => {
    expect(cellText({ kind: "null" })).toBe("NULL");
  });
});

describe("cellToJsValue", () => {
  it("keeps booleans and floats typed", () => {
    expect(cellToJsValue({ kind: "bool", value: true })).toBe(true);
    expect(cellToJsValue({ kind: "float", value: 1.5 })).toBe(1.5);
  });

  it("keeps integer strings to avoid precision loss", () => {
    expect(cellToJsValue({ kind: "signed", value: "9007199254740993" })).toBe("9007199254740993");
  });

  it("parses json cells back into objects", () => {
    expect(cellToJsValue({ kind: "json", value: "{\"name\":\"ann\"}" })).toEqual({ name: "ann" });
    expect(cellToJsValue({ kind: "json", value: "not-json" })).toBe("not-json");
  });

  it("maps null, bytes and geometry to plain values", () => {
    expect(cellToJsValue({ kind: "null" })).toBeNull();
    expect(cellToJsValue(undefined)).toBeNull();
    expect(cellToJsValue({ kind: "bytes", value: { base64: "AAA=", preview: "ab", length: 3 } })).toBe("ab");
    expect(cellToJsValue({ kind: "bytes", value: { base64: "AAA=", length: 3 } })).toBe("AAA=");
    expect(cellToJsValue({ kind: "geometry", value: { wkbBase64: "AAA=", srid: 4326 } })).toEqual({
      srid: 4326,
      wkbBase64: "AAA=",
    });
  });
});

