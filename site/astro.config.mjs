import { defineConfig } from "astro/config";
import react from "@astrojs/react";
import tailwindcss from "@tailwindcss/vite";
import starlight from "@astrojs/starlight";
import starlightDotMd from "starlight-dot-md";

// Starlight docs live at /docs. The landing page stays at /.
// For this to happen, we have to put all docs in content/docs/docs as content/docs itself is the root for starlight.
export default defineConfig({
  site: "https://mew.computer",
  integrations: [
    starlight({
      title: "mew",
      customCss: ["./src/styles/starlight.css"],
      head: [{
        tag: "link",
        attrs: { rel: "stylesheet", href: "https://fonts.googleapis.com/css2?family=Noto+Serif+SC:wght@600&text=%E5%96%B5&display=swap" },
      }],
      social: [
        {
          label: "GitHub",
          href: "https://github.com/mewcomputer/mew",
          icon: "github",
        },
      ],
      components: {
        Header: "./src/components/overrides/Header.astro",
        MarkdownContent: "./src/components/overrides/MarkdownContent.astro",
        PageTitle: "./src/components/overrides/PageTitle.astro",
      },
      sidebar: [
        { label: "Overview", link: "/docs/" },
        {
          label: "Getting started",
          items: [
            "docs/getting-started/installation",
            "docs/getting-started/quick-start",
            "docs/getting-started/configuration",
            "docs/getting-started/context-files",
            "docs/getting-started/keyboard-shortcuts",
            "docs/getting-started/sessions",
          ],
        },
        { label: "Using mew", collapsed: true, items: [{ autogenerate: { directory: "docs/using-mew" } }] },
        { label: "Development", collapsed: true, items: [{ autogenerate: { directory: "docs/development" } }] },
      ],
      plugins: [starlightDotMd({ includeFrontmatter: false })],
    }),
    react(),
  ],
  vite: {
    plugins: [tailwindcss()],
  },
});
