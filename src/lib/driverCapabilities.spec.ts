import { describe, expect, it } from "vitest";
import { driverSupportsObjectGroup, driverTableGroupLabel, isValidElasticsearchIndexName } from "./driverCapabilities";

const groups = ["view", "routine", "trigger", "event"] as const;

describe("driverSupportsObjectGroup", () => {
  it("enables every object group for MySQL and MariaDB", () => {
    for (const kind of ["mysql", "mariadb"] as const) {
      for (const group of groups) expect(driverSupportsObjectGroup(kind, group)).toBe(true);
    }
  });

  it("omits scheduled events for PostgreSQL", () => {
    expect(driverSupportsObjectGroup("postgresql", "view")).toBe(true);
    expect(driverSupportsObjectGroup("postgresql", "routine")).toBe(true);
    expect(driverSupportsObjectGroup("postgresql", "trigger")).toBe(true);
    expect(driverSupportsObjectGroup("postgresql", "event")).toBe(false);
  });

  it("omits routines and events for SQLite", () => {
    expect(driverSupportsObjectGroup("sqlite", "view")).toBe(true);
    expect(driverSupportsObjectGroup("sqlite", "trigger")).toBe(true);
    expect(driverSupportsObjectGroup("sqlite", "routine")).toBe(false);
    expect(driverSupportsObjectGroup("sqlite", "event")).toBe(false);
  });

  it("disables every metadata object group for Elasticsearch", () => {
    for (const group of groups) expect(driverSupportsObjectGroup("elasticsearch", group)).toBe(false);
  });

  it("falls back to the MySQL capability set for unknown kinds", () => {
    for (const group of groups) expect(driverSupportsObjectGroup(undefined, group)).toBe(true);
  });

  it("calls the table group an index for Elasticsearch and a table elsewhere", () => {
    expect(driverTableGroupLabel("elasticsearch")).toBe("索引");
    for (const kind of ["mysql", "mariadb", "postgresql", "sqlite", undefined] as const) {
      expect(driverTableGroupLabel(kind)).toBe("表");
    }
  });
});

describe("isValidElasticsearchIndexName", () => {
  it("accepts lowercase names with digits, dots, hyphens and underscores", () => {
    for (const name of ["orders", "logs-2026.08", "cockpit_it_abc123", ".hidden"]) {
      expect(isValidElasticsearchIndexName(name)).toBe(true);
    }
  });

  it("rejects names that would break URL paths or ES rules", () => {
    for (const name of ["", "-lead", "_lead", "+lead", ".", "..", "Orders", "a/b", "a?x", "a b", "中文"]) {
      expect(isValidElasticsearchIndexName(name)).toBe(false);
    }
  });
});
