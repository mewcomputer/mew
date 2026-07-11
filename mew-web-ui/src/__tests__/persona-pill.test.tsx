import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent } from "@testing-library/react";
import { PersonaPill } from "../components/persona-pill";
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

const mockPersonas: PersonaInfo[] = [
  { name: "code-reviewer", description: "Reviews code", active: false },
  { name: "explainer", description: "Explains things", color: "#ff0000", active: false },
];

/** Render PersonaPill and return helpers for interacting with it. */
function renderPill() {
  const { container } = render(<PersonaPill />);
  const getPillButton = () => container.querySelector('button[title="Persona"]') as HTMLButtonElement;
  const openDropdown = () => fireEvent.click(getPillButton());
  const getDropdown = () => container.querySelector("div.absolute") as HTMLElement;
  const getOption = (name: string) =>
    Array.from(getDropdown().querySelectorAll("button")).find(
      (b) => b.textContent?.includes(name),
    ) as HTMLButtonElement;
  return { container, getPillButton, openDropdown, getDropdown, getOption };
}

describe("PersonaPill", () => {
  beforeEach(() => {
    useSessionStore.setState({
      availablePersonas: [],
      currentPersona: null,
    });
    switchPersonaMock.mockClear();
  });

  it("renders 'default' when no persona is selected", () => {
    const { getPillButton } = renderPill();
    expect(getPillButton().textContent).toContain("default");
  });

  it("shows store personas in dropdown when opened", () => {
    useSessionStore.setState({ availablePersonas: mockPersonas, currentPersona: null });
    const { openDropdown, getOption } = renderPill();
    openDropdown();
    expect(getOption("code-reviewer")).toBeTruthy();
    expect(getOption("explainer")).toBeTruthy();
  });

  it("falls back to just 'default' when availablePersonas is empty", () => {
    const { openDropdown, getDropdown } = renderPill();
    openDropdown();
    const optionButtons = getDropdown().querySelectorAll("button");
    expect(optionButtons.length).toBe(1);
    expect(optionButtons[0]!.textContent).toContain("default");
  });

  it("calls switchPersona when a non-default persona is selected", () => {
    useSessionStore.setState({ availablePersonas: mockPersonas, currentPersona: null });
    const { openDropdown, getOption } = renderPill();
    openDropdown();
    fireEvent.click(getOption("code-reviewer"));
    expect(switchPersonaMock).toHaveBeenCalledWith("code-reviewer");
  });

  it("does NOT call switchPersona when 'default' is selected", () => {
    useSessionStore.setState({ availablePersonas: mockPersonas, currentPersona: "code-reviewer" });
    const { getPillButton, openDropdown, getOption } = renderPill();
    expect(getPillButton().textContent).toContain("code-reviewer");
    openDropdown();
    fireEvent.click(getOption("default"));
    expect(switchPersonaMock).not.toHaveBeenCalled();
  });

  it("shows description as tooltip", () => {
    useSessionStore.setState({ availablePersonas: mockPersonas, currentPersona: null });
    const { openDropdown, getOption } = renderPill();
    openDropdown();
    expect(getOption("code-reviewer").getAttribute("title")).toBe("Reviews code");
  });

  it("renders color dot for personas with color", () => {
    useSessionStore.setState({ availablePersonas: mockPersonas, currentPersona: null });
    const { openDropdown, getOption } = renderPill();
    openDropdown();
    const explainerBtn = getOption("explainer");
    const dot = explainerBtn.querySelector("span[style]");
    expect(dot).toBeTruthy();
    expect(dot?.getAttribute("style")).toContain("background-color");
  });
});
