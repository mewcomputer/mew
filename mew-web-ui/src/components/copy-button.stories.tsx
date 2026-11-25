import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import { CopyButton } from "./copy-button";

const parameters: StoryLiteParameters = {
  renderer: "react",
};

export default {
  title: "UI/CopyButton",
  parameters,
} satisfies StoryLiteMeta;

export const IconOnly: StoryLiteStoryDefinition = {
  render: () => (
    <div className="p-4">
      <CopyButton text="cargo test -p mew-tools --locked" className="opacity-100" />
    </div>
  ),
};

export const WithLabel: StoryLiteStoryDefinition = {
  render: () => (
    <div className="p-4">
      <CopyButton text="pnpm --filter mew-web-ui stories" label="Copy" className="opacity-100" />
    </div>
  ),
};
