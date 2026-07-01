import { defineConfig } from "astro/config";
import react from "@astrojs/react";
import tailwindcss from "@tailwindcss/vite";
import starlight from "@astrojs/starlight";
import starlightDotMd from "starlight-dot-md";
import starlightSidebarTopics from "starlight-sidebar-topics";
import lucode from "lucode-starlight";

// Starlight docs live at /docs. The landing page stays at /.
// For this to happen, we have to put all docs in content/docs/docs as content/docs itself is the root for starlight.
export default defineConfig({
  site: "https://mew.computer",
  integrations: [
    starlight({
      title: "mew",
      logo: {
        src: "./src/assets/mew-logo.svg",
        replacesTitle: false,
      },
      customCss: [
        // Relative path to your @font-face CSS file.
        "./src/styles/banga.css",
        "./src/styles/ioskeley.css",
        "./src/styles/misans.css",
        "./src/styles/starlight.css",
      ],
      social: [
        {
          label: "GitHub",
          href: "https://github.com/mewcomputer/mew",
          icon: "github",
        },
      ],
      components: {
        Sidebar: "./src/components/overrides/Sidebar.astro",
        PageFrame: "./src/components/overrides/PageFrame.astro",
      },
      plugins: [
        starlightDotMd({ includeFrontmatter: false }),
        lucode(),
        starlightSidebarTopics(
          [
            {
              label: "Docs",
              link: "/docs/getting-started/installation/",
              icon: "open-book",
              items: [
                {
                  label: "Getting Started",
                  items: [{ autogenerate: { directory: "docs/getting-started" } }],
                },
                {
                  label: "Using mew",
                  items: [{ autogenerate: { directory: "docs/using-mew" } }],
                },
              ],
            },
            {
              id: "development",
              label: "Development",
              link: "/docs/development/dev-architecture/",
              icon: "laptop",
              items: [
                {
                  label: "Development",
                  items: [{ autogenerate: { directory: "docs/development" } }],
                },
              ],
            },
          ],
        ),
      ],
    }),
    react(),
  ],
  vite: {
    plugins: [tailwindcss()],
  },
});
