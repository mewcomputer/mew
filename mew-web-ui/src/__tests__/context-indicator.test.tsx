import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ContextIndicator } from "../components/session-status";
import { useSessionStore } from "../stores/session";

beforeEach(() => useSessionStore.setState({
  connectionState: "connected",
  contextTokens: 0,
  contextTokensKnown: false,
  contextWindow: 0,
  totalCost: 0,
  attachedClients: [],
  pendingPermissions: [],
}));
afterEach(() => cleanup());

describe("ContextIndicator", () => {
  it("reports an unknown context window without inventing a percentage", () => {
    render(<ContextIndicator />);
    expect(screen.getByRole("button", { name: "Context usage unknown, show session details" })).toBeTruthy();
  });

  it("reports the used share of the model's context window", () => {
    useSessionStore.setState({ contextTokens: 42_000, contextTokensKnown: true, contextWindow: 200_000 });
    render(<ContextIndicator />);
    const button = screen.getByRole("button", { name: "Context 21% used, show session details" });
    expect(button.getAttribute("title")).toBe("Context 42.0k / 200.0k · 21%");
  });

  it("opens the session details that used to hide behind the composer dots", async () => {
    const user = userEvent.setup();
    useSessionStore.setState({ contextTokens: 10_000, contextTokensKnown: true, contextWindow: 100_000, totalCost: 0.1234 });
    render(<ContextIndicator />);
    await user.click(screen.getByRole("button", { name: /show session details/ }));
    expect(screen.getByRole("button", { name: /show session details/ }).getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("connected")).toBeTruthy();
    expect(screen.getByText("$0.1234")).toBeTruthy();
  });

  it("disappears when there is no session to report on", () => {
    useSessionStore.setState({ connectionState: "disconnected" });
    render(<ContextIndicator />);
    expect(screen.queryByRole("button")).toBeNull();
  });
});
