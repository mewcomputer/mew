import type { MewClient } from "@mew/web-client";

/** Move legacy browser preferences only when the daemon has no saved settings. */
export function setupAutomationSync(client: MewClient) {
  let attempted = false;
  let retry = false;
  const read = () => {
    try { client.readAutomationSettings(); retry = false; } catch { retry = true; }
  };
  client.on("open", () => { attempted = false; read(); });
  client.on("session-ready", () => { if (retry) read(); });
  client.on("automation-settings", (settings) => {
    if (settings.initialized !== false || attempted) return;
    try {
      const title = localStorage.getItem("mew.autoTitle");
      const summary = localStorage.getItem("mew.autoSummary");
      attempted = true;
      const titleValue = title === "true" ? true : title === "false" ? false : null;
      const summaryValue = summary === "true" ? true : summary === "false" ? false : null;
      if (titleValue !== null || summaryValue !== null) client.migrateAutomationSettings(titleValue, summaryValue);
    } catch {
      attempted = false;
      retry = true;
    }
  });
}
