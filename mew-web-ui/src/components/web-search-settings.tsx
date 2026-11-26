import type { MewClient } from "@mew/web-client";
import { SettingsGroup, SettingsRow, SettingsSwitch } from "@/components/settings-primitives";
import { useDaemonSettings } from "@/hooks/use-daemon-settings";
import { cn } from "@/lib/utils";

export function WebSearchSettings({
  client,
  connected,
}: {
  client: MewClient;
  connected: boolean;
}) {
  const { webSearch, pendingWebSearch, error, setWebSearchSettings } = useDaemonSettings(
    client,
    connected,
  );
  const enabled = webSearch?.enabled ?? false;
  const provider = webSearch?.provider ?? "";
  const providers = webSearch?.providers ?? [];
  const loaded = webSearch !== null;
  const providerInfo = providers.find((entry) => entry.id === provider);
  const fallback = providers.find((entry) => entry.id !== provider && entry.configured);
  const credential = provider ? `MEW_CRED_${provider.toUpperCase()}` : null;
  const status = providerInfo?.configured
    ? "Ready"
    : fallback
      ? `Fallback: ${fallback.id}`
      : "Missing";

  return (
    <SettingsGroup
      title="Web search"
      description={
        connected
          ? "Applies to sessions created or resumed after the change."
          : "Connect to the local daemon to configure web search."
      }
    >
      <SettingsRow
        label="Enable web search"
        description="Lets the agent search the web before fetching pages."
        trailing={
          <SettingsSwitch
            ariaLabel="Enable web search"
            checked={enabled}
            disabled={!connected || pendingWebSearch || !loaded}
            onChange={() => setWebSearchSettings({ enabled: !enabled })}
          />
        }
      />
      <SettingsRow
        label="Provider"
        description="The search backend used by the single web_search tool. If it fails, the other configured provider is tried."
        trailing={
          <div
            role="radiogroup"
            aria-label="Web search provider"
            className="inline-flex rounded-lg border border-border/80 bg-muted/60 p-0.5"
          >
            {providers.map((entry) => {
              const selected = entry.id === provider;
              return (
                <button
                  key={entry.id}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  title={entry.configured ? `${entry.id} api key found` : `${entry.id} api key missing`}
                  disabled={!connected || pendingWebSearch || !loaded}
                  onClick={() => setWebSearchSettings({ provider: entry.id })}
                  className={cn(
                    "rounded-md px-2.5 py-1 text-xs font-medium transition-all duration-150 ease-out active:scale-[0.97] motion-reduce:transition-none motion-reduce:active:scale-100 disabled:opacity-60",
                    selected
                      ? "bg-accent text-foreground shadow-xs ring-1 ring-border/70"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  {entry.id}
                  <span
                    className={cn(
                      "ml-1.5 inline-block size-1.5 rounded-full align-middle",
                      entry.configured ? "bg-primary" : "bg-muted-foreground/40",
                    )}
                  />
                </button>
              );
            })}
          </div>
        }
      />
      <SettingsRow
        label="API key"
        description={
          providerInfo?.configured ? (
            <>
              <code className="font-mono">{credential}</code> is configured. If it fails, the other
              configured provider is tried.
            </>
          ) : fallback ? (
            <>
              No key for <code className="font-mono">{provider}</code>;{" "}
              <code className="font-mono">{fallback.id}</code> is configured and will be used
              instead. Set <code className="font-mono">{credential}</code> to use your preference.
            </>
          ) : (
            <>
              No key for the selected provider, so the tool is not offered. Set{" "}
              <code className="font-mono">{credential ?? "MEW_CRED_TAVILY"}</code>, add{" "}
              <code className="font-mono">{provider || "tavily"}</code> to the{" "}
              <code className="font-mono">mew</code> keyring, or put{" "}
              <code className="font-mono">{`{"${provider || "tavily"}": "<key>"}`}</code> in{" "}
              <code className="font-mono">credentials.json</code>.
            </>
          )
        }
        trailing={
          <span
            className={
              providerInfo?.configured || fallback
                ? "rounded-full bg-primary/15 px-2 py-0.5 text-[11px] font-medium text-primary"
                : "rounded-full bg-muted px-2 py-0.5 text-[11px] font-medium text-muted-foreground"
            }
          >
            {status}
          </span>
        }
      />
      {error && (
        <p role="alert" className="px-4 py-2.5 text-xs text-destructive">
          {error}
        </p>
      )}
    </SettingsGroup>
  );
}
