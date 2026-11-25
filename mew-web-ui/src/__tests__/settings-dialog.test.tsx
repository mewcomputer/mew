import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SettingsDialog } from "../components/settings-dialog";
import { ThemeProvider } from "../lib/theme";

vi.mock("@tanstack/react-router", () => ({ useRouter: () => ({ navigate: vi.fn() }) }));

afterEach(() => cleanup());

describe("settings dialog", () => {
  it("opens the extracted settings panel in a centered dialog", () => {
    render(<ThemeProvider><SettingsDialog /></ThemeProvider>);
    expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull();

    fireEvent(window, new Event("mew:open-settings"));

    const dialog = screen.getByRole("dialog", { name: "Settings" });
    const nav = within(dialog).getByRole("navigation", { name: "Settings sections" });
    expect(within(nav).getByRole("button", { name: /General/ })).toBeTruthy();

    fireEvent.click(within(nav).getByRole("button", { name: /Appearance/ }));
    expect(within(dialog).getByRole("radiogroup", { name: "Appearance mode" })).toBeTruthy();

    fireEvent.click(within(nav).getByRole("button", { name: /Permissions/ }));
    expect(within(dialog).getByRole("radiogroup", { name: "Permission mode" })).toBeTruthy();

    fireEvent(window, new Event("mew:close-settings"));
    expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull();
  });
});
