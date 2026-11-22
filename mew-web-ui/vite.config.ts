import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import path from "path";

export default defineConfig({
  clearScreen: false,
  plugins: [
    tanstackRouter({
      target: "react",
      autoCodeSplitting: true,
    }),
    react(),
  ],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    port: 5173,
    strictPort: true,
    host: false,
    proxy: {
      "/ws": {
        target: "ws://127.0.0.1:9847",
        ws: true,
        // Use the bridge's Host while preserving the browser's Origin.
        changeOrigin: true,
      },
    },
    watch: {
      ignored: [],
    },
  },
  envPrefix: ["VITE_"],
  build: {
    outDir: "dist",
    sourcemap: true,
    // Shiki's oniguruma engine ships as one ~620 kB inlined wasm chunk. It is
    // loaded lazily on first highlight and cannot be split further.
    chunkSizeWarningLimit: 650,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (/node_modules\/(?:\.pnpm\/[^/]+\/node_modules\/)?(?:react|react-dom|scheduler)\//.test(id)) return "react";
        },
      },
    },
  },
});
