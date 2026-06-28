import { createHighlighterCore } from "shiki/core";
import { createOnigurumaEngine } from "shiki/engine/oniguruma";
import type { HighlighterCore } from "shiki/core";
import type { ResolvedTheme } from "./theme";

const DARK_THEME = "github-dark";
const LIGHT_THEME = "github-light";

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
      githubDark,
      githubLight,
    ] = await Promise.all([
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
      import("shiki/themes/github-dark.mjs"),
      import("shiki/themes/github-light.mjs"),
    ]);

    highlighter = await createHighlighterCore({
      themes: [githubDark.default, githubLight.default],
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
  theme: ResolvedTheme = "dark",
): Promise<string> {
  const h = await initHighlighter();
  const resolvedLang = h.getLoadedLanguages().includes(lang) ? lang : "text";
  const themeName = theme === "dark" ? DARK_THEME : LIGHT_THEME;
  return h.codeToHtml(code, { lang: resolvedLang, theme: themeName });
}
