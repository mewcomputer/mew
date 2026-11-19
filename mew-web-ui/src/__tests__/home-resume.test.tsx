import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { useSessionStore } from "../stores/session";

const fixtures = vi.hoisted(() => ({
  client: { attachSession: vi.fn() },
  router: { navigate: vi.fn(), state: { location: { pathname: "/" } } },
}));
vi.mock("@tanstack/react-router", () => ({
  createFileRoute: () => (options: unknown) => ({ options }),
  useRouter: () => fixtures.router,
}));
vi.mock("../lib/client", () => ({ getClient: () => fixtures.client, SESSION_ID_KEY: "mew.sessionId" }));
vi.mock("../components/conversation-layout", () => ({ ConversationLayout: ({ children }: { children: ReactNode }) => children }));
vi.mock("../components/input-area", () => ({ InputArea: () => null }));

import { Route } from "../routes/index";

afterEach(() => { cleanup(); localStorage.clear(); vi.resetAllMocks(); fixtures.router.state.location = { pathname: "/" }; });

it("does not reopen an older session after the user navigates elsewhere", async () => {
  let finish!: (id: string) => void;
  fixtures.client.attachSession.mockReturnValue(new Promise<string>((resolve) => { finish = resolve; }));
  localStorage.setItem("mew.sessionId", "previous");
  useSessionStore.setState({ connectionState: "connected" });
  const Component = Route.options.component!;
  render(<Component />);
  await waitFor(() => expect(fixtures.client.attachSession).toHaveBeenCalledWith("previous"));
  fixtures.router.state.location = { pathname: "/" };
  await act(async () => { finish("previous"); await Promise.resolve(); });
  expect(fixtures.router.navigate).not.toHaveBeenCalled();
});

it("retries reopening the last session if the socket closes before attach is confirmed", async () => {
  let rejectAttach!: (error: Error) => void;
  fixtures.client.attachSession
    .mockReturnValueOnce(new Promise<string>((_, reject) => { rejectAttach = reject; }))
    .mockResolvedValueOnce("previous");
  localStorage.setItem("mew.sessionId", "previous");
  useSessionStore.setState({ connectionState: "connected" });
  const Component = Route.options.component!;
  render(<Component />);
  await waitFor(() => expect(fixtures.client.attachSession).toHaveBeenCalledTimes(1));

  await act(async () => {
    useSessionStore.setState({ connectionState: "reconnecting" });
    rejectAttach(new Error("Connection closed"));
  });
  await act(async () => useSessionStore.setState({ connectionState: "connected" }));

  expect(fixtures.client.attachSession).toHaveBeenCalledTimes(2);
  expect(fixtures.router.navigate).toHaveBeenCalledWith({ to: "/session/$sessionId", params: { sessionId: "previous" } });
});
