import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MarkdownBody } from "../components/markdown-body";
import { useSessionStore } from "../stores/session";

const { listeners, clientMock } = vi.hoisted(() => {
  type PreviewData = { session_id?: string; path: string; error?: string };
  type Handler = (data: PreviewData) => void;
  const handlers = new Map<string, Set<Handler>>();
  return {
    listeners: handlers,
    clientMock: {
      on: vi.fn((event: string, handler: Handler) => {
        const set = handlers.get(event) ?? new Set<Handler>();
        set.add(handler);
        handlers.set(event, set);
      }),
      off: vi.fn((event: string, handler: Handler) => { handlers.get(event)?.delete(handler); }),
      readFilePreview: vi.fn(),
    },
  };
});
vi.mock("../lib/client-ref", () => ({ getClient: () => clientMock }));

afterEach(() => {
  cleanup();
  listeners.clear();
  clientMock.readFilePreview.mockClear();
  useSessionStore.setState({ sessionId: null, sessionCwd: null, connectionState: "disconnected" });
});

function preview(sessionId: string, path: string, error?: string) {
  const data = { session_id: sessionId, path, error };
  act(() => listeners.get("file-preview")?.forEach((handler) => handler(data)));
}

describe("markdown code blocks", () => {
  it("renders one highlighted fence between prose with copy controls", () => {
    const { container } = render(<div className="typeset"><MarkdownBody highlight>{"Before\n\n```rust\nfn main() {}\n```\n\nAfter with `inline` code."}</MarkdownBody></div>);
    const block = container.querySelector('[data-markdown="code-block"][data-language="rust"]');
    expect(block).toBeTruthy();
    expect(container.querySelectorAll("pre")).toHaveLength(1);
    expect(block?.querySelector("pre pre, pre div")).toBeNull();
    expect(block?.querySelector('code span[style*="light-dark"]')).toBeTruthy();
    expect(screen.getByText("Before")).toBeTruthy();
    expect(screen.getByText("inline").tagName).toBe("CODE");
    expect(screen.getByRole("button", { name: "Copy code" })).toBeTruthy();
  });

  it("renders unlabelled one-line fences as blocks while streaming", () => {
    const { container } = render(<MarkdownBody>{"```\nhello\n```"}</MarkdownBody>);
    expect(container.querySelectorAll("pre")).toHaveLength(1);
    expect(container.querySelector("pre pre, pre div")).toBeNull();
    expect(container.querySelector("pre")?.textContent).toBe("hello\n");
  });
});

describe("file references", () => {
  it("becomes a chip only when the daemon can preview the path", async () => {
    useSessionStore.setState({ sessionId: "chat", connectionState: "connected" });
    const opened: string[] = [];
    const listener = (event: Event) => opened.push((event as CustomEvent<{ path: string }>).detail.path);
    window.addEventListener("mew:open-file", listener);
    try {
      render(<MarkdownBody highlight>{"installed lottie-web in :file[present.json]{path=\"present.json\"} and created :file[missing.tsx]{path=\"missing.tsx\"}"}</MarkdownBody>);
      await waitFor(() => expect(clientMock.readFilePreview).toHaveBeenCalledWith("chat", "present.json", 1));
      expect(listeners.has("file-preview")).toBe(true);
      expect(screen.queryByRole("button", { name: "Open present.json" })).toBeNull();
      preview("chat", "present.json");
      await waitFor(() => expect(screen.getByRole("button", { name: "Open present.json" })).toBeTruthy());
      preview("chat", "missing.tsx", "no such file");
      expect(screen.queryByRole("button", { name: "Open missing.tsx" })).toBeNull();
      const chip = screen.getByRole("button", { name: "Open present.json" });
      expect(chip.querySelector("svg")).toBeTruthy();
      fireEvent.click(chip);
      expect(opened).toEqual(["present.json"]);
    } finally {
      window.removeEventListener("mew:open-file", listener);
    }
  });

  it("probes an absolute path as given", async () => {
    useSessionStore.setState({ sessionId: "chat", sessionCwd: "/work", connectionState: "connected" });
    render(<MarkdownBody highlight>{"see :file[absolute.md]{path=\"/work/absolute.md\"}"}</MarkdownBody>);
    await waitFor(() => expect(clientMock.readFilePreview).toHaveBeenCalledWith("chat", "/work/absolute.md", 1));
    preview("chat", "/work/absolute.md");
    await waitFor(() => expect(screen.getByRole("button", { name: "Open /work/absolute.md" })).toBeTruthy());
  });
});
