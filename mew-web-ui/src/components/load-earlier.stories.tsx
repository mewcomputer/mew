import type {
  StoryLiteMeta,
  StoryLiteParameters,
  StoryLiteStoryDefinition,
} from "@storylite/storylite";
import { LoadEarlier } from "./load-earlier";

const parameters: StoryLiteParameters = {
  renderer: "react",
};

export default {
  title: "Chat/LoadEarlier",
  parameters,
} satisfies StoryLiteMeta;

export const Idle: StoryLiteStoryDefinition = {
  render: () => (
    <div className="relative h-28 max-w-2xl rounded-lg border border-border/50 bg-background">
      <LoadEarlier loading={false} onClick={() => {}} />
    </div>
  ),
};

export const LoadingSkeleton: StoryLiteStoryDefinition = {
  render: () => (
    <div className="relative h-32 max-w-2xl rounded-lg border border-border/50 bg-background">
      <LoadEarlier loading onClick={() => {}} />
    </div>
  ),
};
