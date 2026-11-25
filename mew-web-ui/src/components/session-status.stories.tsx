import { useLayoutEffect } from "react";
import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import { ContextIndicator } from "./session-status";
import { useSessionStore } from "../stores/session";

const parameters: StoryLiteParameters = {
  renderer: "react",
};

function ContextIndicatorPreview({
  contextTokens,
  contextWindow,
  totalCost,
}: {
  contextTokens: number;
  contextWindow: number;
  totalCost: number;
}) {
  useLayoutEffect(() => {
    useSessionStore.setState({
      connectionState: "connected",
      contextTokens,
      contextTokensKnown: true,
      contextWindow,
      totalCost,
      daemonVersion: "0.2.0",
    });
    return () => {
      useSessionStore.setState({
        contextTokens: 0,
        contextTokensKnown: false,
        totalCost: 0,
      });
    };
  }, [contextTokens, contextWindow, totalCost]);

  return (
    <div className="flex items-center gap-3 p-6">
      <ContextIndicator />
      <span className="text-xs text-muted-foreground">Click ring to inspect session details</span>
    </div>
  );
}

export default {
  title: "Chat/ContextIndicator",
  parameters,
} satisfies StoryLiteMeta;

export const LowUsage: StoryLiteStoryDefinition = {
  render: () => (
    <ContextIndicatorPreview
      contextTokens={44_000}
      contextWindow={200_000}
      totalCost={0.14}
    />
  ),
};

export const HighUsage: StoryLiteStoryDefinition = {
  render: () => (
    <ContextIndicatorPreview
      contextTokens={172_000}
      contextWindow={200_000}
      totalCost={1.82}
    />
  ),
};
