import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import { JumpToLatest } from "./scroll-to-latest";

const parameters: StoryLiteParameters = {
  renderer: "react",
};

export default {
  title: "Chat/JumpToLatest",
  parameters,
} satisfies StoryLiteMeta;

export const Default: StoryLiteStoryDefinition = {
  render: () => (
    <div className="relative h-28 max-w-2xl rounded-lg border border-border/50 bg-background">
      <JumpToLatest onClick={() => {}} />
    </div>
  ),
};
