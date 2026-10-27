import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "node:path";
import { readFileSync } from "node:fs";

export default defineConfig({
  root: __dirname,
  publicDir: path.resolve(__dirname, "../../public"),
  plugins: [react(), { name: "composition-fixture", generateBundle() {
    this.emitFile({ type: "asset", fileName: "fixture.html", source: readFileSync(path.resolve(__dirname, "public/fixture.html")) });
  } }],
  resolve: { alias: { "@": path.resolve(__dirname, "../../src") } },
  build: { outDir: path.resolve(__dirname, "../../../target/composition-ui"), emptyOutDir: true },
  test: {
    environment: "jsdom",
    setupFiles: [path.resolve(__dirname, "../../vitest.setup.ts")],
    include: ["*.test.tsx"],
  },
});
