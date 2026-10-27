import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, act, cleanup } from "@testing-library/react";
import { InputArea } from "../components/input-area";
import { useSessionStore } from "../stores/session";
import type { PersonaInfo } from "@mew/web-client";

// Mock getClient — the store imports from client-ref
const switchPersonaMock = vi.fn();
vi.mock("../lib/client-ref", () => ({
  getClient: () => ({
    switchPersona: switchPersonaMock,
    listPersonas: vi.fn(),
  }),
}));

// Mock useSidebar
vi.mock("../components/ui/sidebar", () => ({
  useSidebar: () => ({ isMobile: false }),
}));

const mockPersonas: PersonaInfo[] = [
  { name: "code-reviewer", description: "Reviews code", active: false },
  { name: "explainer", description: "Explains things", active: false },
];

function renderInputArea(overrides: Partial<{ onSend: ReturnType<typeof vi.fn>; connected: boolean }> = {}) {
  const onSend = overrides.onSend ?? vi.fn();
  const onCancel = vi.fn();
  const result = render(
    <InputArea
      onSend={onSend}
      onCancel={onCancel}
      connected={overrides.connected ?? true}
    />,
  );
  return { ...result, onSend };
}

describe("InputArea", () => {
  let chatIndex = 0;
  afterEach(cleanup);
  beforeEach(() => {
    useSessionStore.setState({
      sessionId: `composer-test-${++chatIndex}`,
      availablePersonas: [],
      currentPersona: null,
      streamingPartId: null,
      promptHistory: [],
    });
    switchPersonaMock.mockClear();
  });

  it("keeps composer controls inside the primary composer surface", () => {
    renderInputArea();

    expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "Message prompt" }));
    const surface = screen.getByTestId("composer-surface");
    expect(surface.contains(screen.getByTitle("Persona"))).toBe(true);
    expect(surface.contains(screen.getByTitle("Switch model"))).toBe(true);
    expect(surface.contains(screen.getByTitle("Send"))).toBe(true);
  });

  it("keeps unsent text and attachments owned by their chat across navigation", () => {
    useSessionStore.setState({ sessionId: "draft-a" });
    const { container } = renderInputArea();
    fireEvent.change(container.querySelector("textarea")!, { target: { value: "keep this draft" } });
    fireEvent.change(container.querySelector('input[type="file"]')!, { target: { files: [new File(["hello"], "draft.txt", { type: "text/plain" })] } });
    act(() => useSessionStore.setState({ sessionId: "draft-b" }));
    fireEvent.change(container.querySelector("textarea")!, { target: { value: "another chat" } });
    act(() => useSessionStore.setState({ sessionId: "draft-a" }));
    expect((container.querySelector("textarea") as HTMLTextAreaElement).value).toBe("keep this draft");
    expect(screen.getByText("draft.txt")).toBeTruthy();
  });

  it("sends with Enter, keeps Shift+Enter for a newline, and ignores composing Enter", () => {
    const { container, onSend } = renderInputArea();
    const input = container.querySelector("textarea")!;
    fireEvent.change(input, { target: { value: "hello" } });
    fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    expect(onSend).not.toHaveBeenCalled();
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    expect(onSend).not.toHaveBeenCalled();
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("hello", undefined);
  });

  it("retains the draft until an asynchronous send succeeds and displays a failed send", async () => {
    let rejectSend!: (error: Error) => void;
    const onSend = vi.fn(() => new Promise<void>((_, reject) => { rejectSend = reject; }));
    const { container } = renderInputArea({ onSend });
    const input = container.querySelector("textarea")!;
    fireEvent.change(input, { target: { value: "keep my prompt" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(input.value).toBe("keep my prompt");
    await act(async () => rejectSend(new Error("Could not create the chat")));
    expect(input.value).toBe("keep my prompt");
    expect(screen.getByRole("alert").textContent).toContain("Could not create the chat");
  });

  it("shows send progress and keeps text edited while the previous prompt is pending", async () => {
    let finish!: () => void;
    const onSend = vi.fn(() => new Promise<void>((resolve) => { finish = resolve; }));
    renderInputArea({ onSend });
    const input = screen.getByRole("textbox", { name: "Message prompt" }) as HTMLTextAreaElement;
    fireEvent.change(input, { target: { value: "first prompt" } });
    fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
    expect(screen.getByRole("button", { name: "Send prompt" }).getAttribute("aria-busy")).toBe("true");
    fireEvent.change(input, { target: { value: "my next prompt" } });
    await act(async () => finish());
    expect(input.value).toBe("my next prompt");
    expect(screen.getByRole("button", { name: "Send prompt" }).getAttribute("aria-busy")).toBe("false");
  });

  it("resizes a restored multiline draft to its content", () => {
    const { container } = renderInputArea();
    const input = screen.getByRole("textbox", { name: "Message prompt" }) as HTMLTextAreaElement;
    Object.defineProperty(input, "scrollHeight", { get: () => input.value.split("\n").length * 20 });
    const source = useSessionStore.getState().sessionId;
    fireEvent.change(input, { target: { value: "one\ntwo\nthree" } });
    act(() => useSessionStore.setState({ sessionId: "resize-other" }));
    fireEvent.change(input, { target: { value: "short" } });
    act(() => useSessionStore.setState({ sessionId: source }));
    expect((container.querySelector("textarea") as HTMLTextAreaElement).style.height).toBe("60px");
  });

  it("does not show a previous chat's send error after navigation", async () => {
    let fail!: (reason: Error) => void;
    renderInputArea({ onSend: vi.fn(() => new Promise<void>((_, reject) => { fail = reject; })) });
    fireEvent.change(screen.getByRole("textbox", { name: "Message prompt" }), { target: { value: "send here" } });
    fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
    act(() => useSessionStore.setState({ sessionId: "other-send-chat" }));
    await act(async () => fail(new Error("old chat failed")));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows store personas in @ menu", () => {
    useSessionStore.setState({ availablePersonas: mockPersonas, currentPersona: null });
    const { container } = renderInputArea();
    const textarea = container.querySelector("textarea")!;
    fireEvent.change(textarea, { target: { value: "@" } });
    expect(screen.getByText("code-reviewer")).toBeTruthy();
    expect(screen.getByText("explainer")).toBeTruthy();
  });

  it("calls switchPersona when selecting a persona via @ menu", () => {
    useSessionStore.setState({ availablePersonas: mockPersonas, currentPersona: null, streamingPartId: null });
    const { container } = renderInputArea();
    const textarea = container.querySelector("textarea")!;
    fireEvent.change(textarea, { target: { value: "@" } });
    // The @ menu renders MenuRow buttons that fire onMouseDown
    // Find the button whose primary text is exactly "code-reviewer"
    const allButtons = container.querySelectorAll("button");
    const reviewerBtn = Array.from(allButtons).find(
      (b) => b.textContent?.trim() === "code-reviewer",
    );
    expect(reviewerBtn).toBeTruthy();
    fireEvent.mouseDown(reviewerBtn!);
    expect(switchPersonaMock).toHaveBeenCalledWith("code-reviewer");
  });

  it("does NOT call switchPersona when 'default' is selected via @ menu", () => {
    useSessionStore.setState({ availablePersonas: mockPersonas, currentPersona: null });
    const { container } = renderInputArea();
    const textarea = container.querySelector("textarea")!;
    fireEvent.change(textarea, { target: { value: "@" } });
    // Click "default" in the persona menu
    const defaultOption = screen.getAllByText("default")[0]!;
    fireEvent.click(defaultOption);
    expect(switchPersonaMock).not.toHaveBeenCalled();
  });

  it("passes attachments to onSend with data URLs", async () => {
    const { container, onSend } = renderInputArea();
    const textarea = container.querySelector("textarea")!;

    // Create a mock File
    const file = new File(["test image data"], "screenshot.png", { type: "image/png" });

    // Set text
    fireEvent.change(textarea, { target: { value: "describe this" } });

    // Use the hidden file input to add files
    const fileInput = container.querySelector('input[type="file"]') as HTMLInputElement;
    Object.defineProperty(fileInput, "files", { value: [file] });
    fireEvent.change(fileInput);

    // Submit
    const sendBtn = container.querySelector('button[title="Send"]')!;
    fireEvent.click(sendBtn);

    await waitFor(() => {
      expect(onSend).toHaveBeenCalledTimes(1);
    });

    const callArgs = onSend.mock.calls[0]!;
    expect(callArgs[0]).toBe("describe this");
    const attachments = callArgs[1];
    expect(attachments).toBeDefined();
    expect(attachments!.length).toBe(1);
    expect(attachments![0]!.path).toMatch(/^data:image\/png;base64,/);
    expect(attachments![0]!.mime).toBe("image/png");
  });

  it("preserves the source draft if navigation happens while an attachment is read", async () => {
    useSessionStore.setState({ sessionId: "reading-source" });
    const { container, onSend } = renderInputArea();
    fireEvent.change(container.querySelector("textarea")!, { target: { value: "source prompt" } });
    fireEvent.change(container.querySelector('input[type="file"]')!, { target: { files: [new File(["data"], "reading.txt", { type: "text/plain" })] } });
    fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
    act(() => useSessionStore.setState({ sessionId: "reading-destination" }));
    fireEvent.change(container.querySelector("textarea")!, { target: { value: "destination draft" } });
    await waitFor(() => expect((screen.getByRole("button", { name: "Send prompt" }) as HTMLButtonElement).disabled).toBe(false));
    expect(onSend).not.toHaveBeenCalled();
    act(() => useSessionStore.setState({ sessionId: "reading-source" }));
    expect((container.querySelector("textarea") as HTMLTextAreaElement).value).toBe("source prompt");
    expect(screen.getByText("reading.txt")).toBeTruthy();
  });

  it("prevents double-submit while sending", async () => {
    const { container, onSend } = renderInputArea();
    const textarea = container.querySelector("textarea")!;

    const file = new File(["data"], "test.png", { type: "image/png" });

    fireEvent.change(textarea, { target: { value: "hello" } });
    const fileInput = container.querySelector('input[type="file"]') as HTMLInputElement;
    Object.defineProperty(fileInput, "files", { value: [file] });
    fireEvent.change(fileInput);

    const sendBtn = container.querySelector('button[title="Send"]')!;
    // Click twice rapidly
    fireEvent.click(sendBtn);
    fireEvent.click(sendBtn);

    await waitFor(() => {
      expect(onSend).toHaveBeenCalledTimes(1);
    });
  });

  it("adds pasted files to the files state", () => {
    const { container } = renderInputArea();
    const textarea = container.querySelector("textarea")!;

    const mockFile = new File(["image data"], "pasted.png", { type: "image/png" });
    const mockClipboardData = {
      items: [
        { kind: "file", getAsFile: () => mockFile },
      ],
    };

    fireEvent.paste(textarea, {
      clipboardData: mockClipboardData as unknown as DataTransfer,
    });

    // The file chip should appear
    expect(screen.getByText("pasted.png")).toBeTruthy();
  });

  it("adds dropped files to the files state", () => {
    const { container } = renderInputArea();
    const composerDiv = container.querySelector("div.flex.items-end")!;

    const mockFile = new File(["image data"], "dropped.png", { type: "image/png" });
    const mockDataTransfer = {
      files: [mockFile],
    };

    fireEvent.drop(composerDiv, {
      dataTransfer: mockDataTransfer as unknown as DataTransfer,
    });

    expect(screen.getByText("dropped.png")).toBeTruthy();
  });
});
