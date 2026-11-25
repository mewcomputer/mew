import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { MewClient, type MewWebSocket } from "@mew/web-client";
import { AutomationSettings } from "../components/automation-settings";
import { setupAutomationSync } from "../lib/automation-sync";

afterEach(() => { cleanup(); localStorage.removeItem("mew.autoTitle"); localStorage.removeItem("mew.autoSummary"); });

it("does not offer changes while the daemon is disconnected", () => {
  const client = new MewClient("ws://test");
  render(<AutomationSettings client={client} connected={false} />);
  expect(screen.getByRole("switch", { name: "Auto-summarize idle sessions" }).hasAttribute("disabled")).toBe(true);
  expect(screen.getByRole("switch", { name: "Auto-generate session titles" }).hasAttribute("disabled")).toBe(true);
});

it("migrates legacy preferences only when the daemon has no saved settings", async () => {
  const handlers = new Map<string, Set<(event: any) => void>>();
  const sent: any[] = [];
  const socket = {
    readyState: 1,
    send: (text: string) => sent.push(JSON.parse(text)),
    close: () => {},
    addEventListener: (name: string, fn: (event: any) => void) => {
      const entries = handlers.get(name) ?? new Set(); entries.add(fn); handlers.set(name, entries);
    },
    removeEventListener: (name: string, fn: (event: any) => void) => handlers.get(name)?.delete(fn),
  } as MewWebSocket;
  const client = new MewClient("ws://test", { socketFactory: () => socket });
  localStorage.setItem("mew.autoSummary", "false");
  setupAutomationSync(client);
  const connected = client.connect();
  handlers.get("open")?.forEach((fn) => fn({}));
  await connected;
  const receive = (message: unknown) => handlers.get("message")?.forEach((fn) => fn({ data: JSON.stringify(message) }));
  receive({ type: "automation_settings", auto_title_enabled: true, auto_summary_enabled: true, initialized: false });
  expect(sent).toEqual([{ type: "read_automation_settings" }, { type: "migrate_automation_settings", auto_title_enabled: null, auto_summary_enabled: false }]);
  handlers.get("open")?.forEach((fn) => fn({}));
  receive({ type: "automation_settings", auto_title_enabled: true, auto_summary_enabled: true, initialized: true });
  expect(sent).toEqual([{ type: "read_automation_settings" }, { type: "migrate_automation_settings", auto_title_enabled: null, auto_summary_enabled: false }, { type: "read_automation_settings" }]);
});

it("shows confirmed settings and follows changes from another client", async () => {
  const handlers = new Map<string, Set<(event: any) => void>>();
  const sent: unknown[] = [];
  const socket = {
    readyState: 1,
    send: (text: string) => sent.push(JSON.parse(text)),
    close: () => {},
    addEventListener: (name: string, fn: (event: any) => void) => {
      const entries = handlers.get(name) ?? new Set(); entries.add(fn); handlers.set(name, entries);
    },
    removeEventListener: (name: string, fn: (event: any) => void) => handlers.get(name)?.delete(fn),
  } as MewWebSocket;
  const client = new MewClient("ws://test", { socketFactory: () => socket });
  const connected = client.connect();
  handlers.get("open")?.forEach((fn) => fn({}));
  await connected;
  render(<AutomationSettings client={client} connected />);
  expect(sent).toContainEqual({ type: "read_automation_settings" });
  const receive = (message: unknown) => act(() => handlers.get("message")?.forEach((fn) => fn({ data: JSON.stringify(message) })));
  receive({ type: "automation_settings", auto_title_enabled: true, auto_summary_enabled: false });
  const summary = await screen.findByRole("switch", { name: "Auto-summarize idle sessions" });
  expect(summary.getAttribute("aria-checked")).toBe("false");
  fireEvent.click(summary);
  expect(sent).toContainEqual({ type: "set_auto_summary", enabled: true });
  expect(summary.getAttribute("aria-checked")).toBe("false");
  receive({ type: "auto_summary_changed", enabled: true });
  expect(summary.getAttribute("aria-checked")).toBe("true");
  receive({ type: "automation_settings", auto_title_enabled: false, auto_summary_enabled: true });
  expect(screen.getByRole("switch", { name: "Auto-generate session titles" }).getAttribute("aria-checked")).toBe("false");
  fireEvent.click(screen.getByRole("switch", { name: "Auto-generate session titles" }));
  receive({ type: "auto_summary_changed", enabled: false });
  expect(screen.getByRole("switch", { name: "Auto-generate session titles" }).hasAttribute("disabled")).toBe(true);
  receive({ type: "error", message: "could not save auto-title setting: permission denied" });
  expect(screen.getByRole("alert").textContent).toContain("permission denied");
  expect(screen.getByRole("switch", { name: "Auto-generate session titles" }).getAttribute("aria-checked")).toBe("false");
});
