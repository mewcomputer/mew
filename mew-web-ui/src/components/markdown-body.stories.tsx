import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import { MarkdownBody } from "./markdown-body";

const parameters: StoryLiteParameters = {
  renderer: "react",
};

function renderMarkdown(children: string, highlight: boolean) {
  return (
    <div className="max-w-2xl p-4 typeset">
      <MarkdownBody highlight={highlight}>{children}</MarkdownBody>
    </div>
  );
}

const RICH_MARKDOWN = `### Workspace verification

Updated \`ToolCallCard\` so running tool calls remain collapsed while keeping \`error\` details visible.

| Crate | Target | Status |
| --- | --- | --- |
| \`mew-tools\` | Schema tests | Passing |
| \`mew-web-ui\` | StoryLite build | Passing |

\`\`\`rust
pub fn schema(&self) -> &Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| serde_json::json!({ "type": "object" }))
}
\`\`\`
`;

const STREAMING_MARKDOWN = `Checking the failing test in \`tool-call-card.tsx\`...

\`\`\`tsx
export function ToolCallCard({ part }: { part: ToolPart }) {
  const callDescription = stringField(part.input, "description", "reason");`;

export default {
  title: "Chat/MarkdownBody",
  parameters,
} satisfies StoryLiteMeta;

export const RichDocument: StoryLiteStoryDefinition = {
  render: () => renderMarkdown(RICH_MARKDOWN, true),
};

export const StreamingIncomplete: StoryLiteStoryDefinition = {
  render: () => renderMarkdown(STREAMING_MARKDOWN, false),
};
