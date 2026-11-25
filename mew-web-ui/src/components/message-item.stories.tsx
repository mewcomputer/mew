import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import type { ChatMessage } from "../stores/session";
import { MessageItem } from "./message-item";

const parameters: StoryLiteParameters = {
  renderer: "react",
};

const TINY_PNG =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAFklEQVQYV2NkYGD4z4AEGJG5jAwMDAwAHs0B/W3V4jMAAAAASUVORK5CYII=";

function renderMessage(message: ChatMessage) {
  return (
    <div className="max-w-2xl p-4 typeset">
      <MessageItem message={message} />
    </div>
  );
}

export default {
  title: "Chat/MessageItem",
  parameters,
} satisfies StoryLiteMeta;

export const UserMessage: StoryLiteStoryDefinition = {
  render: () =>
    renderMessage({
      id: "msg-user",
      role: "user",
      timestamp: 1000,
      parts: [
        {
          type: "text",
          text: "Can we make running tool calls stay closed by default and show an optional description?",
        },
      ],
    }),
};

export const AssistantWithReasoningAndTool: StoryLiteStoryDefinition = {
  render: () =>
    renderMessage({
      id: "msg-assistant",
      role: "assistant",
      timestamp: 2000,
      completedAt: 5000,
      parts: [
        {
          type: "reasoning",
          text: "Inspect ToolCallCard and the opaque tool schemas in mew-tools and mew-agent.",
        },
        {
          type: "tool-call",
          callId: "call-1",
          toolName: "bash",
          input: {
            command: "cargo test -p mew-tools --locked",
            description: "Run mew-tools unit tests",
          },
          state: "completed",
          time: { start: 2100, end: 3600 },
        },
        {
          type: "text",
          text: "Added optional `description` support and collapsed running tool cards by default.",
        },
      ],
    }),
};

export const WithFileAttachments: StoryLiteStoryDefinition = {
  render: () =>
    renderMessage({
      id: "msg-files",
      role: "user",
      timestamp: 3000,
      parts: [
        { type: "file", url: TINY_PNG, filename: "screenshot.png", mime: "image/png" },
        { type: "file", url: "file:///tmp/spec.pdf", filename: "design-spec.pdf", mime: "application/pdf" },
        { type: "text", text: "Check the spacing in this screenshot against the spec." },
      ],
    }),
};
