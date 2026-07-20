import { describe, expect, it } from "vitest";
import { joinPath, parentPath } from "../components/file-tree";

describe("file tree paths", () => {
  it("returns the protocol root instead of an absolute slash", () => {
    expect(parentPath("src")).toBeUndefined();
    expect(parentPath("src/components")).toBe("src");
    expect(parentPath("")).toBeUndefined();
  });

  it("joins relative paths without creating an absolute path", () => {
    expect(joinPath(null, "src")).toBe("src");
    expect(joinPath("", "src")).toBe("src");
    expect(joinPath("src/", "main.rs")).toBe("src/main.rs");
  });
});
