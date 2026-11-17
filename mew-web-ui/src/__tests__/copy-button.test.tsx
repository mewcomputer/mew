import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CopyButton } from "../components/copy-button";

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe("copy feedback", () => {
  it("confirms a successful copy accessibly", async () => {
    const user = userEvent.setup();
    vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue(undefined);
    render(<CopyButton text="hello" />);
    await user.click(screen.getByRole("button", { name: "Copy" }));
    expect(screen.getByRole("button", { name: "Copied" })).toBeTruthy();
  });
  it("reports clipboard failure and allows retry", async () => {
    const user = userEvent.setup();
    vi.spyOn(navigator.clipboard, "writeText").mockRejectedValueOnce(new Error("Clipboard blocked")).mockResolvedValue(undefined);
    render(<CopyButton text="hello" />);
    await user.click(screen.getByRole("button", { name: "Copy" }));
    expect(screen.getByRole("alert").textContent).toContain("Could not copy");
    await user.click(screen.getByRole("button", { name: "Copy" }));
    expect(screen.getByRole("button", { name: "Copied" })).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
