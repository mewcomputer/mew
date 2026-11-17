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

afterEach(() => { cleanup(); localStorage.clear(); vi.clearAllMocks(); fixtures.router.state.location.pathname = "/"; });

it("does not reopen an older session after the user navigates elsewhere", async () => {
  let finish!: (id: string) => void;
  fixtures.client.attachSession.mockReturnValue(new Promise<string>((resolve) => { finish = resolve; }));
  localStorage.setItem("mew.sessionId", "previous");
  useSessionStore.setState({ connectionState: "connected" });
  const Component = Route.options.component!;
  render(<Component />);
  await waitFor(() => expect(fixtures.client.attachSession).toHaveBeenCalledWith("previous"));
  fixtures.router.state.location.pathname = "/session/selected";
  await act(async () => { finish("previous"); await Promise.resolve(); });
  expect(fixtures.router.navigate).not.toHaveBeenCalled();
});
