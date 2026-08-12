import { describe, expect, it } from "vitest";
import styles from "./styles.css?raw";

describe("data grid selection styles", () => {
  it("keeps a selected column above the alternating row background", () => {
    const alternateRowSelector = ".data-grid tbody tr.data-row.alternate-row td";
    const selectedColumnSelector = ".data-grid tbody tr.data-row td.selected-column";

    expect(styles).toContain(alternateRowSelector);
    expect(styles.indexOf(selectedColumnSelector)).toBeGreaterThan(styles.indexOf(alternateRowSelector));
  });
});
