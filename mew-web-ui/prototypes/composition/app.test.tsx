import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { CompositionApp, copyBgraFrame } from "./app";

afterEach(cleanup);

describe("composition harness", () => {
  it("retains mounted content and the chat draft while the pane closes and reopens", () => {
    render(<CompositionApp bootstrap={{ engine: "electron-webview", fixtureUrl: "http://localhost/fixture.html", versions: {} }} surface={<input aria-label="Mounted content" defaultValue="retained" />} />);
    const content = screen.getByRole("textbox", { name: "Mounted content" });
    fireEvent.change(screen.getByRole("textbox", { name: "Chat draft" }), { target: { value: "keep this draft" } });
    fireEvent.click(screen.getByRole("button", { name: "Close browser pane" }));
    expect(content.isConnected).toBe(true);
    expect(screen.getByLabelText("Browser pane").hasAttribute("inert")).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Open browser pane" }));
    expect(screen.getByRole("textbox", { name: "Mounted content" })).toBe(content);
    expect((content as HTMLInputElement).value).toBe("retained");
    expect((screen.getByRole("textbox", { name: "Chat draft" }) as HTMLTextAreaElement).value).toBe("keep this draft");
  });

  it("returns focus from the portaled overlay to its trigger on Escape", async () => {
    render(<CompositionApp bootstrap={{ engine: "electron-webview", fixtureUrl: "http://localhost/fixture.html", versions: {} }} surface={<canvas />} />);
    const trigger = screen.getByRole("button", { name: "Overlay note" });
    fireEvent.click(trigger);
    const note = screen.getByRole("textbox", { name: "Overlay note" });
    note.focus();
    fireEvent.keyDown(note, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "Overlay note" })).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });
});

describe("CEF frame conversion", () => {
  it("preserves alpha while converting known BGRA pixels into canvas RGBA", () => {
    const pixels = new Uint8ClampedArray(8);
    copyBgraFrame(new Uint8Array([3, 2, 1, 255, 6, 5, 4, 127]).buffer, 2, 1, pixels);
    expect([...pixels]).toEqual([1, 2, 3, 255, 4, 5, 6, 127]);
  });

  it.each([[0, 1], [-1, 1], [1.5, 1], [4097, 1], [NaN, 1], [2, 2]])("rejects invalid dimensions or truncated frames (%s × %s)", (width, height) => {
    expect(() => copyBgraFrame(new ArrayBuffer(4), width, height, new Uint8ClampedArray(4))).toThrow();
  });
});
