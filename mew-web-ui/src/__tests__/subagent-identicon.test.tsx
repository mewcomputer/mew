import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import {
  SubagentIdenticon,
  SUBAGENT_IDENTICON_PALETTE,
  hashSeedFnv1a,
  identiconCells,
  identiconColor,
} from "../components/subagent-identicon";

afterEach(() => cleanup());

describe("SubagentIdenticon", () => {
  it("produces deterministic FNV-1a colors and horizontally symmetric 5x5 grids", () => {
    const seeds = ["child-session-1", "researcher", "coder-alpha", ""];
    for (const seed of seeds) {
      const cellsA = identiconCells(seed);
      const cellsB = identiconCells(seed);
      expect(cellsA).toEqual(cellsB);
      expect(identiconColor(seed)).toBe(identiconColor(seed));
      expect(SUBAGENT_IDENTICON_PALETTE).toContain(identiconColor(seed));

      expect(cellsA).toHaveLength(5);
      let anyFilled = false;
      for (const row of cellsA) {
        expect(row).toHaveLength(5);
        for (let col = 0; col < 5; col++) {
          expect(row[col]).toBe(row[4 - col]);
          if (row[col]) anyFilled = true;
        }
      }
      expect(anyFilled).toBe(true);
    }

    expect(hashSeedFnv1a("agent-a")).not.toBe(hashSeedFnv1a("agent-b"));
  });

  it("renders an inline crisp SVG identicon for a subagent seed", () => {
    const { container } = render(<SubagentIdenticon seed="child-42" size={20} />);
    const svg = container.querySelector("svg");
    expect(svg).not.toBeNull();
    expect(svg?.getAttribute("width")).toBe("20");
    expect(svg?.getAttribute("height")).toBe("20");
    expect(svg?.querySelectorAll("rect").length).toBeGreaterThan(1);
  });
});
