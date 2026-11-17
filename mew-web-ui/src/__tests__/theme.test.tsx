import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ThemeProvider, useTheme } from "../lib/theme";
import { initializeHost, resetHost } from "../lib/host";

function Appearance() {
  const { themeId, mode, themes, themeMode, setThemeMode, setThemeId } = useTheme();
  return <><output>{`${themeId}:${mode}:${themeMode}`}</output><button onClick={() => setThemeMode("system")}>System</button><button onClick={() => setThemeId("custom-night")}>Custom</button><span>{themes.map((t) => t.id).join(",")}</span></>;
}

afterEach(() => { cleanup(); resetHost(); delete window.mewQuery; localStorage.clear(); });

describe("appearance", () => {
  it("starts with native preference and exposes installed custom themes", async () => {
    window.mewQuery = vi.fn(({ request, onSuccess }) => {
      const value = JSON.parse(request);
      onSuccess(value.type === "bootstrap" ? JSON.stringify({ daemonUrl: "ws://127.0.0.1:25566", themeMode: "dark", lightTheme: "light", darkTheme: "custom-night", themes: [{ id: "light", mode: "light", css: "--background: #ffffff;" }, { id: "custom-night", mode: "dark", css: "--background: #102030; --foreground: #ffffff;" }] }) : "{}");
    });
    await initializeHost();
    render(<ThemeProvider><Appearance /></ThemeProvider>);
    expect(screen.getByRole("status").textContent).toBe("custom-night:dark:dark");
    expect(screen.getByText(/light,custom-night/)).toBeTruthy();
    expect(document.querySelector("style[data-mew-host-themes]")?.textContent).toContain('--background: #102030');
    fireEvent.click(screen.getByText("System"));
    expect(screen.getByRole("status").textContent).toBe("light:light:system");
    fireEvent.click(screen.getByText("Custom"));
    expect(screen.getByRole("status").textContent).toBe("custom-night:dark:dark");
    expect(window.mewQuery).toHaveBeenLastCalledWith(expect.objectContaining({ request: JSON.stringify({ type: "set_theme", themeMode: "dark", lightTheme: "light", darkTheme: "custom-night" }) }));
  });
});
