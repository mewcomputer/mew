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
      social: [
        {
          label: "GitHub",
          href: "https://github.com/mewcomputer/mew",
          icon: "github",
        },
      ],
      components: {
        SiteTitle: "./src/components/overrides/SiteTitle.astro",
        PageTitle: "./src/components/overrides/PageTitle.astro",
      },
      sidebar: [
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
