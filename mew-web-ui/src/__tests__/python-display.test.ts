import { describe, expect, it } from "vitest";
import { formatPythonForDisplay } from "../lib/python-display";

describe("Python display indentation", () => {
  it("uses two spaces per level without changing multiline strings", () => {
    const source = 'if ready:\n    text = """first\n    keep this indentation\n    """\n    if text:\n        print(text)';
    expect(formatPythonForDisplay(source)).toBe('if ready:\n  text = """first\n    keep this indentation\n    """\n  if text:\n    print(text)');
  });

  it("keeps source unchanged when indentation or syntax is incomplete", () => {
    for (const source of ['if x:\n\tprint(x)', 'if x:\n    print(']) {
      expect(formatPythonForDisplay(source)).toBe(source);
    }
  });

  it("preserves indentation inside multiline formatted strings", () => {
    const source = 'if ready:\n    s = f"""hello\n    {name}\n    more"""\n    print(s)';
    expect(formatPythonForDisplay(source)).toBe('if ready:\n  s = f"""hello\n    {name}\n    more"""\n  print(s)');
  });
});
