import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { CodeBlock } from "../components/code-block";

const { pending } = vi.hoisted(() => ({ pending: new Map<string, (html: string) => void>() }));
vi.mock("../lib/shiki", () => ({
  highlightCode: (code: string) => new Promise<string>((resolve) => pending.set(code, resolve)),
}));

afterEach(() => { cleanup(); pending.clear(); });

it("shows the current source while its highlighting is pending", async () => {
  const view = render(<CodeBlock code="old content" lang="text" />);
  await act(async () => pending.get("old content")!("<pre>old content</pre>"));
  view.rerender(<CodeBlock code="new content" lang="text" />);
  expect(screen.getByText("new content")).toBeTruthy();
  expect(screen.queryByText("old content")).toBeNull();
  await act(async () => pending.get("new content")!("<pre>new content</pre>"));
  expect(screen.getByText("new content")).toBeTruthy();
});
