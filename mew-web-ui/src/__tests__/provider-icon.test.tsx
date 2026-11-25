import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
import { ProviderIcon } from "../components/provider-icon";

afterEach(cleanup);

const COLOR_AND_MONO_PROVIDERS = [
  "antigravity",
  "codex",
  "deepseek",
  "kimi-for-coding",
  "alibaba-token-plan",
  "alibaba-token-plan-cn",
];

const MONO_ONLY_PROVIDERS = ["opencode-zen", "opencode-go", "z-ai"];

describe("ProviderIcon", () => {
  it.each([...COLOR_AND_MONO_PROVIDERS, ...MONO_ONLY_PROVIDERS])(
    "renders monochrome icon for %s",
    (provider) => {
      const { container } = render(<ProviderIcon provider={provider} variant="mono" />);
      const el = container.querySelector(`[data-provider-icon="${provider}"]`);
      expect(el).toBeTruthy();
      expect(el?.tagName).toBe("SPAN");
      expect(el?.getAttribute("data-variant")).toBe("mono");
      expect((el as HTMLElement).style.maskImage).toContain("svg");
    },
  );

  it.each(COLOR_AND_MONO_PROVIDERS)("renders color image for %s", (provider) => {
    const { container } = render(<ProviderIcon provider={provider} variant="color" />);
    const el = container.querySelector(`[data-provider-icon="${provider}"]`);
    expect(el).toBeTruthy();
    expect(el?.tagName).toBe("IMG");
    expect(el?.getAttribute("data-variant")).toBe("color");
    expect(el?.getAttribute("src")).toBeTruthy();
  });

  it.each(MONO_ONLY_PROVIDERS)(
    "renders monochrome icon cleanly when color is requested for %s",
    (provider) => {
      const { container } = render(<ProviderIcon provider={provider} variant="color" />);
      const el = container.querySelector(`[data-provider-icon="${provider}"]`);
      expect(el).toBeTruthy();
      expect(el?.tagName).toBe("SPAN");
      expect(el?.getAttribute("data-variant")).toBe("mono");
    },
  );

  it.each(["umans", "custom-provider"])("renders fallback icon for %s", (provider) => {
    const { container } = render(<ProviderIcon provider={provider} variant="color" />);
    const el = container.querySelector(`[data-provider-icon="${provider}"]`);
    expect(el).toBeTruthy();
    expect(el?.tagName.toLowerCase()).toBe("svg");
    expect(el?.getAttribute("data-variant")).toBe("fallback");
  });
});
