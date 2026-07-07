import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import type { SessionInfo } from "@mew/web-client";
import { useSessionStore } from "../stores/session";
import { SidebarProvider } from "../components/ui/sidebar";
import { SessionRail } from "../components/session-rail";

// Mock TanStack Router — SessionRail calls useRouter().navigate.
vi.mock("@tanstack/react-router", () => ({
  useRouter: () => ({
    navigate: vi.fn(),
  }),
}));

function makeSession(overrides: Partial<SessionInfo> = {}): SessionInfo {
  return {
    session_id: "sess_test1",
    state: "idle",
    created_at: 1000,
    client_count: 0,
    ...overrides,
  };
}

function renderRail() {
  return render(
    <SidebarProvider>
      <SessionRail client={null} />
    </SidebarProvider>,
  );
}

/** Click the "W" button in the ViewSwitcher to switch to workspace view. */
function switchToWorkspace() {
  const wsButtons = screen.getAllByText("W");
  fireEvent.click(wsButtons[wsButtons.length - 1]!);
}

describe("SessionRail workspace collapse", () => {
  beforeEach(() => {
    useSessionStore.setState({
      availableSessions: [],
      sessionsLoading: false,
      sessionId: null,
      sessionTitles: new Map(),
      connectionState: "disconnected",
      groups: [],
      projects: [],
      sessionSummaries: new Map(),
    });
  });

  it("switching to workspace view shows folder headers with session counts", () => {
    const sessions = [
      makeSession({
        session_id: "sess_a",
        cwd: "/projects/alpha",
        model: "test/model-a",
        created_at: 1000,
        last_message_at: 5000,
      }),
    ];
    useSessionStore.setState({
      availableSessions: sessions,
      connectionState: "connected",
    });

    renderRail();
    switchToWorkspace();

    // The folder header should show the count (1).
    expect(screen.getByText(/\(1\)/)).toBeTruthy();
  });

  it("clicking a folder header toggles session visibility", () => {
    const sessions = [
      makeSession({
        session_id: "sess_a",
        cwd: "/projects/alpha",
        model: "test/model-a",
        first_message: "Hello world test message",
        created_at: 1000,
        last_message_at: 5000,
      }),
    ];
    useSessionStore.setState({
      availableSessions: sessions,
      connectionState: "connected",
    });

    const { container } = renderRail();
    switchToWorkspace();

    // The session's first_message should be visible (rendered as the title).
    expect(screen.getAllByText("Hello world test message").length).toBeGreaterThan(0);

    // Click the folder header chevron button to collapse.
    const folderButton = container.querySelector(
      'button[title="/projects/alpha"]',
    );
    expect(folderButton).not.toBeNull();
    act(() => { fireEvent.click(folderButton!); });

    // After collapse, SidebarGroupContent should not be rendered.
    expect(container.querySelectorAll("[data-sidebar='group-content']")).toHaveLength(0);

    // Click again to expand.
    act(() => { fireEvent.click(folderButton!); });
    expect(container.querySelectorAll("[data-sidebar='group-content']").length).toBeGreaterThan(0);
  });
});
