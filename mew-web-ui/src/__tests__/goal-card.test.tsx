import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MewClient, type MewWebSocket } from "@mew/web-client";
import { GoalCard } from "../components/goal-card";
import { setClient } from "../lib/client-ref";
import { bridgeClientToStore, useSessionStore } from "../stores/session";

describe("proposed goals", () => {
  afterEach(() => {
    cleanup();
    setClient(null);
    useSessionStore.getState().reset();
  });

  it.each([true, false])("sends the decision %s and removes the submitted goal", async (accepted) => {
    const sent: unknown[] = [];
    const events = new EventTarget();
    const socket: MewWebSocket = {
      send: (data) => { sent.push(JSON.parse(data)); },
      close: (code = 1000, reason = "") => {
        events.dispatchEvent(new CloseEvent("close", { code, reason }));
      },
      addEventListener: (type, listener) => events.addEventListener(type, listener as EventListener),
      removeEventListener: (type, listener) => events.removeEventListener(type, listener as EventListener),
    };
    const client = new MewClient("ws://test/", { socketFactory: () => socket });
    bridgeClientToStore(client);
    setClient(client);
    const connected = client.connect();
    events.dispatchEvent(new Event("open"));
    await connected;
    events.dispatchEvent(new MessageEvent("message", {
      data: JSON.stringify({ type: "goal_proposed", request_id: "g", call_id: "c", objective: "finish the task" }),
    }));

    render(<GoalCard />);
    expect(screen.getByText("finish the task").textContent).toBe("finish the task");
    fireEvent.click(screen.getByRole("button", { name: accepted ? "Accept" : "Reject" }));

    expect(sent).toEqual([{ type: "goal_response", request_id: "g", accepted }]);
    expect(screen.queryByText("finish the task")).toBeNull();
    expect(useSessionStore.getState().pendingGoals).toEqual([]);
  });

  it("keeps a goal available when no client can send the decision", () => {
    useSessionStore.getState().onGoalProposed({ request_id: "g", call_id: "c", objective: "finish the task" });
    render(<GoalCard />);

    fireEvent.click(screen.getByRole("button", { name: "Accept" }));

    expect(useSessionStore.getState().pendingGoals).toHaveLength(1);
    expect(screen.getByText("finish the task").textContent).toBe("finish the task");
  });
});
