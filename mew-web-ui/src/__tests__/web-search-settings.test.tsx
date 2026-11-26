import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { MewClient, type MewWebSocket } from "@mew/web-client";
import { WebSearchSettings } from "../components/web-search-settings";

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

const webSearch = (overrides: Record<string, unknown> = {}) => ({
  enabled: true,
  provider: "tavily",
  providers: [
    { id: "tavily", configured: true },
    { id: "exa", configured: false },
  ],
  ...overrides,
});

function settingsReply(readReq: Record<string, unknown> | undefined, info: unknown) {
  return {
    type: "daemon_settings",
    request_id: readReq?.request_id,
    defaults: null,
    devices: null,
    web_search: info,
  };
}

it("shows provider chips and the selected provider's key requirement", async () => {
  const { client, sent, receive, open } = makeMockClient();
  await open();
  render(<WebSearchSettings client={client} connected />);

  const toggle = screen.getByRole("switch", { name: "Enable web search" });
  expect(toggle.hasAttribute("disabled")).toBe(true);

  const readReq = sent.find((m) => m.type === "read_daemon_settings");
  receive(settingsReply(readReq, webSearch({ provider: "exa" })));

  expect(toggle.getAttribute("aria-checked")).toBe("true");
  expect(toggle.hasAttribute("disabled")).toBe(false);
  const group = screen.getByRole("radiogroup", { name: "Web search provider" });
  expect(group.querySelectorAll('[role="radio"]')).toHaveLength(2);
  expect(screen.getByRole("radio", { name: /exa/ }).getAttribute("aria-checked")).toBe("true");
  expect(screen.getByText(/MEW_CRED_EXA/)).toBeTruthy();
  expect(screen.getByText("Fallback: tavily")).toBeTruthy();
  expect(screen.getByText(/will be used/)).toBeTruthy();
});

it("reports Missing when neither provider has a key", async () => {
  const { client, sent, receive, open } = makeMockClient();
  await open();
  render(<WebSearchSettings client={client} connected />);
  const readReq = sent.find((m) => m.type === "read_daemon_settings");
  receive(
    settingsReply(
      readReq,
      webSearch({
        providers: [
          { id: "tavily", configured: false },
          { id: "exa", configured: false },
        ],
      }),
    ),
  );
  expect(screen.getByText("Missing")).toBeTruthy();
  expect(screen.getByText(/so the tool is not offered/)).toBeTruthy();
});

it("sends a toggle update and resolves pending on the daemon reply", async () => {
  const { client, sent, receive, open } = makeMockClient();
  await open();
  render(<WebSearchSettings client={client} connected />);

  const readReq = sent.find((m) => m.type === "read_daemon_settings");
  receive(settingsReply(readReq, webSearch()));
  expect(screen.getByText("Ready")).toBeTruthy();

  fireEvent.click(screen.getByRole("switch", { name: "Enable web search" }));
  const update = sent.find((m) => m.type === "set_web_search_settings");
  expect(update).toEqual({ type: "set_web_search_settings", enabled: false, request_id: update?.request_id });

  const toggle = screen.getByRole("switch", { name: "Enable web search" });
  expect(toggle.getAttribute("aria-checked")).toBe("true");
  expect(toggle.hasAttribute("disabled")).toBe(true);

  receive(settingsReply(update as Record<string, unknown>, webSearch({ enabled: false })));
  expect(toggle.getAttribute("aria-checked")).toBe("false");
  expect(toggle.hasAttribute("disabled")).toBe(false);
});

it("switches provider with one partial settings write", async () => {
  const { client, sent, receive, open } = makeMockClient();
  await open();
  render(<WebSearchSettings client={client} connected />);

  const readReq = sent.find((m) => m.type === "read_daemon_settings");
  receive(settingsReply(readReq, webSearch()));
  fireEvent.click(screen.getByRole("radio", { name: /exa/ }));

  const update = sent.find((m) => m.type === "set_web_search_settings");
  expect(update).toEqual({
    type: "set_web_search_settings",
    provider: "exa",
    request_id: update?.request_id,
  });
  receive(settingsReply(update as Record<string, unknown>, webSearch({ provider: "exa" })));
  expect(screen.getByRole("radio", { name: /exa/ }).getAttribute("aria-checked")).toBe("true");
});

it("surfaces a daemon rejection and unlocks the controls", async () => {
  const { client, sent, receive, open } = makeMockClient();
  await open();
  render(<WebSearchSettings client={client} connected />);

  const readReq = sent.find((m) => m.type === "read_daemon_settings");
  receive(settingsReply(readReq, webSearch()));
  fireEvent.click(screen.getByRole("radio", { name: /exa/ }));
  const update = sent.find((m) => m.type === "set_web_search_settings");

  receive({ type: "daemon_settings_failed", request_id: update?.request_id, message: "denied" });

  expect(screen.getByRole("alert").textContent).toBe("denied");
  expect(screen.getByRole("switch", { name: "Enable web search" }).hasAttribute("disabled")).toBe(false);
});

it("disables everything and explains why when offline", () => {
  const { client } = makeMockClient();
  render(<WebSearchSettings client={client} connected={false} />);

  expect(screen.getByRole("switch", { name: "Enable web search" }).hasAttribute("disabled")).toBe(true);
  expect(screen.getByText(/Connect to the local daemon to configure web search/)).toBeTruthy();
});
