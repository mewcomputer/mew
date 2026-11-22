import type { HighlighterCore } from "shiki/core";

let highlighter: HighlighterCore | null = null;
let initPromise: Promise<void> | null = null;

async function initHighlighter() {
  if (highlighter) return highlighter;
  if (initPromise) {
    await initPromise;
    return highlighter!;
  }

  initPromise = (async () => {
    const [
      { createHighlighterCore, createCssVariablesTheme },
      { createOnigurumaEngine },
      rust,
      ts,
      js,
      python,
      bash,
      json,
      yaml,
      toml,
      md,
      html,
      css,
      go,
      tsx,
      jsx,
    ] = await Promise.all([
      import("shiki/core"),
      import("shiki/engine/oniguruma"),
      import("@shikijs/langs/rust"),
      import("@shikijs/langs/typescript"),
      import("@shikijs/langs/javascript"),
      import("@shikijs/langs/python"),
      import("@shikijs/langs/bash"),
      import("@shikijs/langs/json"),
      import("@shikijs/langs/yaml"),
      import("@shikijs/langs/toml"),
      import("@shikijs/langs/markdown"),
      import("@shikijs/langs/html"),
      import("@shikijs/langs/css"),
      import("@shikijs/langs/go"),
      import("@shikijs/langs/tsx"),
      import("@shikijs/langs/jsx"),
    ]);

    highlighter = await createHighlighterCore({
      themes: [createCssVariablesTheme({ name: "mew", variablePrefix: "--mew-code-", fontStyle: true })],
      langs: [
        rust.default,
        ts.default,
        js.default,
        python.default,
        bash.default,
        json.default,
        yaml.default,
        toml.default,
        md.default,
        html.default,
        css.default,
        go.default,
        tsx.default,
        jsx.default,
      ],
      engine: createOnigurumaEngine(() => import("shiki/wasm")),
    });
  })();

  await initPromise;
  return highlighter!;
}

export async function highlightCode(
  code: string,
  lang: string,
): Promise<string> {
  const h = await initHighlighter();
  const resolvedLang = h.getLoadedLanguages().includes(lang) ? lang : "text";
  return h.codeToHtml(code, { lang: resolvedLang, theme: "mew" });
}
