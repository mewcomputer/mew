import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { MewClient, type MewWebSocket } from "@mew/web-client";
import { RouterSettings } from "../components/router-settings";
import { useSessionStore } from "../stores/session";

afterEach(() => {
  cleanup();
  useSessionStore.setState({ availableModels: [] });
});

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
      handlers.get("message")?.forEach((fn) => fn({ data: JSON.stringify(message) })),
    );
  const open = async () => {
    const promise = client.connect();
    handlers.get("open")?.forEach((fn) => fn({}));
    await promise;
  };
  return { client, sent, receive, open };
}

const routerInfo = (tiers: { id: string; model: string }[] = []) => ({
  provider: "router",
  tiers,
});

function settingsReply(readReq: Record<string, unknown> | undefined, router: unknown) {
  return {
    type: "daemon_settings",
    request_id: readReq?.request_id,
    defaults: null,
    devices: null,
    router,
  };
}

it("shows the configured tiers and model choices", async () => {
  useSessionStore.setState({
    availableModels: [
      { id: "deepseek/flash", provider: "deepseek", model: "flash" },
      { id: "deepseek/pro", provider: "deepseek", model: "pro" },
      { id: "openai/gpt-5", provider: "openai", model: "gpt-5" },
    ],
  });
  const { client, sent, receive, open } = makeMockClient();
  await open();
  render(<RouterSettings client={client} connected />);

  const readReq = sent.find((message) => message.type === "read_daemon_settings");
  receive(
    settingsReply(readReq, routerInfo([
      { id: "nano", model: "deepseek/flash" },
      { id: "micro", model: "deepseek/pro" },
      { id: "deci", model: "" },
    ])),
  );

  expect(screen.getByText("router")).toBeTruthy();
  const nano = screen.getByLabelText("nano model") as HTMLSelectElement;
  expect(nano.value).toBe("deepseek/flash");
  expect(nano.querySelectorAll("option")).toHaveLength(4);
  expect((screen.getByLabelText("deci model") as HTMLSelectElement).value).toBe("");
});

it("sends all three tiers when one changes and clears pending on reply", async () => {
  const { client, sent, receive, open } = makeMockClient();
  await open();
  render(<RouterSettings client={client} connected />);

  const readReq = sent.find((message) => message.type === "read_daemon_settings");
  receive(
    settingsReply(readReq, routerInfo([
      { id: "nano", model: "deepseek/flash" },
      { id: "micro", model: "deepseek/pro" },
      { id: "deci", model: "openai/gpt-5" },
    ])),
  );

  const nano = screen.getByLabelText("nano model") as HTMLSelectElement;
  fireEvent.change(nano, { target: { value: "openai/gpt-5" } });
  const update = sent.find((message) => message.type === "set_router_tiers");
  expect(update).toEqual({
    type: "set_router_tiers",
    provider: "router",
    nano: "openai/gpt-5",
    micro: "deepseek/pro",
    deci: "openai/gpt-5",
    request_id: update?.request_id,
  });
  expect(nano.disabled).toBe(true);

  receive(
    settingsReply(update as Record<string, unknown>, routerInfo([
      { id: "nano", model: "openai/gpt-5" },
      { id: "micro", model: "deepseek/pro" },
      { id: "deci", model: "openai/gpt-5" },
    ])),
  );
  expect((screen.getByLabelText("nano model") as HTMLSelectElement).disabled).toBe(false);
});

it("explains a missing router provider and disables edits when offline", async () => {
  const first = makeMockClient();
  await first.open();
  const { unmount } = render(<RouterSettings client={first.client} connected />);
  const readReq = first.sent.find((message) => message.type === "read_daemon_settings");
  first.receive(settingsReply(readReq, { provider: "", tiers: [] }));
  expect(screen.getByText(/No router provider/)).toBeTruthy();
  expect(screen.getByText(/kind = \"router\"/)).toBeTruthy();
  unmount();

  const offline = makeMockClient();
  render(<RouterSettings client={offline.client} connected={false} />);
  expect(screen.getByText(/Connect to the local daemon/)).toBeTruthy();
});
