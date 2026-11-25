import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import { CodeBlock } from "./code-block";

const parameters: StoryLiteParameters = {
  renderer: "react",
};

const RUST_SNIPPET = `fn describe_tool(name: &str, reason: Option<&str>) -> String {
    match reason {
        Some(text) if !text.trim().is_empty() => text.to_string(),
        _ => format!("Running {name}"),
    }
}`;

const TS_SNIPPET = `export function formatDuration(ms: number): string {
  if (ms < 1000) return \`\${ms}ms\`;
  return \`\${(ms / 1000).toFixed(1)}s\`;
}`;

const PYTHON_SNIPPET = `schema = tools.read({"path": "crates/mew-tools/src/tools/bash.rs"})
text(f"Loaded {len(schema)} bytes")`;

export default {
  title: "Chat/CodeBlock",
  parameters,
} satisfies StoryLiteMeta;

export const RustWithHeader: StoryLiteStoryDefinition = {
  render: () => (
    <div className="max-w-2xl p-4">
      <CodeBlock code={RUST_SNIPPET} lang="rust" showHeader />
    </div>
  ),
};

export const TypeScriptWithLineNumbers: StoryLiteStoryDefinition = {
  render: () => (
    <div className="max-w-2xl p-4">
      <CodeBlock code={TS_SNIPPET} lang="typescript" showHeader lineNumbers />
    </div>
  ),
};

export const FlushPython: StoryLiteStoryDefinition = {
  render: () => (
    <div className="max-w-2xl p-4">
      <CodeBlock code={PYTHON_SNIPPET} lang="python" showHeader={false} flush />
    </div>
  ),
};
