import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import { AskUserForm } from "./ask-user-card";
import type { PendingAskUser } from "../stores/session";

const parameters: StoryLiteParameters = {
  renderer: "react",
};

function renderAskUser(req: PendingAskUser) {
  return (
    <div className="max-w-xl p-4">
      <AskUserForm req={req} onSubmit={() => {}} />
    </div>
  );
}

export default {
  title: "Chat/AskUserForm",
  parameters,
} satisfies StoryLiteMeta;

export const SingleSelectOptions: StoryLiteStoryDefinition = {
  render: () =>
    renderAskUser({
      requestId: "ask-1",
      callId: "call-ask-1",
      questions: [
        {
          prompt: "Which component workbench should we set up?",
          options: [
            { label: "StoryLite", description: "Lightweight Vite-native story runner" },
            { label: "Storybook", description: "Full addon ecosystem with heavier config" },
          ],
        },
      ],
    }),
};

export const MultipleQuestions: StoryLiteStoryDefinition = {
  render: () =>
    renderAskUser({
      requestId: "ask-2",
      callId: "call-ask-2",
      questions: [
        {
          prompt: "Keep running tool calls closed by default?",
          options: [
            { label: "Yes, keep collapsed", description: "Show errors expanded, fold running output" },
            { label: "No, auto-expand", description: "Stream live tool output inline" },
          ],
        },
        {
          prompt: "Any extra tool parameters to include?",
          options: [],
        },
      ],
    }),
};
