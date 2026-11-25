import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { MewClient, type MewWebSocket } from "@mew/web-client";
import { RemoteSettings } from "../components/remote-settings";

vi.mock("qrcode", () => ({
  default: {
    toDataURL: vi.fn().mockResolvedValue("data:image/png;base64,MOCKQR"),
  },
}));

afterEach(() => cleanup());

function makeMockClient() {
  const handlers = new Map<string, Set<(event: unknown) => void>>();
  const sent: Record<string, unknown>[] = [];
  const socket = {
    readyState: 1,
    send: (text: string) => sent.push(JSON.parse(text)),
    close: () => {},
    addEventListener: (name: string, fn: (event: unknown) => void) => {
      const entries = handlers.get(name) ?? new Set();
      entries.add(fn);
      handlers.set(name, entries);
    },
    removeEventListener: (name: string, fn: (event: unknown) => void) => {
      handlers.get(name)?.delete(fn);
    },
  } as unknown as MewWebSocket;
  const client = new MewClient("ws://test", { socketFactory: () => socket });
  const receive = (message: unknown) =>
    act(() =>
      handlers
        .get("message")
        ?.forEach((fn) => fn({ data: JSON.stringify(message) })),
    );
  const open = async () => {
    const promise = client.connect();
    handlers.get("open")?.forEach((fn) => fn({}));
    await promise;
  };
  return { client, sent, receive, open };
}

it("shows unsupported message when daemon lacks iroh feature", async () => {
  const { client, sent, receive, open } = makeMockClient();
  await open();
  render(<RemoteSettings client={client} connected />);

  const readReq = sent.find((m) => m.type === "read_daemon_settings");
  expect(readReq).toBeTruthy();

  receive({
    type: "daemon_settings",
    request_id: readReq?.request_id,
    defaults: null,
    devices: null,
    remote: { supported: false, enabled: false, node_id: null },
  });

  const toggle = screen.getByRole("switch", { name: "Enable remote access" });
  expect(toggle.hasAttribute("disabled")).toBe(true);
  expect(screen.getByText(/This daemon binary was built without the/i)).toBeTruthy();
});

it("enables remote access, renders pairing QR invite, and revokes devices", async () => {
  const { client, sent, receive, open } = makeMockClient();
  await open();
  render(<RemoteSettings client={client} connected />);

  const readReq = sent.find((m) => m.type === "read_daemon_settings");
  receive({
    type: "daemon_settings",
    request_id: readReq?.request_id,
    defaults: null,
    devices: [
      {
        id: "dev-1",
        name: "Natalie iPhone",
        node_id: "node-phone-1",
        scope: "control",
        paired_at: 100,
      },
    ],
    remote: { supported: true, enabled: false, node_id: "node-daemon-1" },
  });

  const toggle = screen.getByRole("switch", { name: "Enable remote access" });
  expect(toggle.getAttribute("aria-checked")).toBe("false");
  fireEvent.click(toggle);

  const toggleReq = sent.find((m) => m.type === "set_remote_access");
  expect(toggleReq?.enabled).toBe(true);

  receive({
    type: "daemon_settings",
    request_id: toggleReq?.request_id,
    defaults: null,
    devices: [
      {
        id: "dev-1",
        name: "Natalie iPhone",
        node_id: "node-phone-1",
        scope: "control",
        paired_at: 100,
      },
    ],
    remote: { supported: true, enabled: true, node_id: "node-daemon-1" },
  });

  expect(toggle.getAttribute("aria-checked")).toBe("true");

  fireEvent.click(screen.getByRole("button", { name: /Show pairing QR/i }));
  const pairReq = sent.find((m) => m.type === "create_pairing_invite");
  expect(pairReq).toBeTruthy();

  await act(async () => {
    receive({
      type: "pairing_invite",
      request_id: pairReq?.request_id,
      node_id: "node-daemon-1",
      payload: "computer.mew.mew://node-daemon-1?token=mew_secret",
      expires_at: Math.floor(Date.now() / 1000) + 120,
    });
  });

  const qrImg = await screen.findByAltText("Pairing QR code");
  expect(qrImg.getAttribute("src")).toBe("data:image/png;base64,MOCKQR");
  expect(screen.getByText("computer.mew.mew://node-daemon-1?token=mew_secret")).toBeTruthy();

  fireEvent.click(screen.getByRole("button", { name: "Revoke Natalie iPhone" }));
  const revokeReq = sent.find((m) => m.type === "revoke_device");
  expect(revokeReq?.device_id).toBe("dev-1");
});
