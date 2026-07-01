import { defineConfig } from "astro/config";
import react from "@astrojs/react";
import tailwindcss from "@tailwindcss/vite";
import starlight from "@astrojs/starlight";
import starlightDotMd from "starlight-dot-md";
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
      plugins: [starlightDotMd({ includeFrontmatter: false }), lucode()],
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
      sidebar: [
        {
          label: "Getting Started",
          items: [
            { label: "Installation", slug: "docs/installation" },
            { label: "Quick Start", slug: "docs/quick-start" },
            { label: "Configuration", slug: "docs/configuration" },
            { label: "Context Files", slug: "docs/context-files" },
            { label: "Sessions", slug: "docs/sessions" },
          ],
        },
        {
          label: "Using mew",
          items: [
            { label: "Slash Commands", slug: "docs/slash-commands" },
            { label: "Keyboard Shortcuts", slug: "docs/keyboard-shortcuts" },
            { label: "Tips & Tricks", slug: "docs/tips-and-tricks" },
            { label: "Providers", slug: "docs/providers" },
            { label: "Permissions", slug: "docs/permissions" },
            { label: "Tools", slug: "docs/tools" },
            { label: "Hashline Edits", slug: "docs/hashline" },
            { label: "Personas", slug: "docs/personas" },
            { label: "Skills", slug: "docs/skills" },
            { label: "Subagents", slug: "docs/subagents" },
            { label: "Plugins", slug: "docs/plugins" },
            { label: "MCP Servers", slug: "docs/mcp-servers" },
            { label: "Web UI", slug: "docs/web-ui" },
          ],
        },
        {
          label: "Development",
          items: [
            { label: "Architecture", slug: "docs/dev-architecture" },
            { label: "Hashline Internals", slug: "docs/dev-hashline" },
            { label: "Adding a Provider", slug: "docs/dev-providers" },
            { label: "Adding a Tool", slug: "docs/dev-tools" },
            { label: "Daemon Protocol", slug: "docs/dev-protocol" },
            { label: "Testing", slug: "docs/dev-testing" },
            { label: "Web UI Development", slug: "docs/dev-web" },
          ],
        },
      ],
    }),
    react(),
  ],
  vite: {
    plugins: [tailwindcss()],
  },
});
