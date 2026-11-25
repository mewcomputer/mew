import { useEffect, useState } from "react";
import type { MewClient } from "@mew/web-client";
import QRCode from "qrcode";
import { Check, Copy, Info, QrCode, RefreshCw, Trash2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { useDaemonSettings } from "@/hooks/use-daemon-settings";
import { SettingsGroup, SettingsRow, SettingsSwitch } from "@/components/settings-primitives";

function CopyButton({ value, label }: { value: string; label: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard API may be unavailable in restricted contexts.
    }
  };

  return (
    <button
      type="button"
      onClick={handleCopy}
      aria-label={label}
      className="inline-flex h-7 shrink-0 items-center gap-1 rounded-md border border-border/80 bg-background/60 px-2.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground active:scale-[0.97] motion-reduce:transition-none motion-reduce:active:scale-100"
    >
      {copied ? <Check className="h-3 w-3 text-primary" /> : <Copy className="h-3 w-3" />}
      {copied ? "Copied" : "Copy"}
    </button>
  );
}

export function RemoteSettings({
  client,
  connected,
}: {
  client: MewClient;
  connected: boolean;
}) {
  const {
    remote,
    devices,
    invite,
    loading,
    pendingRemote,
    pendingInvite,
    revokingDeviceId,
    error,
    setRemoteEnabled,
    createPairingInvite,
    revokeDevice,
  } = useDaemonSettings(client, connected);

  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [qrError, setQrError] = useState<string | null>(null);
  const [remainingSecs, setRemainingSecs] = useState<number>(0);

  useEffect(() => {
    if (!invite) {
      setQrDataUrl(null);
      setQrError(null);
      setRemainingSecs(0);
      return;
    }

    let cancelled = false;
    setQrError(null);
    QRCode.toDataURL(invite.payload, {
      width: 192,
      margin: 1,
      errorCorrectionLevel: "M",
    })
      .then((url) => {
        if (!cancelled) setQrDataUrl(url);
      })
      .catch((reason) => {
        if (!cancelled) {
          setQrError(reason instanceof Error ? reason.message : "Could not render QR code.");
        }
      });

    const updateCountdown = () => {
      const nowSecs = Math.floor(Date.now() / 1000);
      setRemainingSecs(Math.max(0, invite.expires_at - nowSecs));
    };
    updateCountdown();
    const timer = setInterval(updateCountdown, 1000);

    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [invite]);

  const supported = remote?.supported ?? false;
  const enabled = remote?.enabled ?? false;
  const switchDisabled = !connected || !remote || !supported || pendingRemote;

  const statusBadge = !connected
    ? { label: "Offline", tone: "bg-muted text-muted-foreground" }
    : !supported && remote
      ? { label: "Unavailable", tone: "bg-muted text-muted-foreground" }
      : enabled
        ? { label: "Listening", tone: "bg-primary/15 text-primary" }
        : { label: "Off", tone: "bg-muted text-muted-foreground" };

  return (
    <div className="space-y-6">
      <SettingsGroup
        title="Remote access (iroh)"
        action={
          <span
            className={cn(
              "inline-flex items-center rounded-full px-2 py-0.5 text-[11px] font-medium",
              statusBadge.tone,
            )}
          >
            {statusBadge.label}
          </span>
        }
        description={
          !connected
            ? "Connect to the local daemon to configure remote access and pair devices."
            : undefined
        }
      >
        <SettingsRow
          label="Allow iOS & remote connections"
          description={
            pendingRemote
              ? enabled
                ? "Stopping iroh listener…"
                : "Connecting to iroh relay…"
              : "Accept authenticated peer-to-peer connections from paired devices"
          }
          trailing={
            <SettingsSwitch
              ariaLabel="Enable remote access"
              checked={enabled}
              disabled={switchDisabled}
              onChange={() => setRemoteEnabled(!enabled)}
            />
          }
        />

        {connected && !loading && remote && !remote.supported && (
          <div className="flex items-start gap-2.5 bg-muted/30 px-4 py-3 text-xs leading-relaxed text-muted-foreground">
            <Info className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <div>
              This daemon binary was built without the <code className="font-mono">iroh</code>{" "}
              feature. Build with{" "}
              <code className="font-mono">cargo build --locked -p mew --features iroh</code> to
              enable live iroh pairing from Desktop.
            </div>
          </div>
        )}

        {remote?.node_id && (
          <SettingsRow
            label="Daemon Node ID"
            description={
              <span className="block truncate font-mono text-[11px] text-muted-foreground">
                {remote.node_id}
              </span>
            }
            trailing={<CopyButton value={remote.node_id} label="Copy Node ID" />}
          />
        )}
      </SettingsGroup>

      {enabled && (
        <SettingsGroup
          title="Pair mobile device"
          action={
            <button
              type="button"
              disabled={!connected || pendingInvite}
              onClick={() => createPairingInvite("control", 120)}
              className="inline-flex h-7 items-center gap-1.5 rounded-md border border-border/80 bg-background/80 px-2.5 text-xs font-medium text-foreground transition-colors hover:bg-accent active:scale-[0.97] disabled:opacity-50 motion-reduce:transition-none motion-reduce:active:scale-100"
            >
              {invite ? <RefreshCw className="h-3 w-3" /> : <QrCode className="h-3 w-3" />}
              {pendingInvite
                ? "Generating…"
                : invite
                  ? "New QR invite"
                  : "Show pairing QR"}
            </button>
          }
        >
          {invite ? (
            <div className="p-4">
              <div className="flex flex-col items-center gap-4 sm:flex-row sm:items-start">
                <div className="shrink-0 overflow-hidden rounded-xl border border-border bg-white p-2.5 shadow-sm">
                  {qrDataUrl ? (
                    <img src={qrDataUrl} alt="Pairing QR code" className="h-44 w-44" />
                  ) : (
                    <div className="flex h-44 w-44 items-center justify-center text-xs text-muted-foreground">
                      {qrError ?? "Rendering QR…"}
                    </div>
                  )}
                </div>
                <div className="min-w-0 flex-1 space-y-2.5">
                  <div className="text-[13px] font-medium text-foreground">
                    Scan in Mew iOS app
                  </div>
                  <p className="text-xs leading-relaxed text-muted-foreground">
                    Open Mew on your iPhone, tap <strong>Scan QR Code</strong>, and point the
                    camera here. Single-use invite grants full control access.
                  </p>
                  <div className="text-xs font-medium tabular-nums text-muted-foreground">
                    {remainingSecs > 0
                      ? `Expires in ${remainingSecs}s`
                      : "Invite expired — generate a new one."}
                  </div>
                  <div className="flex items-center gap-2 pt-0.5">
                    <code className="min-w-0 flex-1 truncate rounded-md border border-border/60 bg-muted/60 px-2.5 py-1.5 font-mono text-[11px] text-foreground">
                      {invite.payload}
                    </code>
                    <CopyButton value={invite.payload} label="Copy invite link" />
                  </div>
                </div>
              </div>
            </div>
          ) : (
            <SettingsRow
              label="QR code pairing"
              description="Generate a single-use 120-second invite to pair your iPhone or another Mew client."
            />
          )}
        </SettingsGroup>
      )}

      {remote?.supported && (
        <SettingsGroup title="Paired devices">
          {devices.length === 0 ? (
            <div className="px-4 py-3 text-xs text-muted-foreground">No paired devices yet.</div>
          ) : (
            devices.map((device) => (
              <SettingsRow
                key={device.id}
                label={
                  <span className="flex items-center gap-2">
                    <span className="truncate">{device.name || "Unnamed device"}</span>
                    {device.scope && (
                      <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                        {device.scope}
                      </span>
                    )}
                  </span>
                }
                description={
                  <span className="block truncate font-mono text-[11px] text-muted-foreground">
                    {device.node_id}
                  </span>
                }
                trailing={
                  <button
                    type="button"
                    disabled={revokingDeviceId === device.id}
                    onClick={() => revokeDevice(device.id)}
                    aria-label={`Revoke ${device.name || device.id}`}
                    className="inline-flex h-7 shrink-0 items-center gap-1 rounded-md border border-border/80 px-2.5 text-xs font-medium text-destructive transition-colors hover:bg-destructive/10 active:scale-[0.97] disabled:opacity-50 motion-reduce:transition-none motion-reduce:active:scale-100"
                  >
                    <Trash2 className="h-3 w-3" />
                    {revokingDeviceId === device.id ? "Revoking…" : "Revoke"}
                  </button>
                }
              />
            ))
          )}
        </SettingsGroup>
      )}

      {error && (
        <p role="alert" className="px-1 text-xs text-destructive">
          {error}
        </p>
      )}
    </div>
  );
}
