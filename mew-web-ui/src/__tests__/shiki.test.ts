import { describe, expect, it } from "vitest";
import { highlightCode } from "../lib/shiki";

describe("code theme", () => {
  it("uses shared semantic syntax tokens and escapes source markup", async () => {
    const html = await highlightCode('const value = "<script>"; // example', "javascript");
    expect(html).toContain("var(--mew-code-token-keyword)");
    expect(html).toMatch(/var\(--mew-code-token-string(?:-expression)?\)/);
    expect(html).toContain("var(--mew-code-token-comment)");
    expect(html).not.toContain("<script>");
    expect(html).toContain("&#x3C;script>");
    expect(html).not.toContain("#E1E4E8");
  });
});
