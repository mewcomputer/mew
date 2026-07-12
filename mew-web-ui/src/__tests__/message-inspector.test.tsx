import { describe, it, expect, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { MessageInspector } from "../components/message-inspector";
import type { TurnManifest } from "@mew/web-client";

function makeManifest(overrides: Partial<TurnManifest> = {}): TurnManifest {
  return {
    model: "gpt-4o",
    context_window: 128000,
    input_tokens: 9700,
    output_tokens: 238,
    cache_read_tokens: 5000,
    cache_write_tokens: 0,
    reasoning_tokens: 0,
    segments: [
      {
        label: "scaffold",
        kind: "scaffold",
        tokens: 300,
        tokens_scaled: 300,
        children: [],
      },
      {
        label: "tools (5)",
        kind: "tools",
        tokens: 2000,
        tokens_scaled: 2000,
        children: [
          {
            label: "bash",
            kind: "tools",
            tokens: 400,
            tokens_scaled: 400,
            children: [],
          },
        ],
      },
      {
        label: "history (3 messages)",
        kind: "message",
        tokens: 7400,
        tokens_scaled: 7400,
        children: [
          {
            label: "user",
            kind: "message",
            tokens: 100,
            tokens_scaled: 100,
            children: [],
          },
        ],
      },
    ],
    ...overrides,
  };
}

/** Find the inspector's expand trigger (first button with aria-expanded). */
function getExpandTrigger() {
  const buttons = screen.getAllByRole("button");
  const trigger = buttons.find((b) => b.hasAttribute("aria-expanded"));
  return trigger ?? buttons[0]!;
}

/** Expand the inspector. */
async function expandInspector() {
  const trigger = getExpandTrigger();
  fireEvent.click(trigger);
}

describe("MessageInspector", () => {
  afterEach(cleanup);

  it("renders summary line with ~ prefix and token counts", () => {
    const manifest = makeManifest();
    render(<MessageInspector manifest={manifest} />);

    // Should show input tokens with ~ prefix
    expect(screen.getByText(/~9\.7k ↓/)).toBeTruthy();
    // Should show output tokens
    expect(screen.getByText(/~238 ↑/)).toBeTruthy();
    // Should show utilization percentage
    expect(screen.getByText(/8%/)).toBeTruthy();
  });

  it("shows 'error · structure below' when input_tokens is undefined", () => {
    const manifest = makeManifest({
      input_tokens: undefined,
      output_tokens: undefined,
      cache_read_tokens: undefined,
    });
    render(<MessageInspector manifest={manifest} />);

    expect(screen.getByText("error · structure below")).toBeTruthy();
  });

  it("expands to show segment tree on click", async () => {
    const manifest = makeManifest();
    render(<MessageInspector manifest={manifest} />);

    // Before expanding, segment labels should not be visible
    expect(screen.queryByText("scaffold")).toBeNull();

    // Click the expand button
    await expandInspector();

    // After expanding, segment labels should be visible
    expect(screen.getByText("scaffold")).toBeTruthy();
    expect(screen.getByText("tools (5)")).toBeTruthy();
    expect(screen.getByText("history (3 messages)")).toBeTruthy();
  });

  it("shows segment tree with token counts when expanded", async () => {
    const manifest = makeManifest();
    render(<MessageInspector manifest={manifest} />);

    // Expand
    await expandInspector();

    // Segment labels should be visible
    expect(screen.getByText("scaffold")).toBeTruthy();
    expect(screen.getByText("tools (5)")).toBeTruthy();
    // Token counts should be visible (with ~ prefix)
    expect(screen.getByText(/~300/)).toBeTruthy();
    expect(screen.getByText(/~2\.0k/)).toBeTruthy();
  });

  it("expands child segments when parent is clicked", async () => {
    const manifest = makeManifest();
    render(<MessageInspector manifest={manifest} />);

    // Expand the inspector
    await expandInspector();

    // "tools (5)" should be visible but "bash" (child) should not yet
    expect(screen.getByText("tools (5)")).toBeTruthy();
    expect(screen.queryByText("bash")).toBeNull();

    // Find the "tools (5)" row and click its expand button
    const toolsLabel = screen.getByText("tools (5)");
    const toolsRow = toolsLabel.closest("div");
    const expandButton = toolsRow?.querySelector("button");
    expect(expandButton).toBeTruthy();
    fireEvent.click(expandButton!);

    // Now "bash" should be visible
    expect(screen.getByText("bash")).toBeTruthy();
  });
});
