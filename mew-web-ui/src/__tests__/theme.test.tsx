import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ThemeProvider, useTheme } from "../lib/theme";
import { initializeHost, resetHost } from "../lib/host";

function Appearance() {
  const { themeId, mode, themes, themeMode, setThemeMode, setThemeId, titleFontId, setTitleFontId, bodyFontId, setBodyFontId } = useTheme();
  return <><output>{`${themeId}:${mode}:${themeMode}:${titleFontId}:${bodyFontId}`}</output><button onClick={() => setThemeMode("system")}>System</button><button onClick={() => setThemeId("custom-night")}>Custom</button><button onClick={() => setTitleFontId("banga")}>Title</button><button onClick={() => setBodyFontId("mi-sans")}>Body</button><span>{themes.map((t) => t.id).join(",")}</span></>;
}

afterEach(() => { cleanup(); resetHost(); delete window.mewQuery; localStorage.clear(); });

describe("appearance", () => {
  it("starts with native preference and exposes installed custom themes and independent title/body fonts", async () => {
    window.mewQuery = vi.fn(({ request, onSuccess }) => {
      const value = JSON.parse(request);
      onSuccess(value.type === "bootstrap" ? JSON.stringify({ daemonUrl: "ws://127.0.0.1:25566", themeMode: "dark", lightTheme: "light", darkTheme: "custom-night", themes: [{ id: "light", mode: "light", css: "--background: #ffffff;" }, { id: "custom-night", mode: "dark", css: "--background: #102030; --foreground: #ffffff;" }] }) : "{}");
    });
    await initializeHost();
    render(<ThemeProvider><Appearance /></ThemeProvider>);
    expect(screen.getByRole("status").textContent).toBe("custom-night:dark:dark:sentient:instrument-sans");
    expect(document.documentElement.getAttribute("data-title-font")).toBe("sentient");
    expect(document.documentElement.getAttribute("data-body-font")).toBe("instrument-sans");
    expect(screen.getByText(/light,custom-night/)).toBeTruthy();
    expect(document.querySelector("style[data-mew-host-themes]")?.textContent).toContain('--background: #102030');
    fireEvent.click(screen.getByText("System"));
    expect(screen.getByRole("status").textContent).toBe("light:light:system:sentient:instrument-sans");
    fireEvent.click(screen.getByText("Custom"));
    expect(screen.getByRole("status").textContent).toBe("custom-night:dark:dark:sentient:instrument-sans");
    expect(window.mewQuery).toHaveBeenLastCalledWith(expect.objectContaining({ request: JSON.stringify({ type: "set_theme", themeMode: "dark", lightTheme: "light", darkTheme: "custom-night" }) }));
    fireEvent.click(screen.getByText("Title"));
    fireEvent.click(screen.getByText("Body"));
    expect(screen.getByRole("status").textContent).toBe("custom-night:dark:dark:banga:mi-sans");
    expect(document.documentElement.getAttribute("data-title-font")).toBe("banga");
    expect(document.documentElement.getAttribute("data-body-font")).toBe("mi-sans");
  });
});
