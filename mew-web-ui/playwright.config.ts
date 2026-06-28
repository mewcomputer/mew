import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  retries: 0,
  fullyParallel: false,
  workers: 1,
  use: {
    baseURL: "http://127.0.0.1:19847",
    headless: true,
  },
  // Don't auto-start a web server — our test spawns the daemon + bridge
  // as subprocesses in the test fixture.
  webServer: undefined,
});
