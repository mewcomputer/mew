import { useEffect, useRef, useState } from "react";
import type { MewClient } from "@mew/web-client";
import { SettingsRow, SettingsSwitch } from "@/components/settings-primitives";

type Settings = { auto_title_enabled: boolean; auto_summary_enabled: boolean };

export function AutomationSettings({ client, connected }: { client: MewClient; connected: boolean }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const pendingChange = useRef<{ name: "title" | "summary"; enabled: boolean } | null>(null);
  const timeout = useRef<ReturnType<typeof setTimeout> | null>(null);

  const finish = () => {
    if (timeout.current) clearTimeout(timeout.current);
    timeout.current = null;
    pendingChange.current = null;
    setPending(false);
  };

  useEffect(() => {
    if (!connected) { setSettings(null); finish(); return; }
    const onSettings = (value: Settings) => {
      setSettings(value);
      setError(null);
      const change = pendingChange.current;
      if (change && (change.name === "title" ? value.auto_title_enabled : value.auto_summary_enabled) === change.enabled) finish();
    };
    const onSummary = ({ enabled }: { enabled: boolean }) => {
      setSettings((value) => value && { ...value, auto_summary_enabled: enabled });
      if (pendingChange.current?.name === "summary" && pendingChange.current.enabled === enabled) finish();
    };
    const onError = ({ message }: { message: string }) => {
      if (message.startsWith("could not save auto-") || message.startsWith("could not read saved automation settings") || message.startsWith("could not migrate automation settings")) { setError(message); finish(); }
    };
    client.on("automation-settings", onSettings);
    client.on("auto-summary-changed", onSummary);
    client.on("errorMessage", onError);
    try { client.readAutomationSettings(); } catch (reason) { setError(reason instanceof Error ? reason.message : "Could not read automation settings."); }
    return () => {
      client.off("automation-settings", onSettings);
      client.off("auto-summary-changed", onSummary);
      client.off("errorMessage", onError);
      if (timeout.current) clearTimeout(timeout.current);
    };
  }, [client, connected]);

  const update = (name: "title" | "summary", enabled: boolean) => {
    try {
      setError(null);
      setPending(true);
      pendingChange.current = { name, enabled };
      timeout.current = setTimeout(() => { setError("The daemon did not confirm the setting."); finish(); }, 10000);
      if (name === "title") client.setAutoTitle(enabled);
      else client.setAutoSummary(enabled);
    } catch (reason) { finish(); setError(reason instanceof Error ? reason.message : "Could not update automation settings."); }
  };

  return (
    <>
      {([
        ["title", "Auto-generate session titles", "Creates a short AI title after the first message of each session", settings?.auto_title_enabled],
        ["summary", "Auto-summarize idle sessions", "Generates a 1-2 sentence summary after 10 minutes of inactivity", settings?.auto_summary_enabled],
      ] as const).map(([name, label, description, enabled]) => (
        <SettingsRow
          key={name}
          label={label}
          description={description}
          trailing={
            <SettingsSwitch
              ariaLabel={label}
              checked={enabled ?? false}
              disabled={!connected || pending || enabled === undefined}
              onChange={() => update(name, !enabled)}
            />
          }
        />
      ))}
      {error && (
        <p role="alert" className="px-4 py-2.5 text-xs text-destructive">
          {error}
        </p>
      )}
    </>
  );
}
