import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { MASCOT_ALTERNATES, Mascot, mascotAssetPath, mascotStateForActivity } from "../components/mascot";

afterEach(() => cleanup());

describe("mascotStateForActivity", () => {
  it("maps tool calls to matching harness status animations", () => {
    expect(mascotStateForActivity({ runningToolName: "read" })).toBe("reading-files");
    expect(mascotStateForActivity({ runningToolName: "edit" })).toBe("editing-code");
    expect(mascotStateForActivity({ runningToolName: "write" })).toBe("editing-code");
    expect(mascotStateForActivity({ runningToolName: "grep" })).toBe("searching");
    expect(mascotStateForActivity({ runningToolName: "glob" })).toBe("searching");
    expect(mascotStateForActivity({ runningToolName: "web_fetch" })).toBe("fetching-url");
    expect(mascotStateForActivity({ runningToolName: "python" })).toBe("python");
    expect(mascotStateForActivity({ runningToolName: "browser_navigate" })).toBe("web-search");
    expect(mascotStateForActivity({ runningToolName: "bash", runningCommand: "cargo test -p mew-tools" })).toBe("running-tests");
    expect(mascotStateForActivity({ runningToolName: "bash", runningCommand: "git diff" })).toBe("reviewing-diff");
    expect(mascotStateForActivity({ runningToolName: "bash", runningCommand: "ls -la" })).toBe("shell-command");
  });

  it("maps reasoning and fallback activity", () => {
    expect(mascotStateForActivity({ streamingReasoning: true })).toBe("thinking");
    expect(mascotStateForActivity({})).toBe("busy");
  });
});

describe("Mascot", () => {
  it("resolves asset paths for status, alternates, idle, and neutral states", () => {
    expect(mascotAssetPath("thinking", "dark-ink")).toBe("/mascot/status/dark-ink/thinking.json");
    expect(mascotAssetPath("busy-alt", "light-ink")).toBe("/mascot/status/light-ink/busy-alt.json");
    expect(mascotAssetPath("yarn-bat", "light-ink")).toBe("/mascot/idle/light-ink/yarn-bat.json");
    expect(mascotAssetPath("neutral", "dark-ink")).toBe("/mascot/status/dark-ink/neutral.png");
    expect(Object.keys(MASCOT_ALTERNATES)).toHaveLength(8);
  });

  it("renders static neutral image when state is neutral", () => {
    const { container } = render(<Mascot state="neutral" size={48} ink="light-ink" />);
    const img = container.querySelector("img");
    expect(img).toBeTruthy();
    expect(img?.getAttribute("src")).toBe("/mascot/status/light-ink/neutral.png");
    expect(img?.getAttribute("aria-hidden")).toBe("true");
  });
});
