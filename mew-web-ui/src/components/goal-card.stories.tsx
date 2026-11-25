import { useLayoutEffect } from "react";
import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import { GoalCard } from "./goal-card";
import { useSessionStore, type PendingGoal } from "../stores/session";

const parameters: StoryLiteParameters = {
  renderer: "react",
};

function GoalCardPreview({ goals }: { goals: PendingGoal[] }) {
  useLayoutEffect(() => {
    useSessionStore.setState({ pendingGoals: goals });
    return () => {
      useSessionStore.setState({ pendingGoals: [] });
    };
  }, [goals]);

  return (
    <div className="max-w-xl p-4">
      <GoalCard />
    </div>
  );
}

export default {
  title: "Chat/GoalCard",
  parameters,
} satisfies StoryLiteMeta;

export const ProposedGoals: StoryLiteStoryDefinition = {
  render: () => (
    <GoalCardPreview
      goals={[
        {
          requestId: "goal-1",
          callId: "call-goal-1",
          objective: "Add optional description support to opaque tools and keep running tool cards collapsed.",
        },
        {
          requestId: "goal-2",
          callId: "call-goal-2",
          objective: "Wire up StoryLite component stories for core chat, prompt, and connection UI.",
        },
      ]}
    />
  ),
};
