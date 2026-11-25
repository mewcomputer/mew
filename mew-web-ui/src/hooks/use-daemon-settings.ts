import { useCallback, useEffect, useRef, useState } from "react";
import type {
  DaemonSettings,
  MewClient,
  PairedDevice,
  PairingInvite,
  RemoteAccessInfo,
} from "@mew/web-client";

export interface DaemonSettingsState {
  settings: DaemonSettings | null;
  remote: RemoteAccessInfo | null;
  devices: PairedDevice[];
  invite: PairingInvite | null;
  loading: boolean;
  pendingRemote: boolean;
  pendingInvite: boolean;
  revokingDeviceId: string | null;
  error: string | null;
  refresh: () => void;
  setRemoteEnabled: (enabled: boolean) => void;
  createPairingInvite: (
    scope?: "observe" | "collaborate" | "control",
    ttlSecs?: number,
  ) => void;
  revokeDevice: (deviceId: string) => void;
  clearInvite: () => void;
}

let reqCounter = 0;
function nextRequestId(prefix: string): string {
  reqCounter += 1;
  return `${prefix}-${Date.now()}-${reqCounter}`;
}

export function useDaemonSettings(
  client: MewClient,
  connected: boolean,
): DaemonSettingsState {
  const [settings, setSettings] = useState<DaemonSettings | null>(null);
  const [invite, setInvite] = useState<PairingInvite | null>(null);
  const [loading, setLoading] = useState(false);
  const [pendingRemote, setPendingRemote] = useState(false);
  const [pendingInvite, setPendingInvite] = useState(false);
  const [revokingDeviceId, setRevokingDeviceId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const remoteReqId = useRef<string | null>(null);
  const inviteReqId = useRef<string | null>(null);
  const revokeReqId = useRef<string | null>(null);

  const refresh = useCallback(() => {
    if (!connected) return;
    try {
      setLoading(true);
      setError(null);
      client.readDaemonSettings(nextRequestId("settings-read"));
    } catch (reason) {
      setLoading(false);
      setError(reason instanceof Error ? reason.message : "Could not read daemon settings.");
    }
  }, [client, connected]);

  useEffect(() => {
    if (!connected) {
      setSettings(null);
      setInvite(null);
      setLoading(false);
      setPendingRemote(false);
      setPendingInvite(false);
      setRevokingDeviceId(null);
      remoteReqId.current = null;
      inviteReqId.current = null;
      revokeReqId.current = null;
      return;
    }

    const onSettings = (next: DaemonSettings) => {
      setSettings(next);
      setLoading(false);
      setError(null);
      if (next.remote && !next.remote.enabled) {
        setInvite(null);
      }
      if (next.request_id && next.request_id === remoteReqId.current) {
        remoteReqId.current = null;
        setPendingRemote(false);
      }
      if (next.request_id && next.request_id === revokeReqId.current) {
        revokeReqId.current = null;
        setRevokingDeviceId(null);
      }
      if (next.request_id === null) {
        if (remoteReqId.current) {
          remoteReqId.current = null;
          setPendingRemote(false);
        }
        if (revokeReqId.current) {
          revokeReqId.current = null;
          setRevokingDeviceId(null);
        }
      }
    };

    const onInvite = (next: PairingInvite) => {
      if (inviteReqId.current && next.request_id !== inviteReqId.current) {
        return;
      }
      inviteReqId.current = null;
      setPendingInvite(false);
      setError(null);
      setInvite(next);
    };

    const onFailed = (failure: { request_id: string; message: string }) => {
      if (failure.request_id === remoteReqId.current) {
        remoteReqId.current = null;
        setPendingRemote(false);
      } else if (failure.request_id === inviteReqId.current) {
        inviteReqId.current = null;
        setPendingInvite(false);
      } else if (failure.request_id === revokeReqId.current) {
        revokeReqId.current = null;
        setRevokingDeviceId(null);
      } else {
        setLoading(false);
      }
      setError(failure.message);
    };

    client.on("daemon-settings", onSettings);
    client.on("pairing-invite", onInvite);
    client.on("daemon-settings-failed", onFailed);
    refresh();

    return () => {
      client.off("daemon-settings", onSettings);
      client.off("pairing-invite", onInvite);
      client.off("daemon-settings-failed", onFailed);
    };
  }, [client, connected, refresh]);

  const setRemoteEnabled = useCallback(
    (enabled: boolean) => {
      const id = nextRequestId("remote-toggle");
      remoteReqId.current = id;
      setError(null);
      setPendingRemote(true);
      try {
        client.setRemoteAccess(enabled, id);
      } catch (reason) {
        remoteReqId.current = null;
        setPendingRemote(false);
        setError(reason instanceof Error ? reason.message : "Could not update remote access.");
      }
    },
    [client],
  );

  const createPairingInvite = useCallback(
    (scope: "observe" | "collaborate" | "control" = "control", ttlSecs = 120) => {
      const id = nextRequestId("pair");
      inviteReqId.current = id;
      setError(null);
      setPendingInvite(true);
      try {
        client.createPairingInvite(id, { scope, ttlSecs });
      } catch (reason) {
        inviteReqId.current = null;
        setPendingInvite(false);
        setError(reason instanceof Error ? reason.message : "Could not create pairing invite.");
      }
    },
    [client],
  );

  const revokeDevice = useCallback(
    (deviceId: string) => {
      const id = nextRequestId("revoke");
      revokeReqId.current = id;
      setError(null);
      setRevokingDeviceId(deviceId);
      try {
        client.revokeDevice(deviceId, id);
      } catch (reason) {
        revokeReqId.current = null;
        setRevokingDeviceId(null);
        setError(reason instanceof Error ? reason.message : "Could not revoke device.");
      }
    },
    [client],
  );

  const clearInvite = useCallback(() => setInvite(null), []);

  return {
    settings,
    remote: settings?.remote ?? null,
    devices: settings?.devices ?? [],
    invite,
    loading,
    pendingRemote,
    pendingInvite,
    revokingDeviceId,
    error,
    refresh,
    setRemoteEnabled,
    createPairingInvite,
    revokeDevice,
    clearInvite,
  };
}
