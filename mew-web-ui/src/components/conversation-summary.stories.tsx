import { useLayoutEffect } from "react";
import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import { ConversationSummary } from "./conversation-summary";
import { useSessionStore, type SubagentInfo, type TodoItem } from "../stores/session";

const parameters: StoryLiteParameters = {
  renderer: "react",
};

const SAMPLE_TODOS: TodoItem[] = [
  { id: 1, content: "Add optional description to opaque tool schemas", status: "done", dependsOn: [] },
  { id: 2, content: "Keep running tool cards collapsed by default", status: "in_progress", dependsOn: [1] },
  { id: 3, content: "Add StoryLite stories for core components", status: "pending", dependsOn: [2] },
];

const SAMPLE_SUBAGENTS = new Map<string, SubagentInfo>([
  [
    "sub-1",
    {
      parentCallId: "call-sub-1",
      name: "research",
      childSessionId: "child-1",
      displayName: "Inspect web UI stories",
      status: "running",
      lastProgress: "Scanning mew-web-ui/src/components",
      outcome: null,
      manifests: [],
    },
  ],
]);

function ConversationSummaryPreview() {
  useLayoutEffect(() => {
    useSessionStore.setState({
      sessionCwd: "/Users/natalie/code/mew",
      todos: SAMPLE_TODOS,
      subagents: SAMPLE_SUBAGENTS,
    });
    return () => {
      useSessionStore.setState({
        todos: [],
        subagents: new Map(),
      });
    };
  }, []);

  return (
    <div className="flex items-center gap-3 p-6">
      <ConversationSummary onOpenSurface={() => {}} />
      <span className="text-xs text-muted-foreground">Click icon to open summary popover</span>
    </div>
  );
}

export default {
  title: "Chat/ConversationSummary",
  parameters,
} satisfies StoryLiteMeta;

export const ActiveSession: StoryLiteStoryDefinition = {
  render: () => <ConversationSummaryPreview />,
};
