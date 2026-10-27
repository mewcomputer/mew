import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { SidebarMenuButton, SidebarMenuSubButton, SidebarProvider } from "../components/ui/sidebar";

afterEach(cleanup);

describe("sidebar selection", () => {
  it("marks only the selected chat active and clears that state when selection changes", () => {
    const { rerender } = render(<SidebarProvider><SidebarMenuButton isActive>First chat</SidebarMenuButton><SidebarMenuButton>Second chat</SidebarMenuButton></SidebarProvider>);
    expect(screen.getByRole("button", { name: "First chat" }).hasAttribute("data-active")).toBe(true);
    expect(screen.getByRole("button", { name: "Second chat" }).hasAttribute("data-active")).toBe(false);
    rerender(<SidebarProvider><SidebarMenuButton>First chat</SidebarMenuButton><SidebarMenuButton isActive>Second chat</SidebarMenuButton></SidebarProvider>);
    expect(screen.getByRole("button", { name: "First chat" }).hasAttribute("data-active")).toBe(false);
    expect(screen.getByRole("button", { name: "Second chat" }).hasAttribute("data-active")).toBe(true);
  });

  it("marks only the selected nested chat active and clears that state when selection changes", () => {
    const { rerender } = render(<SidebarProvider><SidebarMenuSubButton href="/first" isActive>First chat</SidebarMenuSubButton><SidebarMenuSubButton href="/second">Second chat</SidebarMenuSubButton></SidebarProvider>);
    expect(screen.getByRole("link", { name: "First chat" }).hasAttribute("data-active")).toBe(true);
    expect(screen.getByRole("link", { name: "Second chat" }).hasAttribute("data-active")).toBe(false);
    rerender(<SidebarProvider><SidebarMenuSubButton href="/first">First chat</SidebarMenuSubButton><SidebarMenuSubButton href="/second" isActive>Second chat</SidebarMenuSubButton></SidebarProvider>);
    expect(screen.getByRole("link", { name: "First chat" }).hasAttribute("data-active")).toBe(false);
    expect(screen.getByRole("link", { name: "Second chat" }).hasAttribute("data-active")).toBe(true);
  });
});
