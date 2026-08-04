import { describe, expect, it } from "vitest";
import { contextPercent, formatTokens } from "../lib/format";

describe("formatTokens", () => {
  it("renders raw counts under a thousand", () => {
    expect(formatTokens(0)).toBe("0");
    expect(formatTokens(999)).toBe("999");
  });

  it("renders thousands with one decimal", () => {
    expect(formatTokens(1_000)).toBe("1.0k");
    expect(formatTokens(12_345)).toBe("12.3k");
  });

  it("renders millions with two decimals", () => {
    expect(formatTokens(1_000_000)).toBe("1.00M");
    expect(formatTokens(2_500_000)).toBe("2.50M");
  });
});

describe("contextPercent", () => {
  it("computes the rounded percentage of the window in use", () => {
    expect(contextPercent(50_000, 200_000)).toBe(25);
    expect(contextPercent(1, 3)).toBe(33);
    expect(contextPercent(2, 3)).toBe(67);
  });

  it("returns 100 when the context fills the window", () => {
    expect(contextPercent(200_000, 200_000)).toBe(100);
  });

  it("preserves values above 100 for an over-window context", () => {
    expect(contextPercent(300_000, 200_000)).toBe(150);
  });

  it("returns null when the window is unknown or invalid", () => {
    expect(contextPercent(1_000, 0)).toBeNull();
    expect(contextPercent(1_000, -1)).toBeNull();
    expect(contextPercent(1_000, Number.NaN)).toBeNull();
  });

  it("treats negative usage as zero", () => {
    expect(contextPercent(-5, 200_000)).toBe(0);
  });
});
