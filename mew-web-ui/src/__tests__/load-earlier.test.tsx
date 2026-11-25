import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LoadEarlier } from "../components/load-earlier";

afterEach(() => cleanup());

describe("LoadEarlier", () => {
  it("offers the older page and reports the click", async () => {
    const onClick = vi.fn();
    render(<LoadEarlier loading={false} onClick={onClick} />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Load earlier messages" }));
    expect(onClick).toHaveBeenCalledTimes(1);
    expect(screen.queryByText(/Loading earlier/)).toBeNull();
  });

  it("swaps the button for a skeleton card while the page is in flight", () => {
    const { container } = render(<LoadEarlier loading onClick={() => {}} />);
    expect(screen.getByText(/Loading earlier messages/)).toBeTruthy();
    expect(screen.queryByRole("button")).toBeNull();
    // Placeholder bars stand in for the rows that are about to arrive.
    expect(container.querySelectorAll(".mew-skeleton")).toHaveLength(3);
    expect(screen.getByText(/Loading earlier messages/).closest("[aria-live]")).toBeTruthy();
  });
});
