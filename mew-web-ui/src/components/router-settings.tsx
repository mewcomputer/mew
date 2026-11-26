import type { MewClient } from "@mew/web-client";
import { SettingsGroup, SettingsRow } from "@/components/settings-primitives";
import { useDaemonSettings } from "@/hooks/use-daemon-settings";
import { useSessionStore } from "@/stores/session";
import { cn } from "@/lib/utils";

const TIERS = [
  { id: "nano", label: "Small (nano)", hint: "Cheapest tier for simple subagent turns." },
  { id: "micro", label: "Medium (micro)", hint: "General-purpose tier and permission classifier." },
  { id: "deci", label: "Large (deci)", hint: "Highest-capability tier for complex turns." },
] as const;

export function RouterSettings({
  client,
  connected,
}: {
  client: MewClient;
  connected: boolean;
}) {
  const { router, pendingRouter, error, setRouterTiers } = useDaemonSettings(client, connected);
  const availableModels = useSessionStore((state) => state.availableModels);
  const provider = router?.provider ?? "";
  const tiers = router?.tiers ?? [];

  const allModels = Array.from(
    new Set([
      ...availableModels.map((model) => model.id),
      ...tiers.map((tier) => tier.model).filter(Boolean),
    ]),
  ).sort();

  const commit = (changed: string, model: string) => {
    if (!connected || pendingRouter || !provider) return;
    const current = (id: string) => tiers.find((tier) => tier.id === id)?.model ?? "";
    setRouterTiers({
      provider,
      nano: changed === "nano" ? model : current("nano"),
      micro: changed === "micro" ? model : current("micro"),
      deci: changed === "deci" ? model : current("deci"),
    });
  };

  return (
    <SettingsGroup
      title="Model sizes"
      description={
        connected
          ? "Router tiers for subagents and the permission classifier. Applies to new and resumed sessions."
          : "Connect to the local daemon to configure model sizes."
      }
    >
      {!provider ? (
        <SettingsRow
          label="No router provider"
          description={
            'Set kind = "router" on a [providers.*] entry in config.toml to enable model-size tiers.'
          }
        />
      ) : (
        <>
          <SettingsRow
            label="Router provider"
            description="The configured router."
            trailing={<span className="rounded-full bg-muted px-2 py-0.5 font-mono text-[11px]">{provider}</span>}
          />
          {TIERS.map((tier) => {
            const current = tiers.find((entry) => entry.id === tier.id)?.model ?? "";
            return (
              <SettingsRow
                key={tier.id}
                label={tier.label}
                description={tier.hint}
                trailing={
                  <select
                    aria-label={`${tier.id} model`}
                    value={current}
                    disabled={!connected || pendingRouter}
                    onChange={(event) => commit(tier.id, event.target.value)}
                    className={cn(
                      "max-w-64 rounded-md border border-border/80 bg-background px-2 py-1 text-xs",
                      "focus:outline-none focus:ring-1 focus:ring-ring disabled:opacity-60",
                    )}
                  >
                    <option value="">(unset)</option>
                    {allModels.map((id) => (
                      <option key={id} value={id}>
                        {id}
                      </option>
                    ))}
                  </select>
                }
              />
            );
          })}
        </>
      )}
      {error && (
        <p role="alert" className="px-4 py-2.5 text-xs text-destructive">
          {error}
        </p>
      )}
    </SettingsGroup>
  );
}
