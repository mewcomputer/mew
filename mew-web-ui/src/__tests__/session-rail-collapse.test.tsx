import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, act, cleanup, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { SessionInfo } from "@mew/web-client";
import { useSessionStore } from "../stores/session";
import { SidebarProvider } from "../components/ui/sidebar";
import { SessionRail } from "../components/session-rail";
import { ProjectPickerModal } from "../components/session-rail";
import { setClient } from "../lib/client-ref";
import type { DirEntry } from "@mew/web-client";

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

function renderRail(client: Parameters<typeof SessionRail>[0]["client"] = null) {
  return render(
    <SidebarProvider>
      <SessionRail client={client} />
    </SidebarProvider>,
  );
}

/** Choose Workspace from the labeled ViewSwitcher menu. */
async function switchToWorkspace() {
  const user = userEvent.setup();
  const trigger = screen.getAllByRole("button", { name: "Session view: Timeline" })[0]!;
  await user.click(trigger);
  await user.click(await screen.findByRole("menuitem", { name: "Workspace" }));
}

describe("SessionRail workspace collapse", () => {
  afterEach(cleanup);

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

  it("reserves space for session actions so they cannot cover the title", () => {
    useSessionStore.setState({
      availableSessions: [
        makeSession({
          session_id: "sess_actions",
          first_message: "can you create a function to reverse this?",
          cwd: "/projects/mew",
        }),
      ],
      connectionState: "connected",
    });

    renderRail();

    const title = screen.getByText("can you create a function to reverse this?");
    const titleRow = title.parentElement?.parentElement;
    expect(titleRow?.className).toContain("pr-24");
  });

  it("clears the previous session before creating a new one", async () => {
    const newSession = vi.fn(async () => {
      expect(useSessionStore.getState().sessionId).toBeNull();
      expect(useSessionStore.getState().messages).toHaveLength(0);
      return "sess_new";
    });
    useSessionStore.setState({
      sessionId: "sess_old",
      messages: [{
        id: "message-1",
        role: "user",
        parts: [{ type: "text", text: "old prompt" }],
        timestamp: 1,
      }],
      connectionState: "connected",
    });

    renderRail({ newSession, listSessions: vi.fn(() => Promise.resolve([])) } as never);
    await userEvent.setup().click(screen.getByRole("button", { name: "New session" }));

    expect(newSession).toHaveBeenCalledOnce();
  });

  it("switching to workspace view shows folder headers with session counts", async () => {
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
    await switchToWorkspace();

    // The folder header should show the count (1).
    expect(screen.getByText(/\(1\)/)).toBeTruthy();
  });

  it("clicking a folder header toggles session visibility", async () => {
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
    await switchToWorkspace();

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

describe("ProjectPickerModal folder browser", () => {
  afterEach(() => {
    cleanup();
    setClient(null);
  });

  it("loads folders, navigates down, and returns to the parent folder", async () => {
    const listeners = new Set<(data: { path: string; entries: DirEntry[] }) => void>();
    const listFilesystemDir = vi.fn();
    const client = {
      on: vi.fn((_event: string, listener: (data: { path: string; entries: DirEntry[] }) => void) => listeners.add(listener)),
      off: vi.fn((_event: string, listener: (data: { path: string; entries: DirEntry[] }) => void) => listeners.delete(listener)),
      listFilesystemDir,
    };
    setClient(client as never);

    render(
      <ProjectPickerModal
        open
        onOpenChange={vi.fn()}
        projects={[]}
        onSelect={vi.fn()}
      />,
    );

    await userEvent.setup().click(screen.getByRole("button", { name: /Browse folders/ }));
    await waitFor(() => expect(listFilesystemDir).toHaveBeenCalledWith(undefined));
    listeners.forEach((listener) => listener({ path: "/Users/tester", entries: [{ name: "projects", is_dir: true, size: undefined }] }));
    await waitFor(() => expect(screen.getByRole("button", { name: "projects" })).toBeTruthy());
    await userEvent.setup().click(screen.getByRole("button", { name: "projects" }));
    expect(listFilesystemDir).toHaveBeenCalledWith("/Users/tester/projects");

    listeners.forEach((listener) => listener({ path: "/Users/tester/projects", entries: [] }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Go to parent folder" }).getAttribute("disabled")).toBeNull());
    await userEvent.setup().click(screen.getByRole("button", { name: "Go to parent folder" }));
    expect(listFilesystemDir).toHaveBeenCalledWith("/Users/tester");
  });
});
