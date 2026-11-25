import { describe, expect, it } from "vitest";
import { describePythonCalls, parsePythonCalls } from "../lib/python-calls";

describe("parsePythonCalls", () => {
  it("reads the tool and its path-like argument out of the source", () => {
    const source = [
      "result = tools.grep({'pattern': 'desktop-electron', 'case_sensitive': True})",
      "print(result.get('output'))",
      "text(tools.read({'path': 'AGENTS.md'}))",
    ].join("\n");
    expect(parsePythonCalls(source)).toEqual([
      { name: "grep", target: "desktop-electron" },
      { name: "read", target: "AGENTS.md" },
    ]);
  });

  it("falls back to the first quoted argument", () => {
    expect(parsePythonCalls("tools.bash('cargo test')")).toEqual([{ name: "bash", target: "cargo test" }]);
  });

  it("ignores calls that are not tool calls and de-duplicates repeats", () => {
    const source = "tools.read({'path': 'a.txt'})\ntools.read({'path': 'a.txt'})\nprint('tools.nope()')";
    expect(parsePythonCalls(source)).toEqual([{ name: "read", target: "a.txt" }]);
  });

  it("describes at most the first few calls", () => {
    const calls = parsePythonCalls("tools.read({'path': 'a'})\ntools.read({'path': 'b'})\ntools.write({'path': 'c'})\ntools.write({'path': 'd'})");
    expect(describePythonCalls(calls)).toBe("read a, read b, write c");
  });
});
