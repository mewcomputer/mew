import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ModelPill } from "../components/model-pill";
import { useSessionStore } from "../stores/session";

const { switchModel, setThinkingVariant } = vi.hoisted(() => ({ switchModel: vi.fn(), setThinkingVariant: vi.fn() }));
vi.mock("../lib/client-ref", () => ({ getClient: () => ({ switchModel, setThinkingVariant }) }));

beforeEach(() => {
  switchModel.mockReset();
  setThinkingVariant.mockReset();
  useSessionStore.setState({
    sessionId: "model-chat", currentProvider: "test", currentModel: "small", currentThinkingVariant: null,
    availableModels: [
      { id: "test/small", provider: "test", model: "small", thinking_variants: [{ name: "high" }] },
      { id: "test/large", provider: "test", model: "large", description: "For larger tasks" },
    ],
  });
});
afterEach(cleanup);

describe("model picker", () => {
  it("closes on chat navigation and does not carry a pending failure into the next chat", async () => {
    const user = userEvent.setup();
    let fail!: (error: Error) => void;
    switchModel.mockImplementation(() => new Promise((_, reject) => { fail = reject; }));
    render(<ModelPill />);
    await user.click(screen.getByTitle("Switch model"));
    await user.click(screen.getByRole("menuitemradio", { name: /large/ }));
    act(() => useSessionStore.setState({ sessionId: "next-model-chat" }));
    expect(screen.queryByRole("menu")).toBeNull();
    await act(async () => fail(new Error("Previous chat failed")));
    await user.click(screen.getByTitle("Switch model"));
    expect(screen.queryByRole("alert")).toBeNull();
  });
  it("supports keyboard navigation and returns focus when dismissed", async () => {
    const user = userEvent.setup();
    render(<ModelPill />);
    const trigger = screen.getByTitle("Switch model");
    trigger.focus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("menuitemradio", { name: "small" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByRole("menuitemradio", { name: /large/ })).toBeTruthy();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("keeps the confirmed model and offers a retry after a failed switch", async () => {
    const user = userEvent.setup();
    switchModel.mockRejectedValueOnce(new Error("Model unavailable"));
    switchModel.mockResolvedValueOnce({ provider: "test", model: "large" });
    render(<ModelPill />);
    await user.click(screen.getByTitle("Switch model"));
    await user.click(screen.getByRole("menuitemradio", { name: /large/ }));
    expect((await screen.findByRole("alert")).textContent).toContain("Model unavailable");
    expect(screen.getByTitle("Switch model").textContent).toContain("small");
    await user.click(screen.getByRole("menuitemradio", { name: /large/ }));
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    expect(screen.getByTitle("Switch model").textContent).toContain("large");
  });
});
