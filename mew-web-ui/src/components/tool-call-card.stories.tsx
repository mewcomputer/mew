import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import type { MessagePart } from "../stores/session";
import { ToolCallCard } from "./tool-call-card";

type ToolPart = Extract<MessagePart, { type: "tool-call" }>;

const parameters: StoryLiteParameters = {
  renderer: "react",
};

function makePart(overrides: Partial<ToolPart> = {}): ToolPart {
  return {
    type: "tool-call",
    callId: "call-1",
    toolName: "bash",
    input: { command: "cargo test -p mew-tools" },
    state: "completed",
    time: { start: 1000, end: 2450 },
    ...overrides,
  };
}

function renderCard(part: ToolPart) {
  return (
    <div className="max-w-2xl p-4">
      <ToolCallCard part={part} />
    </div>
  );
}

export default {
  title: "Chat/ToolCallCard",
  parameters,
} satisfies StoryLiteMeta;

export const CompletedWithDescription: StoryLiteStoryDefinition = {
  render: () =>
    renderCard(
      makePart({
        input: {
          command: "cargo test -p mew-tools --locked",
          description: "Run mew-tools unit tests",
        },
        output: "test result: ok. 134 passed; 0 failed; 1 ignored",
      }),
    ),
};

export const RunningCollapsed: StoryLiteStoryDefinition = {
  render: () =>
    renderCard(
      makePart({
        callId: "call-running",
        state: "running",
        time: { start: Date.now() - 3200, end: null },
        input: {
          command: "pnpm --filter mew-web-ui test",
          description: "Run web UI test suite",
        },
        output: "Running 45 test files...\n✓ tool-call-card.test.tsx (19 tests)",
      }),
    ),
};

export const FailedExpanded: StoryLiteStoryDefinition = {
  render: () =>
    renderCard(
      makePart({
        callId: "call-failed",
        state: "error",
        input: {
          command: "cargo check --workspace",
          description: "Typecheck Rust workspace",
        },
        output: "error[E0308]: mismatched types\n  --> crates/mew-tools/src/tools/bash.rs:42:5",
      }),
    ),
};

export const EditWithDiff: StoryLiteStoryDefinition = {
  render: () =>
    renderCard(
      makePart({
        callId: "call-edit",
        toolName: "edit",
        input: {
          path: "src/components/tool-call-card.tsx",
          old_string: "const showLiveDetail = (state === \"running\" || state === \"error\");",
          new_string: "const showLiveDetail = state === \"error\";\nconst callDescription = stringField(part.input, \"description\");",
        },
        time: { start: 1000, end: 1120 },
      }),
    ),
};

export const PythonWithNestedCalls: StoryLiteStoryDefinition = {
  render: () =>
    renderCard(
      makePart({
        callId: "call-python",
        toolName: "python",
        input: {
          description: "Inspect tool schemas across crates",
          source: "schema = tools.read({'path': 'crates/mew-tools/src/tools/bash.rs'})\ntext(schema[:120])",
        },
        nestedCalls: [
          {
            parent_call_id: "call-python",
            call_id: "nested-1",
            tool_name: "read",
            input: { path: "crates/mew-tools/src/tools/bash.rs" },
            status: "completed",
          },
          {
            parent_call_id: "call-python",
            call_id: "nested-2",
            tool_name: "grep",
            input: { pattern: "fn schema" },
            status: "completed",
          },
        ],
        time: { start: 1000, end: 1850 },
      }),
    ),
};
