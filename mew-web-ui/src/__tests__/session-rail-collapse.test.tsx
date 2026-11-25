import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, act, cleanup, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { toast } from "sonner";
import type { SessionInfo } from "@mew/web-client";
import { useSessionStore } from "../stores/session";
import { SidebarProvider } from "../components/ui/sidebar";
import { SessionRail } from "../components/session-rail";
import { ProjectPickerModal } from "../components/session-rail";
import { setClient } from "../lib/client-ref";
import type { DirEntry } from "@mew/web-client";

const router = vi.hoisted(() => ({ navigate: vi.fn(), state: { location: { pathname: "/session/original" } } }));
vi.mock("@tanstack/react-router", () => ({ useRouter: () => router }));

function makeSession(overrides: Partial<SessionInfo> = {}): SessionInfo {
  return {
    session_id: "sess_test1",
    state: "idle",
    created_at: 1000,
    client_count: 0,
    ...overrides,
  };
}

function renderRail(client: Parameters<typeof SessionRail>[0]["client"] = null, defaultOpen = true) {
  return render(
    <SidebarProvider defaultOpen={defaultOpen}>
      <SessionRail client={client} />
    </SidebarProvider>,
  );
}

/** Choose Workspace from the labeled ViewSwitcher menu. */
async function switchToWorkspace() {
  const user = userEvent.setup();
  const trigger = screen.getAllByRole("button", { name: "mew menu" })[0]!;
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

  it("collapses completely offcanvas when closed", () => {
    useSessionStore.setState({ availableSessions: [makeSession({ first_message: "A chat title that should not clip", model: "fake" })] });
    const { container } = renderRail(null, false);
    const sidebar = container.querySelector('[data-slot="sidebar"]')!;
    expect(sidebar.getAttribute("data-state")).toBe("collapsed");
    expect(sidebar.getAttribute("data-collapsible")).toBe("offcanvas");
    expect(screen.queryByRole("textbox", { name: "Search sessions" })).toBeNull();
  });

  it("opens and focuses the real chat search when toggled", async () => {
    renderRail();
    fireEvent.click(screen.getByRole("button", { name: "Search chats" }));
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "Search sessions" })));
    fireEvent.change(screen.getByRole("textbox", { name: "Search sessions" }), { target: { value: "missing chat" } });
    expect(screen.getByText("No sessions match “missing chat”")).toBeTruthy();
  });

  it("clears a search filter when its control is closed", async () => {
    useSessionStore.setState({ availableSessions: [makeSession({ first_message: "Visible chat" })] });
    const user = userEvent.setup();
    renderRail();
    const search = screen.getByRole("button", { name: "Search chats" });
    await user.click(search);
    expect(search.getAttribute("aria-expanded")).toBe("true");
    await user.type(screen.getByRole("textbox", { name: "Search sessions" }), "missing");
    expect(screen.queryByRole("button", { name: "Visible chat" })).toBeNull();

    await user.click(search);
    expect(search.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("textbox", { name: "Search sessions" })).toBeNull();
    expect(screen.getByRole("button", { name: "Visible chat" })).toBeTruthy();
    await user.click(search);
    expect((screen.getByRole("textbox", { name: "Search sessions" }) as HTMLInputElement).value).toBe("");
  });

  it("closes search on Escape, restores its trigger focus and removes the filter", async () => {
    useSessionStore.setState({ availableSessions: [makeSession({ first_message: "Visible chat" })] });
    const user = userEvent.setup();
    renderRail();
    const search = screen.getByRole("button", { name: "Search chats" });
    await user.click(search);
    await user.type(screen.getByRole("textbox", { name: "Search sessions" }), "missing");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("textbox", { name: "Search sessions" })).toBeNull();
    expect(document.activeElement).toBe(search);
    expect(screen.getByRole("button", { name: "Visible chat" })).toBeTruthy();
  });

  it("shows one loading or empty status in every chat view", async () => {
    const user = userEvent.setup();
    renderRail();
    expect(screen.getByRole("status").textContent).toBe("No sessions yet");
    expect(screen.queryByText("Recent")).toBeNull();
    await switchToWorkspace();
    expect(screen.getByRole("status").textContent).toBe("No sessions yet");
    act(() => useSessionStore.setState({ sessionsLoading: true }));
    expect(screen.getByRole("status").textContent).toBe("Loading sessions…");
    await user.click(screen.getByRole("button", { name: "mew menu" }));
    await user.click(await screen.findByRole("menuitem", { name: "Grouped" }));
    expect(screen.getByRole("status").textContent).toBe("Loading sessions…");
  });

  it("creates a real new chat from the compact icon button", async () => {
    const newSession = vi.fn().mockResolvedValue("compact-new");
    useSessionStore.setState({ connectionState: "connected" });
    renderRail({ newSession, listSessions: vi.fn().mockResolvedValue([]) } as never, false);
    await userEvent.setup().click(screen.getByRole("button", { name: "New chat" }));
    expect(newSession).toHaveBeenCalledOnce();
  });

  it("does not navigate to a new chat after the user selects another page", async () => {
    let finish!: (id: string) => void;
    const newSession = vi.fn(() => new Promise<string>((resolve) => { finish = resolve; }));
    useSessionStore.setState({ connectionState: "connected" });
    router.navigate.mockClear();
    renderRail({ newSession, listSessions: vi.fn().mockResolvedValue([]) } as never, false);
    await userEvent.setup().click(screen.getByRole("button", { name: "New chat" }));
    router.state.location = { pathname: "/settings" };
    await act(async () => { finish("late-chat"); await Promise.resolve(); });
    expect(router.navigate).not.toHaveBeenCalled();
  });

  it("groups pinned chats ahead of recent chats in the notifications view", () => {
    useSessionStore.setState({ availableSessions: [makeSession({ session_id: "pinned", first_message: "Priority work", pinned: true }), makeSession({ session_id: "recent", first_message: "Recent work", created_at: Date.now() })] });
    renderRail();
    expect(screen.getByText("Priority")).toBeTruthy();
    expect(screen.getByText("Today")).toBeTruthy();
    expect(screen.getByRole("button", { name: "mew menu" })).toBeTruthy();
  });

  it("shows the workspace beside the title and a two-line summary in notifications", () => {
    useSessionStore.setState({ availableSessions: [makeSession({
      first_message: "Blume.town", cwd: "/projects/streamplace",
      summary: "Fixed both: selected Bluesky posting waits for permission.", state: "running",
    })] });
    renderRail();
    const row = screen.getByRole("button", { name: /^Blume\.town/ });
    const title = within(row).getByText("Blume.town");
    const workspace = within(row).getByText("streamplace");
    const summary = within(row).getByText("Fixed both: selected Bluesky posting waits for permission.");
    expect(title.closest(".mew-notification-title")?.parentElement).toBe(workspace.parentElement);
    expect(summary.className).toContain("mew-notification-summary");
    expect(row.querySelector(".lucide-folder")).toBeNull();
    expect(row.textContent).not.toMatch(/\d+(?:h|d|mo)$/);
  });

  it("falls back to last_message in notifications when summary is missing", () => {
    useSessionStore.setState({
      availableSessions: [
        makeSession({
          session_id: "fallback",
          first_message: "Initial prompt",
          last_message: "Latest assistant reply",
        }),
        makeSession({
          session_id: "single",
          first_message: "Only message",
          last_message: "Only message",
        }),
      ],
    });
    renderRail();
    const fallbackRow = screen.getByRole("button", { name: /^Initial prompt/ });
    const preview = within(fallbackRow).getByText("Latest assistant reply");
    expect(preview.className).toContain("mew-notification-summary");

    const singleRow = screen.getByRole("button", { name: /^Only message$/ });
    expect(within(singleRow).getAllByText("Only message")).toHaveLength(1);
  });

  it("prefers summary over last_message when both exist", () => {
    useSessionStore.setState({
      availableSessions: [
        makeSession({
          first_message: "Initial prompt",
          summary: "Short session summary",
          last_message: "Latest assistant reply",
        }),
      ],
    });
    renderRail();
    const row = screen.getByRole("button", { name: /^Initial prompt/ });
    expect(within(row).getByText("Short session summary")).toBeTruthy();
    expect(within(row).queryByText("Latest assistant reply")).toBeNull();
  });

  it("scrolls overflowing titles back and forth only on hover at constant speed", () => {
    vi.useFakeTimers();
    const clientWidthSpy = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (this: HTMLElement) {
      return this.className.includes("mew-notification-title") ? 100 : 0;
    });
    const scrollWidthSpy = vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockImplementation(function (this: HTMLElement) {
      return this.textContent === "A very long conversation title that overflows" ? 180 : 60;
    });
    try {
      useSessionStore.setState({
        availableSessions: [
          makeSession({ session_id: "long", title: "A very long conversation title that overflows" }),
          makeSession({ session_id: "short", title: "Short title" }),
        ],
      });
      renderRail();

      const longText = screen.getByText("A very long conversation title that overflows");
      const longRow = longText.closest(".group\\/chat-row")!;
      expect(longText.style.transform).toBe("translateX(0px)");
      expect(longText.style.transition).toBe("none");

      fireEvent.mouseEnter(longRow);
      act(() => {
        vi.advanceTimersByTime(150);
      });
      expect(longText.style.transform).toBe("translateX(-80px)");
      expect(longText.style.transition).toBe("transform 2000ms linear");

      act(() => {
        vi.advanceTimersByTime(2700);
      });
      expect(longText.style.transform).toBe("translateX(0px)");
      expect(longText.style.transition).toBe("transform 2000ms linear");

      act(() => {
        vi.advanceTimersByTime(2700);
      });
      expect(longText.style.transform).toBe("translateX(-80px)");

      fireEvent.mouseLeave(longRow);
      expect(longText.style.transform).toBe("translateX(0px)");
      expect(longText.style.transition).toBe("transform 200ms ease-out");

      const shortText = screen.getByText("Short title");
      expect(shortText.style.transform).toBe("");
      expect(shortText.style.transition).toBe("");
    } finally {
      vi.useRealTimers();
      clientWidthSpy.mockRestore();
      scrollWidthSpy.mockRestore();
    }
  });

  it("labels recent chat groups by their local calendar date", () => {
    const yesterday = new Date(); yesterday.setDate(yesterday.getDate() - 1);
    const weekday = new Date(); weekday.setDate(weekday.getDate() - 3);
    const older = new Date(); older.setDate(older.getDate() - 10);
    useSessionStore.setState({ availableSessions: [makeSession({ session_id: "yesterday", created_at: yesterday.getTime() }), makeSession({ session_id: "weekday", created_at: weekday.getTime() }), makeSession({ session_id: "older", created_at: older.getTime() })] });
    renderRail();
    expect(screen.getByText("Yesterday")).toBeTruthy();
    expect(screen.getByText(weekday.toLocaleDateString(undefined, { weekday: "long" }))).toBeTruthy();
    expect(screen.getByText(older.toLocaleDateString(undefined, { month: "short", day: "numeric" }))).toBeTruthy();
  });

  it("reloads the canonical chat list after a new chat is confirmed", async () => {
    const listSessions = vi.fn().mockResolvedValueOnce([]).mockResolvedValue([makeSession({ session_id: "confirmed-chat", first_message: "A real new chat" })]);
    useSessionStore.setState({ connectionState: "connected" });
    renderRail({ listSessions } as never);
    await waitFor(() => expect(screen.getByText("No sessions yet")).toBeTruthy());
    act(() => useSessionStore.getState().setSessionId("confirmed-chat"));
    await waitFor(() => expect(screen.getByRole("button", { name: /^A real new chat$/ })).toBeTruthy());
    expect(listSessions).toHaveBeenCalledTimes(2);
  });

  it("keeps chat management actions in the row menu", async () => {
    const pinSession = vi.fn();
    useSessionStore.setState({ availableSessions: [makeSession({ first_message: "Review the UI", cwd: "/projects/mew" })] });
    renderRail({ pinSession } as never);
    expect(screen.queryByRole("button", { name: "Pin session" })).toBeNull();
    await userEvent.setup().click(screen.getByRole("button", { name: "Chat actions: Review the UI" }));
    await userEvent.setup().click(screen.getByRole("menuitem", { name: "Pin chat" }));
    expect(pinSession).toHaveBeenCalledWith("sess_test1", true);
  });

  it("exposes the current chat and updates it when selection changes", () => {
    useSessionStore.setState({ sessionId: "first", availableSessions: [makeSession({ session_id: "first", first_message: "First chat" }), makeSession({ session_id: "second", first_message: "Second chat" })] });
    renderRail();
    expect(screen.getByRole("button", { name: "First chat" }).getAttribute("aria-current")).toBe("page");
    expect(screen.getByRole("button", { name: "Second chat" }).hasAttribute("aria-current")).toBe(false);
    act(() => useSessionStore.setState({ sessionId: "second" }));
    expect(screen.getByRole("button", { name: "First chat" }).hasAttribute("aria-current")).toBe(false);
    expect(screen.getByRole("button", { name: "Second chat" }).getAttribute("aria-current")).toBe("page");
  });

  it("preserves the current session and surfaces a toast if creating a new chat fails", async () => {
    const toastError = vi.spyOn(toast, "error").mockImplementation(() => "");
    const newSession = vi.fn().mockRejectedValue(
      new Error("failed to create session: credential not found: opencode-zen"),
    );
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

    try {
      renderRail({ newSession, listSessions: vi.fn(() => Promise.resolve([])) } as never);
      await userEvent.setup().click(screen.getByRole("button", { name: "New chat" }));

      expect(newSession).toHaveBeenCalledOnce();
      const state = useSessionStore.getState();
      expect(state.sessionId).toBe("sess_old");
      expect(state.messages).toHaveLength(1);
      expect(state.messages[0]?.parts).toEqual([{ type: "text", text: "old prompt" }]);
      expect(toastError).toHaveBeenCalledWith(
        "failed to create session: credential not found: opencode-zen",
      );
    } finally {
      toastError.mockRestore();
    }
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

  it("exposes the complete workspace header as a keyboard disclosure", async () => {
    useSessionStore.setState({ availableSessions: [makeSession({ cwd: "/projects/alpha", first_message: "Workspace chat" })] });
    const user = userEvent.setup();
    renderRail();
    await switchToWorkspace();
    const header = screen.getByRole("button", { name: "Collapse workspace alpha" });
    expect(header.getAttribute("aria-expanded")).toBe("true");
    expect(header.textContent).toContain("alpha");
    header.focus();
    await user.keyboard("{Enter}");
    expect(screen.queryByRole("button", { name: "Workspace chat alpha" })).toBeNull();
    const collapsed = screen.getByRole("button", { name: "Expand workspace alpha" });
    expect(collapsed.getAttribute("aria-expanded")).toBe("false");
    await user.keyboard(" ");
    expect(screen.getByRole("button", { name: "Collapse workspace alpha" }).getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("Workspace chat")).toBeTruthy();
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
