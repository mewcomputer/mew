import path from "node:path";
import { defineConfig } from "@storylite/storylite";
import react from "@storylite/renderer-react";
import viteReact from "@vitejs/plugin-react";

export default defineConfig({
  stories: ["./src/**/*.stories.tsx"],
  css: ["./src/index.css"],
  renderers: [react()],
  vitePlugins: () => [
    viteReact(),
    {
      name: "mew-web-ui-alias",
      config: () => ({
        resolve: {
          alias: {
            "@": path.resolve(import.meta.dirname, "../src"),
          },
        },
      }),
    },
  ],
});
