import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import type { Turn } from "../lib/turns";
import type { ChatMessage } from "../stores/session";
import { TurnItem } from "./turn-item";

const parameters: StoryLiteParameters = {
  renderer: "react",
};

function userMessage(text: string, timestamp = 0): ChatMessage {
  return {
    id: `user-${timestamp}`,
    role: "user",
    timestamp,
    parts: [{ type: "text", text }],
  };
}

function renderTurn(turn: Turn) {
  return (
    <div className="max-w-2xl p-4">
      <TurnItem turn={turn} />
    </div>
  );
}

export default {
  title: "Chat/TurnItem",
  parameters,
} satisfies StoryLiteMeta;

export const CompletedTurn: StoryLiteStoryDefinition = {
  render: () =>
    renderTurn({
      key: "turn-completed",
      messageIndex: 0,
      messages: [
        userMessage("can we keep running tool cards collapsed by default?", 0),
        {
          id: "assistant-work",
          role: "assistant",
          timestamp: 1000,
          completedAt: 14000,
          parts: [
            { type: "reasoning", text: "Check ToolCallCard live detail expansion condition." },
            {
              type: "tool-call",
              callId: "t1",
              toolName: "read",
              input: { path: "mew-web-ui/src/components/tool-call-card.tsx" },
              state: "completed",
              time: { start: 1200, end: 1280 },
            },
            {
              type: "tool-call",
              callId: "t2",
              toolName: "bash",
              input: {
                command: "pnpm --filter mew-web-ui test -- tool-call-card",
                description: "Verify ToolCallCard component tests",
              },
              state: "completed",
              time: { start: 2000, end: 4200 },
            },
          ],
        },
        {
          id: "assistant-answer",
          role: "assistant",
          timestamp: 14500,
          completedAt: 18000,
          parts: [
            {
              type: "text",
              text: "Done. Running tool calls now stay collapsed by default and display `description` when provided.",
            },
          ],
        },
      ],
    }),
};

export const WorkingTurn: StoryLiteStoryDefinition = {
  render: () =>
    renderTurn({
      key: "turn-working",
      messageIndex: 0,
      messages: [
        userMessage("run the rust and web ui checks", 0),
        {
          id: "assistant-working",
          role: "assistant",
          timestamp: Date.now() - 4500,
          parts: [
            {
              type: "tool-call",
              callId: "t-working",
              toolName: "bash",
              input: {
                command: "cargo test -p mew-tools --locked",
                description: "Run mew-tools unit tests",
              },
              state: "running",
              time: { start: Date.now() - 3000, end: null },
            },
          ],
        },
      ],
    }),
};

export const TurnWithFailedTool: StoryLiteStoryDefinition = {
  render: () =>
    renderTurn({
      key: "turn-failed",
      messageIndex: 0,
      messages: [
        userMessage("check the build", 0),
        {
          id: "assistant-failed-work",
          role: "assistant",
          timestamp: 1000,
          completedAt: 8000,
          parts: [
            {
              type: "tool-call",
              callId: "t-err",
              toolName: "bash",
              input: {
                command: "cargo check --workspace",
                description: "Typecheck Rust workspace",
              },
              state: "error",
              output: "error[E0308]: mismatched types",
              time: { start: 1500, end: 6200 },
            },
          ],
        },
        {
          id: "assistant-failed-reply",
          role: "assistant",
          timestamp: 8200,
          completedAt: 11000,
          parts: [
            {
              type: "text",
              text: "The workspace check hit a type mismatch in `bash.rs`; see the failed step above.",
            },
          ],
        },
      ],
    }),
};

const MULTI_ROUND_MESSAGES: ChatMessage[] = [
  userMessage("we should probably allow absolute paths in the daemon file endpoints.", 0),
  {
    id: "demo-r1",
    role: "assistant",
    timestamp: 1000,
    completedAt: 4000,
    parts: [
      { type: "reasoning", text: "User wants to allow absolute paths instead of requiring relative." },
      {
        type: "text",
        text: "I wouldn't lift that guard blindly: it's the workspace boundary. Let me look at where the daemon rejects absolute paths before deciding.",
      },
      {
        type: "tool-call",
        callId: "demo-c1",
        toolName: "bash",
        input: { command: "grep -rn 'absolute paths not allowed' crates", description: "Find absolute path checks" },
        state: "completed",
        time: { start: 2000, end: 2400 },
      },
      {
        type: "tool-call",
        callId: "demo-c2",
        toolName: "read",
        input: { path: "crates/mew-daemon/src/files.rs" },
        state: "completed",
        time: { start: 2500, end: 2700 },
      },
      { type: "reasoning", text: "resolve_scoped canonicalizes and enforces containment afterwards." },
      {
        type: "text",
        text: "The containment check already runs after canonicalization, so rejecting absolute paths up front is redundant. I'll let absolute paths through that check instead of bypassing it.",
      },
    ],
  },
  {
    id: "demo-r2",
    role: "assistant",
    timestamp: 5000,
    parts: [
      { type: "reasoning", text: "Add unit tests in files.rs before editing." },
      {
        type: "tool-call",
        callId: "demo-c3",
        toolName: "edit",
        input: { path: "crates/mew-daemon/src/files.rs", old_string: "a", new_string: "a\nb" },
        state: "completed",
        time: { start: 5200, end: 5400 },
      },
      {
        type: "tool-call",
        callId: "demo-c4",
        toolName: "bash",
        input: { command: "cargo test -p mew-daemon files", description: "Run daemon file endpoint tests" },
        state: "running",
        time: { start: Date.now() - 4000, end: null },
      },
    ],
  },
];

export const DemoLiveStreaming: StoryLiteStoryDefinition = {
  render: () =>
    renderTurn({
      key: "demo-live",
      messageIndex: 0,
      messages: MULTI_ROUND_MESSAGES,
    }),
};

export const DemoFinalSettled: StoryLiteStoryDefinition = {
  render: () =>
    renderTurn({
      key: "demo-final",
      messageIndex: 0,
      messages: [
        ...MULTI_ROUND_MESSAGES.slice(0, 2),
        {
          ...MULTI_ROUND_MESSAGES[2]!,
          completedAt: 311_000,
          parts: [
            ...MULTI_ROUND_MESSAGES[2]!.parts.slice(0, -1),
            {
              type: "tool-call",
              callId: "demo-c4",
              toolName: "bash",
              input: { command: "cargo test -p mew-daemon files", description: "Run daemon file endpoint tests" },
              state: "completed",
              time: { start: 6000, end: 9500 },
            },
          ],
        },
        {
          id: "demo-r3-final",
          role: "assistant",
          timestamp: 310_000,
          completedAt: 311_000,
          parts: [
            {
              type: "text",
              text: "Absolute paths inside the configured workspace roots now pass `resolve_scoped`, while escaping paths are still rejected after canonicalization.",
            },
          ],
        },
      ],
    }),
};
