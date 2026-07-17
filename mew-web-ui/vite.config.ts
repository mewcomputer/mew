import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import path from "path";

const tauriDevHost = process.env.TAURI_DEV_HOST;
const tauriPlatform = process.env.TAURI_ENV_PLATFORM;

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
    host: tauriDevHost || false,
    proxy: {
      "/ws": {
        target: "ws://127.0.0.1:9847",
        ws: true,
      },
    },
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    outDir: "dist",
    target: tauriPlatform
      ? tauriPlatform === "windows"
        ? "chrome105"
        : "safari13"
      : undefined,
    minify: process.env.TAURI_ENV_DEBUG ? false : undefined,
    sourcemap: true,
  },
});
