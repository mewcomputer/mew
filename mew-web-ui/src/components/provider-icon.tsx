import type { CSSProperties } from "react";
import { Cpu } from "lucide-react";
import { cn } from "../lib/utils";
import alibabaMono from "../assets/providers/alibabacloud.svg";
import alibabaColor from "../assets/providers/alibabacloud-color.svg";
import antigravityMono from "../assets/providers/antigravity.svg";
import antigravityColor from "../assets/providers/antigravity-color.svg";
import codexMono from "../assets/providers/codex.svg";
import codexColor from "../assets/providers/codex-color.svg";
import deepseekMono from "../assets/providers/deepseek.svg";
import deepseekColor from "../assets/providers/deepseek-color.svg";
import kimiMono from "../assets/providers/kimi.svg";
import kimiColor from "../assets/providers/kimi-color.svg";
import opencodeMono from "../assets/providers/opencode.svg";
import zaiMono from "../assets/providers/zai.svg";

interface ProviderAsset {
  mono: string;
  color?: string;
}

const PROVIDER_ICONS: Record<string, ProviderAsset> = {
  antigravity: { mono: antigravityMono, color: antigravityColor },
  codex: { mono: codexMono, color: codexColor },
  openai: { mono: codexMono, color: codexColor },
  deepseek: { mono: deepseekMono, color: deepseekColor },
  "kimi-for-coding": { mono: kimiMono, color: kimiColor },
  kimi: { mono: kimiMono, color: kimiColor },
  "alibaba-token-plan": { mono: alibabaMono, color: alibabaColor },
  "alibaba-token-plan-cn": { mono: alibabaMono, color: alibabaColor },
  "opencode-zen": { mono: opencodeMono },
  "opencode-go": { mono: opencodeMono },
  "z-ai": { mono: zaiMono },
  zai: { mono: zaiMono },
};

export interface ProviderIconProps {
  provider: string;
  variant?: "mono" | "color";
  className?: string;
}

export function ProviderIcon({ provider, variant = "mono", className }: ProviderIconProps) {
  const asset = PROVIDER_ICONS[provider];
  if (!asset) {
    return (
      <Cpu
        aria-hidden="true"
        data-provider-icon={provider}
        data-variant="fallback"
        className={cn("shrink-0", className)}
      />
    );
  }
  if (variant === "color" && asset.color) {
    return (
      <img
        src={asset.color}
        alt=""
        aria-hidden="true"
        data-provider-icon={provider}
        data-variant="color"
        className={cn("shrink-0 object-contain", className)}
      />
    );
  }
  const maskUrl = `url(${JSON.stringify(asset.mono)})`;
  const style: CSSProperties = {
    maskImage: maskUrl,
    WebkitMaskImage: maskUrl,
    maskRepeat: "no-repeat",
    WebkitMaskRepeat: "no-repeat",
    maskPosition: "center",
    WebkitMaskPosition: "center",
    maskSize: "contain",
    WebkitMaskSize: "contain",
  };
  return (
    <span
      aria-hidden="true"
      data-provider-icon={provider}
      data-variant="mono"
      style={style}
      className={cn("inline-block shrink-0 bg-current", className)}
    />
  );
}
