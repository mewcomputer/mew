import { defineConfig } from "astro/config";
import react from "@astrojs/react";
import tailwindcss from "@tailwindcss/vite";
import starlight from "@astrojs/starlight";

// Starlight docs live at /docs. The landing page stays at /.
// Starlight uses its own routing under /docs and doesn't interfere
// with the existing pages.
export default defineConfig({
  site: "https://mew.sh",
  integrations: [
    starlight({
      title: "mew",
      logo: {
        src: "./src/assets/mew-logo.svg",
        replacesTitle: false,
      },
      social: [
        { label: "GitHub", href: "https://github.com/natalie/mew", icon: "github" },
      ],
      sidebar: [
        {
          label: "Getting Started",
          items: [
            { label: "Installation", slug: "installation" },
            { label: "Quick Start", slug: "quick-start" },
            { label: "Configuration", slug: "configuration" },
          ],
        },
        {
          label: "Using mew",
          items: [
            { label: "Slash Commands", slug: "slash-commands" },
            { label: "Keyboard Shortcuts", slug: "keyboard-shortcuts" },
            { label: "Providers", slug: "providers" },
            { label: "Personas", slug: "personas" },
            { label: "MCP Servers", slug: "mcp-servers" },
            { label: "Web UI", slug: "web-ui" },
          ],
        },
        {
          label: "Development",
          items: [
            { label: "Architecture", slug: "dev-architecture" },
            { label: "Adding a Provider", slug: "dev-providers" },
            { label: "Adding a Tool", slug: "dev-tools" },
            { label: "Daemon Protocol", slug: "dev-protocol" },
            { label: "Testing", slug: "dev-testing" },
            { label: "Web UI Development", slug: "dev-web" },
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
