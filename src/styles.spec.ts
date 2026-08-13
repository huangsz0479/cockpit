import { describe, expect, it } from "vitest";
import styles from "./styles.css?raw";

describe("data grid selection styles", () => {
  it("keeps a selected column above the alternating row background", () => {
    const alternateRowSelector = ".data-grid tbody tr.data-row.alternate-row td";
    const selectedColumnSelector = ".data-grid tbody tr.data-row td.selected-column";

    expect(styles).toContain(alternateRowSelector);
    expect(styles.indexOf(selectedColumnSelector)).toBeGreaterThan(styles.indexOf(alternateRowSelector));
  });

  it("does not include a dark color scheme", () => {
    expect(styles).not.toContain('data-theme="dark"');
    expect(styles).not.toContain("prefers-color-scheme: dark");
    expect(styles).not.toContain("color-scheme: dark");
  });
});
