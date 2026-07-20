import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
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
  beforeEach(() => {
    useSessionStore.setState({
      availablePersonas: [],
      currentPersona: null,
      streamingPartId: null,
      promptHistory: [],
    });
    switchPersonaMock.mockClear();
  });

  it("keeps composer controls inside the primary composer surface", () => {
    renderInputArea();

    const surface = screen.getByTestId("composer-surface");
    expect(surface.contains(screen.getByTitle("Persona"))).toBe(true);
    expect(surface.contains(screen.getByTitle("Switch model"))).toBe(true);
    expect(surface.contains(screen.getByTitle("Send"))).toBe(true);
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
